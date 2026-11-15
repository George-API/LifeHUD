//! Sports widget controller: game tiles and headlines from `services::sports`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{ImageStore, PAGE_SPORTS, sync};
use crate::services::sports::{self, Game, Prefs, Side, State, Story};
use crate::services::{news, poller::Poller, weather::unix_now};
use crate::settings::Settings;
use crate::{GameData, HudWindow, LinkData, SportsState, TeamLine, locale};

pub struct Sports {
    hud: slint::Weak<HudWindow>,
    poller: RefCell<Option<Poller<Prefs>>>,
    prefs: RefCell<Prefs>,
    games: RefCell<Vec<Game>>,
    stories: RefCell<Vec<Story>>,
    loaded: Cell<bool>,
    offline: Cell<bool>,
    use_24h: Cell<bool>,
    images: RefCell<ImageStore>,
    game_rows: Rc<VecModel<GameData>>,
    story_rows: Rc<VecModel<LinkData>>,
}

impl Sports {
    pub fn new(hud: &HudWindow) -> Self {
        let s = Self {
            hud: hud.as_weak(),
            poller: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            games: RefCell::new(Vec::new()),
            stories: RefCell::new(Vec::new()),
            loaded: Cell::new(false),
            offline: Cell::new(false),
            use_24h: Cell::new(false),
            images: RefCell::new(ImageStore::default()),
            game_rows: Rc::new(VecModel::default()),
            story_rows: Rc::new(VecModel::default()),
        };
        let state = hud.global::<SportsState>();
        state.set_games(ModelRc::from(s.game_rows.clone()));
        state.set_stories(ModelRc::from(s.story_rows.clone()));
        state.on_open(|url| {
            if sports::link_ok(&url) {
                super::open_link(&url);
            }
        });
        state.on_customize(|| crate::app::post(|app| app.open_settings(Some(PAGE_SPORTS))));
        s
    }

    pub fn start(&self, settings: &Settings) {
        let prefs = settings.sports.clone();
        if let Some((games, stories)) = sports::cached(&prefs) {
            *self.games.borrow_mut() = games;
            *self.stories.borrow_mut() = stories;
            self.loaded.set(true);
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render(settings.use_24h);
        let poller = sports::spawn(prefs, |update| {
            crate::app::post(move |app| app.feeds.sports.on_update(update));
        });
        *self.poller.borrow_mut() = Some(poller);
    }

    pub fn set_active(&self, active: bool) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.set_active(active);
        }
    }

    /// New leagues or favourites: refetch, and show "Loading" rather than stale games meanwhile.
    pub fn configure(&self, settings: &Settings) {
        let prefs = settings.sports.clone();
        if prefs == *self.prefs.borrow() {
            return;
        }
        if prefs.leagues != self.prefs.borrow().leagues {
            self.games.borrow_mut().clear();
            self.stories.borrow_mut().clear();
            self.loaded.set(false);
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render(settings.use_24h);
        if let Some(p) = self.poller.borrow().as_ref() {
            p.configure(prefs);
        }
    }

    pub fn on_update(&self, update: sports::Update) {
        match update {
            sports::Update::Games(games) => {
                *self.games.borrow_mut() = games;
                self.loaded.set(true);
                self.offline.set(false);
            }
            sports::Update::Stories(stories) => *self.stories.borrow_mut() = stories,
            sports::Update::Offline => self.offline.set(true),
        }
        self.render(self.use_24h.get());
    }

    pub fn render(&self, use_24h: bool) {
        self.use_24h.set(use_24h);
        let Some(hud) = self.hud.upgrade() else { return };
        let state = hud.global::<SportsState>();
        let prefs = self.prefs.borrow();
        let names: Vec<&str> = prefs.leagues.iter().filter_map(|id| sports::league(id)).map(|l| l.short).collect();
        state.set_label(super::join_names(&names, 3).into());

        let games = self.games.borrow();
        let stories = self.stories.borrow();
        let now = unix_now();
        let today = locale::now();
        let mut images = self.images.borrow_mut();
        let rows = game_rows(&games, now, &today, use_24h, &mut images);
        let links: Vec<LinkData> = stories
            .iter()
            .map(|s| {
                let thumb = images.get(&s.image_url, s.image.as_ref());
                let age = news::age_label(s.published, now);
                LinkData {
                    title: s.title.as_str().into(),
                    meta: if age.is_empty() {
                        s.league.as_str().into()
                    } else {
                        format!("{} · {age}", s.league).into()
                    },
                    short: SharedString::new(),
                    url: s.url.as_str().into(),
                    has_thumb: thumb.is_some(),
                    thumb: thumb.unwrap_or_default(),
                }
            })
            .collect();
        images.retain(
            games
                .iter()
                .flat_map(|g| [&g.first, &g.second])
                .flat_map(|s| [s.logo_url.as_str(), s.logo_dark_url.as_str()])
                .chain(stories.iter().map(|s| s.image_url.as_str())),
        );

        state.set_live_count(games.iter().filter(|g| g.state == State::Live).count() as i32);
        state.set_placeholder(
            if prefs.leagues.is_empty() {
                "Follow a league to see its scores here."
            } else if !self.loaded.get() && self.offline.get() {
                "Scores will appear when you're online."
            } else if !self.loaded.get() {
                "Loading scores…"
            } else {
                "No games in the coming week."
            }
            .into(),
        );
        state.set_news_placeholder(
            if prefs.leagues.is_empty() {
                "Follow a league to see its headlines here."
            } else {
                "Loading headlines…"
            }
            .into(),
        );
        sync(&self.game_rows, rows);
        sync(&self.story_rows, links);
    }
}

pub(crate) fn game_rows(
    games: &[Game],
    now: i64,
    today: &windows::Win32::Foundation::SYSTEMTIME,
    use_24h: bool,
    images: &mut ImageStore,
) -> Vec<GameData> {
    let mut last_section = "";
    games
        .iter()
        .map(|g| {
            let section = section(g);
            let header = if section != last_section { section } else { "" };
            last_section = section;
            let (status, sub) = status_lines(g, now, today, use_24h);
            let called_off = is_called_off(&g.detail);
            let done = g.state == State::Final;
            let decided = done && (g.first.winner || g.second.winner);
            let line = |s: &Side, images: &mut ImageStore| {
                let logo = images.get(&s.logo_url, s.logo.as_ref());
                let logo_dark = images.get(&s.logo_dark_url, s.logo_dark.as_ref());
                TeamLine {
                    abbr: s.abbr.as_str().into(),
                    name: s.name.as_str().into(),
                    score: if called_off { SharedString::new() } else { s.score.as_str().into() },
                    record: s.record.as_str().into(),
                    has_logo: logo.is_some(),
                    logo: logo.unwrap_or_default(),
                    has_logo_dark: logo_dark.is_some(),
                    logo_dark: logo_dark.unwrap_or_default(),
                    winner: decided && s.winner,
                    dim: decided && !s.winner,
                }
            };
            GameData {
                id: g.id.as_str().into(),
                league: g.league.as_str().into(),
                status: status.into(),
                sub: sub.into(),
                live: g.state == State::Live,
                done: done && !called_off,
                favorite: g.favorite,
                header: header.into(),
                first: line(&g.first, images),
                second: line(&g.second, images),
                url: g.url.as_str().into(),
            }
        })
        .collect()
}

fn section(g: &Game) -> &'static str {
    if g.favorite {
        return "YOUR TEAMS";
    }
    match g.state {
        State::Live => "LIVE",
        State::Pre => "UPCOMING",
        State::Final => "FINAL",
    }
}

fn is_called_off(detail: &str) -> bool {
    let d = detail.to_ascii_lowercase();
    ["postponed", "canceled", "cancelled", "suspended", "abandoned", "delayed"].iter().any(|w| d.contains(w))
}

/// Two short lines for a game's status: ("7:24", "3rd"), ("Final", "Yesterday"), ("7:00 PM", "Today").
pub(crate) fn status_lines(
    g: &Game,
    now: i64,
    today: &windows::Win32::Foundation::SYSTEMTIME,
    use_24h: bool,
) -> (String, String) {
    let local = locale::local_from_unix(g.start);
    let day = local.as_ref().map(|st| locale::day_label(st, today)).unwrap_or_default();
    if is_called_off(&g.detail) {
        return (g.detail.clone(), day);
    }
    match g.state {
        State::Live => match g.detail.split_once(" - ") {
            Some((clock, period)) => (clock.trim().to_string(), period.trim().to_string()),
            None => (g.detail.clone(), String::new()),
        },
        State::Final => {
            let label = if g.detail.is_empty() { "Final".to_string() } else { g.detail.clone() };
            (label, if day == "Today" { String::new() } else { day })
        }
        State::Pre => {
            let time = local.as_ref().map(|st| locale::time_label(st, use_24h)).unwrap_or_default();
            if g.start > now && g.start - now < 3600 {
                (time, format!("in {}m", ((g.start - now) / 60).max(1)))
            } else {
                // "Today" under every game is noise; only name other days.
                (time, if day == "Today" { String::new() } else { day })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(state: State, detail: &str, start: i64, favorite: bool) -> Game {
        let side = |abbr: &str, winner| Side {
            team_id: abbr.into(),
            abbr: abbr.into(),
            name: abbr.into(),
            score: "3".into(),
            record: String::new(),
            winner,
            logo_url: String::new(),
            logo_dark_url: String::new(),
            logo: None,
            logo_dark: None,
        };
        Game {
            id: detail.into(),
            league: "NHL".into(),
            start,
            state,
            detail: detail.into(),
            url: String::new(),
            favorite,
            first: side("TOR", true),
            second: side("MTL", false),
        }
    }

    #[test]
    fn status_lines_split_live_clock_and_period() {
        let today = locale::now();
        let now = unix_now();
        assert_eq!(
            status_lines(&game(State::Live, "12:34 - 2nd", now, false), now, &today, true),
            ("12:34".into(), "2nd".into())
        );
        assert_eq!(status_lines(&game(State::Live, "Halftime", now, false), now, &today, true).0, "Halftime");
        assert_eq!(
            status_lines(&game(State::Final, "Final/OT", now, false), now, &today, true),
            ("Final/OT".into(), "".into())
        );
        let soon = status_lines(&game(State::Pre, "", now + 1500, false), now, &today, true);
        assert_eq!(soon.1, "in 25m");
        assert_eq!(status_lines(&game(State::Final, "Postponed", now, false), now, &today, true).0, "Postponed");
    }

    #[test]
    fn rows_get_section_headers_and_winner_styling() {
        let now = unix_now();
        let games = vec![
            game(State::Final, "Final", now, true),
            game(State::Live, "1:00 - 3rd", now, false),
            game(State::Live, "2:00 - 1st", now, false),
            game(State::Final, "Postponed", now, false),
        ];
        let rows = game_rows(&games, now, &locale::now(), false, &mut ImageStore::default());
        let headers: Vec<&str> = rows.iter().map(|r| r.header.as_str()).collect();
        assert_eq!(headers, ["YOUR TEAMS", "LIVE", "", "FINAL"]);
        assert!(rows[0].first.winner && rows[0].second.dim && rows[0].done);
        assert!(!rows[1].first.winner && !rows[1].second.dim);
        assert!(!rows[3].done && rows[3].first.score.is_empty(), "no scores for a postponed game");
    }
}
