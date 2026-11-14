//! Notification-area icon with a native Windows 11 menu (follows light/dark mode).

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GetCursorPos,
    GetSystemMetrics, HICON, HMENU, HWND_MESSAGE, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, s, w};

const CALLBACK: u32 = WM_APP + 1;
const ID_TOGGLE: usize = 1;
const ID_TOP: usize = 2;
const ID_SIDE: usize = 3;
const ID_SETTINGS: usize = 4;
const ID_QUIT: usize = 5;

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    Click,
    ToggleHud,
    TopLayout,
    SidebarLayout,
    Settings,
    Quit,
}

/// What the menu should show when opened.
#[derive(Debug, Clone, Copy)]
pub struct MenuState {
    pub hud_visible: bool,
    pub top_layout: bool,
}

struct Handlers {
    on_event: Box<dyn Fn(TrayEvent)>,
    menu_state: Box<dyn Fn() -> MenuState>,
    taskbar_created: u32,
    icon: HICON,
}

thread_local! {
    static HANDLERS: RefCell<Option<Handlers>> = const { RefCell::new(None) };
}

pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    pub fn create(
        on_event: impl Fn(TrayEvent) + 'static,
        menu_state: impl Fn() -> MenuState + 'static,
    ) -> Option<Self> {
        unsafe {
            allow_dark_menus();
            let hinstance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: w!("LifeHUD.Tray"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("LifeHUD.Tray"),
                w!("Life HUD"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )
            .ok()?;
            let icon = LoadImageW(
                Some(hinstance.into()),
                PCWSTR(std::ptr::dangling::<u16>()), // resource id embedded by build.rs
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .unwrap_or_default();

            HANDLERS.with(|h| {
                *h.borrow_mut() = Some(Handlers {
                    on_event: Box::new(on_event),
                    menu_state: Box::new(menu_state),
                    taskbar_created: RegisterWindowMessageW(&HSTRING::from("TaskbarCreated")),
                    icon,
                })
            });
            add_icon(hwnd, icon);
            Some(Self { hwnd })
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyWindow(self.hwnd);
        }
        HANDLERS.with(|h| h.borrow_mut().take());
    }
}

unsafe fn add_icon(hwnd: HWND, icon: HICON) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: CALLBACK,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = "Life HUD".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

/// Opt this process into dark context menus when Windows is in dark mode (uxtheme ordinals 135/136).
unsafe fn allow_dark_menus() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(set_mode) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(1); // AllowDark
        }
        if let Some(flush) = GetProcAddress(uxtheme, PCSTR(136 as *const u8)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
    let _ = s!("");
}

fn emit(event: TrayEvent) {
    // Defer out of the window procedure so handlers can freely touch the UI.
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        HANDLERS.with(|h| {
            if let Some(h) = h.borrow().as_ref() {
                (h.on_event)(event);
            }
        });
    });
}

unsafe fn show_menu(hwnd: HWND) {
    let state = HANDLERS.with(|h| h.borrow().as_ref().map(|h| (h.menu_state)()));
    let Some(state) = state else { return };
    unsafe {
        let Ok(menu): Result<HMENU, _> = CreatePopupMenu() else { return };
        let add = |id: usize, text: &str, checked: bool| {
            let text = HSTRING::from(text);
            let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, flags, id, &text);
        };
        add(ID_TOGGLE, if state.hud_visible { "Hide Life HUD" } else { "Show Life HUD" }, false);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(ID_TOP, "Top", state.top_layout);
        add(ID_SIDE, "Sidebar", !state.top_layout);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(ID_SETTINGS, "Settings…", false);
        add(ID_QUIT, "Quit Life HUD", false);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        let event = match cmd.0 as usize {
            ID_TOGGLE => Some(TrayEvent::ToggleHud),
            ID_TOP => Some(TrayEvent::TopLayout),
            ID_SIDE => Some(TrayEvent::SidebarLayout),
            ID_SETTINGS => Some(TrayEvent::Settings),
            ID_QUIT => Some(TrayEvent::Quit),
            _ => None,
        };
        if let Some(e) = event {
            emit(e);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == CALLBACK {
        match (lparam.0 & 0xFFFF) as u32 {
            WM_LBUTTONUP => emit(TrayEvent::Click),
            WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_menu(hwnd) },
            _ => {}
        }
        return LRESULT(0);
    }
    let recreated = HANDLERS.with(|h| h.borrow().as_ref().filter(|h| h.taskbar_created == msg).map(|h| h.icon));
    if let Some(icon) = recreated {
        unsafe { add_icon(hwnd, icon) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
