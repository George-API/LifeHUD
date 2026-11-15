//! The mood themes' sticker tiles, rasterized once and kept as bitmaps.
//!
//! Slint's software renderer re-rasterizes an SVG image on every draw. The tiles are large and
//! mask-heavy (about 4 ms each at 150 % scaling), and cards repaint once a second while music
//! plays, so drawing them as SVGs cost a steady slice of the UI thread. Rendered here once, at 2×,
//! they become plain bitmap blits (see `StickerPattern` in ui/stickers.slint).

use std::cell::RefCell;

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Raster scale: crisp up to 200 % display scaling. ui/stickers.slint divides by the same factor.
const SCALE: f32 = 2.0;

fn source(set: i32) -> Option<&'static [u8]> {
    Some(match set {
        1 => include_bytes!("../ui/stickers/tile-hearth.svg"),
        2 => include_bytes!("../ui/stickers/tile-atelier.svg"),
        3 => include_bytes!("../ui/stickers/tile-metro.svg"),
        4 => include_bytes!("../ui/stickers/tile-evergreen.svg"),
        5 => include_bytes!("../ui/stickers/tile-neon.svg"),
        6 => include_bytes!("../ui/stickers/tile-coast.svg"),
        _ => return None,
    })
}

/// The tile for a sticker set (an empty image for none). UI thread only.
///
/// Only the last set asked for is kept: every card of the current theme shares it, and each
/// caller's binding holds its own handle, so Settings' six theme swatches free their tiles
/// (~1.5 MB each) when the window closes instead of staying cached for good.
pub fn tile(set: i32) -> Image {
    thread_local! {
        static LAST: RefCell<Option<(i32, Image)>> = const { RefCell::new(None) };
    }
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        match last.as_ref() {
            Some((s, img)) if *s == set => img.clone(),
            _ => {
                let img = render(set).unwrap_or_default();
                *last = Some((set, img.clone()));
                img
            }
        }
    })
}

fn render(set: i32) -> Option<Image> {
    let tree = resvg::usvg::Tree::from_data(source(set)?, &resvg::usvg::Options::default())
        .inspect_err(|e| crate::storage::log(format!("sticker tile {set}: {e}")))
        .ok()?;
    let size = tree.size();
    let (w, h) = ((size.width() * SCALE).round() as u32, (size.height() * SCALE).round() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pixmap.as_mut());
    // tiny-skia pixels are premultiplied RGBA.
    Some(Image::from_rgba8_premultiplied(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pixmap.data(), w, h)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_set_renders_at_twice_its_size() {
        for set in 1..=6 {
            let size = super::render(set).expect("tile renders").size();
            assert_eq!((size.width, size.height), (720, 544), "set {set}");
        }
        assert!(super::render(0).is_none());
    }
}
