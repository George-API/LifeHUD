//! Posts from your subreddits and r/popular, through Reddit's public Atom feeds (its JSON API
//! needs OAuth). Both tabs together cost two requests per refresh: your subreddits are fetched as
//! one combined "multireddit" (`r/a+b+c`).
//!
//! Reddit allows anonymous clients about one request a minute, and says so in its
//! `x-ratelimit-*` headers. The fetcher follows them: it waits for the window to reset between
//! requests instead of being refused, and after a refusal it retries once the window resets.
//! Each tab has its own schedule and cache, so switching subreddits refetches only "Yours".

use std::time::{Duration, Instant};

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::services::{http, images, news, poller};
use crate::storage;

const CACHE: &str = "reddit-cache.json";
const REFRESH: Duration = Duration::from_secs(15 * 60);
/// After a failed round, try again this soon (the rate-limit window is about a minute).
const RETRY: Duration = Duration::from_secs(3 * 60);
/// Never wait longer than this for a rate-limit window, whatever the server says.
const MAX_GATE: Duration = Duration::from_secs(90);
/// Subreddit lists kept in the cache, so switching between them shows posts at once.
const CACHED_LISTS: usize = 8;
const LIMIT: usize = 20;

/// Ready-made subreddits for Settings and the widget's picker: (name, description).
pub const PRESETS: [(&str, &str); 10] = [
    ("classicwow", "WoW Classic"),
    ("wow", "World of Warcraft"),
    ("gaming", "Gaming"),
    ("pcgaming", "PC gaming"),
    ("technology", "Technology"),
    ("science", "Science"),
    ("worldnews", "World news"),
    ("space", "Space"),
    ("movies", "Movies"),
    ("todayilearned", "Today I learned"),
];
pub const MAX_SUBREDDITS: usize = 25;
pub const THUMB_PX: u32 = 92;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Hot,
    Top,
    New,
    Rising,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Hot, Sort::Top, Sort::New, Sort::Rising];

    fn path(self) -> &'static str {
        match self {
            Sort::Hot => "hot",
            Sort::Top => "top",
            Sort::New => "new",
            Sort::Rising => "rising",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Hot => "Hot",
            Sort::Top => "Top today",
            Sort::New => "New",
            Sort::Rising => "Rising",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Names without the "r/" prefix, as the user typed them (Reddit's casing is cosmetic).
    pub subreddits: Vec<String>,
    pub sort: Sort,
    /// Narrows "Yours" to one subreddit, picked from the widget ("" = all you follow together).
    pub view: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { subreddits: vec!["technology".into(), "science".into()], sort: Sort::Hot, view: String::new() }
    }
}

impl Prefs {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.subreddits.retain(|s| is_name(s) && seen.insert(s.to_ascii_lowercase()));
        self.subreddits.truncate(MAX_SUBREDDITS);
        if !is_name(&self.view) {
            self.view.clear();
        }
    }

    /// What the "Yours" tab shows: the picked subreddit, or everything you follow.
    pub fn yours(&self) -> Vec<String> {
        if self.view.is_empty() { self.subreddits.clone() } else { vec![self.view.clone()] }
    }
}

/// Reddit names: 2–21 letters, digits or underscores (a few old ones are 3+; allow 2).
pub fn is_name(s: &str) -> bool {
    (2..=21).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// "r/rust", "/r/rust/", "https://www.reddit.com/r/rust/comments/…" or "rust" → "rust".
pub fn name_from(input: &str) -> Option<String> {
    let s = input.trim();
    let after = match s.to_ascii_lowercase().find("r/") {
        Some(i) if s.contains("reddit.com") || i == 0 || s[..i].ends_with('/') => &s[i + 2..],
        _ => s,
    };
    let name: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    // The name must end the input or a path segment: "not a sub!" isn't "r/not".
    let rest = &after[name.len()..];
    let clean_end = rest.is_empty() || rest.starts_with(['/', '?', '#']);
    (is_name(&name) && clean_end).then_some(name)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Post {
    pub title: String,
    /// The comments page.
    pub url: String,
    pub subreddit: String,
    pub author: String,
    pub published: Option<i64>,
    /// Linked site for link posts ("github.com"); empty for text posts.
    pub domain: String,
    pub thumb_url: String,
    #[serde(skip)]
    pub thumb: Option<images::Shared>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Yours,
    Popular,
}

pub enum Update {
    Posts(Tab, Vec<Post>),
    /// Rate-limited or offline; keep showing what we have.
    Failed(Tab),
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    /// r/popular, keyed by sort: it doesn't depend on which subreddits you follow.
    popular_key: String,
    popular: Vec<Post>,
    /// Recent "Yours" lists, newest first, keyed by subreddits + sort.
    lists: Vec<(String, Vec<Post>)>,
}

impl Cache {
    fn yours(&self, key: &str) -> Option<&Vec<Post>> {
        self.lists.iter().find(|(k, _)| k == key).map(|(_, p)| p)
    }

    fn put_yours(&mut self, key: String, posts: Vec<Post>) {
        self.lists.retain(|(k, _)| *k != key);
        self.lists.insert(0, (key, posts));
        self.lists.truncate(CACHED_LISTS);
    }
}

fn yours_key(prefs: &Prefs) -> String {
    format!("{}/{}", prefs.yours().join("+").to_ascii_lowercase(), prefs.sort.path())
}

fn popular_key(prefs: &Prefs) -> String {
    prefs.sort.path().to_string()
}

/// Cached posts for each tab, if any: (yours, popular).
pub fn cached(prefs: &Prefs) -> (Option<Vec<Post>>, Option<Vec<Post>>) {
    let Some(c) = storage::load_json::<Cache>(CACHE) else { return (None, None) };
    let yours = c.yours(&yours_key(prefs)).cloned();
    let popular = (c.popular_key == popular_key(prefs) && !c.popular.is_empty()).then_some(c.popular);
    (yours, popular)
}

pub fn feed_url(subreddits: &[String], sort: Sort) -> String {
    let t = if sort == Sort::Top { "&t=day" } else { "" };
    format!("https://www.reddit.com/r/{}/{}/.rss?limit={LIMIT}{t}", subreddits.join("+"), sort.path())
}

pub fn post_url_ok(url: &str) -> bool {
    url.starts_with("https://www.reddit.com/") || url.starts_with("https://old.reddit.com/")
}

pub fn spawn(prefs: Prefs, sink: impl Fn(Update) + Send + 'static) -> poller::Poller<Prefs> {
    let mut fetcher = Fetcher::new();
    poller::spawn("reddit", prefs, move |prefs: &Prefs| fetcher.run(prefs, &sink))
}

pub struct Fetcher {
    thumbs: images::Cache,
    /// Don't send the next request before this (Reddit's rate-limit window).
    next_allowed: Instant,
    /// When each tab was last fetched, and for which key.
    yours_at: Option<(String, Instant)>,
    popular_at: Option<(String, Instant)>,
}

impl Fetcher {
    pub fn new() -> Self {
        Self {
            thumbs: images::Cache::new(THUMB_PX, THUMB_PX, &["redditmedia.com", "redd.it"]),
            next_allowed: Instant::now(),
            yours_at: None,
            popular_at: None,
        }
    }

    pub fn run(&mut self, prefs: &Prefs, sink: &dyn Fn(Update)) -> Duration {
        let mut cache: Cache = storage::load_json(CACHE).unwrap_or_default();
        let (ykey, pkey) = (yours_key(prefs), popular_key(prefs));
        // A tab is due when it's never been fetched, its subreddits or sort changed, or it's stale.
        let due = |at: &Option<(String, Instant)>, key: &str| {
            at.as_ref().is_none_or(|(k, t)| k != key || t.elapsed() + Duration::from_secs(30) >= REFRESH)
        };
        let subs = prefs.yours();
        let yours_due = !subs.is_empty() && due(&self.yours_at, &ykey);
        let popular_due = due(&self.popular_at, &pkey);
        // What the user just changed goes first; otherwise Popular, the default tab.
        let yours_changed = self.yours_at.as_ref().is_some_and(|(k, _)| *k != ykey);
        let mut jobs = Vec::new();
        if popular_due {
            jobs.push((Tab::Popular, vec!["popular".to_string()]));
        }
        if yours_due {
            let job = (Tab::Yours, subs);
            if yours_changed { jobs.insert(0, job) } else { jobs.push(job) }
        }

        let mut failed = false;
        for (tab, subs) in jobs {
            match self.fetch(&feed_url(&subs, prefs.sort)) {
                Ok(body) => {
                    let mut posts = parse(&body);
                    self.thumbs.prefetch(posts.iter().map(|p| p.thumb_url.as_str()));
                    for p in &mut posts {
                        p.thumb = self.thumbs.get(&p.thumb_url);
                    }
                    match tab {
                        Tab::Yours => {
                            cache.put_yours(ykey.clone(), posts.clone());
                            self.yours_at = Some((ykey.clone(), Instant::now()));
                        }
                        Tab::Popular => {
                            cache.popular_key = pkey.clone();
                            cache.popular = posts.clone();
                            self.popular_at = Some((pkey.clone(), Instant::now()));
                        }
                    }
                    sink(Update::Posts(tab, posts));
                }
                Err(e) => {
                    storage::log(format!("reddit refresh failed ({}): {e}", subs.join("+")));
                    sink(Update::Failed(tab));
                    failed = true;
                }
            }
        }
        // Decoded thumbnails stay in memory only for what's on screen: Popular and the current
        // "Yours" list. Older lists keep their posts in the cache file; their thumbnails reload.
        let keep = thumbs_to_keep(&cache, &ykey);
        self.thumbs.retain(keep.iter().map(|u| u.as_str()));
        storage::save_json(CACHE, &cache);
        if failed { RETRY } else { REFRESH }
    }

    /// One feed, respecting the rate-limit window, with one retry after a refusal.
    fn fetch(&mut self, url: &str) -> Result<String, String> {
        for attempt in 0..2 {
            let wait = self.next_allowed.saturating_duration_since(Instant::now()).min(MAX_GATE);
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            let reply = http::get_reply(url).map_err(|e| e.to_string())?;
            let window = Duration::from_secs(reply.reset_secs.unwrap_or(60) + 1).min(MAX_GATE);
            // Spent the window (or refused): the next request waits for the reset.
            self.next_allowed = if reply.status == 429 || reply.remaining.is_some_and(|r| r < 1.0) {
                Instant::now() + window
            } else {
                Instant::now() + Duration::from_secs(2)
            };
            match reply.status {
                200 => return Ok(reply.body),
                429 if attempt == 0 => continue,
                status => return Err(format!("http status: {status}")),
            }
        }
        Err("http status: 429".into())
    }
}

/// Thumbnail URLs worth keeping decoded: Popular's and the current "Yours" list's.
fn thumbs_to_keep(cache: &Cache, yours_key: &str) -> Vec<String> {
    cache.yours(yours_key).into_iter().flatten().chain(&cache.popular).map(|p| p.thumb_url.clone()).collect()
}

/// Whether a subreddit exists (404 = no). Rate limits and network trouble count as "probably",
/// so a flaky connection doesn't block adding one. Blocking.
pub fn exists(name: &str) -> Result<bool, String> {
    match http::get(&format!("https://www.reddit.com/r/{name}/new/.rss?limit=1")) {
        Ok(_) => Ok(true),
        Err(e) if matches!(http::status(&e), Some(403 | 404)) => Ok(false),
        Err(e) if http::status(&e).is_some() => Ok(true),
        Err(_) => Err("Couldn't reach Reddit. Check your connection.".into()),
    }
}

#[derive(Default)]
struct Draft {
    title: String,
    url: String,
    subreddit: String,
    author: String,
    date: String,
    content: String,
    thumb: String,
}

/// Parses a Reddit Atom feed.
pub fn parse(xml: &str) -> Vec<Post> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut cur: Option<Draft> = None;
    let mut field = String::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().as_ref().to_string();
                if name == "entry" {
                    cur = Some(Draft::default());
                } else if let Some(d) = cur.as_mut() {
                    for a in e.attributes().flatten() {
                        let val = a
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map(|c| c.into_owned())
                            .unwrap_or_default();
                        match (name.as_str(), a.key.local_name().as_ref()) {
                            ("link", "href") if d.url.is_empty() => d.url = val,
                            ("category", "label") => d.subreddit = val,
                            ("thumbnail", "url") => d.thumb = val,
                            _ => {}
                        }
                    }
                }
                field = name;
            }
            Event::Text(t) => push(&mut cur, &field, &t.xml10_content()),
            Event::CData(t) => push(&mut cur, &field, &t.xml10_content()),
            Event::GeneralRef(r) => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.xml10_content().as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => String::new(),
                    },
                };
                push(&mut cur, &field, &text);
            }
            Event::End(e) => {
                if e.local_name().as_ref() == "entry"
                    && let Some(d) = cur.take()
                {
                    let title = d.title.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !title.is_empty() && post_url_ok(&d.url) {
                        out.push(Post {
                            title,
                            subreddit: d.subreddit.trim().to_string(),
                            author: d.author.trim().trim_start_matches("/u/").to_string(),
                            published: news::parse_date(d.date.trim()),
                            domain: link_domain(&d.content),
                            thumb_url: if d.thumb.starts_with("https://") { d.thumb } else { String::new() },
                            thumb: None,
                            url: d.url,
                        });
                    }
                }
                if e.local_name().as_ref() != "entry" {
                    field.clear();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn push(cur: &mut Option<Draft>, field: &str, text: &str) {
    let Some(d) = cur.as_mut() else { return };
    match field {
        "title" => d.title.push_str(text),
        "name" => d.author.push_str(text),
        "published" => d.date.push_str(text),
        "content" => d.content.push_str(text),
        _ => {}
    }
}

/// The external site a link post points at, from the "[link]" anchor in the entry's HTML.
fn link_domain(content: &str) -> String {
    let Some(end) = content.find("\">[link]</a>") else { return String::new() };
    let Some(start) = content[..end].rfind("href=\"") else { return String::new() };
    let url = &content[start + 6..end];
    let host =
        url.trim_start_matches("https://").trim_start_matches("http://").split(['/', '?', '#']).next().unwrap_or("");
    let host = host.trim_start_matches("www.");
    if host.is_empty() || host.ends_with("reddit.com") || host.ends_with("redd.it") {
        String::new()
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn only_visible_lists_keep_thumbnails() {
        let post = |t: &str| super::Post {
            title: String::new(),
            url: String::new(),
            subreddit: String::new(),
            author: String::new(),
            published: None,
            domain: String::new(),
            thumb_url: t.into(),
            thumb: None,
        };
        let mut c = super::Cache::default();
        c.put_yours("old".into(), vec![post("old.jpg")]);
        c.put_yours("now".into(), vec![post("now.jpg")]);
        c.popular = vec![post("pop.jpg")];
        assert_eq!(super::thumbs_to_keep(&c, "now"), ["now.jpg", "pop.jpg"]);
    }

    #[test]
    fn view_narrows_yours_and_is_validated() {
        let mut p = super::Prefs { view: "classicwow".into(), ..Default::default() };
        assert_eq!(p.yours(), ["classicwow"]);
        p.view = "not a sub!".into();
        p.normalize();
        assert_eq!(p.view, "");
        assert_eq!(p.yours(), ["technology", "science"]);
        assert!(super::PRESETS.iter().any(|(n, _)| *n == "classicwow"));
        assert!(super::PRESETS.iter().all(|(n, _)| super::is_name(n)));
    }

    #[test]
    fn cache_keeps_recent_lists_newest_first() {
        let mut c = super::Cache::default();
        for i in 0..10 {
            c.put_yours(format!("k{i}"), Vec::new());
        }
        c.put_yours("k5".into(), Vec::new());
        assert_eq!(c.lists.len(), super::CACHED_LISTS);
        assert_eq!(c.lists[0].0, "k5");
        assert!(c.yours("k0").is_none());
    }
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/"><category term="multi" label="r/multi"/><updated>2026-11-14T21:29:40+00:00</updated>
      <entry><author><name>/u/NXGZ</name><uri>https://www.reddit.com/user/NXGZ</uri></author><category term="programming" label="r/programming"/>
        <content type="html">&amp;#32; submitted by &lt;a href=&quot;https://www.reddit.com/user/NXGZ&quot;&gt; /u/NXGZ &lt;/a&gt; &lt;span&gt;&lt;a href=&quot;https://lyra.horse/blog/2026/11/x/&quot;&gt;[link]&lt;/a&gt;&lt;/span&gt; &lt;span&gt;&lt;a href=&quot;https://www.reddit.com/r/programming/comments/1xvr455/x/&quot;&gt;[comments]&lt;/a&gt;&lt;/span&gt;</content>
        <id>t3_1xvr455</id><media:thumbnail url="https://b.thumbs.redditmedia.com/abc.jpg" /><link href="https://www.reddit.com/r/programming/comments/1xvr455/x/" /><updated>2026-11-14T02:38:21+00:00</updated><published>2026-11-14T02:38:21+00:00</published><title>How I &amp; accidentally became a millionaire</title></entry>
      <entry><author><name>/u/someone</name></author><category term="rust" label="r/rust"/>
        <content type="html">&lt;a href=&quot;https://www.reddit.com/r/rust/comments/2/y/&quot;&gt;[link]&lt;/a&gt;</content>
        <link href="https://www.reddit.com/r/rust/comments/2/y/" /><published>2026-11-14T03:00:00+00:00</published><title>Text post</title></entry>
    </feed>"#;

    #[test]
    fn parses_atom_posts() {
        let p = parse(FEED);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].title, "How I & accidentally became a millionaire");
        assert_eq!(p[0].subreddit, "r/programming");
        assert_eq!(p[0].author, "NXGZ");
        assert_eq!(p[0].domain, "lyra.horse");
        assert_eq!(p[0].thumb_url, "https://b.thumbs.redditmedia.com/abc.jpg");
        assert_eq!(p[0].published, news::parse_date("2026-11-14T02:38:21Z"));
        assert_eq!(p[1].domain, "", "self posts link to Reddit itself");
        assert!(p[1].thumb_url.is_empty());
    }

    #[test]
    fn subreddit_names_from_input() {
        assert_eq!(name_from("rust").as_deref(), Some("rust"));
        assert_eq!(name_from("r/rust").as_deref(), Some("rust"));
        assert_eq!(name_from("/r/AskHistorians/").as_deref(), Some("AskHistorians"));
        assert_eq!(name_from("https://www.reddit.com/r/rust/comments/abc/x/").as_deref(), Some("rust"));
        assert_eq!(name_from("not a sub!").as_deref(), None);
        assert_eq!(name_from("x").as_deref(), None);
    }

    #[test]
    fn feed_urls_and_prefs() {
        assert_eq!(
            feed_url(&["rust".into(), "programming".into()], Sort::Hot),
            "https://www.reddit.com/r/rust+programming/hot/.rss?limit=20"
        );
        assert!(feed_url(&["popular".into()], Sort::Top).ends_with("/top/.rss?limit=20&t=day"));
        let mut p = Prefs {
            subreddits: vec!["Rust".into(), "rust".into(), "bad name".into()],
            sort: Sort::New,
            view: String::new(),
        };
        p.normalize();
        assert_eq!(p.subreddits, ["Rust"]);
    }

    #[test]
    #[ignore = "network"]
    fn live_reddit() {
        let body = http::get(&feed_url(&["rust".into(), "programming".into()], Sort::Hot)).unwrap();
        let posts = parse(&body);
        println!("{} posts; first: {:?}", posts.len(), posts.first());
        assert!(posts.len() >= 5);
        // Anonymous clients get rate-limited quickly; a 429 means we can't tell (and exists()
        // deliberately says "probably" then), so only check the answer when Reddit gives one.
        let probe = http::get("https://www.reddit.com/r/thisdoesnotexistxyz123/new/.rss?limit=1");
        if probe.as_ref().err().and_then(http::status) != Some(429) {
            assert_eq!(exists("thisdoesnotexistxyz123"), Ok(false));
        }
    }
}
