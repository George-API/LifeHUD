//! Titles of open browser windows, used to tell which site a browser's media session belongs to
//! (Windows reports a browser's audio/video, but not the page it comes from). Titles are only
//! compared in memory, never stored or logged.

use std::collections::HashMap;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PWSTR};

const BROWSERS: [&str; 6] = ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe"];

pub struct BrowserWindow {
    pub hwnd: HWND,
    pub title: String,
}

struct Scan {
    names: HashMap<u32, bool>,
    found: Vec<BrowserWindow>,
}

/// Visible top-level windows that belong to a web browser, with their titles (the active tab's).
pub fn windows() -> Vec<BrowserWindow> {
    let mut scan = Scan { names: HashMap::new(), found: Vec::new() };
    // SAFETY: `scan` outlives the synchronous EnumWindows call that receives a pointer to it.
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut scan as *mut Scan as isize));
    }
    scan.found
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut Scan` passed by `windows()` for the duration of EnumWindows.
    let scan = unsafe { &mut *(lparam.0 as *mut Scan) };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let is_browser = *scan.names.entry(pid).or_insert_with(|| is_browser_process(pid));
        if is_browser {
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, &mut buf);
            scan.found.push(BrowserWindow { hwnd, title: String::from_utf16_lossy(&buf[..n.max(0) as usize]) });
        }
    }
    BOOL(1)
}

fn is_browser_process(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return false;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
        BROWSERS.iter().any(|b| path.ends_with(&format!("\\{b}")))
    }
}

/// Brings a browser window to the front (restoring it if minimised).
pub fn activate(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}
