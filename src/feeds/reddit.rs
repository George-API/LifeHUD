//! Reddit widget controller.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::{ImageStore, PAGE_REDDIT, sync};
use crate::services::reddit::{self, Post, Prefs, Tab};
use crate::services::{news, poller::Poller, weather::unix_now};
use crate::settings::Settings;
use crate::{HudWindow, LinkData, RedditState, app};

#[derive(Default)]
struct Feed {
    posts: Vec<Post>,
    loaded: bool,
    failed: bool,
}

pub struct Reddit {
    hud: slint::Weak<HudWindow>,
    poller: RefCell<Option<Poller<Prefs>>>,
    prefs: RefCell<Prefs>,
    yours: RefCell<Feed>,
    popular: RefCell<Feed>,
    images: RefCell<ImageStore>,
    yours_rows: Rc<VecModel<LinkData>>,
    popular_rows: Rc<VecModel<LinkData>>,
}

impl Reddit {
    pub fn new(hud: &HudWindow) -> Self {
        let s = Self {
            hud: hud.as_weak(),
            poller: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            yours: RefCell::default(),
            popular: RefCell::default(),
            images: RefCell::default(),
            yours_rows: Rc::new(VecModel::default()),
            popular_rows: Rc::new(VecModel::default()),
        };
        let state = hud.global::<RedditState>();
        state.set_yours(ModelRc::from(s.yours_rows.clone()));
        state.set_popular(ModelRc::from(s.popular_rows.clone()));
        state.on_open(|url| {
            if reddit::post_url_ok(&url) {
                super::open_link(&url);
            }
        });
        state.on_customize(|| app::post(|a| a.open_settings(Some(PAGE_REDDIT))));
        // Deferred out of the click handler: the menu is its own window.
        state.on_choose(|| {
            slint::Timer::single_shot(std::time::Duration::ZERO, || app::post(show_picker));
        });
        s
    }

    pub fn start(&self, settings: &Settings) {
        let prefs = settings.reddit.clone();
        let (yours, popular) = reddit::cached(&prefs);
        if let Some(posts) = yours {
            *self.yours.borrow_mut() = Feed { posts, loaded: true, failed: false };
        }
        if let Some(posts) = popular {
            *self.popular.borrow_mut() = Feed { posts, loaded: true, failed: false };
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render();
        let poller = reddit::spawn(prefs, |u| app::post(move |a| a.feeds.reddit.on_update(u)));
        *self.poller.borrow_mut() = Some(poller);
    }

    pub fn set_active(&self, active: bool) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.set_active(active);
        }
    }

    pub fn configure(&self, settings: &Settings) {
        let prefs = settings.reddit.clone();
        if prefs == *self.prefs.borrow() {
            return;
        }
        // Different subreddits, pick or sort: show what's cached for the new choice straight away
        // (switching back to a recent subreddit is instant), else "Loading…" until it arrives.
        // Popular depends only on the sort.
        let (yours, popular) = reddit::cached(&prefs);
        let old = self.prefs.borrow().clone();
        if prefs.yours() != old.yours() || prefs.sort != old.sort {
            *self.yours.borrow_mut() =
                yours.map(|posts| Feed { posts, loaded: true, failed: false }).unwrap_or_default();
        }
        if prefs.sort != old.sort {
            *self.popular.borrow_mut() =
                popular.map(|posts| Feed { posts, loaded: true, failed: false }).unwrap_or_default();
        }
        *self.prefs.borrow_mut() = prefs.clone();
        self.render();
        if let Some(p) = self.poller.borrow().as_ref() {
            p.configure(prefs);
        }
    }

    pub fn on_update(&self, update: reddit::Update) {
        let feed = |tab: Tab| if tab == Tab::Yours { &self.yours } else { &self.popular };
        match update {
            reddit::Update::Posts(tab, posts) => *feed(tab).borrow_mut() = Feed { posts, loaded: true, failed: false },
            reddit::Update::Failed(tab) => feed(tab).borrow_mut().failed = true,
        }
        self.render();
    }

    pub fn render(&self) {
        let Some(hud) = self.hud.upgrade() else { return };
        let state = hud.global::<RedditState>();
        let prefs = self.prefs.borrow();
        let now = unix_now();
        let mut images = self.images.borrow_mut();
        let (yours, popular) = (self.yours.borrow(), self.popular.borrow());
        let rows = |feed: &Feed, images: &mut ImageStore| -> Vec<LinkData> {
            feed.posts.iter().map(|p| link(p, now, images)).collect()
        };
        let y = rows(&yours, &mut images);
        let p = rows(&popular, &mut images);
        images.retain(yours.posts.iter().chain(&popular.posts).map(|p| p.thumb_url.as_str()));

        let subs = match (prefs.view.as_str(), prefs.subreddits.as_slice()) {
            (view, _) if !view.is_empty() => format!("r/{view}"),
            (_, []) => String::new(),
            (_, [one]) => format!("r/{one}"),
            (_, [first, rest @ ..]) => format!("r/{first} +{}", rest.len()),
        };
        state.set_label(subs.into());
        state.set_sort(prefs.sort.label().into());
        state.set_has_subreddits(!prefs.yours().is_empty());
        let placeholder = |feed: &Feed| {
            if feed.loaded {
                "No posts right now."
            } else if feed.failed {
                "Reddit isn't answering right now. Trying again shortly."
            } else {
                "Loading posts…"
            }
        };
        state.set_placeholder(
            if prefs.yours().is_empty() { "Choose subreddits to follow." } else { placeholder(&yours) }.into(),
        );
        state.set_popular_placeholder(placeholder(&popular).into());
        sync(&self.yours_rows, y);
        sync(&self.popular_rows, p);
    }
}

/// "r/rust · 3h · github.com"
fn link(p: &Post, now: i64, images: &mut ImageStore) -> LinkData {
    let thumb = images.get(&p.thumb_url, p.thumb.as_ref());
    let age = news::age_label(p.published, now);
    let meta: Vec<String> =
        [p.subreddit.clone(), age.clone(), p.domain.clone()].into_iter().filter(|s| !s.is_empty()).collect();
    let short = if age.is_empty() { p.subreddit.clone() } else { format!("{} · {age}", p.subreddit) };
    LinkData {
        title: p.title.as_str().into(),
        meta: meta.join(" · ").into(),
        short: short.into(),
        url: p.url.as_str().into(),
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
    }
}

/// The "Yours" picker: everything you follow together, one of them, or a ready-made suggestion.
fn show_picker(a: &Rc<app::App>) {
    const ALL: usize = 1;
    const MANAGE: usize = 2;
    const MINE: usize = 100;
    const PRESET: usize = 200;
    use crate::platform::menu::Item;

    let (palette, prefs) = {
        let s = a.settings();
        (crate::theme::resolve(&s.theme, s.appearance), s.reddit.clone())
    };
    let followed = prefs.subreddits.clone();
    let is_followed = |n: &str| followed.iter().any(|f| f.eq_ignore_ascii_case(n));
    let viewing = |n: &str| prefs.view.eq_ignore_ascii_case(n);

    let mut items = vec![Item::Header("Show in Yours".into())];
    if !followed.is_empty() {
        items.push(Item::Entry { id: ALL, text: "All my subreddits".into(), checked: prefs.view.is_empty() });
        for (i, n) in followed.iter().enumerate() {
            items.push(Item::Entry { id: MINE + i, text: format!("r/{n}"), checked: viewing(n) });
        }
        items.push(Item::Separator);
    }
    let suggestions: Vec<usize> = (0..reddit::PRESETS.len()).filter(|&i| !is_followed(reddit::PRESETS[i].0)).collect();
    if !suggestions.is_empty() {
        items.push(Item::Header("Suggestions".into()));
        for i in suggestions {
            let (name, about) = reddit::PRESETS[i];
            items.push(Item::Entry { id: PRESET + i, text: format!("r/{name} · {about}"), checked: viewing(name) });
        }
        items.push(Item::Separator);
    }
    items.push(Item::Entry { id: MANAGE, text: "Manage subreddits…".into(), checked: false });

    crate::context_menu::show(&palette, items, move |id| {
        let view = match id {
            ALL => String::new(),
            MANAGE => return app::post(|a| a.open_settings(Some(PAGE_REDDIT))),
            i if i >= PRESET => match reddit::PRESETS.get(i - PRESET) {
                Some((name, _)) => name.to_string(),
                None => return,
            },
            i if i >= MINE => match followed.get(i - MINE) {
                Some(name) => name.clone(),
                None => return,
            },
            _ => return,
        };
        app::post(move |a| {
            a.update_settings(|s| s.reddit.view = view);
            // Show what was just picked.
            if let Some(hud) = a.feeds.reddit.hud.upgrade() {
                hud.global::<RedditState>().set_tab(0);
            }
        });
    });
}
