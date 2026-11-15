//! Per-app volume for the music player: the player's own audio session in the Windows mixer
//! (Core Audio's `ISimpleAudioVolume`), never the system volume. Moving a "Spotify" slider should
//! only ever change Spotify.
//!
//! The media service reports which app is playing (its SMTC app id). This worker finds that app's
//! audio session by process name, caches the match, and reports the level back. When there's no
//! session to control (nothing playing, or an app whose id can't be matched) it reports `None` and
//! the slider hides.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{Interface, PWSTR};

pub enum Msg {
    /// The music player's SMTC app id ("Spotify.exe", a Store app id, or "" when nothing plays).
    Source(String),
    /// Set the player's volume (0..1).
    Set(f32),
}

/// Re-read the level this often while something plays, so changes made in the Windows mixer show.
const POLL: Duration = Duration::from_secs(2);
/// Look again this often when the app has no session yet (it may not have played a sound yet).
const RETRY: Duration = Duration::from_secs(5);
/// Re-match now and then even when matched: the default output device can change (headphones).
const REMATCH: Duration = Duration::from_secs(30);

struct Session {
    control: IAudioSessionControl,
    volume: ISimpleAudioVolume,
    at: Instant,
}

impl Session {
    fn usable(&self) -> bool {
        self.at.elapsed() < REMATCH && unsafe { self.control.GetState() }.is_ok_and(|s| s != AudioSessionStateExpired)
    }
}

/// Starts the worker. `sink` gets the player's volume whenever it changes, or `None` when there's
/// nothing to control.
pub fn spawn(sink: impl Fn(Option<f32>) + Send + 'static) -> Sender<Msg> {
    let (tx, rx) = mpsc::channel::<Msg>();
    std::thread::Builder::new()
        .name("volume".into())
        .spawn(move || {
            crate::platform::init_worker_thread();
            let mut source = String::new();
            let mut session: Option<Session> = None;
            let mut tried: Option<Instant> = None;
            let mut reported: Option<Option<f32>> = None;
            loop {
                let timeout = if source.is_empty() { Duration::from_secs(3600) } else { POLL };
                let mut msgs = Vec::new();
                match rx.recv_timeout(timeout) {
                    Ok(m) => msgs.push(m),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                msgs.extend(rx.try_iter());
                // A drag sends a burst of levels; only the last one matters.
                let mut set = None;
                for m in msgs {
                    match m {
                        Msg::Source(s) if s != source => {
                            source = s;
                            session = None;
                            tried = None;
                        }
                        Msg::Source(_) => {}
                        Msg::Set(v) => set = Some(v.clamp(0.0, 1.0)),
                    }
                }
                if session.as_ref().is_some_and(|s| !s.usable()) {
                    session = None;
                    tried = None;
                }
                if session.is_none() && !source.is_empty() && tried.is_none_or(|t| t.elapsed() >= RETRY) {
                    tried = Some(Instant::now());
                    session = find(&source).ok().flatten();
                }
                if let (Some(s), Some(v)) = (&session, set) {
                    let _ = unsafe { s.volume.SetMasterVolume(v, std::ptr::null()) };
                }
                let level = session
                    .as_ref()
                    .and_then(|s| unsafe { s.volume.GetMasterVolume() }.ok())
                    .map(|v| (v * 1000.0).round() / 1000.0);
                if reported != Some(level) {
                    reported = Some(level);
                    sink(level);
                }
            }
        })
        .expect("spawn volume thread");
    tx
}

/// The audio session on the default output device that belongs to `app_id`'s process, preferring
/// one that's currently making sound.
fn find(app_id: &str) -> windows::core::Result<Option<Session>> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        let mut best: Option<(bool, IAudioSessionControl)> = None;
        for i in 0..sessions.GetCount()? {
            let Ok(control) = sessions.GetSession(i) else { continue };
            let Ok(pid) = control.cast::<IAudioSessionControl2>().and_then(|c| c.GetProcessId()) else { continue };
            if pid == 0 || !process_path(pid).is_some_and(|p| matches(app_id, &p)) {
                continue;
            }
            let state = control.GetState().unwrap_or(AudioSessionStateExpired);
            if state == AudioSessionStateExpired {
                continue;
            }
            let active = state == AudioSessionStateActive;
            if best.as_ref().is_none_or(|(was_active, _)| active && !was_active) {
                best = Some((active, control));
            }
        }
        Ok(best.and_then(|(_, control)| {
            let volume = control.cast::<ISimpleAudioVolume>().ok()?;
            Some(Session { control, volume, at: Instant::now() })
        }))
    }
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Whether the process at `exe_path` is the app with SMTC id `app_id`. Desktop apps report their
/// exe ("Spotify.exe"); Store apps a package id whose parts name the app
/// ("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"); browsers a short name ("MSEdge", "Chrome").
/// Whole parts only, so "Spot.exe" doesn't match Spotify.
pub fn matches(app_id: &str, exe_path: &str) -> bool {
    let app = app_id.to_ascii_lowercase();
    let file = exe_path.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
    let stem = file.strip_suffix(".exe").unwrap_or(&file);
    if stem.len() < 3 {
        return false;
    }
    app == file || app == stem || app.split(['!', '.', '_']).any(|part| part == stem)
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn matches_players_by_process() {
        let spotify = r"C:\Users\x\AppData\Roaming\Spotify\Spotify.exe";
        assert!(matches("Spotify.exe", spotify));
        assert!(matches("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", r"C:\Program Files\WindowsApps\x\Spotify.exe"));
        assert!(matches("MSEdge", r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"));
        assert!(matches("Chrome", r"C:\Program Files\Google\Chrome\Application\chrome.exe"));
        assert!(!matches("Spotify.exe", r"C:\x\Spot.exe"), "whole names only");
        assert!(!matches("Spotify.exe", r"C:\x\msedge.exe"));
        assert!(!matches("308046B0AF4A39CB", r"C:\x\firefox.exe"), "unmatchable ids hide the slider");
        assert!(!matches("", spotify));
        assert!(!matches("a!b", r"C:\x\b.exe"), "too short to trust");
    }

    #[test]
    #[ignore] // touches the real audio stack
    fn finds_nothing_for_an_unknown_app_without_failing() {
        crate::platform::init_worker_thread();
        assert!(super::find("NoSuchPlayer.exe").unwrap().is_none());
    }
}
