//! "Open at login" through the per-user Run key (no admin rights needed).

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "LifeHUD";

pub fn is_enabled() -> bool {
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    status.is_ok() && size > 2
}

pub fn set_enabled(enable: bool) {
    if !enable {
        remove();
    } else if let Ok(exe) = std::env::current_exe() {
        write(&exe);
    }
}

/// After installing or moving, keeps "Open at login" (if it's on) pointing at `exe`.
pub fn repoint(exe: &std::path::Path) {
    if is_enabled() {
        write(exe);
    }
}

fn remove() {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(VALUE));
    }
}

fn write(exe: &std::path::Path) {
    // Quoted, so a path with spaces can't be misread as a different program.
    let command = crate::platform::to_wide(&format!("\"{}\"", exe.display()));
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE),
            REG_SZ.0,
            Some(command.as_ptr() as *const _),
            (command.len() * 2) as u32,
        )
    };
    if status.is_err() {
        crate::storage::log(format!("could not enable open at login: {status:?}"));
    }
}
