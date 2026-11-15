//! A browser window docked into a HUD widget: an Edge `--app` window (Edge ships with Windows and
//! carries the user's sign-ins and DRM, so YouTube and Netflix just work) with its frame removed,
//! owned by the HUD so it stays above it and hides with it, and kept exactly over the widget's
//! player area.
//!
//! This drives another program's window, which Windows allows but doesn't promise to keep easy. Every
//! step checks that the window still exists, the window is found by what it *is* (a new top-level
//! Edge window), never by its title, and anything unexpected falls back to leaving a normal Edge
//! window on screen.

use std::collections::HashSet;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_SHOW, EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetClassNameW,
    GetClientRect, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, PostMessageW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
    WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::{BOOL, PWSTR};

/// Edge, if installed (it is on every supported Windows unless removed).
pub fn edge() -> Option<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"].iter().find_map(|var| {
        let p = PathBuf::from(std::env::var_os(var)?).join(r"Microsoft\Edge\Application\msedge.exe");
        p.exists().then_some(p)
    })
}

/// Opens `url` as an Edge app window of about `size` DIPs at `at` (DIPs). It appears there with a
/// frame for a moment, until [`adopt`] takes it over.
pub fn launch(url: &str, at: (i32, i32), size: (i32, i32)) -> bool {
    let Some(edge) = edge() else { return false };
    std::process::Command::new(edge)
        .arg(format!("--app={url}"))
        .arg(format!("--window-position={},{}", at.0, at.1))
        .arg(format!("--window-size={},{}", size.0.max(200), size.1.max(150)))
        .spawn()
        .is_ok()
}

/// Every top-level Edge browser window right now (so a launch can tell which one is new).
pub fn edge_windows() -> HashSet<isize> {
    let mut found: Vec<HWND> = Vec::new();
    // SAFETY: `found` outlives the synchronous EnumWindows call that receives a pointer to it.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<HWND> as isize));
    }
    found.into_iter().filter(|h| is_edge_frame(*h)).map(|h| h.0 as isize).collect()
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut Vec<HWND>` passed by `edge_windows` for the EnumWindows call.
    let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    found.push(hwnd);
    BOOL(1)
}

/// A visible, unowned Chromium frame window of an msedge.exe process.
fn is_edge_frame(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class);
        if String::from_utf16_lossy(&class[..n.max(0) as usize]) != "Chrome_WidgetWin_1" {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        ok && String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase().ends_with("\\msedge.exe")
    }
}

/// A pending launch: the Edge windows that existed before it, and what to do with the new one.
type Pending = (HashSet<isize>, Box<dyn FnMut(HWND)>);

thread_local! {
    static PENDING: std::cell::RefCell<Option<Pending>> = const { std::cell::RefCell::new(None) };
}

/// Catches the new Edge window the instant Windows shows it, before it has painted, so it never
/// appears on screen as a separate window: `on_show` adopts and places it right there. Lives until
/// dropped. The hook runs on this (the UI) thread as it pumps messages.
pub struct ShowWatch(HWINEVENTHOOK);

pub fn watch_for_new(before: HashSet<isize>, on_show: impl FnMut(HWND) + 'static) -> Option<ShowWatch> {
    PENDING.with(|p| *p.borrow_mut() = Some((before, Box::new(on_show))));
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_SHOW,
            None,
            Some(on_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    (!hook.is_invalid()).then_some(ShowWatch(hook))
}

impl Drop for ShowWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
        PENDING.with(|p| p.borrow_mut().take());
    }
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    // Whole top-level windows only (not carets, menus, …).
    if object != 0 || child != 0 || hwnd.is_invalid() {
        return;
    }
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let Some((before, _)) = p.as_ref() else { return };
        if before.contains(&(hwnd.0 as isize)) || !is_edge_frame(hwnd) {
            return;
        }
        if let Some((_, mut on_show)) = p.take() {
            drop(p);
            on_show(hwnd);
        }
    });
}

/// The Edge window that appeared since `before` was taken, if it has.
pub fn find_new(before: &HashSet<isize>) -> Option<HWND> {
    let now = edge_windows();
    now.into_iter().find(|h| !before.contains(h)).map(|h| HWND(h as *mut _))
}

/// Takes the window over: no frame, no taskbar button, owned by the HUD (so it stays above it and
/// minimises with it), with Windows 11's rounded corners.
pub fn adopt(hwnd: HWND, owner: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let strip = (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, ((style & !strip) | WS_POPUP.0) as isize);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ((ex & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0) as isize);
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
        // No explicit topmost: an owned window always stays above its owner, so it follows the HUD
        // (which steps out of the topmost band while a fullscreen app runs).
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Edge keeps drawing its own frame inside the window even without a native one: a title bar on
/// top and thin borders around the page. (left, top, right, bottom) in physical pixels, measured at
/// 96 DPI and scaled.
pub fn frame_insets(hwnd: HWND) -> (i32, i32, i32, i32) {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96) as f32;
    let px = |v: f32| (v * dpi / 96.0).round() as i32;
    (px(7.0), px(30.0), px(7.0), px(7.0))
}

/// Puts the window's page area exactly on `rect` (physical pixels), with corners rounded by
/// `radius`: the window is larger than the page, and a window region shows only the page.
pub fn place(hwnd: HWND, rect: RECT, radius: i32) {
    let (l, t, r, b) = frame_insets(hwnd);
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    unsafe {
        let _ =
            SetWindowPos(hwnd, None, rect.left - l, rect.top - t, w + l + r, h + t + b, SWP_NOACTIVATE | SWP_NOZORDER);
        // Round-rect regions include their right and bottom edges' last pixel minus one.
        let region = CreateRoundRectRgn(l, t, l + w + 1, t + h + 1, radius * 2, radius * 2);
        // The system owns the region from here on.
        let _ = SetWindowRgn(hwnd, Some(region), true);
    }
}

/// Still frameless (Edge can restore its frame, e.g. after a display change).
pub fn is_adopted(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    style & WS_CAPTION.0 == 0
}

/// The window's title (the page's).
pub fn title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

#[cfg(test)]
pub fn rect(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut r) }.ok().map(|_| r)
}

pub fn alive(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

pub fn set_shown(hwnd: HWND, shown: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

pub fn is_shown(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Clicks the middle of the page, as the user would: browsers only start sound after a click *in the
/// page*, and the click that asked for the video happened in the HUD. Posted to the page's own
/// window, so the real mouse pointer doesn't move.
pub fn click_page(hwnd: HWND) {
    unsafe {
        let mut page = HWND::default();
        let _ = EnumChildWindows(Some(hwnd), Some(find_page), LPARAM(&mut page as *mut HWND as isize));
        if page.is_invalid() {
            return;
        }
        let mut r = RECT::default();
        if GetClientRect(page, &mut r).is_err() {
            return;
        }
        let at = LPARAM((((r.bottom / 2) as u32) << 16 | (r.right / 2) as u32 & 0xFFFF) as isize);
        let _ = PostMessageW(Some(page), WM_LBUTTONDOWN, WPARAM(0x0001) /* MK_LBUTTON */, at);
        let _ = PostMessageW(Some(page), WM_LBUTTONUP, WPARAM(0), at);
    }
}

unsafe extern "system" fn find_page(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut class) };
    if String::from_utf16_lossy(&class[..n.max(0) as usize]) == "Chrome_RenderWidgetHostHWND" {
        // SAFETY: lparam is the `&mut HWND` passed by `click_page` for the EnumChildWindows call.
        unsafe { *(lparam.0 as *mut HWND) = hwnd };
        return BOOL(0);
    }
    BOOL(1)
}

/// Asks the window to close, as its own close button would.
pub fn close(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    /// Live check of the whole dance: launch, find, adopt, place, close. Opens a real Edge window
    /// (muted, and fully transparent so it never shows over whatever the user is doing), and saves
    /// what the window draws to `DOCK_SHOT` (raw BGRA) for inspection. Ignored by default.
    #[test]
    #[ignore]
    fn docks_an_edge_window() {
        use windows::Win32::Foundation::{COLORREF, HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
        };
        let before = super::edge_windows();
        let url = std::env::var("DOCK_URL")
            .unwrap_or_else(|_| crate::services::embed::youtube_url("aqz-KE-bpKQ").unwrap() + "/muted");
        // The show hook should catch it before it paints; pump messages as the UI thread does.
        let caught = std::rc::Rc::new(std::cell::Cell::new(None::<isize>));
        let c = caught.clone();
        let watch = super::watch_for_new(before.clone(), move |h| {
            // Invisible for the test, so nothing shows over whatever the user is doing.
            unsafe {
                let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
                SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA);
            }
            super::adopt(h, HWND::default());
            c.set(Some(h.0 as isize));
        })
        .expect("hook");
        let started = Instant::now();
        assert!(super::launch(&url, (200, 200), (480, 300)));
        let hwnd = loop {
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
                };
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if let Some(h) = caught.get() {
                println!("caught by the show hook after {:?}", started.elapsed());
                break HWND(h as *mut _);
            }
            assert!(started.elapsed() < Duration::from_secs(15), "the show hook never fired");
            std::thread::sleep(Duration::from_millis(5));
        };
        drop(watch);
        super::place(hwnd, RECT { left: 200, top: 200, right: 840, bottom: 560 }, 8);
        assert!(super::is_adopted(hwnd));
        // Let the page load (and keep the embed server running for it), click play, let it run.
        std::thread::sleep(Duration::from_secs(std::env::var("HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(4)));
        let started = Instant::now();
        while super::title(hwnd) != crate::services::embed::PLAYING_TITLE {
            assert!(started.elapsed() < Duration::from_secs(10), "never started: {:?}", super::title(hwnd));
            super::click_page(hwnd);
            std::thread::sleep(Duration::from_millis(500));
        }
        println!("playing after {:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(4));
        println!("window rect {:?}, dpi {}", super::rect(hwnd), unsafe {
            windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd)
        });
        if let Ok(path) = std::env::var("DOCK_SHOT") {
            let (w, h, px) = shot(hwnd);
            std::fs::write(&path, &px).unwrap();
            println!("shot {w}x{h} -> {path}");
        }
        super::close(hwnd);
        std::thread::sleep(Duration::from_millis(600));
        assert!(!super::alive(hwnd), "closed");
    }

    /// The window's own pixels (not the screen's), via PrintWindow.
    fn shot(hwnd: windows::Win32::Foundation::HWND) -> (i32, i32, Vec<u8>) {
        use windows::Win32::Graphics::Gdi::*;
        use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
        let r = super::rect(hwnd).unwrap();
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(dc, bmp.into());
            let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)); // PW_RENDERFULLCONTENT
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut px = vec![0u8; (w * h * 4) as usize];
            GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            let mut out = w.to_le_bytes().to_vec();
            out.extend(h.to_le_bytes());
            out.extend(px);
            (w, h, out)
        }
    }
}
