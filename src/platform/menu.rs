//! The Windows side of Life HUD's context menus (the menu itself is `ui/menu.slint`): where to put
//! it so it stays on screen, and noticing when focus leaves it so it can close like a native menu.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
use windows::Win32::Graphics::Gdi::MonitorFromPoint;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, WA_INACTIVE, WM_ACTIVATE};

use crate::platform::monitors;

pub enum Item {
    /// A non-clickable section label.
    Header(String),
    /// A clickable entry.
    Entry {
        id: usize,
        text: String,
        checked: bool,
    },
    /// A clickable entry with a colour preview (theme menus). Colours are 0xAARRGGBB.
    Swatch {
        id: usize,
        text: String,
        checked: bool,
        surface: u32,
        /// The theme's ambient light, corner to corner.
        glow_a: u32,
        glow_b: u32,
        card: u32,
        accent: u32,
    },
    Separator,
}

/// Where a menu of `width`×`height` logical pixels opens for the current cursor position: down and to
/// the right of the pointer, flipped up or left when that would run off the monitor's work area.
/// Returns the physical top-left corner and the monitor's scale factor.
pub fn place_at_cursor(width: f32, height: f32) -> (i32, i32, f32) {
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    let monitor = monitors::query(unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) });
    let scale = monitor.as_ref().map(|m| m.scale).unwrap_or(1.0);
    let (w, h) = ((width * scale).round() as i32, (height * scale).round() as i32);
    let Some(wa) = monitor.map(|m| m.work_area) else { return (pt.x, pt.y, scale) };
    (fit(pt.x, w, wa.left, wa.right), fit(pt.y, h, wa.top, wa.bottom), scale)
}

/// Opens at `at` going forward if it fits, else backward from `at`, and never past `lo`..`hi`.
fn fit(at: i32, size: i32, lo: i32, hi: i32) -> i32 {
    let start = if at + size <= hi { at } else { at - size };
    start.clamp(lo, (hi - size).max(lo))
}

/// Calls `f` whenever the window loses activation (a click anywhere else). Dropping the returned
/// hook detaches it, so drop it before the window is destroyed.
pub struct DeactivateHook {
    hwnd: HWND,
    data: *mut Box<dyn Fn()>,
}

const HOOK_ID: usize = 2;

pub fn on_deactivate(hwnd: HWND, f: impl Fn() + 'static) -> DeactivateHook {
    let data: *mut Box<dyn Fn()> = Box::into_raw(Box::new(Box::new(f)));
    // SAFETY: `data` stays valid until `Drop` removes the subclass and frees it.
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), HOOK_ID, data as usize);
    }
    DeactivateHook { hwnd, data }
}

impl Drop for DeactivateHook {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), HOOK_ID);
            // SAFETY: reclaims the box handed to SetWindowSubclass in `on_deactivate`.
            drop(Box::from_raw(self.data));
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_ACTIVATE && (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
        // SAFETY: `data` is the box registered in `on_deactivate`, alive while the subclass exists.
        // The callback only schedules work; it never re-enters the UI from inside this procedure.
        let f = unsafe { &*(data as *const Box<dyn Fn()>) };
        f();
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn menus_stay_on_screen() {
        // Room below/right of the pointer: open there.
        assert_eq!(fit(100, 200, 0, 1440), 100);
        // Near the bottom edge: flip up so the menu ends at the pointer.
        assert_eq!(fit(1400, 200, 0, 1440), 1200);
        // Taller than the space either way: pinned inside the work area.
        assert_eq!(fit(100, 300, 0, 250), 0);
    }
}
