//! Themed context menus: a small floating window (`ui/menu.slint`) at the cursor, styled with the
//! active theme, that closes on a choice, Escape or a click anywhere else. One menu at a time.

use std::cell::RefCell;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, PhysicalPosition, Timer, VecModel};

use crate::platform::menu::{self, DeactivateHook, Item};
use crate::platform::{effects, hwnd_of};
use crate::theme::Palette;
use crate::{MenuRow, MenuState, MenuWindow, Theme, storage};

/// Logical sizes, matching `ui/menu.slint`.
const WIDTH: f32 = 240.0;
const PADDING: f32 = 5.0;
const ENTRY: f32 = 28.0;
const HEADER: f32 = 24.0;
const SEPARATOR: f32 = 9.0;

struct Open {
    // Declared first so the subclass is removed before the window it's attached to goes away.
    hook: Option<DeactivateHook>,
    window: MenuWindow,
    on_choose: Option<Box<dyn FnOnce(usize)>>,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
}

/// Shows `items` at the cursor. `on_choose` runs with the chosen entry's id; it isn't called if the
/// menu is dismissed.
pub fn show(palette: &Palette, items: Vec<Item>, on_choose: impl FnOnce(usize) + 'static) {
    close();
    let window = match MenuWindow::new() {
        Ok(w) => w,
        Err(e) => {
            storage::log(format!("could not open menu: {e}"));
            return;
        }
    };
    crate::theme::apply(&window.global::<Theme>(), palette);

    let rows: Vec<MenuRow> = items.iter().map(row).collect();
    let height = PADDING * 2.0 + rows.iter().map(|r| row_height(r.kind)).sum::<f32>();
    let kinds: Vec<i32> = rows.iter().map(|r| r.kind).collect();
    let st = window.global::<MenuState>();
    st.set_rows(ModelRc::new(VecModel::from(rows)));
    st.on_step(move |from, step| next_entry(&kinds, from, step));
    st.on_chosen(|id| choose(id.max(0) as usize));
    st.on_dismiss(defer_close);

    let (x, y, _) = menu::place_at_cursor(WIDTH, height);
    window.window().set_position(PhysicalPosition::new(x, y));
    if let Err(e) = window.show() {
        storage::log(format!("could not show menu: {e}"));
        return;
    }
    OPEN.with(|o| *o.borrow_mut() = Some(Open { hook: None, window, on_choose: Some(Box::new(on_choose)) }));
    style_native(palette.tint, 0);
}

/// The native window appears a moment after `show`; give it the panel treatment once it exists.
fn style_native(tint: u32, attempt: u32) {
    let hwnd = OPEN.with(|o| o.borrow().as_ref().and_then(|m| hwnd_of(m.window.window())));
    let Some(hwnd) = hwnd else {
        if attempt < 50 && OPEN.with(|o| o.borrow().is_some()) {
            Timer::single_shot(Duration::from_millis(10), move || style_native(tint, attempt + 1));
        }
        return;
    };
    effects::hide_from_switcher(hwnd);
    effects::round_corners(hwnd);
    effects::enable_acrylic(hwnd, tint);
    effects::bring_to_front(hwnd);
    // A click anywhere else takes focus away: close, as a native menu would.
    let hook = menu::on_deactivate(hwnd, defer_close);
    OPEN.with(|o| {
        if let Some(m) = o.borrow_mut().as_mut() {
            m.hook = Some(hook);
        }
    });
}

fn choose(id: usize) {
    let on_choose = OPEN.with(|o| o.borrow_mut().as_mut().and_then(|m| m.on_choose.take()));
    defer_close();
    if let Some(f) = on_choose {
        // After this turn of the event loop, so the action never runs inside the menu's own callback.
        Timer::single_shot(Duration::ZERO, move || f(id));
    }
}

/// Hides now, frees on the next turn of the event loop (never from inside the menu's own callbacks).
fn defer_close() {
    OPEN.with(|o| {
        if let Some(m) = o.borrow().as_ref() {
            let _ = m.window.hide();
        }
    });
    Timer::single_shot(Duration::ZERO, close);
}

fn close() {
    let open = OPEN.with(|o| o.borrow_mut().take());
    if let Some(mut m) = open {
        m.hook.take();
        let _ = m.window.hide();
    }
}

fn row(item: &Item) -> MenuRow {
    let base = |kind: i32, id: usize, text: &str, checked: bool| MenuRow {
        kind,
        id: id as i32,
        text: text.into(),
        checked,
        has_swatch: false,
        swatch: Default::default(),
        accent: Default::default(),
    };
    match item {
        Item::Entry { id, text, checked } => base(0, *id, text, *checked),
        Item::Swatch { id, text, checked, surface, accent } => MenuRow {
            has_swatch: true,
            swatch: crate::theme::argb(*surface),
            accent: crate::theme::argb(*accent),
            ..base(0, *id, text, *checked)
        },
        Item::Header(text) => base(1, 0, text, false),
        Item::Separator => base(2, 0, "", false),
    }
}

/// Whether a menu is showing (so hover-to-open doesn't stack a second one).
pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

fn row_height(kind: i32) -> f32 {
    match kind {
        0 => ENTRY,
        1 => HEADER,
        _ => SEPARATOR,
    }
}

/// Keyboard navigation: the next entry (kind 0) from `from` in direction `step`, wrapping around.
/// `from` = -1 starts at the first (down) or last (up) entry.
fn next_entry(kinds: &[i32], from: i32, step: i32) -> i32 {
    let n = kinds.len() as i32;
    if n == 0 || !kinds.contains(&0) {
        return -1;
    }
    let step = if step < 0 { -1 } else { 1 };
    let mut i = if from < 0 { if step > 0 { -1 } else { n } } else { from };
    loop {
        i = (i + step).rem_euclid(n);
        if kinds[i as usize] == 0 {
            return i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::next_entry;

    #[test]
    fn keyboard_skips_headers_and_separators() {
        // header, entry, entry, separator, entry
        let kinds = [1, 0, 0, 2, 0];
        assert_eq!(next_entry(&kinds, -1, 1), 1);
        assert_eq!(next_entry(&kinds, 2, 1), 4);
        assert_eq!(next_entry(&kinds, 4, 1), 1, "wraps to the top");
        assert_eq!(next_entry(&kinds, -1, -1), 4);
        assert_eq!(next_entry(&kinds, 1, -1), 4, "wraps to the bottom");
        assert_eq!(next_entry(&[1, 2], -1, 1), -1, "nothing to select");
    }
}
