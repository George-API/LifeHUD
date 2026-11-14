//! Keeps ordinary windows fitted to the usable desktop when the HUD claims or releases space.
//!
//! Windows only re-fits *maximized* windows when the work area changes. Here every normal window
//! on the monitor is remapped proportionally from the old work area to the new one, so a window
//! snapped to the left half stays the left half of the space that's left.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};
use windows::core::BOOL;

struct Job {
    from: RECT,
    to: RECT,
    own_pid: u32,
    moved: usize,
}

fn width(r: &RECT) -> i32 {
    r.right - r.left
}

fn height(r: &RECT) -> i32 {
    r.bottom - r.top
}

/// Proportionally maps `r` from the `from` area into the `to` area.
pub fn remap(r: RECT, from: RECT, to: RECT) -> RECT {
    let sx = width(&to) as f64 / width(&from).max(1) as f64;
    let sy = height(&to) as f64 / height(&from).max(1) as f64;
    let x = |v: i32| to.left + ((v - from.left) as f64 * sx).round() as i32;
    let y = |v: i32| to.top + ((v - from.top) as f64 * sy).round() as i32;
    RECT { left: x(r.left), top: y(r.top), right: x(r.right), bottom: y(r.bottom) }
}

/// Whether a window rectangle belongs to (mostly sits within) an area, allowing for the
/// invisible resize borders Windows adds around frames.
pub fn within(r: &RECT, area: &RECT) -> bool {
    const SLACK: i32 = 16;
    r.left >= area.left - SLACK
        && r.top >= area.top - SLACK
        && r.right <= area.right + SLACK
        && r.bottom <= area.bottom + SLACK
}

unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the &mut Job passed by `reflow` for the duration of EnumWindows.
    let job = unsafe { &mut *(data.0 as *mut Job) };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
            return BOOL(1);
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return BOOL(1); // dialogs move with their owner
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_CAPTION.0 != WS_CAPTION.0 || ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == job.own_pid {
            return BOOL(1);
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return BOOL(1); // on another virtual desktop, or a suspended app
        }

        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() || !within(&r, &job.from) {
            return BOOL(1);
        }
        // Leave windows that already fit the new area alone (only when shrinking the area).
        let shrinking = width(&job.to) <= width(&job.from) && height(&job.to) <= height(&job.from);
        if shrinking && within(&r, &job.to) {
            return BOOL(1);
        }
        let mut n = remap(r, job.from, job.to);
        if style & WS_THICKFRAME.0 == 0 {
            // Fixed-size window: move it, keep its size, keep it on screen.
            let (w, h) = (width(&r), height(&r));
            n.left = n.left.clamp(job.to.left, (job.to.right - w).max(job.to.left));
            n.top = n.top.clamp(job.to.top, (job.to.bottom - h).max(job.to.top));
            n.right = n.left + w;
            n.bottom = n.top + h;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            n.left,
            n.top,
            width(&n),
            height(&n),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        );
        job.moved += 1;
    }
    BOOL(1)
}

/// Refits ordinary windows from the `from` work area to the `to` work area. Returns how many moved.
pub fn reflow(from: RECT, to: RECT) -> usize {
    if from == to || width(&to) < 200 || height(&to) < 200 {
        return 0;
    }
    let mut job = Job { from, to, own_pid: std::process::id(), moved: 0 };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut job as *mut Job as isize));
    }
    job.moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, rt: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: rt, bottom: b }
    }

    #[test]
    fn left_half_stays_left_half() {
        let from = r(0, 0, 2560, 1440);
        let to = r(0, 127, 2560, 1440);
        assert_eq!(remap(r(0, 0, 1280, 1440), from, to), r(0, 127, 1280, 1440));
    }

    #[test]
    fn sidebar_squeezes_horizontally() {
        let from = r(0, 0, 2560, 1440);
        let to = r(358, 0, 2560, 1440);
        let n = remap(r(1280, 0, 2560, 1440), from, to);
        assert_eq!((n.left, n.right), (1459, 2560));
    }

    #[test]
    fn within_allows_frame_slack() {
        assert!(within(&r(-7, 0, 1287, 1447), &r(0, 0, 2560, 1440)));
        assert!(!within(&r(-100, 0, 500, 500), &r(0, 0, 2560, 1440)));
    }
}
