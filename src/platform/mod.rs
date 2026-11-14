//! Everything that talks to Windows directly. Unsafe code is confined to this module and kept
//! to thin, documented wrappers around Win32/WinRT calls.

pub mod appbar;
pub mod effects;
pub mod instance;
pub mod monitors;
pub mod reflow;
pub mod startup;
pub mod tray;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// The native handle behind a Slint window, once it has been created.
pub fn hwnd_of(window: &slint::Window) -> Option<HWND> {
    let handle = window.window_handle();
    let raw = handle.window_handle().ok()?.as_raw();
    match raw {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// The UI thread must be an STA (winit/OLE expect it) before any WinRT call.
pub fn init_ui_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Worker threads that call WinRT join the multithreaded apartment.
pub fn init_worker_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

pub fn system_prefers_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &HSTRING::from("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

pub fn system_accent(dark: bool) -> slint::Color {
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    let read = || -> windows::core::Result<slint::Color> {
        let c = UISettings::new()?.GetColorValue(if dark {
            UIColorType::AccentLight2
        } else {
            UIColorType::AccentDark1
        })?;
        Ok(slint::Color::from_argb_u8(c.A, c.R, c.G, c.B))
    };
    read().unwrap_or(slint::Color::from_rgb_u8(0x00, 0x78, 0xD4))
}

/// Opens a URL, URI scheme (e.g. `spotify:`) or folder with the default handler.
pub fn shell_open(target: &str) -> bool {
    let target = HSTRING::from(target);
    let r =
        unsafe { ShellExecuteW(None, &HSTRING::from("open"), &target, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 as isize > 32
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
