//! Headlines from two free feeds at once: a curated (tech/STEM-leaning) tab and a general
//! top-news tab. RSS 2.0, RSS 1.0/RDF and Atom are supported. Runs on a background thread.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::services::http;
use crate::storage;

const CACHE: &str = "news-cache.json";
const REFRESH: Duration = Duration::from_secs(20 * 60);
const RETRY: Duration = Duration::from_secs(3 * 60);
const MAX_ARTICLES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Curated,
    Top,
}

pub struct Source {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
}

const fn s(id: &'static str, name: &'static str, url: &'static str) -> Source {
    Source { id, name, url }
}

/// Left tab: highlights with a technology and science lean.
pub const CURATED: &[Source] = &[
    s("bloomberg-tech", "Bloomberg Technology", "https://feeds.bloomberg.com/technology/news.rss"),
    s(
        "google-tech",
        "Google News · Technology",
        "https://news.google.com/rss/headlines/section/topic/TECHNOLOGY?hl=en-US&gl=US&ceid=US:en",
    ),
    s(
        "google-science",
        "Google News · Science",
        "https://news.google.com/rss/headlines/section/topic/SCIENCE?hl=en-US&gl=US&ceid=US:en",
    ),
    s("verge", "The Verge", "https://www.theverge.com/rss/index.xml"),
    s("techcrunch", "TechCrunch", "https://techcrunch.com/feed/"),
    s("ars", "Ars Technica", "https://feeds.arstechnica.com/arstechnica/index"),
    s("wired", "Wired", "https://www.wired.com/feed/rss"),
    s("mit-tr", "MIT Technology Review", "https://www.technologyreview.com/feed/"),
    s("hn", "Hacker News", "https://hnrss.org/frontpage"),
    s("nature", "Nature", "https://www.nature.com/nature.rss"),
    s("sciencedaily", "ScienceDaily", "https://www.sciencedaily.com/rss/top.xml"),
    s("nasa", "NASA", "https://www.nasa.gov/news-release/feed/"),
];

/// Right tab: the day's top stories.
pub const TOP: &[Source] = &[
    s("google-ca", "Google News · Canada", "https://news.google.com/rss?hl=en-CA&gl=CA&ceid=CA:en"),
    s("google-us", "Google News · United States", "https://news.google.com/rss?hl=en-US&gl=US&ceid=US:en"),
    s("google-gb", "Google News · United Kingdom", "https://news.google.com/rss?hl=en-GB&gl=GB&ceid=GB:en"),
    s("google-au", "Google News · Australia", "https://news.google.com/rss?hl=en-AU&gl=AU&ceid=AU:en"),
    s("google-in", "Google News · India", "https://news.google.com/rss?hl=en-IN&gl=IN&ceid=IN:en"),
    s("bbc", "BBC News", "https://feeds.bbci.co.uk/news/rss.xml"),
    s("cbc", "CBC News", "https://www.cbc.ca/webfeed/rss/rss-topstories"),
    s("nyt", "The New York Times", "https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml"),
    s("guardian", "The Guardian", "https://www.theguardian.com/world/rss"),
    s("npr", "NPR", "https://feeds.npr.org/1001/rss.xml"),
    s("aljazeera", "Al Jazeera", "https://www.aljazeera.com/xml/rss/all.xml"),
];

pub const DEFAULT_CURATED: &str = "bloomberg-tech";

pub fn sources(feed: Feed) -> &'static [Source] {
    match feed {
        Feed::Curated => CURATED,
        Feed::Top => TOP,
    }
}

pub fn find(feed: Feed, id: &str) -> &'static Source {
    let list = sources(feed);
    list.iter().find(|s| s.id == id).unwrap_or(&list[0])
}

/// Google News for the user's own region when we have an edition for it, else the US edition.
pub fn default_top() -> &'static str {
    let id = format!("google-{}", crate::locale::region().to_ascii_lowercase());
    TOP.iter().find(|s| s.id == id).map(|s| s.id).unwrap_or("google-us")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Article {
    pub title: String,
    pub url: String,
    /// Unix seconds, when the feed provides a parseable date.
    pub published: Option<i64>,
    /// Original publisher, for aggregators like Google News.
    #[serde(default)]
    pub publisher: Option<String>,
}

pub enum Update {
    Articles { feed: Feed, source: String, articles: Vec<Article> },
    Failed { feed: Feed, source: String },
}

pub fn cached(source_id: &str) -> Option<Vec<Article>> {
    storage::load_json::<HashMap<String, Vec<Article>>>(CACHE).and_then(|mut m| m.remove(source_id))
}

/// Starts the fetcher; send `(curated_id, top_id)` to change sources.
pub fn spawn(curated: String, top: String, sink: impl Fn(Update) + Send + 'static) -> Sender<(String, String)> {
    let (tx, rx) = mpsc::channel::<(String, String)>();
    std::thread::Builder::new()
        .name("news".into())
        .spawn(move || {
            let mut current = (curated, top);
            let mut wait = Duration::ZERO;
            loop {
                match rx.recv_timeout(wait) {
                    Ok(ids) => current = ids,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                while let Ok(ids) = rx.try_recv() {
                    current = ids;
                }
                let mut cache: HashMap<String, Vec<Article>> = storage::load_json(CACHE).unwrap_or_default();
                let mut all_ok = true;
                for (feed, id) in [(Feed::Curated, &current.0), (Feed::Top, &current.1)] {
                    let src = find(feed, id);
                    match fetch(src) {
                        Ok(articles) => {
                            cache.insert(src.id.into(), articles.clone());
                            sink(Update::Articles { feed, source: src.id.into(), articles });
                        }
                        Err(e) => {
                            storage::log(format!("news refresh failed ({}): {e}", src.name));
                            sink(Update::Failed { feed, source: src.id.into() });
                            all_ok = false;
                        }
                    }
                }
                cache.retain(|k, _| *k == current.0 || *k == current.1);
                storage::save_json(CACHE, &cache);
                wait = if all_ok { REFRESH } else { RETRY };
            }
        })
        .expect("spawn news thread");
    tx
}

fn fetch(src: &Source) -> Result<Vec<Article>, String> {
    let body = http::get(src.url).map_err(|e| e.to_string())?;
    let mut articles = parse(&body)?;
    if articles.is_empty() {
        return Err("feed had no articles".into());
    }
    if src.id.starts_with("google-") {
        for a in &mut articles {
            split_publisher(a);
        }
    }
    Ok(articles)
}

/// Google News titles end in " - Publisher"; show the publisher separately.
fn split_publisher(a: &mut Article) {
    if let Some(i) = a.title.rfind(" - ") {
        let publisher = a.title[i + 3..].trim();
        if !publisher.is_empty() && publisher.len() < 48 {
            a.publisher = Some(publisher.to_string());
            a.title.truncate(i);
        }
    }
}
#[derive(Default)]
struct Draft {
    title: String,
    url: String,
    date: String,
}

/// Parses RSS 2.0, RSS 1.0 (RDF) and Atom into articles, newest first.
pub fn parse(xml: &str) -> Result<Vec<Article>, String> {
    let mut reader = Reader::from_str(xml);
    // No per-fragment trimming: it would eat the spaces around entities ("Rock &amp; roll").
    // Whitespace is normalised once over the whole title in `clean()`.

    let mut out = Vec::new();
    let mut item: Option<Draft> = None;
    let mut field: Option<&'static str> = None;
    let mut depth_in_item = 0usize;

    loop {
        let event = reader.read_event().map_err(|e| format!("xml error at {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => {
                let name = e.local_name();
                let name = name.as_ref();
                if item.is_none() {
                    if name == "item" || name == "entry" {
                        item = Some(Draft::default());
                        depth_in_item = 0;
                    }
                    continue;
                }
                depth_in_item += 1;
                if depth_in_item == 1 {
                    field = match name {
                        "title" => Some("title"),
                        "link" => {
                            atom_link(&e, item.as_mut().unwrap());
                            Some("link")
                        }
                        "pubDate" | "published" | "updated" | "date" => Some("date"),
                        _ => None,
                    };
                }
            }
            Event::Empty(e) => {
                if let Some(d) = item.as_mut()
                    && depth_in_item == 0
                    && e.local_name().as_ref() == "link"
                {
                    atom_link(&e, d);
                }
            }
            Event::Text(t) => push_text(&mut item, field, &t.xml10_content()),
            Event::CData(t) => push_text(&mut item, field, &t.xml10_content()),
            Event::GeneralRef(r) => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.xml10_content().as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        "nbsp" => " ".into(),
                        other => format!("&{other};"),
                    },
                };
                push_text(&mut item, field, &text);
            }
            Event::End(e) => {
                if item.is_none() {
                    continue;
                }
                if depth_in_item == 0 {
                    let name = e.local_name();
                    if name.as_ref() == "item" || name.as_ref() == "entry" {
                        let d = item.take().unwrap();
                        let title = clean(&d.title);
                        if !title.is_empty() && d.url.starts_with("http") {
                            out.push(Article {
                                title,
                                url: d.url.trim().to_string(),
                                published: parse_date(d.date.trim()),
                                publisher: None,
                            });
                        }
                    }
                } else {
                    depth_in_item -= 1;
                    if depth_in_item == 0 {
                        field = None;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    // Most feeds are already newest-first; sort when dates allow, keeping feed order for ties.
    if out.iter().all(|a| a.published.is_some()) {
        out.sort_by_key(|a| std::cmp::Reverse(a.published));
    }
    out.truncate(MAX_ARTICLES);
    Ok(out)
}

fn atom_link(e: &quick_xml::events::BytesStart, d: &mut Draft) {
    let mut href = None;
    let mut rel = None;
    for a in e.attributes().flatten() {
        match a.key.local_name().as_ref() {
            "href" => href = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()),
            "rel" => rel = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()),
            _ => {}
        }
    }
    if let Some(h) = href
        && rel.as_deref().is_none_or(|r| r == "alternate")
        && d.url.is_empty()
    {
        d.url = h;
    }
}

fn push_text(item: &mut Option<Draft>, field: Option<&str>, text: &str) {
    let Some(d) = item.as_mut() else { return };
    match field {
        Some("title") => d.title.push_str(text),
        Some("link") => {
            if d.url.is_empty() || !d.url.starts_with("http") {
                d.url.push_str(text.trim());
            }
        }
        Some("date") if d.date.is_empty() => {
            d.date.push_str(text);
        }
        _ => {}
    }
}

/// Collapses whitespace and strips any stray markup from a headline.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, offset_min: i64) -> i64 {
    days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - offset_min * 60
}

fn offset_minutes(tz: &str) -> i64 {
    match tz {
        "" | "Z" | "GMT" | "UT" | "UTC" => 0,
        "EST" => -300,
        "EDT" => -240,
        "CST" => -360,
        "CDT" => -300,
        "MST" => -420,
        "MDT" => -360,
        "PST" => -480,
        "PDT" => -420,
        t if (t.starts_with('+') || t.starts_with('-')) && t.len() >= 5 => {
            let digits: String = t[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let (h, m) = (
                digits.get(..2).and_then(|v| v.parse::<i64>().ok()),
                digits.get(2..4).and_then(|v| v.parse::<i64>().ok()),
            );
            let sign = if t.starts_with('-') { -1 } else { 1 };
            sign * (h.unwrap_or(0) * 60 + m.unwrap_or(0))
        }
        _ => 0,
    }
}

/// RFC 2822 ("Sat, 14 Nov 2026 13:05:00 GMT") or ISO 8601 ("2026-11-14T13:05:00Z").
pub fn parse_date(s: &str) -> Option<i64> {
    if s.len() >= 19 && s.as_bytes().get(4) == Some(&b'-') {
        let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
        let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
        let rest = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        return Some(unix(y, mo, d, h, mi, se, offset_minutes(rest)));
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    let parts = if parts.first().is_some_and(|p| p.ends_with(',')) { &parts[1..] } else { &parts[..] };
    let [d, mon, y, time, rest @ ..] = parts else { return None };
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| mon.to_ascii_lowercase().starts_with(m))? as i64
        + 1;
    let mut t = time.split(':').map(|v| v.parse::<i64>().ok());
    let (h, mi, se) = (t.next()??, t.next()??, t.next().flatten().unwrap_or(0));
    let mut year = y.parse::<i64>().ok()?;
    if year < 100 {
        year += 2000;
    }
    Some(unix(year, month, d.parse().ok()?, h, mi, se, offset_minutes(rest.first().copied().unwrap_or(""))))
}

/// "now", "12m", "3h", "2d".
pub fn age_label(published: Option<i64>, now: i64) -> String {
    let Some(p) = published else { return String::new() };
    let mins = ((now - p) / 60).max(0);
    match mins {
        0 => "now".into(),
        1..=59 => format!("{mins}m"),
        60..=1439 => format!("{}h", mins / 60),
        _ => format!("{}d", mins / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss2_with_cdata_and_entities() {
        let xml = r#"<?xml version="1.0"?><rss><channel><title>Feed</title>
            <item><title><![CDATA[Chips &amp; <b>AI</b>]]></title><link>https://a.example/1</link>
              <pubDate>Sat, 14 Nov 2026 13:05:00 GMT</pubDate></item>
            <item><title>Rock &amp; roll &#8217;24</title><link>https://a.example/2</link>
              <pubDate>Sat, 14 Nov 2026 14:00:00 +0100</pubDate></item>
        </channel></rss>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].title, "Chips &amp; AI"); // CDATA is literal
        assert_eq!(a[1].title, "Rock & roll \u{2019}24");
        assert_eq!(a[0].published, parse_date("2026-11-14T13:05:00Z"));
    }

    #[test]
    fn parses_atom_links_from_attributes() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom">
            <title>Feed</title><link href="https://site.example/"/>
            <entry><title>Hello</title><link rel="alternate" href="https://site.example/hello"/>
              <updated>2026-11-14T10:00:00-05:00</updated></entry>
        </feed>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].url, "https://site.example/hello");
        assert_eq!(a[0].published, parse_date("2026-11-14T15:00:00Z"));
    }

    #[test]
    fn parses_rdf() {
        let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
            <channel><title>Nature</title></channel>
            <item><title>Discovery</title><link>https://n.example/x</link><dc:date>2026-11-13</dc:date></item>
        </rdf:RDF>"#;
        let a = parse(xml).unwrap();
        assert_eq!(a[0].title, "Discovery");
    }

    #[test]
    fn dates_and_ages() {
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date("Thu, 01 Jan 1970 01:00:00 +0100"), Some(0));
        assert_eq!(age_label(Some(0), 59), "now");
        assert_eq!(age_label(Some(0), 3 * 3600), "3h");
        assert_eq!(age_label(None, 10), "");
    }

    #[test]
    fn source_lookup_falls_back_to_first() {
        assert_eq!(find(Feed::Curated, "nope").id, DEFAULT_CURATED);
        assert_eq!(find(Feed::Curated, "hn").name, "Hacker News");
        assert_eq!(find(Feed::Top, "google-ca").name, "Google News · Canada");
    }

    #[test]
    fn google_titles_split_publisher() {
        let mut a =
            Article { title: "Big story - CBC News".into(), url: "https://x".into(), published: None, publisher: None };
        split_publisher(&mut a);
        assert_eq!(a.title, "Big story");
        assert_eq!(a.publisher.as_deref(), Some("CBC News"));
    }

    #[test]
    fn source_ids_are_unique_within_each_tab() {
        for list in [CURATED, TOP] {
            let mut ids: Vec<_> = list.iter().map(|s| s.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), list.len());
        }
    }
}
