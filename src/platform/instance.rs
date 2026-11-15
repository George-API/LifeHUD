//! Single instance. Launching again opens Settings in the running copy; `--exit` asks it to quit.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE,
    SetEvent, WaitForMultipleObjects,
};
use windows::core::HSTRING;

const MUTEX: &str = r"Local\LifeHUD.Instance";
const ACTIVATE: &str = r"Local\LifeHUD.Activate";
const EXIT: &str = r"Local\LifeHUD.Exit";

pub enum Signal {
    Activate,
    Exit,
}

pub struct Instance {
    _mutex: HANDLE,
}

struct SendHandles([HANDLE; 2]);
// SAFETY: event handles are process-wide kernel objects, safe to wait on from any thread.
unsafe impl Send for SendHandles {}

/// Returns `None` if another instance is already running (after signalling it).
pub fn acquire(exit_requested: bool) -> Option<Instance> {
    unsafe {
        let mutex = CreateMutexW(None, true, &HSTRING::from(MUTEX)).ok()?;
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        if already || exit_requested {
            let name = HSTRING::from(if exit_requested { EXIT } else { ACTIVATE });
            if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                let _ = SetEvent(ev);
                let _ = CloseHandle(ev);
            }
            let _ = CloseHandle(mutex);
            return None;
        }
        Some(Instance { _mutex: mutex })
    }
}

/// Asks a running copy to quit (it releases its reserved screen space) and waits until it has, up to
/// `timeout`. Returns true if no copy is running any more.
pub fn request_exit_and_wait(timeout: std::time::Duration) -> bool {
    let running = || unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(MUTEX)) {
            Ok(h) => {
                let _ = CloseHandle(h);
                true
            }
            Err(_) => false,
        }
    };
    if !running() {
        return true;
    }
    unsafe {
        if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(EXIT)) {
            let _ = SetEvent(ev);
            let _ = CloseHandle(ev);
        }
    }
    let deadline = std::time::Instant::now() + timeout;
    while running() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    true
}

/// Listens for signals from later launches and forwards them to the UI thread.
pub fn listen(on_signal: impl Fn(Signal) + Send + 'static) {
    let events = unsafe {
        let a = CreateEventW(None, false, false, &HSTRING::from(ACTIVATE));
        let e = CreateEventW(None, false, false, &HSTRING::from(EXIT));
        match (a, e) {
            (Ok(a), Ok(e)) => SendHandles([a, e]),
            _ => return,
        }
    };
    std::thread::Builder::new()
        .name("instance-signals".into())
        .spawn(move || {
            let events = events;
            loop {
                let r = unsafe { WaitForMultipleObjects(&events.0, false, INFINITE) };
                if r == WAIT_OBJECT_0 {
                    on_signal(Signal::Activate);
                } else if r.0 == WAIT_OBJECT_0.0 + 1 {
                    on_signal(Signal::Exit);
                    break;
                } else {
                    break;
                }
            }
        })
        .ok();
}
