//! YouTube widget controller: Latest and Watch Later lists, saving, pasting links and playback.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, VecModel};

use super::{ImageStore, PAGE_YOUTUBE, sync};
use crate::services::youtube::{self, Prefs, Video};
use crate::services::{news, poller::Poller, weather::unix_now};
use crate::settings::Settings;
use crate::{HudWindow, VideoData, YouTubeState, app};

/// Pop-out player size, in DIPs: 16:9 video plus the window's title bar.
const PLAYER: (i32, i32) = (640, 392);

pub struct YouTube {
    hud: slint::Weak<HudWindow>,
    poller: RefCell<Option<Poller<Prefs>>>,
    prefs: RefCell<Prefs>,
    videos: RefCell<Vec<Video>>,
    thumbs: RefCell<HashMap<String, Image>>,
    loaded: Cell<bool>,
    offline: Cell<bool>,
    status_timer: Timer,
    latest_rows: Rc<VecModel<VideoData>>,
    saved_rows: Rc<VecModel<VideoData>>,
}

impl YouTube {
    pub fn new(hud: &HudWindow) -> Self {
        let s = Self {
            hud: hud.as_weak(),
            poller: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            videos: RefCell::new(Vec::new()),
            thumbs: RefCell::new(HashMap::new()),
            loaded: Cell::new(false),
            offline: Cell::new(false),
            status_timer: Timer::default(),
            latest_rows: Rc::new(VecModel::default()),
            saved_rows: Rc::new(VecModel::default()),
        };
        let state = hud.global::<YouTubeState>();
        state.set_latest(ModelRc::from(s.latest_rows.clone()));
        state.set_saved(ModelRc::from(s.saved_rows.clone()));
        state.on_play(|id| app::post(move |a| a.feeds.youtube.play(a, &id)));
        state.on_toggle_save(|id| app::post(move |a| a.feeds.youtube.toggle_save(a, &id)));
        state.on_add_link(|text| app::post(move |a| a.feeds.youtube.add_link(&text)));
        state.on_customize(|| app::post(|a| a.open_settings(Some(PAGE_YOUTUBE))));
        s
    }

    pub fn start(&self, settings: &Settings) {
        let prefs = settings.youtube.clone();
        if let Some(videos) = youtube::cached(&prefs) {
            *self.videos.borrow_mut() = videos;
            self.loaded.set(true);
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render();
        let poller = youtube::spawn(prefs, |u| app::post(move |a| a.feeds.youtube.on_update(u)));
        *self.poller.borrow_mut() = Some(poller);
    }

    pub fn set_active(&self, active: bool) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.set_active(active);
        }
    }

    pub fn configure(&self, settings: &Settings) {
        let prefs = settings.youtube.clone();
        if prefs == *self.prefs.borrow() {
            return;
        }
        if prefs.channels != self.prefs.borrow().channels {
            self.videos.borrow_mut().clear();
            self.loaded.set(false);
            self.offline.set(false);
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render();
        if let Some(p) = self.poller.borrow().as_ref() {
            p.configure(prefs);
        }
    }

    pub fn on_update(&self, update: youtube::Update) {
        match update {
            youtube::Update::Videos(videos) => {
                *self.videos.borrow_mut() = videos;
                self.loaded.set(true);
                self.offline.set(false);
            }
            youtube::Update::Thumbs(list) => {
                let mut store = ImageStore::default();
                let mut thumbs = self.thumbs.borrow_mut();
                let next: HashMap<String, Image> = list
                    .into_iter()
                    .filter_map(|(id, px)| {
                        let img = thumbs.remove(&id).or_else(|| store.get(&id, Some(&px)))?;
                        Some((id, img))
                    })
                    .collect();
                *thumbs = next;
            }
            youtube::Update::Offline => self.offline.set(true),
        }
        self.render();
    }

    pub fn render(&self) {
        let Some(hud) = self.hud.upgrade() else { return };
        let state = hud.global::<YouTubeState>();
        let prefs = self.prefs.borrow();
        let videos = self.videos.borrow();
        let thumbs = self.thumbs.borrow();
        let now = unix_now();
        let saved_ids: Vec<&str> = prefs.saved.iter().map(|v| v.id.as_str()).collect();
        let data = |id: &str, title: &str, channel: &str, meta: String, duration: &str| {
            let thumb = thumbs.get(id).cloned();
            VideoData {
                id: id.into(),
                title: title.into(),
                channel: channel.into(),
                meta: meta.into(),
                duration: duration.into(),
                has_thumb: thumb.is_some(),
                thumb: thumb.unwrap_or_default(),
                saved: saved_ids.contains(&id),
            }
        };
        let latest: Vec<VideoData> =
            videos.iter().map(|v| data(&v.id, &v.title, &v.channel, meta(v, now), &v.duration)).collect();
        // Watch Later borrows durations and stats from Latest when the video is there too.
        let saved: Vec<VideoData> = prefs
            .saved
            .iter()
            .map(|s| match videos.iter().find(|v| v.id == s.id) {
                Some(v) => data(&s.id, &s.title, &s.channel, meta(v, now), &v.duration),
                None => data(&s.id, &s.title, &s.channel, String::new(), ""),
            })
            .collect();

        state.set_has_channels(!prefs.channels.is_empty());
        state.set_label(
            match prefs.channels.len() {
                0 => String::new(),
                1 => prefs.channels[0].name.clone(),
                n => format!("{n} channels"),
            }
            .into(),
        );
        state.set_placeholder(
            if prefs.channels.is_empty() {
                "Follow channels to see their new videos here."
            } else if !self.loaded.get() && self.offline.get() {
                "Videos will appear when you're online."
            } else if !self.loaded.get() {
                "Loading videos…"
            } else {
                "No new videos."
            }
            .into(),
        );
        sync(&self.latest_rows, latest);
        sync(&self.saved_rows, saved);
    }

    fn play(&self, a: &app::App, id: &str) {
        if !youtube::is_video_id(id) {
            return;
        }
        let (in_widget, popout) = {
            let p = self.prefs.borrow();
            (p.in_widget, p.popout)
        };
        if in_widget && a.feeds.player.available() {
            let title = self.title_of(id);
            a.feeds.player.play_youtube(id, &title);
        } else if !(popout && open_player(id)) {
            super::open_link(&youtube::watch_url(id));
        }
    }

    /// A video's title, from Latest or Watch Later.
    fn title_of(&self, id: &str) -> String {
        let latest = self.videos.borrow().iter().find(|v| v.id == id).map(|v| v.title.clone());
        latest
            .or_else(|| self.prefs.borrow().saved.iter().find(|v| v.id == id).map(|v| v.title.clone()))
            .unwrap_or_default()
    }

    fn toggle_save(&self, a: &app::App, id: &str) {
        let video = self.videos.borrow().iter().find(|v| v.id == id).cloned();
        a.update_settings(|s| {
            let saved = &mut s.youtube.saved;
            if let Some(i) = saved.iter().position(|v| v.id == id) {
                saved.remove(i);
            } else if let Some(v) = video {
                saved.insert(0, youtube::SavedVideo { id: v.id, title: v.title, channel: v.channel });
            }
        });
    }

    /// Saves a pasted link: the title is looked up on a worker thread.
    fn add_link(&self, text: &str) {
        let Some(id) = youtube::video_id_from(text) else {
            self.set_status("That isn't a YouTube video link.");
            return;
        };
        if self.prefs.borrow().saved.iter().any(|v| v.id == id) {
            self.set_status("Already in Watch Later.");
            self.clear_link();
            return;
        }
        self.set_status("Adding…");
        std::thread::spawn(move || {
            let result = youtube::resolve_video(&id);
            app::post(move |a| match result {
                Ok(video) => {
                    a.update_settings(|s| {
                        if !s.youtube.saved.iter().any(|v| v.id == video.id) {
                            s.youtube.saved.insert(0, video);
                        }
                    });
                    a.feeds.youtube.clear_link();
                    a.feeds.youtube.set_status("Saved");
                }
                Err(e) => a.feeds.youtube.set_status(&e),
            });
        });
    }

    fn clear_link(&self) {
        if let Some(hud) = self.hud.upgrade() {
            hud.global::<YouTubeState>().set_link(SharedString::new());
        }
    }

    /// A short note in the header that fades back to the usual label.
    fn set_status(&self, text: &str) {
        let Some(hud) = self.hud.upgrade() else { return };
        hud.global::<YouTubeState>().set_link_status(text.into());
        let weak = hud.as_weak();
        self.status_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(4), move || {
            if let Some(hud) = weak.upgrade() {
                hud.global::<YouTubeState>().set_link_status(SharedString::new());
            }
        });
    }
}

/// "3h · 1.2M views"
fn meta(v: &Video, now: i64) -> String {
    let age = news::age_label(v.published, now);
    match (age.is_empty(), v.views) {
        (false, Some(n)) => format!("{age} · {}", youtube::views_label(n)),
        (true, Some(n)) => youtube::views_label(n),
        (false, None) => age,
        (true, None) => String::new(),
    }
}

/// Plays a video in a small pop-out player: YouTube's own player on Life HUD's local page (see
/// `services::embed`), in an Edge app window parked in the bottom-right corner. Returns false if
/// Edge or the page isn't available.
fn open_player(id: &str) -> bool {
    let Some(page) = crate::services::embed::youtube_url(id) else { return false };
    let (w, h) = PLAYER;
    let (mut x, mut y) = (100, 100);
    if let Some(m) = crate::platform::monitors::find(None) {
        let s = m.scale.max(1.0);
        x = (m.work_area.right as f32 / s) as i32 - w - 24;
        y = (m.work_area.bottom as f32 / s) as i32 - h - 24;
    }
    crate::platform::docked::launch(&page, (x, y), (w, h))
}

#[cfg(test)]
mod tests {
    #[test]
    fn meta_joins_age_and_views() {
        let v = crate::services::youtube::Video {
            id: "aaaaaaaaaaa".into(),
            title: String::new(),
            channel: String::new(),
            published: Some(0),
            views: Some(1_200_000),
            duration: String::new(),
            channel_id: String::new(),
        };
        assert_eq!(super::meta(&v, 3 * 3600), "3h · 1.2M views");
        assert_eq!(super::meta(&crate::services::youtube::Video { published: None, views: None, ..v }, 0), "");
    }
}
