//! Scores, schedules and headlines for the leagues you follow, from ESPN's public site API (the
//! same JSON espn.com's scoreboards use; no key or account). Refreshes every 30 seconds while a
//! game is live, every minute just before a start, and every 10 minutes otherwise.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::services::{http, images, news, poller};
use crate::storage;

const CACHE: &str = "sports-cache.json";
const API: &str = "https://site.api.espn.com/apis/site/v2/sports";
const MAX_GAMES: usize = 30;
const MAX_STORIES: usize = 15;
const NEWS_EVERY: Duration = Duration::from_secs(20 * 60);
const RETRY: Duration = Duration::from_secs(3 * 60);
pub const MAX_FAVORITES: usize = 40;
/// ESPN requests in flight at once.
const PARALLEL: usize = 6;
/// Team logos are drawn at 20px; decode at 2× for crisp high-DPI rendering.
pub const LOGO_PX: u32 = 44;
pub const THUMB: (u32, u32) = (128, 72);
/// ESPN's image CDN; nothing else is fetched for this widget.
const IMAGE_HOSTS: &[&str] = &["espncdn.com"];

/// Only ESPN pages are opened from scores and headlines.
pub fn link_ok(url: &str) -> bool {
    images::host_allowed(url, &["espn.com"])
}

pub struct League {
    pub id: &'static str,
    pub name: &'static str,
    /// Short label shown on game rows ("NBA", "EPL").
    pub short: &'static str,
    path: &'static str,
    /// Soccer lists the home side first; North American leagues write "away @ home".
    pub home_first: bool,
}

const fn l(id: &'static str, name: &'static str, short: &'static str, path: &'static str, home_first: bool) -> League {
    League { id, name, short, path, home_first }
}

pub const LEAGUES: &[League] = &[
    l("nba", "NBA", "NBA", "basketball/nba", false),
    l("nhl", "NHL", "NHL", "hockey/nhl", false),
    l("nfl", "NFL", "NFL", "football/nfl", false),
    l("mlb", "MLB", "MLB", "baseball/mlb", false),
    l("wnba", "WNBA", "WNBA", "basketball/wnba", false),
    l("mls", "MLS", "MLS", "soccer/usa.1", true),
    l("epl", "Premier League", "EPL", "soccer/eng.1", true),
    l("laliga", "LaLiga", "LIGA", "soccer/esp.1", true),
    l("bundesliga", "Bundesliga", "BUND", "soccer/ger.1", true),
    l("seriea", "Serie A", "SERA", "soccer/ita.1", true),
    l("ligue1", "Ligue 1", "L1", "soccer/fra.1", true),
    l("ucl", "Champions League", "UCL", "soccer/uefa.champions", true),
    l("cfb", "College Football", "NCAAF", "football/college-football", false),
];

pub fn league(id: &str) -> Option<&'static League> {
    LEAGUES.iter().find(|l| l.id == id)
}

/// A sensible first-run pick from the user's region.
pub fn default_leagues() -> Vec<String> {
    let ids: &[&str] = match crate::locale::region().as_str() {
        "US" => &["nfl", "nba"],
        "CA" => &["nhl", "nba"],
        "GB" | "IE" => &["epl", "ucl"],
        "ES" => &["laliga", "ucl"],
        "DE" | "AT" => &["bundesliga", "ucl"],
        "IT" => &["seriea", "ucl"],
        "FR" => &["ligue1", "ucl"],
        _ => &["epl", "nba"],
    };
    ids.iter().map(|s| s.to_string()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub league: String,
    /// ESPN team id (stable across seasons; abbreviations occasionally change).
    pub id: String,
    pub abbr: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub leagues: Vec<String>,
    pub favorites: Vec<Favorite>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { leagues: default_leagues(), favorites: Vec::new() }
    }
}

impl Prefs {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.leagues.retain(|id| league(id).is_some() && seen.insert(id.clone()));
        // Favourites in a league you stop following are kept (just not shown), so following it
        // again brings them back.
        let mut seen = std::collections::HashSet::new();
        self.favorites.retain(|f| league(&f.league).is_some() && seen.insert((f.league.clone(), f.id.clone())));
        self.favorites.truncate(MAX_FAVORITES);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Pre,
    Live,
    Final,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Side {
    pub team_id: String,
    pub abbr: String,
    pub name: String,
    pub score: String,
    pub record: String,
    pub winner: bool,
    pub logo_url: String,
    /// ESPN's variant for dark backgrounds (dark logos vanish on dark glass otherwise).
    #[serde(default)]
    pub logo_dark_url: String,
    #[serde(skip)]
    pub logo: Option<images::Shared>,
    #[serde(skip)]
    pub logo_dark: Option<images::Shared>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub league: String,
    pub start: i64,
    pub state: State,
    /// ESPN's short status: "7:24 - 3rd", "Halftime", "Final/OT", "FT", "Postponed".
    pub detail: String,
    pub url: String,
    pub favorite: bool,
    /// Listed first: home in soccer, away elsewhere.
    pub first: Side,
    pub second: Side,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Story {
    pub title: String,
    pub url: String,
    pub published: Option<i64>,
    pub league: String,
    pub image_url: String,
    #[serde(skip)]
    pub image: Option<images::Shared>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    leagues: Vec<String>,
    games: Vec<Game>,
    stories: Vec<Story>,
}

pub enum Update {
    Games(Vec<Game>),
    Stories(Vec<Story>),
    /// Nothing could be loaded and there's nothing to show.
    Offline,
}

/// Last saved games and stories, if they were for these leagues.
pub fn cached(prefs: &Prefs) -> Option<(Vec<Game>, Vec<Story>)> {
    let c: Cache = storage::load_json(CACHE)?;
    (c.leagues == prefs.leagues).then_some((c.games, c.stories))
}

pub fn spawn(prefs: Prefs, sink: impl Fn(Update) + Send + 'static) -> poller::Poller<Prefs> {
    let mut fetcher = Fetcher::new();
    poller::spawn("sports", prefs, move |prefs: &Prefs| fetcher.run(prefs, &sink))
}

/// One refresh cycle's state: the images on screen and when headlines were last fetched.
/// One league's latest scoreboard and when to fetch it again.
struct Board {
    at: Instant,
    every: Duration,
    games: Vec<Game>,
}

pub struct Fetcher {
    logos: images::Cache,
    thumbs: images::Cache,
    /// By league id. Each league refreshes on its own cadence, so one live NHL game doesn't make
    /// the app poll every other league every 30 seconds.
    boards: HashMap<String, Board>,
    news_at: Option<(Instant, Vec<String>)>,
    have_data: bool,
}

impl Fetcher {
    pub fn new() -> Self {
        Self {
            logos: images::Cache::new(LOGO_PX, LOGO_PX, IMAGE_HOSTS),
            thumbs: images::Cache::new(THUMB.0, THUMB.1, IMAGE_HOSTS),
            boards: HashMap::new(),
            news_at: None,
            have_data: false,
        }
    }

    /// Fetches what's due, reports it through `sink`, and returns when to refresh next.
    pub fn run(&mut self, prefs: &Prefs, sink: &dyn Fn(Update)) -> Duration {
        let now = crate::services::weather::unix_now();
        self.boards.retain(|id, _| prefs.leagues.contains(id));
        let mut failures = 0;
        let due: Vec<&League> = prefs
            .leagues
            .iter()
            .filter(|id| !self.boards.get(*id).is_some_and(|b| b.at.elapsed() < b.every))
            .filter_map(|id| league(id))
            .collect();
        // A response without an `events` list means ESPN changed its format (or sent an error
        // page); treat it as a failure, not as "no games".
        let bodies = poller::fan_out(&due, PARALLEL, |lg| {
            http::get(&format!("{API}/{}/scoreboard", lg.path))
                .map_err(|e| e.to_string())
                .and_then(|body| if is_scoreboard(&body) { Ok(body) } else { Err("unexpected response".to_string()) })
        });
        for (lg, fetched) in due.into_iter().zip(bodies) {
            let id = &lg.id.to_string();
            match fetched {
                Ok(body) => {
                    let games = parse_scoreboard(&body, lg, &[]);
                    let every = next_refresh(&select(games.clone(), now), now, false);
                    self.boards.insert(id.clone(), Board { at: Instant::now(), every, games });
                }
                Err(e) => {
                    failures += 1;
                    storage::log(format!("sports refresh failed ({}): {e}", lg.name));
                    // Keep showing the last scoreboard; try again in a few minutes.
                    let board = self.boards.entry(id.clone()).or_insert(Board {
                        at: Instant::now(),
                        every: RETRY,
                        games: Vec::new(),
                    });
                    board.at = Instant::now();
                    board.every = RETRY;
                }
            }
        }
        let all_failed = failures > 0 && failures == prefs.leagues.len();

        // Favourites are applied here rather than when parsing, so editing them needs no refetch.
        let mut games: Vec<Game> = Vec::new();
        for (id, board) in &self.boards {
            for g in &board.games {
                let mut g = g.clone();
                g.favorite = prefs
                    .favorites
                    .iter()
                    .any(|f| f.league == *id && (f.id == g.first.team_id || f.id == g.second.team_id));
                games.push(g);
            }
        }
        let games = select(games, now);
        let mut cache: Cache = storage::load_json(CACHE).unwrap_or_default();
        if !all_failed || self.have_data {
            let mut games = games;
            let urls: Vec<String> = games
                .iter()
                .flat_map(|g| [&g.first, &g.second])
                .flat_map(|s| [logo_url(&s.logo_url), logo_url(&s.logo_dark_url)])
                .collect();
            self.logos.prefetch(urls.iter().map(|u| u.as_str()));
            for g in &mut games {
                for side in [&mut g.first, &mut g.second] {
                    side.logo = self.logos.get(&logo_url(&side.logo_url));
                    side.logo_dark = self.logos.get(&logo_url(&side.logo_dark_url));
                }
            }
            // The cache is keyed by the resized URL, so keep exactly those.
            self.logos.retain(urls.iter().map(|u| u.as_str()));
            cache.games = games.clone();
            sink(Update::Games(games));
            self.have_data = true;
        }

        // Headlines change slowly; fetch them on their own, slower clock.
        let news_due =
            self.news_at.as_ref().is_none_or(|(at, leagues)| at.elapsed() >= NEWS_EVERY || *leagues != prefs.leagues);
        if news_due && !all_failed {
            let leagues: Vec<&League> = prefs.leagues.iter().filter_map(|id| league(id)).collect();
            let mut stories: Vec<Story> = poller::fan_out(&leagues, PARALLEL, |lg| {
                http::get(&format!("{API}/{}/news?limit=10", lg.path)).map(|b| parse_news(&b, lg)).unwrap_or_default()
            })
            .into_iter()
            .flatten()
            .collect();
            stories.sort_by_key(|s| std::cmp::Reverse(s.published));
            let mut seen = std::collections::HashSet::new();
            stories.retain(|s| seen.insert(s.url.clone()));
            stories.truncate(MAX_STORIES);
            self.thumbs.prefetch(stories.iter().map(|s| s.image_url.as_str()));
            for s in &mut stories {
                s.image = self.thumbs.get(&s.image_url);
            }
            self.thumbs.retain(stories.iter().map(|s| s.image_url.as_str()));
            cache.stories = stories.clone();
            sink(Update::Stories(stories));
            self.news_at = Some((Instant::now(), prefs.leagues.clone()));
        }
        cache.leagues = prefs.leagues.clone();
        if !all_failed {
            storage::save_json(CACHE, &cache);
        } else if !self.have_data {
            sink(Update::Offline);
        }
        // Wake for whichever league (or the headlines) is due first.
        let news_wait = self.news_at.as_ref().map(|(at, _)| NEWS_EVERY.saturating_sub(at.elapsed())).unwrap_or(RETRY);
        self.boards
            .values()
            .map(|b| b.every.saturating_sub(b.at.elapsed()))
            .chain([news_wait])
            .min()
            .unwrap_or(Duration::from_secs(10 * 60))
            .max(Duration::from_secs(5))
    }
}

/// 30s while anything is live, 1 min when a game starts within 15 min, else 10 min.
fn next_refresh(games: &[Game], now: i64, failed: bool) -> Duration {
    if failed {
        return RETRY;
    }
    if games.iter().any(|g| g.state == State::Live) {
        Duration::from_secs(30)
    } else if games.iter().any(|g| g.state == State::Pre && g.start - now < 15 * 60) {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(10 * 60)
    }
}

/// ESPN's image resizer returns a small PNG instead of the 500px original.
fn logo_url(url: &str) -> String {
    match url.split_once("a.espncdn.com") {
        Some((_, path)) if path.starts_with("/i/") => {
            format!("https://a.espncdn.com/combiner/i?img={path}&w={LOGO_PX}&h={LOGO_PX}")
        }
        _ => url.to_string(),
    }
}

/// Keeps what's relevant now (live games, finals from the last day and a half, the coming week)
/// and orders it: your teams first, then live, upcoming soonest-first, then recent finals.
pub fn select(mut games: Vec<Game>, now: i64) -> Vec<Game> {
    games.retain(|g| match g.state {
        State::Live => true,
        State::Pre => g.start - now < 8 * 86400,
        State::Final => now - g.start < if g.favorite { 3 * 86400 } else { 30 * 3600 },
    });
    games.sort_by_key(|g| {
        let state = match g.state {
            State::Live => 0,
            State::Pre => 1,
            State::Final => 2,
        };
        let time = if g.state == State::Final { -g.start } else { g.start };
        (!g.favorite, state, time)
    });
    games.dedup_by(|a, b| a.id == b.id);
    games.truncate(MAX_GAMES);
    games
}

/// Whether a body looks like an ESPN scoreboard at all.
pub fn is_scoreboard(body: &str) -> bool {
    serde_json::from_str::<Value>(body).is_ok_and(|v| v["events"].is_array())
}

pub fn parse_scoreboard(body: &str, lg: &League, favorites: &[Favorite]) -> Vec<Game> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut out = Vec::new();
    for e in v["events"].as_array().into_iter().flatten() {
        let c = &e["competitions"][0];
        let status = &c["status"]["type"];
        let state = match status["state"].as_str() {
            Some("in") => State::Live,
            Some("post") => State::Final,
            _ => State::Pre,
        };
        let side = |t: &Value| Side {
            team_id: s(&t["team"]["id"]),
            abbr: s(&t["team"]["abbreviation"]),
            name: {
                let short = s(&t["team"]["shortDisplayName"]);
                if short.is_empty() { s(&t["team"]["displayName"]) } else { short }
            },
            score: s(&t["score"]),
            record: s(&t["records"][0]["summary"]),
            winner: t["winner"].as_bool().unwrap_or(false),
            logo_url: s(&t["team"]["logo"]),
            logo_dark_url: s(&t["team"]["logoDark"]),
            logo: None,
            logo_dark: None,
        };
        let comps = c["competitors"].as_array().cloned().unwrap_or_default();
        let find = |which: &str| comps.iter().find(|t| t["homeAway"] == which).map(side);
        let (Some(home), Some(away)) = (find("home"), find("away")) else { continue };
        let favorite = favorites.iter().any(|f| f.league == lg.id && (f.id == home.team_id || f.id == away.team_id));
        let (first, second) = if lg.home_first { (home, away) } else { (away, home) };
        // Without a start time a game can't be placed or labelled ("TBD" fixtures); skip it.
        let Some(start) = news::parse_date(&iso_seconds(&s(&e["date"]))) else { continue };
        let url = e["links"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["href"].as_str().is_some_and(|h| h.starts_with("https://"))))
            .map(|l| s(&l["href"]))
            .unwrap_or_default();
        out.push(Game {
            id: s(&e["id"]),
            league: lg.short.into(),
            start,
            state,
            detail: s(&status["shortDetail"]),
            url,
            favorite,
            first,
            second,
        });
    }
    out
}

/// ESPN writes "2026-11-15T00:00Z"; add the seconds our ISO parser expects.
fn iso_seconds(s: &str) -> String {
    if s.len() == 17 && s.ends_with('Z') { format!("{}:00Z", &s[..16]) } else { s.to_string() }
}

pub fn parse_news(body: &str, lg: &League) -> Vec<Story> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    v["articles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let title = a["headline"].as_str()?.trim().to_string();
            let url = a["links"]["web"]["href"].as_str()?.to_string();
            (!title.is_empty() && url.starts_with("https://")).then(|| Story {
                title,
                url,
                published: a["published"].as_str().and_then(news::parse_date),
                league: lg.short.into(),
                image_url: a["images"][0]["url"].as_str().unwrap_or("").to_string(),
                image: None,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub league: &'static str,
    pub id: String,
    pub abbr: String,
    pub name: String,
}

/// Every team in a league, for picking favourites in Settings.
/// Errors are reported (not an empty list) so Settings can tell "offline" from "no such team".
pub fn teams(league_id: &str) -> Result<Vec<Team>, String> {
    let Some(lg) = league(league_id) else { return Ok(Vec::new()) };
    let body = http::get(&format!("{API}/{}/teams?limit=400", lg.path)).map_err(|e| e.to_string())?;
    let teams = parse_teams(&body, lg);
    if teams.is_empty() { Err("ESPN sent no teams".into()) } else { Ok(teams) }
}

pub fn parse_teams(body: &str, lg: &'static League) -> Vec<Team> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let mut out: Vec<Team> = v["sports"][0]["leagues"][0]["teams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let t = &t["team"];
            Some(Team {
                league: lg.id,
                id: t["id"].as_str()?.to_string(),
                abbr: t["abbreviation"].as_str().unwrap_or("").to_string(),
                name: t["displayName"].as_str()?.to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"{"events":[
      {"id":"1","date":"2026-11-15T00:00Z","links":[{"href":"https://www.espn.com/nba/game/_/gameId/1"}],
       "competitions":[{"status":{"type":{"state":"pre","shortDetail":"11/14 - 7:00 PM EST"}},
         "competitors":[
           {"homeAway":"home","score":"0","team":{"id":"3","abbreviation":"NO","shortDisplayName":"Pelicans","logo":"https://a.espncdn.com/i/teamlogos/nba/500/scoreboard/no.png"},"records":[{"summary":"4-9"}]},
           {"homeAway":"away","score":"0","team":{"id":"7","abbreviation":"DEN","shortDisplayName":"Nuggets"}}]}]},
      {"id":"2","date":"2026-11-14T20:30Z","links":[],
       "competitions":[{"status":{"type":{"state":"in","shortDetail":"7:24 - 3rd"}},
         "competitors":[
           {"homeAway":"home","score":"88","team":{"id":"2","abbreviation":"BOS","shortDisplayName":"Celtics"}},
           {"homeAway":"away","score":"91","team":{"id":"5","abbreviation":"CLE","shortDisplayName":"Cavaliers"}}]}]}
    ]}"#;

    #[test]
    fn parses_scoreboard_sides_and_states() {
        let fav =
            [Favorite { league: "nba".into(), id: "7".into(), abbr: "DEN".into(), name: "Denver Nuggets".into() }];
        let games = parse_scoreboard(BOARD, league("nba").unwrap(), &fav);
        assert_eq!(games.len(), 2);
        let g = &games[0];
        assert_eq!((g.first.abbr.as_str(), g.second.abbr.as_str()), ("DEN", "NO"), "away @ home");
        assert_eq!(g.second.record, "4-9");
        assert!(g.favorite);
        assert_eq!(g.state, State::Pre);
        assert_eq!(g.start, news::parse_date("2026-11-15T00:00:00Z").unwrap());
        assert_eq!(games[1].state, State::Live);
        assert_eq!(games[1].first.score, "91");
        assert!(games[1].url.is_empty());
    }

    #[test]
    fn format_changes_are_failures_not_empty_days() {
        assert!(is_scoreboard(BOARD));
        assert!(is_scoreboard(r#"{"events":[]}"#), "a genuinely empty day");
        assert!(!is_scoreboard(r#"{"code":503,"message":"unavailable"}"#));
        assert!(!is_scoreboard("<html>Service Unavailable</html>"));
    }

    #[test]
    fn soccer_lists_home_first() {
        let games = parse_scoreboard(BOARD, league("epl").unwrap(), &[]);
        assert_eq!(games[0].first.abbr, "NO");
    }

    #[test]
    fn selection_orders_favourites_live_upcoming_final() {
        let now = 1_000_000;
        let g = |id: &str, state, start, favorite| Game {
            id: id.into(),
            league: "NBA".into(),
            start,
            state,
            detail: String::new(),
            url: String::new(),
            favorite,
            first: side(),
            second: side(),
        };
        let games = vec![
            g("old-final", State::Final, now - 40 * 3600, false),
            g("final", State::Final, now - 3 * 3600, false),
            g("later", State::Pre, now + 7200, false),
            g("soon", State::Pre, now + 600, false),
            g("live", State::Live, now - 3600, false),
            g("fav-final", State::Final, now - 50 * 3600, true),
            g("far", State::Pre, now + 10 * 86400, false),
        ];
        let ids: Vec<String> = select(games, now).into_iter().map(|g| g.id).collect();
        assert_eq!(ids, ["fav-final", "live", "soon", "later", "final"]);
        assert_eq!(next_refresh(&[], now, true), Duration::from_secs(180));
    }

    fn side() -> Side {
        Side {
            team_id: String::new(),
            abbr: String::new(),
            name: String::new(),
            score: String::new(),
            record: String::new(),
            winner: false,
            logo_url: String::new(),
            logo_dark_url: String::new(),
            logo: None,
            logo_dark: None,
        }
    }

    #[test]
    fn resizes_logos_through_the_cdn() {
        assert_eq!(
            logo_url("https://a.espncdn.com/i/teamlogos/nba/500/scoreboard/no.png"),
            format!(
                "https://a.espncdn.com/combiner/i?img=/i/teamlogos/nba/500/scoreboard/no.png&w={LOGO_PX}&h={LOGO_PX}"
            )
        );
        assert_eq!(logo_url("https://example.com/x.png"), "https://example.com/x.png");
    }

    #[test]
    fn prefs_drop_unknown_leagues_and_duplicate_favourites() {
        let f = |league: &str, id: &str| Favorite {
            league: league.into(),
            id: id.into(),
            abbr: "X".into(),
            name: "X".into(),
        };
        let mut p = Prefs {
            leagues: vec!["nba".into(), "bogus".into(), "nba".into()],
            favorites: vec![f("nhl", "1"), f("nba", "2"), f("nhl", "1"), f("bogus", "3")],
        };
        p.normalize();
        assert_eq!(p.leagues, ["nba"]);
        assert_eq!(
            p.favorites,
            [f("nhl", "1"), f("nba", "2")],
            "kept across leagues, deduped, unknown leagues dropped"
        );
    }

    #[test]
    fn league_ids_are_unique() {
        let mut ids: Vec<_> = LEAGUES.iter().map(|l| l.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), LEAGUES.len());
    }

    #[test]
    #[ignore = "network"]
    fn live_espn_endpoints() {
        for lg in LEAGUES {
            let body = http::get(&format!("{API}/{}/scoreboard", lg.path)).unwrap();
            let games = parse_scoreboard(&body, lg, &[]);
            println!("{}: {} games", lg.name, games.len());
        }
        let lg = league("nba").unwrap();
        assert!(!parse_news(&http::get(&format!("{API}/{}/news", lg.path)).unwrap(), lg).is_empty());
        assert!(teams("nhl").unwrap().len() >= 32);
    }
}
