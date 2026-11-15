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

/// Loads a file of the user's own data (settings, tasks), keeping as much of it as possible.
///
/// [`load_json`] is all or nothing, which is right for caches but not for things people can't
/// re-download. Here a value that doesn't fit (a wrong type, an option from a newer version of Life
/// HUD, one damaged entry in a list) costs only that value: it falls back to its default, or the entry
/// is dropped, and everything else is kept. The original file is kept aside as `*.corrupt` and the
/// log names what was dropped. Only a file that isn't JSON at all starts fresh.
pub fn load_json_salvaged<T: Serialize + DeserializeOwned + Default>(name: &str) -> Option<T> {
    let p = path(name);
    let bytes = fs::read(&p).ok()?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    if let Ok(v) = serde_json::from_slice::<T>(bytes) {
        return Some(v);
    }
    let _ = fs::copy(&p, p.with_extension("corrupt"));
    let aside = p.with_extension("corrupt").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Ok(file) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        log(format!("could not read {name} (not JSON); starting fresh. The original is kept as {aside}"));
        return None;
    };
    let (value, dropped) = salvage::<T>(file);
    match serde_json::from_value::<T>(value) {
        Ok(v) => {
            log(format!(
                "{name}: reset {} to the default; kept everything else. The original is kept as {aside}",
                dropped.join(", ")
            ));
            Some(v)
        }
        Err(e) => {
            log(format!("could not read {name}: {e}; starting fresh. The original is kept as {aside}"));
            None
        }
    }
}

/// Grafts `file` onto `T`'s defaults, one value at a time, keeping each one only if `T` still
/// parses with it. Objects are merged key by key and lists entry by entry, so damage stays local.
/// Returns the merged JSON and the paths of what was left out (e.g. `sports.teams[2]`).
pub fn salvage<T: Serialize + DeserializeOwned + Default>(file: serde_json::Value) -> (serde_json::Value, Vec<String>) {
    let fits = |v: &serde_json::Value| serde_json::from_value::<T>(v.clone()).is_ok();
    let mut root = serde_json::to_value(T::default()).unwrap_or(serde_json::Value::Null);
    let mut dropped = Vec::new();
    graft(&mut root, &mut Vec::new(), file, &fits, &mut dropped);
    (root, dropped)
}

#[derive(Clone)]
enum Step {
    Key(String),
    Index(usize),
}

fn graft(
    root: &mut serde_json::Value,
    path: &mut Vec<Step>,
    incoming: serde_json::Value,
    fits: &dyn Fn(&serde_json::Value) -> bool,
    dropped: &mut Vec<String>,
) {
    use serde_json::Value;
    let old = put(root, path, Some(incoming.clone()));
    if fits(root) {
        return;
    }
    match (incoming, old) {
        // Both objects: merge key by key onto what was there.
        (Value::Object(map), Some(Value::Object(prev))) => {
            put(root, path, Some(Value::Object(prev)));
            for (k, v) in map {
                path.push(Step::Key(k));
                graft(root, path, v, fits, dropped);
                path.pop();
            }
        }
        // A list: keep the entries that fit, in order.
        (Value::Array(items), old) => {
            put(root, path, Some(Value::Array(Vec::new())));
            if !fits(root) {
                // Not a list here at all.
                put(root, path, old);
                dropped.push(label(path));
                return;
            }
            let mut kept = 0;
            for (i, item) in items.into_iter().enumerate() {
                path.push(Step::Index(kept));
                put(root, path, Some(item));
                if fits(root) {
                    kept += 1;
                } else {
                    put(root, path, None);
                    let at = path.len() - 1;
                    path[at] = Step::Index(i);
                    dropped.push(label(path));
                }
                path.pop();
            }
        }
        (_, old) => {
            put(root, path, old);
            dropped.push(label(path));
        }
    }
}

/// Sets (or with `None`, removes) the value at `path`, returning what was there. Missing parents
/// are left alone; appending to a list works by indexing one past its end.
fn put(root: &mut serde_json::Value, path: &[Step], value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    use serde_json::Value;
    let Some((last, parents)) = path.split_last() else {
        return Some(std::mem::replace(root, value.unwrap_or(Value::Null)));
    };
    let mut at = root;
    for step in parents {
        at = match (step, at) {
            (Step::Key(k), Value::Object(m)) => m.get_mut(k)?,
            (Step::Index(i), Value::Array(a)) => a.get_mut(*i)?,
            _ => return None,
        };
    }
    match (last, at, value) {
        (Step::Key(k), Value::Object(m), Some(v)) => m.insert(k.clone(), v),
        (Step::Key(k), Value::Object(m), None) => m.remove(k),
        (Step::Index(i), Value::Array(a), Some(v)) if *i < a.len() => Some(std::mem::replace(&mut a[*i], v)),
        (Step::Index(i), Value::Array(a), Some(v)) if *i == a.len() => {
            a.push(v);
            None
        }
        (Step::Index(i), Value::Array(a), None) if *i < a.len() => Some(a.remove(*i)),
        _ => None,
    }
}

fn label(path: &[Step]) -> String {
    let mut out = String::new();
    for step in path {
        match step {
            Step::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Step::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    if out.is_empty() { "the whole file".into() } else { out }
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
