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
    unsafe {
        if enable {
            let Ok(exe) = std::env::current_exe() else { return };
            let command = crate::platform::to_wide(&format!("\"{}\"", exe.display()));
            let status = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(VALUE),
                REG_SZ.0,
                Some(command.as_ptr() as *const _),
                (command.len() * 2) as u32,
            );
            if status.is_err() {
                crate::storage::log(format!("could not enable open at login: {status:?}"));
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(VALUE));
        }
    }
}
