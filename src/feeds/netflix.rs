//! Netflix widget controller: ranks the cached chart for the current setting, so toggling
//! "Include non-English titles" re-ranks instantly without a network round trip.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::sync;
use crate::services::netflix::{self, Chart, Entry, Kind, Move, Prefs};
use crate::services::poller::Poller;
use crate::settings::Settings;
use crate::{HudWindow, NetflixState, TitleData, app};

pub struct Netflix {
    hud: slint::Weak<HudWindow>,
    poller: RefCell<Option<Poller<()>>>,
    prefs: RefCell<Prefs>,
    chart: RefCell<Option<Chart>>,
    offline: Cell<bool>,
    shows_rows: Rc<VecModel<TitleData>>,
    films_rows: Rc<VecModel<TitleData>>,
}

impl Netflix {
    pub fn new(hud: &HudWindow) -> Self {
        let s = Self {
            hud: hud.as_weak(),
            poller: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            chart: RefCell::new(None),
            offline: Cell::new(false),
            shows_rows: Rc::new(VecModel::default()),
            films_rows: Rc::new(VecModel::default()),
        };
        let state = hud.global::<NetflixState>();
        state.set_shows(ModelRc::from(s.shows_rows.clone()));
        state.set_films(ModelRc::from(s.films_rows.clone()));
        state.on_open(|url| {
            if url.starts_with("https://www.netflix.com/") {
                app::post(move |a| a.feeds.netflix.open(a, &url));
            }
        });
        s
    }

    /// A title: in the widget (the Netflix page docked there), or the browser.
    fn open(&self, a: &app::App, url: &str) {
        if self.prefs.borrow().in_widget && a.feeds.player.available() {
            a.feeds.player.open_netflix(url, &title_from(url));
        } else {
            super::open_link(url);
        }
    }

    pub fn start(&self, settings: &Settings) {
        *self.prefs.borrow_mut() = settings.netflix.clone();
        *self.chart.borrow_mut() = netflix::cached();
        self.render();
        let poller = netflix::spawn(|u| app::post(move |a| a.feeds.netflix.on_update(u)));
        *self.poller.borrow_mut() = Some(poller);
    }

    pub fn set_active(&self, active: bool) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.set_active(active);
        }
    }

    pub fn configure(&self, settings: &Settings) {
        if settings.netflix != *self.prefs.borrow() {
            *self.prefs.borrow_mut() = settings.netflix.clone();
            self.render();
        }
    }

    pub fn on_update(&self, update: netflix::Update) {
        match update {
            netflix::Update::Chart(chart) => {
                *self.chart.borrow_mut() = Some(chart);
                self.offline.set(false);
            }
            netflix::Update::Offline => self.offline.set(true),
        }
        self.render();
    }

    pub fn render(&self) {
        let Some(hud) = self.hud.upgrade() else { return };
        let state = hud.global::<NetflixState>();
        let non_english = self.prefs.borrow().non_english;
        let chart = self.chart.borrow();
        let (shows, films, week) = match chart.as_ref() {
            Some(c) => (
                top_rows(c, Kind::Shows, non_english),
                top_rows(c, Kind::Films, non_english),
                netflix::week_label(&c.week),
            ),
            None => (Vec::new(), Vec::new(), String::new()),
        };
        state.set_week(week.into());
        state.set_placeholder(
            if self.offline.get() {
                "The Top 10 will appear when you're online."
            } else {
                "Loading this week's Top 10…"
            }
            .into(),
        );
        sync(&self.shows_rows, shows);
        sync(&self.films_rows, films);
    }
}

/// The title a Netflix search URL asks for ("…/search?q=Stranger%20Things" → "Stranger Things").
fn title_from(url: &str) -> String {
    let q = url.split_once("q=").map(|(_, q)| q.split('&').next().unwrap_or(q)).unwrap_or("");
    let bytes = q.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    let title = String::from_utf8_lossy(&out).trim().to_string();
    if title.is_empty() { "Netflix".into() } else { title }
}

fn top_rows(chart: &Chart, kind: Kind, non_english: bool) -> Vec<TitleData> {
    netflix::top(chart, kind, non_english).iter().map(title_data).collect()
}

pub(crate) fn title_data(e: &Entry) -> TitleData {
    let weeks = match e.weeks {
        0 | 1 => "1st week".to_string(),
        n => format!("{n} weeks"),
    };
    let detail = if e.season.is_empty() { weeks } else { format!("{} · {weeks}", e.season) };
    let (movement, trend) = match e.movement {
        Move::New => ("NEW".to_string(), 2),
        Move::Up(n) => (format!("▲ {n}"), 1),
        Move::Down(n) => (format!("▼ {n}"), -1),
        Move::Back => ("BACK".to_string(), 0),
        Move::Same => (String::new(), 0),
    };
    TitleData {
        rank: e.rank as i32,
        title: e.title.as_str().into(),
        detail: detail.into(),
        views: netflix::views_label(e.views).into(),
        movement: movement.into(),
        trend,
        url: netflix::search_url(&e.title).into(),
    }
}

#[cfg(test)]
mod tests {
    use crate::services::netflix::{Entry, Move};

    #[test]
    fn rows_describe_season_weeks_and_movement() {
        let e = Entry {
            rank: 3,
            title: "Lupin".into(),
            season: "Part 4".into(),
            views: 6_100_000,
            weeks: 3,
            english: false,
            movement: Move::Up(2),
        };
        let r = super::title_data(&e);
        assert_eq!((r.detail.as_str(), r.movement.as_str(), r.trend), ("Part 4 · 3 weeks", "▲ 2", 1));
        assert_eq!(r.views, "6.1M views");
        let r = super::title_data(&Entry { season: String::new(), weeks: 1, movement: Move::New, ..e });
        assert_eq!((r.detail.as_str(), r.movement.as_str(), r.trend), ("1st week", "NEW", 2));
    }
}
