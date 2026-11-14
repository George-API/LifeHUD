//! Window styling: acrylic glass, square flush edges, dark title bars, topmost and Alt+Tab visibility.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowLongPtrW, SetWindowPos, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, s, w};

fn set_attr<T>(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: &T) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, attr, value as *const T as *const _, std::mem::size_of::<T>() as u32);
    }
}

/// Keeps the HUD out of Alt+Tab, Task View and the taskbar.
pub fn hide_from_switcher(hwnd: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex = (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
    }
}

/// Docked surfaces sit flush against the screen edge: square corners, no DWM outline.
pub fn make_flush(hwnd: HWND) {
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND);
    set_attr(hwnd, DWMWA_BORDER_COLOR, &0xFFFF_FFFEu32); // DWMWA_COLOR_NONE
}

pub fn set_dark_title_bar(hwnd: HWND, dark: bool) {
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(dark));
}

pub fn set_topmost(hwnd: HWND, topmost: bool) {
    let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe {
        let _ = SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        if !topmost {
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

#[repr(C)]
struct AccentPolicy {
    state: i32,
    flags: i32,
    gradient_color: u32,
    animation_id: i32,
}

#[repr(C)]
struct CompositionAttributeData {
    attribute: i32,
    data: *mut core::ffi::c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttributeData) -> BOOL;

/// Live acrylic blur behind the window. Unlike DWM system backdrops, this stays live while the
/// window is inactive, which an always-on HUD almost always is. `tint` is 0xAARRGGBB.
pub fn enable_acrylic(hwnd: HWND, tint: u32) -> bool {
    unsafe {
        let Ok(user32) = LoadLibraryW(w!("user32.dll")) else { return false };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return false };
        // SAFETY: documented-by-convention signature of this long-standing user32 export.
        let f: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let (a, r, g, b) = ((tint >> 24) & 0xFF, (tint >> 16) & 0xFF, (tint >> 8) & 0xFF, tint & 0xFF);
        let mut policy = AccentPolicy {
            state: 4, // ACCENT_ENABLE_ACRYLICBLURBEHIND
            flags: 2,
            gradient_color: (a << 24) | (b << 16) | (g << 8) | r, // AABBGGRR
            animation_id: 0,
        };
        let mut data = CompositionAttributeData {
            attribute: 19, // WCA_ACCENT_POLICY
            data: &mut policy as *mut _ as *mut _,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        f(hwnd, &mut data).as_bool()
    }
}
