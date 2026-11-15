//! Netflix's official weekly Top 10 (published every Tuesday as a public TSV at
//! netflix.com/tudum/top10). The file holds every week since 2021, so the previous week is right
//! there for rank movement. It's fetched with `If-Modified-Since`, so most refreshes cost a 304.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::services::{http, news, poller};
use crate::storage;

const URL: &str = "https://www.netflix.com/tudum/top10/data/all-weeks-global.tsv";
const CACHE: &str = "netflix-cache.json";
const REFRESH: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(30 * 60);
const TOP: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Rank English and non-English titles together (Netflix lists them separately).
    pub non_english: bool,
    /// Open titles in the widget itself (a docked Edge window) instead of the browser.
    pub in_widget: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { non_english: true, in_widget: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Shows,
    Films,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Week ending (Sunday), "2026-11-08".
    pub week: String,
    pub kind: Kind,
    pub english: bool,
    pub rank: u32,
    pub title: String,
    /// "Season 3", "Limited Series", "Part 4"; empty for films.
    pub season: String,
    pub views: u64,
    pub hours: u64,
    pub weeks: u32,
}

/// The latest two weeks of the chart.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub week: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    New,
    Up(u32),
    Down(u32),
    Same,
    /// Back in the list after time away.
    Back,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub rank: u32,
    pub title: String,
    pub season: String,
    pub views: u64,
    pub weeks: u32,
    pub english: bool,
    pub movement: Move,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    last_modified: Option<String>,
    chart: Chart,
}

pub fn cached() -> Option<Chart> {
    storage::load_json::<Cache>(CACHE).map(|c| c.chart).filter(|c| !c.rows.is_empty())
}

pub enum Update {
    Chart(Chart),
    Offline,
}

/// The chart doesn't depend on any setting (ranking happens in [`top`]), so the poller's
/// configuration is empty.
pub fn spawn(sink: impl Fn(Update) + Send + 'static) -> poller::Poller<()> {
    poller::spawn("netflix", (), move |_| fetch(&sink))
}

pub fn fetch(sink: &dyn Fn(Update)) -> Duration {
    let mut cache: Cache = storage::load_json(CACHE).unwrap_or_default();
    // Netflix's CDN ignores If-Modified-Since and sends no ETag, so every request costs the full
    // ~1 MB file. The chart only changes on Tuesdays, so skip the request while ours is current.
    if !cache.chart.rows.is_empty() && cache.chart.week >= latest_published_week(crate::services::weather::unix_now()) {
        sink(Update::Chart(cache.chart));
        return REFRESH;
    }
    let known = (!cache.chart.rows.is_empty()).then_some(cache.last_modified.as_deref()).flatten();
    match http::get_if_modified(URL, known) {
        Ok(None) => {
            sink(Update::Chart(cache.chart));
            REFRESH
        }
        Ok(Some((body, last_modified))) => {
            let chart = parse(&body);
            if chart.rows.is_empty() {
                storage::log("netflix: the Top 10 file had no rows");
                sink(if cache.chart.rows.is_empty() { Update::Offline } else { Update::Chart(cache.chart) });
                return RETRY;
            }
            cache = Cache { last_modified, chart };
            storage::save_json(CACHE, &cache);
            sink(Update::Chart(cache.chart));
            REFRESH
        }
        Err(e) => {
            storage::log(format!("netflix refresh failed: {e}"));
            sink(if cache.chart.rows.is_empty() { Update::Offline } else { Update::Chart(cache.chart) });
            RETRY
        }
    }
}

/// Keeps the newest two weeks of the TSV (it's ordered newest first).
pub fn parse(tsv: &str) -> Chart {
    let mut lines = tsv.lines();
    let Some(header) = lines.next() else { return Chart::default() };
    let cols: Vec<&str> = header.trim_start_matches('\u{feff}').split('\t').collect();
    let col = |name: &str| cols.iter().position(|c| c.trim() == name);
    let (Some(week), Some(cat), Some(rank), Some(title), Some(season), Some(views), Some(hours), Some(weeks)) = (
        col("week"),
        col("category"),
        col("weekly_rank"),
        col("show_title"),
        col("season_title"),
        col("weekly_views"),
        col("weekly_hours_viewed"),
        col("cumulative_weeks_in_top_10"),
    ) else {
        return Chart::default();
    };
    let mut weeks_seen: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        let get = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
        let w = get(week);
        if w.len() != 10 {
            continue;
        }
        if !weeks_seen.iter().any(|x| x == w) {
            if weeks_seen.len() == 2 {
                break;
            }
            weeks_seen.push(w.to_string());
        }
        let category = get(cat);
        let kind = if category.starts_with("TV") {
            Kind::Shows
        } else if category.starts_with("Films") {
            Kind::Films
        } else {
            continue;
        };
        let show = get(title);
        if show.is_empty() {
            continue;
        }
        rows.push(Row {
            week: w.to_string(),
            kind,
            english: !category.contains("Non-English"),
            rank: get(rank).parse().unwrap_or(0),
            title: show.to_string(),
            season: season_label(show, get(season)),
            views: get(views).parse().unwrap_or(0),
            hours: get(hours).parse().unwrap_or(0),
            weeks: get(weeks).parse().unwrap_or(1),
        });
    }
    let week = weeks_seen.into_iter().max().unwrap_or_default();
    Chart { week, rows }
}

/// "Lupin: Part 4" → "Part 4"; "N/A" → "".
fn season_label(show: &str, season: &str) -> String {
    if season.is_empty() || season == "N/A" || season == show {
        return String::new();
    }
    season.strip_prefix(show).map(|s| s.trim_start_matches(':').trim()).unwrap_or(season).to_string()
}

fn ranked<'a>(chart: &'a Chart, week: &str, kind: Kind, non_english: bool) -> Vec<&'a Row> {
    let mut rows: Vec<&Row> =
        chart.rows.iter().filter(|r| r.week == week && r.kind == kind && (non_english || r.english)).collect();
    // Views make English and non-English lists comparable (hours favour long seasons).
    rows.sort_by_key(|r| (std::cmp::Reverse(r.views), std::cmp::Reverse(r.hours), r.rank));
    rows.truncate(TOP);
    rows
}

/// This week's Top 10 for a kind, with each title's movement since last week.
pub fn top(chart: &Chart, kind: Kind, non_english: bool) -> Vec<Entry> {
    let previous_week = chart.rows.iter().map(|r| r.week.as_str()).filter(|w| *w < chart.week.as_str()).max();
    let previous = previous_week.map(|w| ranked(chart, w, kind, non_english)).unwrap_or_default();
    ranked(chart, &chart.week, kind, non_english)
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let rank = i as u32 + 1;
            let before = previous.iter().position(|p| p.title == r.title && p.season == r.season).map(|p| p as u32 + 1);
            let movement = match before {
                Some(b) if b > rank => Move::Up(b - rank),
                Some(b) if b < rank => Move::Down(rank - b),
                Some(_) => Move::Same,
                None if r.weeks <= 1 => Move::New,
                None => Move::Back,
            };
            Entry {
                rank,
                title: r.title.clone(),
                season: r.season.clone(),
                views: r.views,
                weeks: r.weeks,
                english: r.english,
                movement,
            }
        })
        .collect()
}

/// "Nov 2 – 8" (or "Oct 27 – Nov 2") for a week ending on `week_end`.
pub fn week_label(week_end: &str) -> String {
    // Calendar dates, not instants: plain UTC date arithmetic, so no time zone can shift a day.
    let Some(end) = news::parse_date(&format!("{week_end}T00:00:00Z")) else { return String::new() };
    let (a, b) = (utc_date(end - 6 * 86400), utc_date(end));
    let (first, last) = (crate::locale::short_date(&a), crate::locale::short_date(&b));
    if a.wMonth == b.wMonth {
        // The shared month once: "Nov 2 – 8", or "2 – 8 Nov" where the day comes first.
        let day = b.wDay.to_string();
        if last.ends_with(&day) {
            format!("{first} – {day}")
        } else if first.starts_with(&a.wDay.to_string()) {
            format!("{} – {last}", a.wDay)
        } else {
            format!("{first} – {last}")
        }
    } else {
        format!("{first} – {last}")
    }
}

/// The newest week ending (a Sunday, "2026-11-08") that Netflix should have published by `now`.
/// Charts come out on Tuesday afternoons (Pacific); allow until 22:00 UTC Tuesday.
pub fn latest_published_week(now: i64) -> String {
    let t = now - (2 * 86400 + 22 * 3600);
    let day = t.div_euclid(86400);
    // 1970-01-01 was a Thursday: (day + 4) % 7 is 0 on Sundays.
    let sunday = day - (day + 4).rem_euclid(7);
    let st = utc_date(sunday * 86400);
    format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay)
}

/// The UTC calendar date of a Unix time.
fn utc_date(t: i64) -> windows::Win32::Foundation::SYSTEMTIME {
    // Howard Hinnant's civil_from_days.
    let z = t.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    windows::Win32::Foundation::SYSTEMTIME { wYear: y as u16, wMonth: m as u16, wDay: d as u16, ..Default::default() }
}

pub fn search_url(title: &str) -> String {
    format!("https://www.netflix.com/search?q={}", http::encode(title))
}

/// "12.9M views"
pub fn views_label(n: u64) -> String {
    match n {
        0 => String::new(),
        // Rounded, so 999,500 and up reads as "1M", never "1000K".
        1..=999_499 => format!("{}K views", (n as f64 / 1e3).round() as u64),
        _ => format!("{:.1}M views", n as f64 / 1e6).replace(".0M", "M"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "week\tcategory\tweekly_rank\tshow_title\tseason_title\tweekly_hours_viewed\truntime\tweekly_views\tcumulative_weeks_in_top_10
2026-11-08\tFilms (English)\t1\tA Dog's Perfect Christmas\tN/A\t14500000\t1.6833\t8600000\t1
2026-11-08\tFilms (Non-English)\t1\tCollision Course\tN/A\t30300000\t1.7667\t17200000\t2
2026-11-08\tTV (English)\t1\tTaken\tTaken: Season 1\t25400000\t1.9667\t12900000\t1
2026-11-08\tTV (English)\t2\tNobody Wants This\tNobody Wants This: Season 3\t19000000\t4.75\t4000000\t3
2026-11-08\tTV (Non-English)\t1\tLupin\tLupin: Part 4\t36600000\t6\t6100000\t3
2026-11-08\tTV (Non-English)\t2\tOld Friend\tOld Friend: Season 1\t1000000\t6\t3000000\t5
2026-11-01\tTV (English)\t1\tNobody Wants This\tNobody Wants This: Season 3\t29000000\t4.75\t9000000\t2
2026-11-01\tTV (Non-English)\t1\tLupin\tLupin: Part 4\t36600000\t6\t5000000\t2
2026-10-25\tTV (English)\t1\tOlder\tOlder: Season 1\t1\t1\t1\t1
";

    #[test]
    fn parses_the_latest_two_weeks() {
        let c = parse(TSV);
        assert_eq!(c.week, "2026-11-08");
        assert_eq!(c.rows.len(), 8, "the third week is ignored");
        assert_eq!(c.rows[2].season, "Season 1");
        assert_eq!(c.rows[0].season, "");
        assert!(!c.rows[1].english);
    }

    #[test]
    fn ranks_by_views_with_movement() {
        let c = parse(TSV);
        let shows = top(&c, Kind::Shows, true);
        let titles: Vec<&str> = shows.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Taken", "Lupin", "Nobody Wants This", "Old Friend"]);
        assert_eq!(shows[0].movement, Move::New);
        assert_eq!(shows[1].movement, Move::Same, "Lupin: 2nd last week too");
        assert_eq!(shows[2].movement, Move::Down(2));
        assert_eq!(shows[3].movement, Move::Back, "5 weeks in, absent last week");
        let english = top(&c, Kind::Shows, false);
        assert_eq!(english.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Taken", "Nobody Wants This"]);
        assert_eq!(english[1].movement, Move::Down(1));
        assert_eq!(top(&c, Kind::Films, true)[0].title, "Collision Course");
    }

    #[test]
    fn labels() {
        assert_eq!(views_label(12_900_000), "12.9M views");
        assert_eq!(views_label(3_000_000), "3M views");
        assert_eq!(views_label(830_000), "830K views");
        assert_eq!(search_url("Black Doves"), "https://www.netflix.com/search?q=Black%20Doves");
        assert!(week_label("2026-11-08").contains('8'));
        let st = utc_date(news::parse_date("2024-02-29T00:00:00Z").unwrap());
        assert_eq!((st.wYear, st.wMonth, st.wDay), (2024, 2, 29));
        let st = utc_date(news::parse_date("2026-11-02T00:00:00Z").unwrap() - 6 * 86400);
        assert_eq!((st.wYear, st.wMonth, st.wDay), (2026, 10, 27), "a week spanning two months");
        assert_eq!(week_label("not a date"), "");
        let at = |s: &str| news::parse_date(s).unwrap();
        // Before Tuesday's release the newest chart is the week before last...
        assert_eq!(latest_published_week(at("2026-11-10T12:00:00Z")), "2026-11-01");
        // ...and after it, last week's.
        assert_eq!(latest_published_week(at("2026-11-10T23:00:00Z")), "2026-11-08");
        assert_eq!(latest_published_week(at("2026-11-14T21:00:00Z")), "2026-11-08");
        assert_eq!(latest_published_week(at("2026-11-17T23:00:00Z")), "2026-11-15");
        assert_eq!(parse("garbage").rows.len(), 0);
    }

    #[test]
    #[ignore = "network"]
    fn live_netflix() {
        let body = http::get(URL).unwrap();
        let c = parse(&body);
        println!("week {} ({}): {:?}", c.week, week_label(&c.week), top(&c, Kind::Shows, true).first());
        assert_eq!(top(&c, Kind::Shows, true).len(), 10);
        assert_eq!(top(&c, Kind::Films, false).len(), 10);
    }
}
