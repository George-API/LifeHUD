//! "Now playing" from Windows' System Media Transport Controls. Spotify desktop publishes its
//! track, artwork, timeline and controls there, so no sign-in or Web API is needed.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
    ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::IRandomAccessStream;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::core::Interface;

use crate::storage;

const ART_SIZE: u32 = 160;

pub enum Command {
    PlayPause,
    Next,
    Previous,
    PreferSpotify(bool),
}

#[derive(Clone)]
pub struct Artwork {
    pub rgba: Vec<u8>,
    pub size: u32,
    /// A representative colour for the card glow, if the art is colourful enough.
    pub glow: Option<(u8, u8, u8)>,
}

#[derive(Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub source: String,
    pub is_spotify: bool,
}

#[derive(Clone)]
pub enum ArtChange {
    Keep,
    Clear,
    Set(Artwork),
}

#[derive(Clone)]
pub struct Update {
    pub track: Option<Track>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub can_previous: bool,
    pub can_next: bool,
    pub art: ArtChange,
}

pub fn spawn(prefer_spotify: bool, sink: impl Fn(Update) + Send + 'static) -> Sender<Command> {
    let (tx, rx) = mpsc::channel::<Command>();
    std::thread::Builder::new()
        .name("media".into())
        .spawn(move || {
            crate::platform::init_worker_thread();
            let mut prefer_spotify = prefer_spotify;
            let mut manager: Option<Manager> = None;
            let mut art_key = String::new();
            let mut last: Option<(Option<Track>, bool, u64, bool, bool)> = None;
            let mut playing = false;

            loop {
                // Poll faster while playing (progress), slower when idle. Commands wake us at once.
                let wait = if playing { Duration::from_millis(1000) } else { Duration::from_millis(2500) };
                match rx.recv_timeout(wait) {
                    Ok(cmd) => {
                        let session = manager.as_ref().and_then(|m| pick(m, prefer_spotify));
                        match cmd {
                            Command::PreferSpotify(v) => prefer_spotify = v,
                            Command::PlayPause => {
                                run(session.and_then(|s| s.TryTogglePlayPauseAsync().ok()).map(|op| op.join()))
                            }
                            Command::Next => run(session.and_then(|s| s.TrySkipNextAsync().ok()).map(|op| op.join())),
                            Command::Previous => {
                                run(session.and_then(|s| s.TrySkipPreviousAsync().ok()).map(|op| op.join()))
                            }
                        }
                        std::thread::sleep(Duration::from_millis(120)); // let the player publish its new state
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }

                if manager.is_none() {
                    manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
                    if manager.is_none() {
                        continue;
                    }
                }
                let session = manager.as_ref().and_then(|m| pick(m, prefer_spotify));
                let update = read(session.as_ref(), &mut art_key);
                playing = update.playing;

                let key = (
                    update.track.clone(),
                    update.playing,
                    update.position_ms / 1000,
                    update.can_previous,
                    update.can_next,
                );
                if last.as_ref() != Some(&key) || !matches!(update.art, ArtChange::Keep) {
                    last = Some(key);
                    sink(update);
                }
            }
        })
        .expect("spawn media thread");
    tx
}

fn run(result: Option<windows::core::Result<bool>>) {
    if let Some(Err(e)) = result {
        storage::log(format!("media command failed: {e}"));
    }
}

fn pick(manager: &Manager, prefer_spotify: bool) -> Option<Session> {
    if prefer_spotify && let Ok(sessions) = manager.GetSessions() {
        for s in sessions {
            if s.SourceAppUserModelId().map(|id| is_spotify(&id.to_string())).unwrap_or(false) {
                return Some(s);
            }
        }
    }
    manager.GetCurrentSession().ok()
}

fn is_spotify(app_id: &str) -> bool {
    app_id.to_ascii_lowercase().contains("spotify")
}

fn friendly_source(app_id: &str) -> String {
    let id = app_id.to_ascii_lowercase();
    let name = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("zunemusic", "Media Player"),
        ("applemusic", "Apple Music"),
        ("itunes", "Apple Music"),
    ]
    .iter()
    .find(|(k, _)| id.contains(k))
    .map(|(_, v)| *v)
    .unwrap_or("Now Playing");
    name.to_string()
}

fn filetime_now() -> i64 {
    let ft = unsafe { GetSystemTimePreciseAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn read(session: Option<&Session>, art_key: &mut String) -> Update {
    let empty = Update {
        track: None,
        playing: false,
        position_ms: 0,
        duration_ms: 0,
        can_previous: false,
        can_next: false,
        art: ArtChange::Keep,
    };
    let Some(s) = session else {
        let art = if art_key.is_empty() {
            ArtChange::Keep
        } else {
            art_key.clear();
            ArtChange::Clear
        };
        return Update { art, ..empty };
    };

    let result = (|| -> windows::core::Result<Update> {
        let app_id = s.SourceAppUserModelId()?.to_string();
        let props = s.TryGetMediaPropertiesAsync()?.join()?;
        let title = props.Title()?.to_string();
        let mut artist = props.Artist()?.to_string();
        if artist.is_empty() {
            artist = props.AlbumArtist().map(|a| a.to_string()).unwrap_or_default();
        }

        let info = s.GetPlaybackInfo()?;
        let playing = info.PlaybackStatus()? == Status::Playing;
        let controls = info.Controls()?;

        let timeline = s.GetTimelineProperties()?;
        let start = timeline.StartTime()?.Duration;
        let end = timeline.EndTime()?.Duration;
        let mut position = timeline.Position()?.Duration - start;
        if playing {
            // Players publish the timeline sporadically; extrapolate from when it was last updated.
            let since = filetime_now() - timeline.LastUpdatedTime()?.UniversalTime;
            if since > 0 && since < 6 * 3600 * 10_000_000 {
                position += since;
            }
        }
        let duration = (end - start).max(0);
        let position = position.clamp(0, duration.max(0));

        let key = format!("{title}\u{1f}{artist}");
        let art = if key != *art_key {
            match props.Thumbnail().ok().and_then(|t| decode_art(&t).ok()) {
                Some(a) => {
                    *art_key = key;
                    ArtChange::Set(a)
                }
                // Some players publish text before artwork: clear once, then retry quietly each poll.
                None if art_key.is_empty() => ArtChange::Keep,
                None => {
                    art_key.clear();
                    ArtChange::Clear
                }
            }
        } else {
            ArtChange::Keep
        };

        let has_track = !title.is_empty() || !artist.is_empty();
        Ok(Update {
            track: has_track.then(|| Track {
                title,
                artist,
                source: friendly_source(&app_id),
                is_spotify: is_spotify(&app_id),
            }),
            playing,
            position_ms: (position / 10_000) as u64,
            duration_ms: (duration / 10_000) as u64,
            can_previous: controls.IsPreviousEnabled().unwrap_or(false),
            can_next: controls.IsNextEnabled().unwrap_or(false),
            art,
        })
    })();

    result.unwrap_or(empty)
}

fn decode_art(thumb: &windows::Storage::Streams::IRandomAccessStreamReference) -> windows::core::Result<Artwork> {
    let stream: IRandomAccessStream = thumb.OpenReadAsync()?.join()?.cast()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(ART_SIZE)?;
    transform.SetScaledHeight(ART_SIZE)?;
    transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    let pixels = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Straight,
            &transform,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?
        .DetachPixelData()?
        .to_vec();
    if pixels.len() != (ART_SIZE * ART_SIZE * 4) as usize {
        return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED));
    }
    let glow = glow_color(&pixels);
    Ok(Artwork { rgba: pixels, size: ART_SIZE, glow })
}

/// Weighted average favouring vivid, mid-bright pixels; `None` for near-grey art.
pub fn glow_color(rgba: &[u8]) -> Option<(u8, u8, u8)> {
    let (mut r, mut g, mut b, mut total) = (0.0, 0.0, 0.0, 0.0);
    for px in rgba.as_chunks::<4>().0.iter().step_by(7) {
        let (pr, pg, pb) = (px[0] as f64, px[1] as f64, px[2] as f64);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
        let lum = max / 255.0;
        let w = 0.05 + sat * sat * (1.0 - (lum - 0.6).abs());
        r += pr * w;
        g += pg * w;
        b += pb * w;
        total += w;
    }
    if total <= 0.0 {
        return None;
    }
    let (r, g, b) = ((r / total) as u8, (g / total) as u8, (b / total) as u8);
    let hi = r.max(g).max(b) as f64;
    let lo = r.min(g).min(b) as f64;
    (hi > 0.0 && (hi - lo) / hi >= 0.18).then_some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_art_has_no_glow() {
        let grey = [128u8, 128, 128, 255].repeat(64);
        assert!(glow_color(&grey).is_none());
    }

    #[test]
    fn colourful_art_glows() {
        let red = [220u8, 40, 40, 255].repeat(64);
        let (r, g, _) = glow_color(&red).unwrap();
        assert!(r > g);
    }

    #[test]
    fn source_names() {
        assert_eq!(friendly_source("Spotify.exe"), "Spotify");
        assert_eq!(friendly_source("something.else"), "Now Playing");
        assert!(is_spotify("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"));
    }
}
