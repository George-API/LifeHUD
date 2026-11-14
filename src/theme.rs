//! Themes are a handful of colours; every other token is derived here so themes stay consistent.
//! Adding a theme is one entry in [`PALETTES`].

use slint::{Color, ModelRc, VecModel};

use crate::settings::Appearance;
use crate::{Theme, ThemeSwatch};

pub const CLASSIC: &str = "classic";

/// Colours are 0xAARRGGBB.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    /// Laid over the acrylic blur; its alpha decides how much of the desktop shows through.
    pub tint: u32,
    /// Soft diagonal light that gives the theme its mood.
    pub ambient: &'static [u32],
    pub card: u32,
    pub card_border: u32,
    pub text: u32,
    /// `None` = the Windows accent colour.
    pub accent: Option<u32>,
    pub on_accent: u32,
    pub accent_play_button: bool,
    /// Decorative sticker art for mood themes (0 = none). See ui/stickers.slint.
    pub stickers: i32,
}

#[allow(clippy::too_many_arguments)] // a compact, table-like palette constructor
const fn p(
    id: &'static str,
    name: &'static str,
    dark: bool,
    tint: u32,
    ambient: &'static [u32],
    card: u32,
    card_border: u32,
    text: u32,
    accent: Option<u32>,
    on_accent: u32,
    accent_play_button: bool,
) -> Palette {
    Palette {
        id,
        name,
        dark,
        tint,
        ambient,
        card,
        card_border,
        text,
        accent,
        on_accent,
        accent_play_button,
        stickers: 0,
    }
}

impl Palette {
    const fn with_stickers(mut self, set: i32) -> Self {
        self.stickers = set;
        self
    }
}

pub const CLASSIC_LIGHT: Palette =
    p(CLASSIC, "Classic", false, 0xA8F4F4F7, &[], 0x9EFFFFFF, 0x10000000, 0xE6000000, None, 0xFFFFFFFF, false);
pub const CLASSIC_DARK: Palette =
    p(CLASSIC, "Classic", true, 0xB41A1A1E, &[], 0x12FFFFFF, 0x14FFFFFF, 0xFFFFFFFF, None, 0xFF000000, false);

pub const PALETTES: &[Palette] = &[
    CLASSIC_DARK,
    p(
        "graphite",
        "Graphite",
        true,
        0xC41C1C1F,
        &[0x1CFFFFFF, 0x00FFFFFF],
        0x14FFFFFF,
        0x1CFFFFFF,
        0xFFF5F5F7,
        Some(0xFFD6D6DB),
        0xFF1C1C1F,
        false,
    ),
    p(
        "midnight",
        "Midnight",
        true,
        0xC80B1220,
        &[0x332F4A80, 0x000B1220],
        0x14A8C0FF,
        0x1FA8C0FF,
        0xFFEEF2FF,
        Some(0xFF8FB3FF),
        0xFF0B1220,
        false,
    ),
    p(
        "starlight",
        "Starlight",
        false,
        0xC2F3E9D8,
        &[0x44FFF4DC, 0x28E2C99A],
        0xA8FFFDF8,
        0x16583F1A,
        0xF02B2118,
        Some(0xFFAD8648),
        0xFFFFFFFF,
        false,
    ),
    p(
        "titanium",
        "Titanium",
        false,
        0xC8D3D1CC,
        &[0x38FFFFFF, 0x20928A7E],
        0x80F7F6F3,
        0x14000000,
        0xEB1F1E1C,
        Some(0xFF857A6C),
        0xFFFFFFFF,
        false,
    ),
    p(
        "rose",
        "Rosé",
        false,
        0xC2F5E0DA,
        &[0x44FFD6CA, 0x24E9AE9E],
        0xA8FFF8F6,
        0x14602A1E,
        0xEB3A2622,
        Some(0xFFC27A68),
        0xFFFFFFFF,
        false,
    ),
    p(
        "hearth",
        "Hearth",
        true,
        0xCC241A14,
        &[0x40FF9A4D, 0x10FF7A3D, 0x00241A14],
        0x18FFE2C4,
        0x22FFD8B0,
        0xFFFFF3E6,
        Some(0xFFFFB067),
        0xFF2A1A0E,
        false,
    )
    .with_stickers(1),
    p(
        "atelier",
        "Atelier",
        false,
        0xB4FBF7F0,
        &[0x40FF6B6B, 0x2848C9B0, 0x38FFD166],
        0xB8FFFFFF,
        0x14000000,
        0xEB1E1B2E,
        Some(0xFFFF5A5F),
        0xFFFFFFFF,
        true,
    )
    .with_stickers(2),
    p(
        "metropolis",
        "Metropolis",
        true,
        0xC8161A21,
        &[0x30F5A623, 0x00161A21, 0x2433A1FD],
        0x12FFFFFF,
        0x1AFFFFFF,
        0xFFE8ECF2,
        Some(0xFFF5A623),
        0xFF161A21,
        false,
    )
    .with_stickers(3),
    p(
        "evergreen",
        "Evergreen",
        true,
        0xC80F1F18,
        &[0x3346A36B, 0x08A3C46B, 0x000F1F18],
        0x16C8F5D8,
        0x20C8F5D8,
        0xFFEAF5EE,
        Some(0xFF7BD389),
        0xFF0B1F12,
        false,
    )
    .with_stickers(4),
    p(
        "neon",
        "Neon",
        true,
        0xD00A0612,
        &[0x48FF2BD6, 0x000A0612, 0x4000F0FF],
        0x14FFFFFF,
        0x48FF2BD6,
        0xFFF4F0FF,
        Some(0xFF00F0FF),
        0xFF0A0612,
        true,
    )
    .with_stickers(5),
    p(
        "nebula",
        "Nebula",
        true,
        0xCC0B0A1F,
        &[0x446A3DE8, 0x24E8508A, 0x000B0A1F],
        0x14C9B8FF,
        0x26C9B8FF,
        0xFFEEEAFF,
        Some(0xFFB79CFF),
        0xFF140B2E,
        true,
    )
    .with_stickers(6),
];

pub fn find(id: &str) -> Option<&'static Palette> {
    PALETTES.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// The concrete palette for a theme id, resolving Classic against light/dark mode.
pub fn resolve(id: &str, appearance: Appearance) -> Palette {
    match find(id) {
        Some(p) if p.id != CLASSIC => *p,
        _ => {
            let dark = match appearance {
                Appearance::Dark => true,
                Appearance::Light => false,
                Appearance::Auto => crate::platform::system_prefers_dark(),
            };
            if dark { CLASSIC_DARK } else { CLASSIC_LIGHT }
        }
    }
}

pub fn argb(v: u32) -> Color {
    Color::from_argb_u8((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn with_alpha(c: Color, a: u8) -> Color {
    Color::from_argb_u8(a, c.red(), c.green(), c.blue())
}

/// The palette's ambient light as three gradient stops (start, middle, end).
pub fn ambient_stops(p: &Palette) -> [Color; 3] {
    let clear = Color::from_argb_u8(0, 0, 0, 0);
    match p.ambient {
        [] => [clear; 3],
        [a] => [argb(*a); 3],
        [a, b] => [argb(*a), mix(argb(*a), argb(*b)), argb(*b)],
        [a, b, c, ..] => [argb(*a), argb(*b), argb(*c)],
    }
}

fn mix(a: Color, b: Color) -> Color {
    let m = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
    Color::from_argb_u8(m(a.alpha(), b.alpha()), m(a.red(), b.red()), m(a.green(), b.green()), m(a.blue(), b.blue()))
}

pub fn accent(p: &Palette) -> Color {
    p.accent.map(argb).unwrap_or_else(|| crate::platform::system_accent(p.dark))
}

/// Writes every design token for `p` into a window's Theme global.
pub fn apply(theme: &Theme, p: &Palette) {
    let d = p.dark;
    let text = argb(p.text);
    let ink = |a: u8| with_alpha(text, a);
    let tint = argb(p.tint);
    let accent = accent(p);

    theme.set_dark(d);
    theme.set_sticker_set(p.stickers);
    let [a, b, c] = ambient_stops(p);
    theme.set_ambient_a(a);
    theme.set_ambient_b(b);
    theme.set_ambient_c(c);
    theme.set_edge(ink(if d { 0x1A } else { 0x14 }));
    theme.set_card(argb(p.card));
    theme.set_card_border(argb(p.card_border));
    theme.set_text(text);
    theme.set_text2(ink(if d { 0xB8 } else { 0xA0 }));
    theme.set_text3(ink(if d { 0x78 } else { 0x6E }));
    theme.set_fill(ink(if d { 0x14 } else { 0x0C }));
    theme.set_fill_hover(ink(if d { 0x22 } else { 0x16 }));
    theme.set_input(ink(if d { 0x10 } else { 0x0A }));
    theme.set_divider(ink(if d { 0x18 } else { 0x12 }));
    theme.set_track(ink(if d { 0x2E } else { 0x1C }));
    theme.set_accent(accent);
    theme.set_on_accent(if p.accent.is_some() {
        argb(p.on_accent)
    } else if d {
        Color::from_rgb_u8(0, 0, 0)
    } else {
        Color::from_rgb_u8(255, 255, 255)
    });
    if p.accent_play_button {
        theme.set_play_bg(accent);
        theme.set_play_fg(argb(p.on_accent | 0xFF00_0000));
    } else {
        theme.set_play_bg(ink(if d { 0xF2 } else { 0xEB }));
        theme.set_play_fg(Color::from_rgb_u8(tint.red(), tint.green(), tint.blue()));
    }
}

/// Swatches for the Appearance gallery.
pub fn swatches(selected: &str, appearance: Appearance) -> ModelRc<ThemeSwatch> {
    let items: Vec<ThemeSwatch> = PALETTES
        .iter()
        .map(|entry| {
            let p = resolve(entry.id, appearance);
            let t = argb(p.tint);
            let [ambient_a, ambient_b, ambient_c] = ambient_stops(&p);
            ThemeSwatch {
                id: p.id.into(),
                name: p.name.into(),
                surface: Color::from_rgb_u8(t.red(), t.green(), t.blue()),
                ambient_a,
                ambient_b,
                ambient_c,
                card: argb(p.card),
                text: argb(p.text),
                accent: accent(&p),
                selected: p.id.eq_ignore_ascii_case(selected),
                sticker: p.stickers,
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// iOS-style temperature colour scale, by °C.
pub fn temperature_color(celsius: f64) -> Color {
    const SCALE: [(f64, (u8, u8, u8)); 6] = [
        (-10.0, (0x5E, 0x7B, 0xFF)),
        (0.0, (0x5A, 0xC8, 0xFA)),
        (10.0, (0x5F, 0xD6, 0x9A)),
        (18.0, (0xFF, 0xD6, 0x0A)),
        (25.0, (0xFF, 0x9F, 0x0A)),
        (33.0, (0xFF, 0x45, 0x3A)),
    ];
    if celsius <= SCALE[0].0 {
        let (r, g, b) = SCALE[0].1;
        return Color::from_rgb_u8(r, g, b);
    }
    for w in SCALE.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if celsius <= t1 {
            let f = (celsius - t0) / (t1 - t0);
            let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f) as u8;
            return Color::from_rgb_u8(lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    let (r, g, b) = SCALE[5].1;
    Color::from_rgb_u8(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_ids_are_unique() {
        let mut ids: Vec<_> = PALETTES.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PALETTES.len());
    }

    #[test]
    fn non_classic_resolves_to_itself() {
        assert_eq!(resolve("neon", Appearance::Light).id, "neon");
        assert_eq!(resolve("unknown", Appearance::Dark).id, CLASSIC);
    }

    #[test]
    fn temperature_scale_is_monotonic_in_red_at_the_hot_end() {
        assert!(temperature_color(33.0).red() >= temperature_color(18.0).red());
    }
}
