//! Install on first run, per user, with no admin rights and no wizard.
//!
//! Launching the downloaded exe copies it to `%LOCALAPPDATA%\Programs\LifeHUD`, adds a Start menu
//! shortcut and an entry in Settings → Apps, then relaunches from there. Running a newer download
//! later updates it in place the same way. Dev builds (`target\debug`, `target\release`) never install
//! themselves, and an empty `portable` (or `portable.txt`) file next to the exe keeps a copy portable.

use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellExecuteW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW, SHOW_WINDOW_CMD, SW_HIDE, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, Interface, PCWSTR};

use crate::platform::{instance, startup};
use crate::storage;

const EXE: &str = "lifehud.exe";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\LifeHUD";
const SHORTCUT: &str = "Life HUD.lnk";

/// What `main` should do after [`on_launch`].
pub enum Next {
    /// Carry on starting the app from this exe.
    Run,
    /// The installed copy was launched; this process should exit.
    Exit,
}

/// Installs or updates on a plain launch. `--install` forces it (used by `tools/install.ps1`).
/// Any other argument (`--exit`, `--uninstall`, …) skips installing.
pub fn on_launch(args: &[String]) -> Next {
    let force = args.iter().any(|a| a == "--install");
    if !args.is_empty() && !force {
        return Next::Run;
    }
    let (Ok(current), Some(target)) = (std::env::current_exe(), installed_exe()) else { return Next::Run };
    if same_file(&current, &target) {
        // Keep the Apps entry's version current after an in-place update.
        register(&target);
        return Next::Run;
    }
    if !force && (cfg!(debug_assertions) || is_dev_build(&current) || is_portable(&current)) {
        return Next::Run;
    }
    match install(&current, &target) {
        Ok(()) => {
            if launch(&target.display().to_string(), "", SW_SHOWNORMAL) {
                Next::Exit
            } else {
                storage::log("installed, but could not start the installed copy");
                Next::Run
            }
        }
        Err(e) => {
            // Never block the user: run from wherever they launched it.
            storage::log(format!("install failed, running in place: {e}"));
            Next::Run
        }
    }
}

/// `lifehud.exe --uninstall`, run by Settings → Apps. Keeps the user's data folder. `quiet` skips
/// the confirmation (package managers like winget uninstall without UI).
pub fn uninstall(quiet: bool) {
    if !quiet {
        let text = HSTRING::from(
            "Remove Life HUD from this PC?\n\nYour tasks, notes and settings stay in %LOCALAPPDATA%\\LifeHUD.",
        );
        let answer =
            unsafe { MessageBoxW(None, &text, &HSTRING::from("Uninstall Life HUD"), MB_YESNO | MB_ICONQUESTION) };
        if answer != IDYES {
            return;
        }
    }
    instance::request_exit_and_wait(Duration::from_secs(8));
    startup::set_enabled(false);
    if let Some(lnk) = shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY));
    }
    // A running exe can't delete itself: a hidden shell removes it once this process has exited.
    // Only our own file and (if then empty) our own folder are touched.
    if let Some(exe) = installed_exe()
        && let Some(dir) = exe.parent()
    {
        let script =
            format!("ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"", exe.display(), dir.display());
        launch("cmd.exe", &format!("/c {script}"), SW_HIDE);
    }
}

/// Starts a program that outlives this process. Goes through the shell rather than
/// `std::process::Command`, which lets the child inherit this process's handles, including a caller's
/// stdout pipe (then e.g. `install.ps1 | Select-String …` would wait until the HUD exits).
fn launch(file: &str, params: &str, show: SHOW_WINDOW_CMD) -> bool {
    let params = HSTRING::from(params);
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(file),
            if params.is_empty() { PCWSTR::null() } else { PCWSTR(params.as_ptr()) },
            PCWSTR::null(),
            show,
        )
    };
    r.0 as isize > 32
}

fn install(current: &Path, target: &Path) -> Result<(), String> {
    // An older copy may be running from the install folder: ask it to quit so the file is free.
    instance::request_exit_and_wait(Duration::from_secs(8));
    let dir = target.parent().ok_or("no install folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    // The old process can hold the file for a moment after it exits.
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::copy(current, target) {
            Ok(_) => {
                last.clear();
                break;
            }
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if !last.is_empty() {
        return Err(format!("copy to {}: {last}", target.display()));
    }
    register(target);
    if let Err(e) = create_shortcut(target) {
        storage::log(format!("could not create the Start menu shortcut: {e}"));
    }
    // "Open at login" may still point at the downloaded copy.
    startup::repoint(target);
    storage::log(format!("installed {} to {}", env!("CARGO_PKG_VERSION"), target.display()));
    Ok(())
}

/// `%LOCALAPPDATA%\Programs\LifeHUD\lifehud.exe`
fn installed_exe() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("Programs").join("LifeHUD").join(EXE))
}

fn shortcut_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs").join(SHORTCUT))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
    }
}

/// `…\target\debug\lifehud.exe` or `…\target\release\lifehud.exe`: a developer's build.
fn is_dev_build(exe: &Path) -> bool {
    let name = |p: Option<&Path>| p.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase());
    let profile = name(exe.parent());
    matches!(profile.as_deref(), Some("debug" | "release"))
        && name(exe.parent().and_then(Path::parent)).as_deref() == Some("target")
}

fn is_portable(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| dir.join("portable").exists() || dir.join("portable.txt").exists())
}

/// The Settings → Apps entry (per user, so no admin rights are needed).
fn register(exe: &Path) {
    let dir = exe.parent().unwrap_or(exe);
    let size_kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    let strings = [
        ("DisplayName", "Life HUD".to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "Life HUD".to_string()),
        ("DisplayIcon", format!("{},0", exe.display())),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{}\" --uninstall", exe.display())),
        ("QuietUninstallString", format!("\"{}\" --uninstall --quiet", exe.display())),
        ("URLInfoAbout", env!("CARGO_PKG_REPOSITORY").to_string()),
    ];
    let dwords = [("NoModify", 1u32), ("NoRepair", 1), ("EstimatedSize", size_kb)];
    unsafe {
        let mut key = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(UNINSTALL_KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            storage::log(format!("could not register in Settings → Apps: {status:?}"));
            return;
        }
        for (name, value) in strings {
            let wide = crate::platform::to_wide(&value);
            // SAFETY: a u16 slice viewed as bytes, valid for the duration of the call.
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes));
        }
        for (name, value) in dwords {
            let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_DWORD, Some(&value.to_le_bytes()));
        }
        let _ = RegCloseKey(key);
    }
}

fn create_shortcut(exe: &Path) -> windows::core::Result<()> {
    let Some(lnk) = shortcut_path() else { return Ok(()) };
    let dir = exe.parent().unwrap_or(exe);
    unsafe {
        // Harmless if COM is already initialised on this thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        link.SetDescription(&HSTRING::from("Life HUD"))?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_dev_builds() {
        assert!(is_dev_build(Path::new(r"C:\src\LifeHUD\target\release\lifehud.exe")));
        assert!(is_dev_build(Path::new(r"C:\src\LifeHUD\target\debug\lifehud.exe")));
        assert!(!is_dev_build(Path::new(r"C:\Users\me\Downloads\lifehud.exe")));
        assert!(!is_dev_build(Path::new(r"C:\Users\me\AppData\Local\Programs\LifeHUD\lifehud.exe")));
    }

    #[test]
    fn relaunches_through_the_shell() {
        // std::process::Command lets the child inherit a caller's stdout pipe, which made a piped
        // `install.ps1` hang until the HUD exited. Relaunches must go through `launch` (ShellExecute).
        let src = include_str!("install.rs");
        let spawns = src.matches(concat!("process::", "Command::new")).count();
        assert_eq!(spawns, 0, "use launch(), not std::process::Command, to start long-lived processes");
    }

    #[test]
    fn installs_under_local_programs() {
        let exe = installed_exe().unwrap();
        assert!(exe.ends_with(r"Programs\LifeHUD\lifehud.exe"), "{}", exe.display());
    }
}
