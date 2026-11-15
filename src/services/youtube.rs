//! New videos from the channels you follow, plus a "Watch Later" list, without an account or API
//! key. Each channel's public RSS feed is tried first (light, exact timestamps); YouTube serves
//! those unreliably for some channels, so the channel's Videos page is the fallback (it also gives
//! durations). Shorts are left out. Thumbnails come from YouTube's image CDN.

use std::time::{Duration, Instant};

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::services::{http, images, news, poller};
use crate::storage;

const CACHE: &str = "youtube-cache.json";
const REFRESH: Duration = Duration::from_secs(30 * 60);
const RETRY: Duration = Duration::from_secs(5 * 60);
const MAX_VIDEOS: usize = 24;
const PER_CHANNEL: usize = 8;
/// A channel whose feed is down falls back to its ~1.3 MB Videos page at most this often.
const PAGE_EVERY: Duration = Duration::from_secs(2 * 3600);
/// Channel feeds fetched at once.
const CHANNELS_AT_ONCE: usize = 4;
pub const MAX_CHANNELS: usize = 20;
pub const MAX_SAVED: usize = 30;
/// 16:9 at 2× the Sidebar's 112px-wide thumbnail.
pub const THUMB: (u32, u32) = (224, 126);
const LANG: [(&str, &str); 1] = [("Accept-Language", "en-US,en;q=0.8")];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// "UC…" channel id.
    pub id: String,
    pub name: String,
    /// "@handle", when known.
    #[serde(default)]
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedVideo {
    pub id: String,
    pub title: String,
    pub channel: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub channels: Vec<Channel>,
    pub saved: Vec<SavedVideo>,
    /// Play in the widget itself (a docked Edge window), when Edge is available.
    pub in_widget: bool,
    /// Otherwise: a small pop-out player window instead of a browser tab.
    pub popout: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { channels: Vec::new(), saved: Vec::new(), in_widget: true, popout: true }
    }
}

impl Prefs {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.channels.retain(|c| is_channel_id(&c.id) && seen.insert(c.id.clone()));
        self.channels.truncate(MAX_CHANNELS);
        let mut seen = std::collections::HashSet::new();
        self.saved.retain(|v| is_video_id(&v.id) && seen.insert(v.id.clone()));
        self.saved.truncate(MAX_SAVED);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    /// Unix seconds (approximate when only "3 days ago" is known).
    pub published: Option<i64>,
    pub views: Option<u64>,
    /// "15:14", when known.
    #[serde(default)]
    pub duration: String,
    /// The followed channel this came from, so a channel that fails to refresh keeps its videos.
    #[serde(default)]
    pub channel_id: String,
}

pub fn thumb_url(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}

pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

pub enum Update {
    Videos(Vec<Video>),
    /// Thumbnails by video id, for both tabs.
    Thumbs(Vec<(String, images::Shared)>),
    Offline,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    channels: Vec<String>,
    videos: Vec<Video>,
}

pub fn cached(prefs: &Prefs) -> Option<Vec<Video>> {
    let c: Cache = storage::load_json(CACHE)?;
    (c.channels == channel_ids(prefs)).then_some(c.videos)
}

fn channel_ids(prefs: &Prefs) -> Vec<String> {
    prefs.channels.iter().map(|c| c.id.clone()).collect()
}

pub fn spawn(prefs: Prefs, sink: impl Fn(Update) + Send + 'static) -> poller::Poller<Prefs> {
    let mut fetcher = Fetcher::new();
    poller::spawn("youtube", prefs, move |prefs: &Prefs| fetcher.run(prefs, &sink))
}

pub struct Fetcher {
    thumbs: images::Cache,
    /// Channels are refetched on their own clock; a Watch Later edit only needs thumbnails.
    fetched: Option<(Instant, Vec<String>)>,
    videos: Vec<Video>,
    /// When each channel last needed its heavy page fallback.
    page_at: std::collections::HashMap<String, Instant>,
}

impl Fetcher {
    pub fn new() -> Self {
        Self {
            thumbs: images::Cache::new(THUMB.0, THUMB.1, &["ytimg.com"]),
            fetched: None,
            videos: Vec::new(),
            page_at: std::collections::HashMap::new(),
        }
    }

    pub fn run(&mut self, prefs: &Prefs, sink: &dyn Fn(Update)) -> Duration {
        let ids = channel_ids(prefs);
        self.page_at.retain(|id, _| ids.contains(id));
        let due = self.fetched.as_ref().is_none_or(|(at, chans)| at.elapsed() >= REFRESH || *chans != ids);
        let mut failed = false;
        if self.videos.is_empty()
            && let Some(cached) = cached(prefs)
        {
            self.videos = cached; // after a restart, so a partial failure still has something to keep
        }
        if due {
            let mut all = Vec::new();
            let mut failures = 0;
            let jobs: Vec<(&Channel, bool)> = prefs
                .channels
                .iter()
                .map(|ch| (ch, self.page_at.get(&ch.id).is_none_or(|at| at.elapsed() >= PAGE_EVERY)))
                .collect();
            // A few at a time: 20 channels take a couple of seconds, not 20 round trips in a row.
            let results = poller::fan_out(&jobs, CHANNELS_AT_ONCE, |(ch, page_ok)| channel_videos(ch, *page_ok));
            for (ch, result) in prefs.channels.iter().zip(results) {
                if let Ok((_, true)) = &result {
                    self.page_at.insert(ch.id.clone(), Instant::now());
                }
                match result {
                    Ok((v, _)) => all.extend(v),
                    Err(miss) => {
                        if let Miss::Failed(e) = miss {
                            failures += 1;
                            storage::log(format!("youtube refresh failed ({}): {e}", ch.name));
                        }
                        // Keep what we last had from this channel rather than dropping it.
                        all.extend(self.videos.iter().filter(|v| v.channel_id == ch.id).cloned());
                    }
                }
            }
            failed = failures > 0 && failures == prefs.channels.len();
            if failed && self.videos.is_empty() {
                sink(Update::Offline);
            } else if !failed {
                self.videos = merge(all);
                self.fetched = Some((Instant::now(), ids.clone()));
                storage::save_json(CACHE, &Cache { channels: ids, videos: self.videos.clone() });
                sink(Update::Videos(self.videos.clone()));
            }
        }

        let wanted: Vec<String> =
            self.videos.iter().map(|v| v.id.clone()).chain(prefs.saved.iter().map(|s| s.id.clone())).collect();
        let urls: Vec<String> = wanted.iter().map(|id| thumb_url(id)).collect();
        self.thumbs.prefetch(urls.iter().map(|u| u.as_str()));
        let thumbs: Vec<(String, images::Shared)> =
            wanted.iter().filter_map(|id| self.thumbs.get(&thumb_url(id)).map(|px| (id.clone(), px))).collect();
        self.thumbs.retain(urls.iter().map(|u| u.as_str()));
        sink(Update::Thumbs(thumbs));

        if failed {
            RETRY
        } else {
            let elapsed = self.fetched.as_ref().map(|(at, _)| at.elapsed()).unwrap_or_default();
            REFRESH.saturating_sub(elapsed).max(Duration::from_secs(60))
        }
    }
}

/// Newest first, one entry per video.
fn merge(mut all: Vec<Video>) -> Vec<Video> {
    all.sort_by_key(|v| std::cmp::Reverse(v.published.unwrap_or(0)));
    let mut seen = std::collections::HashSet::new();
    all.retain(|v| seen.insert(v.id.clone()));
    all.truncate(MAX_VIDEOS);
    all
}

/// Why a channel produced no fresh videos this time.
enum Miss {
    /// The feed failed and the page fallback ran recently; keep the last videos quietly.
    Throttled,
    Failed(String),
}

/// The channel's RSS feed (~20 KB), or its Videos page (~1.3 MB) when the feed is down and
/// `page_ok` allows the heavier request.
fn channel_videos(ch: &Channel, page_ok: bool) -> Result<(Vec<Video>, bool), Miss> {
    let rss = http::get(&format!("https://www.youtube.com/feeds/videos.xml?channel_id={}", ch.id));
    let (mut videos, used_page) = match rss {
        Ok(body) => (parse_feed(&body), false),
        Err(_) if !page_ok => return Err(Miss::Throttled),
        Err(_) => {
            let page = http::get_with(&format!("https://www.youtube.com/channel/{}/videos", ch.id), &LANG)
                .map_err(|e| Miss::Failed(e.to_string()))?;
            (parse_videos_page(&page, &ch.name, crate::services::weather::unix_now()), true)
        }
    };
    if videos.is_empty() {
        return Err(Miss::Failed("no videos found".into()));
    }
    for v in &mut videos {
        if v.channel.is_empty() {
            v.channel = ch.name.clone();
        }
        v.channel_id = ch.id.clone();
    }
    videos.truncate(PER_CHANNEL);
    Ok((videos, used_page))
}

/// A channel's Atom feed. Entries whose link is a /shorts/ URL are skipped.
pub fn parse_feed(xml: &str) -> Vec<Video> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    // (video, link, title done: media:group repeats the title as media:title)
    let mut cur: Option<(Video, String, bool)> = None;
    let mut field = String::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().as_ref().to_string();
                if name == "entry" {
                    cur = Some((
                        Video {
                            id: String::new(),
                            title: String::new(),
                            channel: String::new(),
                            published: None,
                            views: None,
                            duration: String::new(),
                            channel_id: String::new(),
                        },
                        String::new(),
                        false,
                    ));
                } else if let Some((v, link, _)) = cur.as_mut() {
                    for a in e.attributes().flatten() {
                        let key = a.key.local_name();
                        let val = a
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map(|c| c.into_owned())
                            .unwrap_or_default();
                        match (name.as_str(), key.as_ref()) {
                            ("link", "href") if link.is_empty() => *link = val,
                            ("statistics", "views") => v.views = val.parse().ok(),
                            _ => {}
                        }
                    }
                }
                field = name;
            }
            Event::Text(t) => {
                if let Some((v, _, title_done)) = cur.as_mut() {
                    push_field(v, &field, &t.xml10_content(), *title_done);
                }
            }
            Event::GeneralRef(r) => {
                if let Some((v, _, title_done)) = cur.as_mut() {
                    let text = match r.resolve_char_ref() {
                        Ok(Some(c)) => c.to_string(),
                        _ => entity(&r.xml10_content()).to_string(),
                    };
                    push_field(v, &field, &text, *title_done);
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == "title"
                    && let Some((_, _, done)) = cur.as_mut()
                {
                    *done = true;
                }
                if e.local_name().as_ref() == "entry"
                    && let Some((v, link, _)) = cur.take()
                    && is_video_id(&v.id)
                    && !link.contains("/shorts/")
                {
                    out.push(Video { title: v.title.trim().to_string(), ..v });
                }
                field.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn push_field(v: &mut Video, field: &str, text: &str, title_done: bool) {
    match field {
        "videoId" => v.id.push_str(text.trim()),
        "title" if !title_done => v.title.push_str(text),
        "name" => v.channel.push_str(text),
        "published" => v.published = news::parse_date(text.trim()).or(v.published),
        _ => {}
    }
}

fn entity(name: &str) -> &'static str {
    match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
}

/// The JSON a YouTube page embeds as `var ytInitialData = {...};`.
fn initial_data(html: &str) -> Option<Value> {
    let start = html.find("var ytInitialData = ")? + "var ytInitialData = ".len();
    let rest = &html[start..];
    let end = rest.find(";</script>")?;
    serde_json::from_str(&rest[..end]).ok()
}

/// Videos from a channel's Videos tab (which never includes Shorts). Understands both YouTube's
/// current `lockupViewModel` markup and the older `videoRenderer`.
pub fn parse_videos_page(html: &str, channel: &str, now: i64) -> Vec<Video> {
    let Some(data) = initial_data(html) else { return Vec::new() };
    let mut out = Vec::new();
    walk(&data, &mut |key, v| match key {
        "lockupViewModel" if v["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO" => {
            let id = v["contentId"].as_str().unwrap_or("");
            let meta = &v["metadata"]["lockupMetadataViewModel"];
            let title = meta["title"]["content"].as_str().unwrap_or("");
            let parts: Vec<&str> = meta["metadata"]["contentMetadataViewModel"]["metadataRows"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|r| r["metadataParts"].as_array().into_iter().flatten())
                .filter_map(|p| p["text"]["content"].as_str())
                .collect();
            let mut duration = String::new();
            walk(&v["contentImage"], &mut |k, b| {
                if k == "thumbnailBadgeViewModel"
                    && let Some(t) = b["text"].as_str()
                    && t.contains(':')
                    && duration.is_empty()
                {
                    duration = t.to_string();
                }
            });
            if is_video_id(id) && !title.is_empty() {
                out.push(Video {
                    id: id.into(),
                    title: title.into(),
                    channel: channel.into(),
                    published: parts.iter().find_map(|p| relative_age(p)).and_then(|ago| now.checked_sub(ago)),
                    views: parts.iter().find_map(|p| parse_views(p)),
                    duration,
                    channel_id: String::new(),
                });
            }
        }
        "videoRenderer" => {
            let id = v["videoId"].as_str().unwrap_or("");
            let title = v["title"]["runs"][0]["text"].as_str().unwrap_or("");
            if is_video_id(id) && !title.is_empty() {
                out.push(Video {
                    id: id.into(),
                    title: title.into(),
                    channel: channel.into(),
                    published: v["publishedTimeText"]["simpleText"]
                        .as_str()
                        .and_then(relative_age)
                        .and_then(|ago| now.checked_sub(ago)),
                    views: v["viewCountText"]["simpleText"].as_str().and_then(parse_views),
                    duration: v["lengthText"]["simpleText"].as_str().unwrap_or("").into(),
                    channel_id: String::new(),
                });
            }
        }
        _ => {}
    });
    let mut seen = std::collections::HashSet::new();
    out.retain(|v| seen.insert(v.id.clone()));
    out
}

fn walk<'a>(v: &'a Value, f: &mut dyn FnMut(&str, &'a Value)) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                f(k, child);
                walk(child, f);
            }
        }
        Value::Array(items) => items.iter().for_each(|c| walk(c, f)),
        _ => {}
    }
}

/// Seconds ago for "3d ago", "3 days ago", "Streamed 2 weeks ago", "1mo ago".
pub fn relative_age(s: &str) -> Option<i64> {
    let s = s.to_ascii_lowercase();
    if !s.contains("ago") {
        return None;
    }
    let digits_at = s.find(|c: char| c.is_ascii_digit())?;
    let rest = &s[digits_at..];
    let num_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let n: i64 = rest[..num_end].parse().ok()?;
    let unit = rest[num_end..].trim_start();
    let secs = if unit.starts_with("mo") {
        30 * 86400
    } else if unit.starts_with("se") || unit.starts_with("s ") {
        1
    } else if unit.starts_with('m') {
        60
    } else if unit.starts_with('h') {
        3600
    } else if unit.starts_with('d') {
        86400
    } else if unit.starts_with('w') {
        7 * 86400
    } else if unit.starts_with('y') {
        365 * 86400
    } else {
        return None;
    };
    // A hostile page could say "9223372036854 years ago": no overflow, and nothing older than
    // YouTube itself is plausible.
    n.checked_mul(secs).filter(|s| *s <= 30 * 365 * 86400)
}

/// "5.5M", "5.5M views", "12,345 views", "1.2K".
pub fn parse_views(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    if s.contains("ago") || s.contains(':') {
        return None;
    }
    let num: String =
        s.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').filter(|c| *c != ',').collect();
    let value: f64 = num.parse().ok()?;
    let rest = s[s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(s.len())..].trim_start();
    let mult = match rest.chars().next() {
        Some('k') => 1e3,
        Some('m') => 1e6,
        Some('b') => 1e9,
        Some('v') | None => 1.0,
        _ => return None,
    };
    Some((value * mult) as u64)
}

/// "1.2M views", "830K views", "12 views".
pub fn views_label(n: u64) -> String {
    // Truncated, as YouTube does: 999,999 is "999K", 1,950,000 is "1.9M".
    let short = |v: f64, suffix: &str| {
        if v < 10.0 {
            let tenths = (v * 10.0).floor() as u64;
            if tenths.is_multiple_of(10) {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            }
        } else {
            format!("{}{suffix}", v as u64)
        }
    };
    let label = match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => short(n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => short(n as f64 / 1e6, "M"),
        _ => short(n as f64 / 1e9, "B"),
    };
    format!("{label} {}", if n == 1 { "view" } else { "views" })
}

pub fn is_channel_id(s: &str) -> bool {
    s.len() == 24 && s.starts_with("UC") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The video id in a pasted link (watch, youtu.be, shorts, live, embed) or a bare id.
pub fn video_id_from(input: &str) -> Option<String> {
    let s = input.trim();
    if is_video_id(s) {
        return Some(s.to_string());
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.contains("youtube.com") || lower.contains("youtu.be")) {
        return None;
    }
    // The `v` query parameter exactly (not "ev=" or "dev=").
    let query = s.split_once('?').map(|(_, q)| q).unwrap_or("");
    let param = query.split(['&', '#']).find_map(|kv| kv.strip_prefix("v="));
    let candidate = if let Some(v) = param {
        v
    } else {
        ["youtu.be/", "/shorts/", "/live/", "/embed/"].iter().find_map(|m| s.find(m).map(|i| &s[i + m.len()..]))?
    };
    let id: String = candidate.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    is_video_id(&id).then_some(id)
}

/// What a channel input points at: `Ok(url)` of a page to resolve, or `Err` for unrecognised text.
pub fn channel_page(input: &str) -> Result<String, &'static str> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err("Enter a channel's @handle or link.");
    }
    if is_channel_id(s) {
        return Ok(format!("https://www.youtube.com/channel/{s}"));
    }
    let path = s
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .trim_start_matches("youtube.com");
    let path = path.split(['?', '#']).next().unwrap_or("").trim_start_matches('/');
    let valid =
        |h: &str| !h.is_empty() && h.len() <= 100 && h.chars().all(|c| c.is_alphanumeric() || "._-·".contains(c));
    // Legacy custom and username URLs still redirect to the channel.
    for legacy in ["c/", "user/"] {
        if let Some(rest) = path.strip_prefix(legacy) {
            let name = rest.split('/').next().unwrap_or("");
            return if valid(name) {
                Ok(format!("https://www.youtube.com/{legacy}{}", http::encode(name)))
            } else {
                Err("That channel link isn't valid.")
            };
        }
    }
    if let Some(rest) = path.strip_prefix("channel/") {
        let id = rest.split('/').next().unwrap_or("");
        return if is_channel_id(id) {
            Ok(format!("https://www.youtube.com/channel/{id}"))
        } else {
            Err("That channel link isn't valid.")
        };
    }
    let handle = path.strip_prefix('@').or_else(|| (!s.contains('/') && !s.contains('.')).then_some(path));
    match handle.map(|h| h.split('/').next().unwrap_or("")) {
        Some(h) if valid(h) => Ok(format!("https://www.youtube.com/@{}", http::encode(h))),
        _ => Err("Use a channel's @handle or its youtube.com link."),
    }
}

/// Resolves a channel input to its id and name by reading the channel page. Blocking.
pub fn resolve_channel(input: &str) -> Result<Channel, String> {
    let url = channel_page(input)?;
    let html = http::get_with(&url, &LANG).map_err(|e| match http::status(&e) {
        Some(404) => "No channel by that name.".to_string(),
        _ => "Couldn't reach YouTube. Check your connection.".to_string(),
    })?;
    parse_channel_page(&html).ok_or_else(|| "No channel by that name.".to_string())
}

pub fn parse_channel_page(html: &str) -> Option<Channel> {
    let id = attr_after(html, "<link rel=\"canonical\" href=\"https://www.youtube.com/channel/")?;
    let name = attr_after(html, "<meta property=\"og:title\" content=\"").map(|n| unescape_html(&n))?;
    let handle = attr_after(html, "\"canonicalBaseUrl\":\"/").filter(|h| h.starts_with('@')).unwrap_or_default();
    is_channel_id(&id).then_some(Channel { id, name, handle })
}

fn attr_after(html: &str, marker: &str) -> Option<String> {
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find('"')?;
    Some(html[start..start + end].to_string())
}

fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

/// A pasted video's title and channel, via YouTube's public oEmbed endpoint. Blocking.
pub fn resolve_video(id: &str) -> Result<SavedVideo, String> {
    let url = format!("https://www.youtube.com/oembed?format=json&url={}", http::encode(&watch_url(id)));
    let body = http::get(&url).map_err(|e| match http::status(&e) {
        Some(400..=499) => "That video is private or unavailable.".to_string(),
        _ => "Couldn't reach YouTube. Check your connection.".to_string(),
    })?;
    let v: Value = serde_json::from_str(&body).map_err(|_| "YouTube sent an unexpected reply.".to_string())?;
    Ok(SavedVideo {
        id: id.into(),
        title: v["title"].as_str().unwrap_or("YouTube video").into(),
        channel: v["author_name"].as_str().unwrap_or("").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_feed_entries_and_skips_shorts() {
        let xml = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
          <title>Fireship</title>
          <entry><yt:videoId>SOwUWPtXQ9I</yt:videoId><title>Chips &amp; Dips</title>
            <link rel="alternate" href="https://www.youtube.com/watch?v=SOwUWPtXQ9I"/>
            <author><name>Fireship</name></author><published>2026-11-12T21:17:10+00:00</published>
            <media:group><media:community><media:statistics views="1234567"/></media:community></media:group></entry>
          <entry><yt:videoId>abcdefghijk</yt:videoId><title>Short</title>
            <link rel="alternate" href="https://www.youtube.com/shorts/abcdefghijk"/></entry>
        </feed>"#;
        let v = parse_feed(xml);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "SOwUWPtXQ9I");
        assert_eq!(v[0].title, "Chips & Dips");
        assert_eq!(v[0].channel, "Fireship");
        assert_eq!(v[0].views, Some(1_234_567));
        assert_eq!(v[0].published, news::parse_date("2026-11-12T21:17:10Z"));
    }

    #[test]
    fn parses_videos_page_lockups() {
        let html = r#"<script>var ytInitialData = {"contents":[{"richItemRenderer":{"content":{"lockupViewModel":{
            "contentId":"hdMszZ2ah5Q","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
            "contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"15:14"}}]}}]}},
            "metadata":{"lockupMetadataViewModel":{"title":{"content":"Macbook Review"},
              "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"5.5M"}},{"text":{"content":"3d ago"}}]}]}}}}}}}}]};</script>"#;
        let v = parse_videos_page(html, "MKBHD", 1_000_000);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].id.as_str(), v[0].duration.as_str(), v[0].views), ("hdMszZ2ah5Q", "15:14", Some(5_500_000)));
        assert_eq!(v[0].published, Some(1_000_000 - 3 * 86400));
        assert_eq!(v[0].channel, "MKBHD");
    }

    #[test]
    fn relative_ages_and_views() {
        assert_eq!(relative_age("3d ago"), Some(3 * 86400));
        assert_eq!(relative_age("Streamed 2 weeks ago"), Some(14 * 86400));
        assert_eq!(relative_age("1mo ago"), Some(30 * 86400));
        assert_eq!(relative_age("45 minutes ago"), Some(45 * 60));
        assert_eq!(relative_age("5.5M"), None);
        assert_eq!(parse_views("5.5M"), Some(5_500_000));
        assert_eq!(parse_views("12,345 views"), Some(12_345));
        assert_eq!(parse_views("3d ago"), None);
        assert_eq!(views_label(5_500_000), "5.5M views");
        assert_eq!(views_label(830_000), "830K views");
        assert_eq!(views_label(1_000_000), "1M views");
        assert_eq!(views_label(1), "1 view");
    }

    #[test]
    fn video_ids_from_links() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        ] {
            assert_eq!(video_id_from(link).as_deref(), Some("dQw4w9WgXcQ"), "{link}");
        }
        assert_eq!(video_id_from("https://example.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(video_id_from("hello"), None);
    }

    #[test]
    fn channel_inputs() {
        assert_eq!(channel_page("@mkbhd").unwrap(), "https://www.youtube.com/@mkbhd");
        assert_eq!(channel_page("mkbhd").unwrap(), "https://www.youtube.com/@mkbhd");
        assert_eq!(channel_page("https://www.youtube.com/@mkbhd/videos").unwrap(), "https://www.youtube.com/@mkbhd");
        assert_eq!(
            channel_page("youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ").unwrap(),
            "https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ"
        );
        assert!(channel_page("https://example.com/x").is_err());
        assert!(channel_page("").is_err());
        let html = r#"<link rel="canonical" href="https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ"><meta property="og:title" content="Marques Brownlee &amp; Co">"canonicalBaseUrl":"/@mkbhd""#;
        let c = parse_channel_page(html).unwrap();
        assert_eq!(
            (c.id.as_str(), c.name.as_str(), c.handle.as_str()),
            ("UCBJycsmduvYEL83R_U4JriQ", "Marques Brownlee & Co", "@mkbhd")
        );
    }

    #[test]
    fn merge_dedupes_newest_first() {
        let v = |id: &str, t| Video {
            id: id.into(),
            title: id.into(),
            channel: String::new(),
            published: Some(t),
            views: None,
            duration: String::new(),
            channel_id: String::new(),
        };
        let merged = merge(vec![v("aaaaaaaaaaa", 1), v("bbbbbbbbbbb", 3), v("aaaaaaaaaaa", 1)]);
        assert_eq!(merged.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
    }

    #[test]
    #[ignore = "network"]
    fn live_youtube() {
        let ch = resolve_channel("@mkbhd").unwrap();
        assert_eq!(ch.id, "UCBJycsmduvYEL83R_U4JriQ");
        let Ok((videos, _)) = channel_videos(&ch, true) else { panic!("no videos") };
        println!("{} videos, first: {:?}", videos.len(), videos.first());
        assert!(!videos.is_empty());
        let page = http::get_with(&format!("https://www.youtube.com/channel/{}/videos", ch.id), &LANG).unwrap();
        assert!(!parse_videos_page(&page, &ch.name, 0).is_empty(), "page fallback still parses");
        assert!(resolve_video("dQw4w9WgXcQ").unwrap().title.contains("Never Gonna"));
    }
}
