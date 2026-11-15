//! The docked video player: an Edge window kept exactly over the YouTube or Netflix widget's player
//! area (the window mechanics are in `platform::docked`). One at a time; opening another replaces it.
//!
//! A short timer runs only while a player is open. Each tick it finds the new window (once), keeps it
//! adopted and over the widget's player area, hides it while the HUD folds between layouts, starts
//! YouTube playback with a click in the page, and gives up cleanly: if the window never appears it's
//! left as an ordinary Edge window; if the user closes it, the widget goes back to normal; if the
//! widget leaves the HUD (or the HUD is hidden) for a few seconds, the video is closed rather than
//! left playing out of sight.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};
use windows::Win32::Foundation::{HWND, RECT};

use crate::platform::docked;
use crate::services::embed;
use crate::{HudState, HudWindow, PlayerState, app};

const TICK: Duration = Duration::from_millis(120);
/// How long to wait for Edge's window before leaving it be.
const FIND_FOR: Duration = Duration::from_secs(12);
/// Out of sight this long (widget removed, HUD hidden) and the video is closed.
const CLOSE_HIDDEN_AFTER: Duration = Duration::from_secs(3);
/// Between clicks while waiting for YouTube's page to start.
const CLICK_EVERY: Duration = Duration::from_millis(600);
const MAX_CLICKS: u32 = 15;

struct Session {
    site: &'static str,
    /// Where "Open in browser" goes.
    browser_url: String,
    before: HashSet<isize>,
    started: Instant,
    hwnd: Option<HWND>,
    /// Catches the window the moment it shows (until it's found).
    watch: Option<docked::ShowWatch>,
    /// YouTube's page waits for a click before it may play with sound.
    needs_click: bool,
    clicks: u32,
    last_click: Option<Instant>,
    placed: Option<RECT>,
    hidden_since: Option<Instant>,
}

pub struct Player {
    hud: slint::Weak<HudWindow>,
    session: RefCell<Option<Session>>,
    timer: Timer,
    /// Widget ids on the HUD right now (empty while the HUD is hidden).
    on_hud: RefCell<Vec<String>>,
    /// Edge isn't installed (or wouldn't start): videos open in the browser instead.
    edge_missing: Cell<bool>,
}

impl Player {
    pub fn new(hud: &HudWindow) -> Self {
        let state = hud.global::<PlayerState>();
        state.on_close(|| app::post(|a| a.feeds.player.close()));
        state.on_pop_out(|| app::post(|a| a.feeds.player.pop_out()));
        Self {
            hud: hud.as_weak(),
            session: RefCell::new(None),
            timer: Timer::default(),
            on_hud: RefCell::new(Vec::new()),
            edge_missing: Cell::new(docked::edge().is_none()),
        }
    }

    /// Whether videos can play in a widget at all (Edge is installed).
    pub fn available(&self) -> bool {
        !self.edge_missing.get()
    }

    /// Plays a YouTube video in the YouTube widget.
    pub fn play_youtube(&self, id: &str, title: &str) {
        let browser = crate::services::youtube::watch_url(id);
        match embed::youtube_url(id) {
            Some(page) => self.open("youtube", &page, &browser, title, true),
            None => super::open_link(&browser),
        }
    }

    /// Opens a Netflix page in the Netflix widget (the user picks and plays there).
    pub fn open_netflix(&self, url: &str, title: &str) {
        self.open("netflix", url, url, title, false);
    }

    fn open(&self, site: &'static str, page: &str, browser_url: &str, title: &str, needs_click: bool) {
        self.close();
        let Some(hud) = self.hud.upgrade() else { return };
        // Start near where it'll end up, so the moment before it's adopted is less jarring.
        let state = hud.global::<PlayerState>();
        let scale = hud.window().scale_factor();
        let pos = hud.window().position();
        let at = ((pos.x as f32 / scale + state.get_x()) as i32, (pos.y as f32 / scale + state.get_y()) as i32);
        let size = (state.get_width().max(320.0) as i32, state.get_height().max(180.0) as i32);
        // The widget switches to its player area first, so it's laid out (and has reported where it
        // is) by the time Edge's window shows.
        state.set_site(site.into());
        state.set_title(title.into());
        state.set_loading(true);
        state.set_active(true);
        let before = docked::edge_windows();
        // Adopt and place the window the instant it shows, before it paints: it never appears as a
        // separate window. (The tick's polling below is only the fallback.)
        let weak = self.hud.clone();
        let watch = docked::watch_for_new(before.clone(), move |h| {
            let Some(hud) = weak.upgrade() else { return };
            if let Some(owner) = crate::platform::hwnd_of(hud.window()) {
                docked::adopt(h, owner);
            }
            match target(&hud) {
                Some(rect) => docked::place(h, rect, radius(&hud)),
                None => docked::set_shown(h, false),
            }
            let raw = h.0 as isize;
            app::post(move |a| a.feeds.player.found(HWND(raw as *mut _)));
        });
        if !docked::launch(page, at, size) {
            drop(watch);
            self.end();
            self.edge_missing.set(true);
            super::open_link(browser_url);
            return;
        }
        *self.session.borrow_mut() = Some(Session {
            site,
            browser_url: browser_url.to_string(),
            before,
            started: Instant::now(),
            hwnd: None,
            watch,
            needs_click,
            clicks: 0,
            last_click: None,
            placed: None,
            hidden_since: None,
        });
        self.timer.start(TimerMode::Repeated, TICK, || app::post(|a| a.feeds.player.tick()));
    }

    /// The show hook caught the window (already adopted and placed).
    fn found(&self, hwnd: HWND) {
        if let Some(s) = self.session.borrow_mut().as_mut()
            && s.hwnd.is_none()
        {
            s.hwnd = Some(hwnd);
            s.watch = None;
        }
    }

    /// Closes the docked video, if any.
    pub fn close(&self) {
        if let Some(s) = self.session.borrow_mut().take()
            && let Some(h) = s.hwnd
            && docked::alive(h)
        {
            docked::close(h);
        }
        self.end();
    }

    /// Continues in the browser.
    fn pop_out(&self) {
        let url = self.session.borrow().as_ref().map(|s| s.browser_url.clone());
        self.close();
        if let Some(url) = url {
            super::open_link(&url);
        }
    }

    /// The widgets on the HUD now (from the layout); a docked video whose widget left is closed.
    pub fn set_on_hud(&self, ids: &[&str]) {
        *self.on_hud.borrow_mut() = ids.iter().map(|s| s.to_string()).collect();
    }

    /// Back to normal widgets: no session, no timer.
    fn end(&self) {
        self.timer.stop();
        if let Some(hud) = self.hud.upgrade() {
            let state = hud.global::<PlayerState>();
            state.set_active(false);
            state.set_loading(false);
        }
    }

    fn tick(&self) {
        let Some(hud) = self.hud.upgrade() else { return };
        let owner = crate::platform::hwnd_of(hud.window());
        let mut guard = self.session.borrow_mut();
        let Some(s) = guard.as_mut() else {
            drop(guard);
            self.end();
            return;
        };

        // 1. Find Edge's new window, once.
        let hwnd = match s.hwnd {
            Some(h) => h,
            None => match docked::find_new(&s.before) {
                Some(h) => {
                    if let Some(owner) = owner {
                        docked::adopt(h, owner);
                    }
                    s.hwnd = Some(h);
                    s.watch = None;
                    h
                }
                None if s.started.elapsed() > FIND_FOR => {
                    // Edge may still open it, as an ordinary window; leave that be.
                    crate::storage::log("docked player: Edge's window never appeared");
                    guard.take();
                    drop(guard);
                    self.end();
                    return;
                }
                None => return,
            },
        };

        // 2. Closed from Edge's side (Alt+F4, a crash): back to the normal widget.
        if !docked::alive(hwnd) {
            guard.take();
            drop(guard);
            self.end();
            return;
        }
        if !docked::is_adopted(hwnd) {
            if let Some(owner) = owner {
                docked::adopt(hwnd, owner);
            }
            s.placed = None;
        }

        // 3. On screen only while its widget is, and not while the HUD folds between layouts.
        let state = hud.global::<PlayerState>();
        let widget_here = self.on_hud.borrow().iter().any(|id| id == s.site);
        let hud_shown = owner.is_some_and(docked::is_shown);
        let settled = !hud.global::<HudState>().get_transitioning();
        let area = target(&hud);
        if !(widget_here && hud_shown && settled && area.is_some()) {
            if docked::is_shown(hwnd) {
                docked::set_shown(hwnd, false);
            }
            let since = *s.hidden_since.get_or_insert_with(Instant::now);
            if !(widget_here && hud_shown) && since.elapsed() > CLOSE_HIDDEN_AFTER {
                drop(guard);
                self.close();
            }
            return;
        }
        s.hidden_since = None;
        if let Some(rect) = area
            && s.placed != Some(rect)
        {
            docked::place(hwnd, rect, radius(&hud));
            s.placed = Some(rect);
        }
        if !docked::is_shown(hwnd) {
            docked::set_shown(hwnd, true);
        }

        // 4. Start YouTube playback: click in the page until it says it's playing.
        if s.needs_click {
            if docked::title(hwnd) == embed::PLAYING_TITLE {
                s.needs_click = false;
                state.set_loading(false);
            } else if s.clicks < MAX_CLICKS && s.last_click.is_none_or(|t| t.elapsed() >= CLICK_EVERY) {
                docked::click_page(hwnd);
                s.clicks += 1;
                s.last_click = Some(Instant::now());
            }
        } else if state.get_loading() {
            state.set_loading(false);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(h) = self.session.get_mut().take().and_then(|s| s.hwnd)
            && docked::alive(h)
        {
            docked::close(h);
        }
    }
}

/// Where the widget's player area is on screen (physical pixels), once it has one.
fn target(hud: &HudWindow) -> Option<RECT> {
    let state = hud.global::<PlayerState>();
    if state.get_width() <= 1.0 || state.get_height() <= 1.0 {
        return None;
    }
    let scale = hud.window().scale_factor();
    let pos = hud.window().position();
    let px = |v: f32| (v * scale).round() as i32;
    Some(RECT {
        left: pos.x + px(state.get_x()),
        top: pos.y + px(state.get_y()),
        right: pos.x + px(state.get_x() + state.get_width()),
        bottom: pos.y + px(state.get_y() + state.get_height()),
    })
}

/// The player area's corner radius (8 DIPs, as drawn), in physical pixels.
fn radius(hud: &HudWindow) -> i32 {
    (8.0 * hud.window().scale_factor()).round() as i32
}
