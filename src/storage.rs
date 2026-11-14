//! Local persistence: everything lives in `%LOCALAPPDATA%\LifeHUD`. Writes are atomic
//! (write a temp file, then rename over the original), so a crash never leaves a torn file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn data_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = base.join("LifeHUD");
        let _ = fs::create_dir_all(&dir);
        dir
    })
}

pub fn path(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// Loads JSON, or `None` if the file is missing. A corrupt file is kept aside as `*.corrupt`.
pub fn load_json<T: DeserializeOwned>(name: &str) -> Option<T> {
    let p = path(name);
    let bytes = fs::read(&p).ok()?;
    // Tolerate a UTF-8 byte-order mark (Notepad and PowerShell add one).
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    match serde_json::from_slice(bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            log(format!("could not parse {name}: {e}; starting fresh"));
            let _ = fs::copy(&p, p.with_extension("corrupt"));
            None
        }
    }
}

pub fn save_json<T: Serialize>(name: &str, value: &T) {
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => write_atomic(name, &bytes),
        Err(e) => log(format!("could not serialise {name}: {e}")),
    }
}

pub fn read_text(name: &str) -> Option<String> {
    fs::read_to_string(path(name)).ok().map(|s| s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s))
}

pub fn write_atomic(name: &str, bytes: &[u8]) {
    let target = path(name);
    let tmp = target.with_extension("tmp");
    let result = fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, &target));
    if let Err(e) = result {
        log(format!("could not save {name}: {e}"));
    }
}

/// Small rolling log for diagnostics. Never panics.
pub fn log(message: impl AsRef<str>) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock();
    let p = path("lifehud.log");
    if fs::metadata(&p).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = fs::remove_file(&p);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {}", crate::locale::timestamp(), message.as_ref());
    }
    #[cfg(debug_assertions)]
    eprintln!("{}", message.as_ref());
}
