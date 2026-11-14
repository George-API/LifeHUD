//! Shell AppBar registration: Windows shrinks the work area around the HUD, so maximized windows
//! fill the rest of the desktop instead of sliding underneath it. All coordinates are physical pixels.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Shell::{
    ABE_LEFT, ABE_TOP, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_WINDOWPOSCHANGED,
    ABN_FULLSCREENAPP, ABN_POSCHANGED, APPBARDATA, DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    RegisterWindowMessageW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    WINDOWPOS, WM_ACTIVATE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WM_WINDOWPOSCHANGED,
    WM_WINDOWPOSCHANGING,
};
use windows::core::HSTRING;

use crate::settings::Layout;

#[derive(Debug, Clone, Copy)]
pub struct DockRequest {
    pub edge: Layout,
    pub monitor_bounds: RECT,
    pub work_area: RECT,
    pub thickness: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEvent {
    /// The taskbar or another appbar moved; reclaim our slot.
    PositionChanged,
    /// A fullscreen app opened (true) or closed (false).
    Fullscreen(bool),
    /// Explorer restarted; every registration was lost.
    TaskbarCreated,
    /// Resolution, monitor layout or DPI changed.
    DisplayChanged,
    /// A system setting changed (light/dark mode, accent colour).
    SettingChanged,
}

struct Inner {
    hwnd: HWND,
    callback_message: u32,
    taskbar_created: u32,
    registered: Cell<bool>,
    positioning: Cell<bool>,
    bounds: Cell<Option<RECT>>,
    on_event: Box<dyn Fn(ShellEvent)>,
}

/// Remembered for the panic hook, so a crash still releases the reserved strip.
static LAST_HWND: AtomicIsize = AtomicIsize::new(0);

pub struct AppBar {
    inner: Rc<Inner>,
    raw: *const Inner,
}

impl AppBar {
    /// Subclasses `hwnd` to receive shell notifications. `on_event` runs on the UI thread, deferred
    /// out of the window procedure so it can safely call back into the UI.
    pub fn attach(hwnd: HWND, on_event: impl Fn(ShellEvent) + 'static) -> Self {
        let inner = Rc::new(Inner {
            hwnd,
            callback_message: unsafe { RegisterWindowMessageW(&HSTRING::from("LifeHUD.AppBarNotify")) },
            taskbar_created: unsafe { RegisterWindowMessageW(&HSTRING::from("TaskbarCreated")) },
            registered: Cell::new(false),
            positioning: Cell::new(false),
            bounds: Cell::new(None),
            on_event: Box::new(on_event),
        });
        let raw = Rc::into_raw(inner.clone());
        // SAFETY: `raw` stays valid until `Drop` removes the subclass and reclaims it.
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(subclass_proc), 1, raw as usize);
        }
        LAST_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        Self { inner, raw }
    }

    pub fn register(&self) {
        if self.inner.registered.get() {
            return;
        }
        let mut abd = self.data();
        abd.uCallbackMessage = self.inner.callback_message;
        let ok = unsafe { SHAppBarMessage(ABM_NEW, &mut abd) } != 0;
        self.inner.registered.set(ok);
        if !ok {
            crate::storage::log("ABM_NEW failed");
        }
    }

    pub fn unregister(&self) {
        if !self.inner.registered.replace(false) {
            return;
        }
        let mut abd = self.data();
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut abd) };
        self.inner.bounds.set(None);
    }

    /// Negotiates the requested edge with the shell, claims it, and moves the window into it.
    pub fn reposition(&self, req: DockRequest) {
        if !self.inner.registered.get() {
            return;
        }
        let mut abd = self.data();
        let (mb, wa, t) = (req.monitor_bounds, req.work_area, req.thickness);
        match req.edge {
            Layout::Top => {
                abd.uEdge = ABE_TOP;
                abd.rc = RECT { left: wa.left, right: wa.right, top: mb.top, bottom: mb.top + t };
            }
            Layout::Left => {
                abd.uEdge = ABE_LEFT;
                abd.rc = RECT { left: mb.left, right: mb.left + t, top: wa.top, bottom: wa.bottom };
            }
        }
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut abd) };
        match req.edge {
            Layout::Top => abd.rc.bottom = abd.rc.top + t,
            Layout::Left => abd.rc.right = abd.rc.left + t,
        }
        unsafe { SHAppBarMessage(ABM_SETPOS, &mut abd) };

        let rc = abd.rc;
        self.inner.bounds.set(Some(rc));
        self.inner.positioning.set(true);
        unsafe {
            let _ = SetWindowPos(
                self.inner.hwnd,
                None,
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        self.inner.positioning.set(false);
    }

    /// Moves the window without the slot pinning (used while animating between layouts).
    pub fn move_freely(&self, rc: RECT) {
        self.inner.positioning.set(true);
        unsafe {
            let _ = SetWindowPos(
                self.inner.hwnd,
                None,
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        self.inner.positioning.set(false);
    }

    fn data(&self) -> APPBARDATA {
        APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: self.inner.hwnd, ..Default::default() }
    }

    /// Last-chance release from a panic hook (any thread).
    pub fn emergency_remove() {
        let raw = LAST_HWND.load(Ordering::Relaxed);
        if raw == 0 {
            return;
        }
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: HWND(raw as *mut _),
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut abd) };
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        self.unregister();
        unsafe {
            let _ = RemoveWindowSubclass(self.inner.hwnd, Some(subclass_proc), 1);
            // SAFETY: reclaims the reference handed to SetWindowSubclass in `attach`.
            drop(Rc::from_raw(self.raw));
        }
    }
}

fn defer(inner: &Inner, event: ShellEvent) {
    // SAFETY: the subclass holds a strong reference until Drop, and timers run on this same thread.
    let ptr = inner as *const Inner;
    let strong = unsafe {
        Rc::increment_strong_count(ptr);
        Rc::from_raw(ptr)
    };
    slint::Timer::single_shot(std::time::Duration::ZERO, move || (strong.on_event)(event));
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the pointer registered in `attach`, valid while the subclass exists.
    let inner = unsafe { &*(data as *const Inner) };

    if msg == inner.callback_message {
        match wparam.0 as u32 {
            ABN_POSCHANGED => defer(inner, ShellEvent::PositionChanged),
            ABN_FULLSCREENAPP => defer(inner, ShellEvent::Fullscreen(lparam.0 != 0)),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == inner.taskbar_created {
        inner.registered.set(false);
        defer(inner, ShellEvent::TaskbarCreated);
    }

    match msg {
        WM_ACTIVATE | WM_WINDOWPOSCHANGED if inner.registered.get() => {
            let mut abd =
                APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };
            let m = if msg == WM_ACTIVATE { ABM_ACTIVATE } else { ABM_WINDOWPOSCHANGED };
            unsafe { SHAppBarMessage(m, &mut abd) };
        }
        WM_WINDOWPOSCHANGING if inner.registered.get() && !inner.positioning.get() => {
            // Pin the window to its reserved slot (blocks Win+Arrow, DPI auto-resize, stray moves).
            if let Some(b) = inner.bounds.get() {
                // SAFETY: lParam of WM_WINDOWPOSCHANGING points to a WINDOWPOS we may modify.
                let wp = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
                let both = SWP_NOMOVE | SWP_NOSIZE;
                if (wp.flags & both) != both {
                    wp.x = b.left;
                    wp.y = b.top;
                    wp.cx = b.right - b.left;
                    wp.cy = b.bottom - b.top;
                    wp.flags &= !both;
                }
            }
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => defer(inner, ShellEvent::DisplayChanged),
        WM_SETTINGCHANGE => defer(inner, ShellEvent::SettingChanged),
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
