//! "Now playing" from Windows' System Media Transport Controls. Spotify desktop publishes its
//! track, artwork, timeline and controls there, so no sign-in or Web API is needed.
//!
//! Browsers publish their audio/video there too. A browser session that belongs to a YouTube or
//! Netflix tab becomes "Now watching" (for those widgets) instead of the Music card's track.

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
    /// Jump to this fraction (0..1) of the current track.
    Seek(f32),
    PreferSpotify(bool),
    /// Play/pause the YouTube or Netflix video playing in a browser.
    WatchPlayPause,
    /// Jump to this fraction (0..1) of that video.
    WatchSeek(f32),
    /// The HUD was shown or hidden. Hidden, the poller parks until shown again (then reads at once).
    Visible(bool),
}

/// Which site a browser video belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    YouTube,
    Netflix,
}

/// A YouTube or Netflix video playing in a browser tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Watching {
    pub site: Site,
    pub title: String,
    /// The channel on YouTube (empty when the page doesn't say).
    pub subtitle: String,
    /// "Chrome", "Edge", …
    pub browser: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub can_seek: bool,
}

/// A video thumbnail, decoded at 16:9.
#[derive(Clone)]
pub struct Thumb {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub enum ThumbChange {
    Keep,
    Clear,
    Set(Thumb),
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
    /// The player accepts position changes (Spotify and most browsers do).
    pub can_seek: bool,
    pub art: ArtChange,
    /// The music player's SMTC app id (e.g. `Spotify.exe` or a Store app id), for per-app volume.
    pub source_app: String,
    /// A YouTube or Netflix video playing in a browser, if any.
    pub watching: Option<Watching>,
    pub watch_thumb: ThumbChange,
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
            let mut last: Option<(Option<Track>, bool, u64, bool, bool, bool)> = None;
            let mut playing = false;
            let mut thumb_key = String::new();
            // The browser session currently treated as "now watching", by app id.
            let mut watch_app: Option<String> = None;
            let mut last_watch: Option<(Site, String, bool, u64, bool)> = None;
            let mut titles = TitleCache::default();
            let mut visible = true;

            loop {
                // Poll faster while playing (progress), slower when idle, not at all while the HUD
                // is hidden (nothing shows it). Commands wake us at once.
                let wait = match (visible, playing) {
                    (false, _) => Duration::from_secs(3600),
                    (true, true) => Duration::from_millis(1000),
                    (true, false) => Duration::from_millis(2500),
                };
                match rx.recv_timeout(wait) {
                    Ok(Command::Visible(v)) => {
                        visible = v;
                        if !visible {
                            continue;
                        }
                    }
                    Ok(cmd) => {
                        let session = manager.as_ref().and_then(|m| pick(m, prefer_spotify, watch_app.as_deref()));
                        let watched = manager.as_ref().zip(watch_app.as_deref()).and_then(|(m, id)| by_app(m, id));
                        match cmd {
                            Command::PreferSpotify(v) => prefer_spotify = v,
                            Command::PlayPause => {
                                run(session.and_then(|s| s.TryTogglePlayPauseAsync().ok()).map(|op| op.join()))
                            }
                            Command::Next => run(session.and_then(|s| s.TrySkipNextAsync().ok()).map(|op| op.join())),
                            Command::Previous => {
                                run(session.and_then(|s| s.TrySkipPreviousAsync().ok()).map(|op| op.join()))
                            }
                            Command::Seek(fraction) => run(session.and_then(|s| seek(&s, fraction))),
                            Command::WatchPlayPause => {
                                run(watched.and_then(|s| s.TryTogglePlayPauseAsync().ok()).map(|op| op.join()))
                            }
                            Command::WatchSeek(fraction) => run(watched.and_then(|s| seek(&s, fraction))),
                            Command::Visible(_) => unreachable!("handled above"),
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
                let (watching, watch_thumb, app) = match manager.as_ref() {
                    Some(m) => find_watching(m, &mut thumb_key, &mut titles),
                    None => (None, ThumbChange::Keep, None),
                };
                watch_app = app;
                let session = manager.as_ref().and_then(|m| pick(m, prefer_spotify, watch_app.as_deref()));
                let mut update = read(session.as_ref(), &mut art_key);
                // Keep polling briskly while either the music or the video is playing.
                playing = update.playing || watching.as_ref().is_some_and(|w| w.playing);
                let watch_key =
                    watching.as_ref().map(|w| (w.site, w.title.clone(), w.playing, w.position_ms / 1000, w.can_seek));
                let watch_changed = last_watch != watch_key || !matches!(watch_thumb, ThumbChange::Keep);
                last_watch = watch_key;
                update.watching = watching;
                update.watch_thumb = watch_thumb;

                let key = (
                    update.track.clone(),
                    update.playing,
                    update.position_ms / 1000,
                    update.can_previous,
                    update.can_next,
                    update.can_seek,
                );
                if last.as_ref() != Some(&key) || !matches!(update.art, ArtChange::Keep) || watch_changed {
                    last = Some(key);
                    sink(update);
                }
            }
        })
        .expect("spawn media thread");
    tx
}

/// Moves playback to `fraction` of the current track, if the player allows it.
fn seek(s: &Session, fraction: f32) -> Option<windows::core::Result<bool>> {
    let enabled = s.GetPlaybackInfo().and_then(|i| i.Controls()).and_then(|c| c.IsPlaybackPositionEnabled());
    if !enabled.unwrap_or(false) {
        return None;
    }
    let timeline = s.GetTimelineProperties().ok()?;
    let (start, end) = (timeline.StartTime().ok()?.Duration, timeline.EndTime().ok()?.Duration);
    if end <= start {
        return None;
    }
    let target = seek_target(start, end, fraction);
    Some(s.TryChangePlaybackPositionAsync(target).and_then(|op| op.join()))
}

/// The timeline position (100 ns ticks) for a fraction of the track, kept inside the track.
fn seek_target(start: i64, end: i64, fraction: f32) -> i64 {
    let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) as f64 } else { 0.0 };
    start + ((end - start) as f64 * fraction) as i64
}

fn run(result: Option<windows::core::Result<bool>>) {
    if let Some(Err(e)) = result {
        storage::log(format!("media command failed: {e}"));
    }
}

fn app_id(s: &Session) -> String {
    s.SourceAppUserModelId().map(|id| id.to_string()).unwrap_or_default()
}

/// The session for the Music card. A browser session that's "now watching" belongs to the YouTube or
/// Netflix widget instead, so it's skipped.
fn pick(manager: &Manager, prefer_spotify: bool, exclude: Option<&str>) -> Option<Session> {
    if prefer_spotify && let Ok(sessions) = manager.GetSessions() {
        for s in sessions {
            if is_spotify(&app_id(&s)) {
                return Some(s);
            }
        }
    }
    let current = manager.GetCurrentSession().ok();
    match (current, exclude) {
        (Some(s), Some(x)) if app_id(&s) == x => manager.GetSessions().ok()?.into_iter().find(|s| app_id(s) != x),
        (current, _) => current,
    }
}

fn by_app(manager: &Manager, id: &str) -> Option<Session> {
    manager.GetSessions().ok()?.into_iter().find(|s| app_id(s) == id)
}

fn is_browser_app(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    // Firefox registers under a fixed hash rather than its name.
    ["chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "308046b0af4a39cb"].iter().any(|b| id.contains(b))
}

/// Which site a browser's media belongs to, from its title and the titles of open browser windows
/// (each shows its active tab). `None` when it can't be told, so nothing is shown rather than a guess.
pub fn classify(media_title: &str, windows: &[&str]) -> Option<Site> {
    let t = media_title.trim();
    let youtube = |w: &str| w.contains("YouTube");
    let netflix = |w: &str| w.starts_with("Netflix") || w.contains(" - Netflix") || w.contains("| Netflix");
    if !t.is_empty() {
        if windows.iter().any(|w| youtube(w) && w.contains(t)) {
            return Some(Site::YouTube);
        }
        if windows.iter().any(|w| netflix(w) && w.contains(t)) {
            return Some(Site::Netflix);
        }
        // The media's own page is showing and it's some other site (a music site, a podcast…).
        if windows.iter().any(|w| w.contains(t)) {
            return None;
        }
    }
    // Netflix's player tab is titled just "Netflix" while the media title is the show's name.
    windows.iter().any(|w| netflix(w)).then_some(Site::Netflix)
}

/// Browser window titles, reused between polls: listing every window and its process each second
/// would be wasted work, since tabs rarely change while a video plays. Refreshed after a few seconds,
/// or at once when the browser's media changes.
#[derive(Default)]
struct TitleCache {
    titles: Vec<String>,
    taken: Option<std::time::Instant>,
    /// Media (app id + title) already looked up against this snapshot. A set rather than the last
    /// one: with several browser sessions the lookups alternate, which must not force a rescan
    /// (every window, every process) on each one.
    media: std::collections::HashSet<String>,
}

impl TitleCache {
    const FRESH: Duration = Duration::from_secs(5);

    fn get(&mut self, media: &str) -> &[String] {
        let stale = self.taken.is_none_or(|t| t.elapsed() >= Self::FRESH);
        if stale || !self.media.contains(media) {
            self.titles = crate::platform::browsers::windows().into_iter().map(|w| w.title).collect();
            self.taken = Some(std::time::Instant::now());
            self.media.clear();
        }
        self.media.insert(media.to_string());
        &self.titles
    }
}

/// The YouTube or Netflix video playing in a browser, its thumbnail change, and its session's app id.
fn find_watching(
    manager: &Manager,
    thumb_key: &mut String,
    cache: &mut TitleCache,
) -> (Option<Watching>, ThumbChange, Option<String>) {
    let clear = |key: &mut String| {
        if key.is_empty() {
            ThumbChange::Keep
        } else {
            key.clear();
            ThumbChange::Clear
        }
    };
    let browser_sessions: Vec<(Session, String)> = match manager.GetSessions() {
        Ok(list) => list
            .into_iter()
            .map(|s| (app_id(&s), s))
            .filter(|(id, _)| is_browser_app(id))
            .map(|(id, s)| (s, id))
            .collect(),
        Err(_) => Vec::new(),
    };
    if browser_sessions.is_empty() {
        return (None, clear(thumb_key), None);
    }
    for (s, id) in browser_sessions {
        let Ok(props) = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()) else { continue };
        let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
        let titles: Vec<&str> = cache.get(&format!("{id}\u{1f}{title}")).iter().map(String::as_str).collect();
        let Some(site) = classify(&title, &titles) else { continue };
        let Ok(info) = s.GetPlaybackInfo() else { continue };
        let playing = info.PlaybackStatus().map(|st| st == Status::Playing).unwrap_or(false);
        let can_seek = info.Controls().and_then(|c| c.IsPlaybackPositionEnabled()).unwrap_or(false);
        let (position, duration) = timeline(&s, playing).unwrap_or((0, 0));
        let key = format!("{site:?}\u{1f}{title}");
        let thumb = if key != *thumb_key {
            match props.Thumbnail().ok().and_then(|t| decode_thumb(&t).ok()) {
                Some(t) => {
                    *thumb_key = key;
                    ThumbChange::Set(t)
                }
                None => clear(thumb_key),
            }
        } else {
            ThumbChange::Keep
        };
        let watching = Watching {
            site,
            title: if title.is_empty() { "Netflix".into() } else { title },
            subtitle: props.Artist().map(|a| a.to_string()).unwrap_or_default(),
            browser: friendly_source(&id),
            playing,
            position_ms: (position / 10_000) as u64,
            duration_ms: (duration / 10_000) as u64,
            can_seek,
        };
        return (Some(watching), thumb, Some(id));
    }
    (None, clear(thumb_key), None)
}

/// (position, duration) in 100 ns ticks, extrapolating the position while playing.
fn timeline(s: &Session, playing: bool) -> windows::core::Result<(i64, i64)> {
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
    Ok((position.clamp(0, duration), duration))
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
        ("308046b0af4a39cb", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vivaldi", "Vivaldi"),
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
        can_seek: false,
        art: ArtChange::Keep,
        source_app: String::new(),
        watching: None,
        watch_thumb: ThumbChange::Keep,
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

        let (position, duration) = timeline(s, playing)?;

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
            can_seek: controls.IsPlaybackPositionEnabled().unwrap_or(false),
            art,
            source_app: app_id,
            watching: None,
            watch_thumb: ThumbChange::Keep,
        })
    })();

    result.unwrap_or(empty)
}

fn decode_thumb(thumb: &windows::Storage::Streams::IRandomAccessStreamReference) -> windows::core::Result<Thumb> {
    const W: u32 = 192;
    const H: u32 = 108;
    let rgba = decode_rgba(thumb, W, H)?;
    Ok(Thumb { rgba, width: W, height: H })
}

fn decode_rgba(
    thumb: &windows::Storage::Streams::IRandomAccessStreamReference,
    width: u32,
    height: u32,
) -> windows::core::Result<Vec<u8>> {
    let stream: IRandomAccessStream = thumb.OpenReadAsync()?.join()?.cast()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(width)?;
    transform.SetScaledHeight(height)?;
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
    if pixels.len() != (width * height * 4) as usize {
        return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED));
    }
    Ok(pixels)
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

    #[test]
    fn classifies_browser_media_by_window_titles() {
        use super::{Site, classify};
        let windows = ["(91) wow forever - YouTube - Google Chrome", "YouTube", "Inbox - Outlook"];
        assert_eq!(classify("wow forever", &windows), Some(Site::YouTube));
        // Media from some other page: not shown as YouTube just because a YouTube tab is open.
        assert_eq!(classify("Gemini Notebook", &windows), None);
        assert_eq!(classify("Inbox", &["Inbox - Outlook", "Netflix"]), None);
        // Netflix's player tab is titled "Netflix"; its media title is the show.
        assert_eq!(classify("Stranger Things", &["Netflix - Google Chrome"]), Some(Site::Netflix));
        assert_eq!(classify("", &["Netflix"]), Some(Site::Netflix));
        assert_eq!(classify("Anything", &[]), None);
    }

    #[test]
    fn recognises_browsers() {
        assert!(super::is_browser_app("Chrome"));
        assert!(super::is_browser_app("MSEdge"));
        assert!(super::is_browser_app("308046B0AF4A39CB"));
        assert!(!super::is_browser_app("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"));
    }

    #[test]
    fn seek_targets_stay_inside_the_track() {
        let (start, end) = (1_000, 11_000);
        assert_eq!(super::seek_target(start, end, 0.5), 6_000);
        assert_eq!(super::seek_target(start, end, -1.0), start);
        assert_eq!(super::seek_target(start, end, 2.0), end);
        assert_eq!(super::seek_target(start, end, f32::NAN), start);
    }
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
