use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

#[derive(Debug, Clone)]
pub struct Monitor {
    pub device: String,
    pub bounds: RECT,
    pub work_area: RECT,
    pub primary: bool,
    pub scale: f32,
}

impl Monitor {
    pub fn label(&self, index: usize) -> String {
        let w = self.bounds.right - self.bounds.left;
        let h = self.bounds.bottom - self.bounds.top;
        format!("Display {}{} · {w} × {h}", index + 1, if self.primary { " (main)" } else { "" })
    }
}

unsafe extern "system" fn collect(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the &mut Vec passed by `all()` for the duration of EnumDisplayMonitors.
    let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    list.push(hmon);
    BOOL(1)
}

pub fn query(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    if !ok.as_bool() {
        return None;
    }
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
    Some(Monitor {
        device: String::from_utf16_lossy(&info.szDevice[..end]),
        bounds: info.monitorInfo.rcMonitor,
        work_area: info.monitorInfo.rcWork,
        primary: info.monitorInfo.dwFlags & 1 != 0,
        scale: dx.max(96) as f32 / 96.0,
    })
}

/// All monitors: primary first, then left to right.
pub fn all() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let mut list: Vec<Monitor> = handles.into_iter().filter_map(query).collect();
    list.sort_by_key(|m| (!m.primary, m.bounds.left, m.bounds.top));
    list
}

pub fn find(device: Option<&str>) -> Option<Monitor> {
    let list = all();
    let wanted = device.and_then(|d| list.iter().find(|m| m.device.eq_ignore_ascii_case(d)).cloned());
    wanted.or_else(|| list.iter().find(|m| m.primary).cloned()).or_else(|| list.into_iter().next())
}
