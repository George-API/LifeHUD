use serde::{Deserialize, Serialize};

use crate::{locale, storage};

const FILE: &str = "settings.json";

pub const TOP_SIZE_RANGE: (f32, f32) = (6.0, 16.0);
/// The Top HUD's default height, as a percentage of the screen's height.
pub const DEFAULT_TOP_SIZE: f32 = 9.0;
pub const SIDE_SIZE_RANGE: (f32, f32) = (10.0, 25.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Top,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Auto,
    Light,
    Dark,
}

/// A geocoded city, cached so we don't look it up on every refresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCity {
    pub query: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Preferences for the optional AI chat panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiPrefs {
    /// Show the chat button in the HUD.
    pub show_button: bool,
    /// Model per provider, in `services::ai::Provider::ALL` order.
    pub models: [String; 3],
    /// The panel tab last used: 0 Google search, then ChatGPT, Claude, Gemini.
    pub tab: usize,
}

impl Default for AiPrefs {
    fn default() -> Self {
        use crate::services::ai::Provider;
        Self { show_button: true, models: Provider::ALL.map(|p| p.default_model().to_string()), tab: 0 }
    }
}

impl AiPrefs {
    fn normalize(&mut self) {
        use crate::services::ai::{Provider, valid_model};
        for (model, provider) in self.models.iter_mut().zip(Provider::ALL) {
            let trimmed = model.trim();
            *model = if valid_model(trimmed) { trimmed.to_string() } else { provider.default_model().to_string() };
        }
        // Panel tabs: Google search, then one per provider.
        self.tab = self.tab.min(Provider::ALL.len());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub layout: Layout,
    /// Percent of the monitor's height used by the Top HUD.
    pub top_size_percent: f32,
    /// Percent of the monitor's width used by the Sidebar.
    pub side_size_percent: f32,
    /// Win32 device name (e.g. `\\.\DISPLAY2`); `None` = primary display.
    pub monitor: Option<String>,
    pub hud_visible: bool,
    /// Refit ordinary windows into the space left when the HUD docks, resizes or hides.
    pub fit_windows: bool,

    pub theme: String,
    pub appearance: Appearance,
    pub use_24h: bool,
    pub show_seconds: bool,

    /// Empty = use Windows location.
    pub weather_city: String,
    pub celsius: bool,
    pub resolved_city: Option<ResolvedCity>,

    pub prefer_spotify: bool,
    /// Saved Spotify links for the music widget's right-click menu.
    pub playlists: Vec<crate::playlists::Playlist>,
    /// AI chat (Settings → AI). API keys live in Windows Credential Manager, never in this file.
    pub ai: AiPrefs,

    /// News tabs: ids from `services::news::CURATED` and `services::news::TOP`.
    pub news_curated: String,
    pub news_top: String,

    /// The HUD's widgets, in order, by id (see [`WIDGETS`]): at most [`MAX_WIDGETS`]. Both layouts
    /// show the same list: the Sidebar stacks it under the clock; the Top HUD puts the first two
    /// left of the clock and the rest right of it. (Kept under its original name so old saves
    /// load; their `topLeft`/`topRight` keys are simply ignored.)
    pub sidebar_order: Vec<String>,

    pub sports: crate::services::sports::Prefs,
    pub youtube: crate::services::youtube::Prefs,
    pub reddit: crate::services::reddit::Prefs,
    pub netflix: crate::services::netflix::Prefs,
    pub stocks: crate::services::stocks::Prefs,
}

/// Every widget: (id, display name). Index = the UI's card kind.
pub const WIDGETS: [(&str, &str); 9] = [
    ("weather", "Weather"),
    ("music", "Music"),
    ("today", "Tasks & Notes"),
    ("news", "News"),
    ("sports", "Sports"),
    ("youtube", "YouTube"),
    ("reddit", "Reddit"),
    ("netflix", "Netflix"),
    ("stocks", "Stocks"),
];

/// How many widgets the HUD shows: two each side of the clock on the Top HUD, and as many as the
/// Sidebar can give a useful height.
pub const MAX_WIDGETS: usize = 4;

/// New installs and *Reset layout*: the day at a glance first, music at the bottom.
const DEFAULT_SIDEBAR: [&str; 4] = ["weather", "today", "reddit", "music"];

pub fn widget_kind(id: &str) -> Option<usize> {
    WIDGETS.iter().position(|(w, _)| *w == id)
}

pub fn widget_name(id: &str) -> &'static str {
    WIDGETS.iter().find(|(w, _)| *w == id).map(|(_, n)| *n).unwrap_or("")
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            layout: Layout::Top,
            top_size_percent: DEFAULT_TOP_SIZE,
            side_size_percent: 14.0,
            monitor: None,
            hud_visible: true,
            fit_windows: true,
            theme: crate::theme::CLASSIC.into(),
            appearance: Appearance::Auto,
            use_24h: locale::prefers_24h(),
            show_seconds: false,
            weather_city: String::new(),
            celsius: locale::is_metric(),
            resolved_city: None,
            prefer_spotify: true,
            playlists: Vec::new(),
            ai: AiPrefs::default(),
            news_curated: crate::services::news::DEFAULT_CURATED.into(),
            news_top: crate::services::news::default_top().into(),
            sidebar_order: DEFAULT_SIDEBAR.iter().map(|id| id.to_string()).collect(),
            sports: Default::default(),
            youtube: Default::default(),
            reddit: Default::default(),
            netflix: Default::default(),
            stocks: Default::default(),
        }
    }
}

impl Settings {
    /// Returns the settings and whether this is the first run.
    pub fn load() -> (Self, bool) {
        match storage::load_json::<Settings>(FILE) {
            Some(mut s) => {
                s.normalize();
                (s, false)
            }
            None => (Settings::default(), true),
        }
    }

    pub fn save(&self) {
        storage::save_json(FILE, self);
    }

    pub fn normalize(&mut self) {
        let clamp = |v: f32, (lo, hi): (f32, f32), d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.top_size_percent = clamp(self.top_size_percent, TOP_SIZE_RANGE, DEFAULT_TOP_SIZE);
        self.side_size_percent = clamp(self.side_size_percent, SIDE_SIZE_RANGE, 14.0);
        if self.theme == "nebula" {
            self.theme = "neon".into(); // retired in favour of Coast
        }
        if crate::theme::find(&self.theme).is_none() {
            self.theme = crate::theme::CLASSIC.into();
        }
        self.weather_city = self.weather_city.trim().to_string();
        crate::playlists::normalize(&mut self.playlists);
        self.ai.normalize();
        self.news_curated =
            crate::services::news::find(crate::services::news::Feed::Curated, &self.news_curated).id.into();
        self.news_top = crate::services::news::find(crate::services::news::Feed::Top, &self.news_top).id.into();
        // Known widgets once each, in the saved order, at most MAX_WIDGETS. An empty HUD is never
        // useful.
        let mut seen = std::collections::HashSet::new();
        self.sidebar_order.retain(|id| widget_kind(id).is_some() && seen.insert(id.clone()));
        self.sidebar_order.truncate(MAX_WIDGETS);
        if self.sidebar_order.is_empty() {
            self.sidebar_order = DEFAULT_SIDEBAR.iter().map(|id| id.to_string()).collect();
        }
        self.sports.normalize();
        self.youtube.normalize();
        self.reddit.normalize();
        self.stocks.normalize();
    }

    /// The widgets on the HUD, in order (both layouts).
    pub fn hud_widgets(&self) -> &[String] {
        &self.sidebar_order
    }

    /// No room for another widget until one is removed.
    pub fn hud_full(&self) -> bool {
        self.sidebar_order.len() >= MAX_WIDGETS
    }

    /// Card kinds (indexes into [`WIDGETS`]) in display order.
    pub fn sidebar_kinds(&self) -> Vec<i32> {
        self.sidebar_order.iter().filter_map(|id| widget_kind(id)).map(|i| i as i32).collect()
    }

    /// Removes a sidebar card, keeping at least one.
    pub fn remove_card(&mut self, index: usize) {
        if index < self.sidebar_order.len() && self.sidebar_order.len() > 1 {
            self.sidebar_order.remove(index);
        }
    }

    pub fn add_card(&mut self, id: &str) {
        if !self.hud_full() && widget_kind(id).is_some() && !self.sidebar_order.iter().any(|o| o == id) {
            self.sidebar_order.push(id.to_string());
        }
    }

    /// Adds `id` to the sidebar before the card at `at` (or at the end).
    pub fn insert_card(&mut self, id: &str, at: usize) {
        if !self.hud_full() && widget_kind(id).is_some() && !self.sidebar_order.iter().any(|o| o == id) {
            let at = at.min(self.sidebar_order.len());
            self.sidebar_order.insert(at, id.to_string());
        }
    }

    /// Moves the card at `from` so it lands before the card now at `to` (`to` may equal the length:
    /// the end). This is what a drag and drop between two cards means.
    pub fn place_card(&mut self, from: usize, to: usize) {
        let len = self.sidebar_order.len();
        if from >= len || to > len {
            return;
        }
        let card = self.sidebar_order.remove(from);
        let to = if to > from { to - 1 } else { to };
        self.sidebar_order.insert(to, card);
    }

    /// Puts `id` where the card at `index` is; that card goes back to the extras. This is how a full
    /// HUD takes a new widget: drop it onto the one it should replace.
    pub fn replace_card(&mut self, index: usize, id: &str) {
        if index < self.sidebar_order.len() && widget_kind(id).is_some() && !self.sidebar_order.iter().any(|o| o == id)
        {
            self.sidebar_order[index] = id.to_string();
        }
    }

    /// Widgets not in the sidebar, in catalogue order.
    pub fn sidebar_available(&self) -> Vec<&'static str> {
        WIDGETS.iter().map(|(id, _)| *id).filter(|id| !self.sidebar_order.iter().any(|o| o == id)).collect()
    }

    /// Widget ids currently on screen.
    pub fn visible_widgets(&self) -> Vec<&str> {
        if !self.hud_visible {
            return Vec::new();
        }
        self.hud_widgets().iter().map(|s| s.as_str()).collect()
    }

    /// Restores the default widgets and their order, and HUD sizes. Leaves the chosen layout,
    /// display and window fitting alone.
    pub fn reset_layout(&mut self) {
        let d = Settings::default();
        self.sidebar_order = d.sidebar_order;
        self.top_size_percent = d.top_size_percent;
        self.side_size_percent = d.side_size_percent;
    }

    pub fn move_card(&mut self, index: usize, delta: i32) {
        let target = index as i32 + delta;
        if index < self.sidebar_order.len() && target >= 0 && (target as usize) < self.sidebar_order.len() {
            self.sidebar_order.swap(index, target as usize);
        }
    }

    pub fn size_percent(&self) -> f32 {
        match self.layout {
            Layout::Top => self.top_size_percent,
            Layout::Left => self.side_size_percent,
        }
    }

    pub fn size_range(&self) -> (f32, f32) {
        match self.layout {
            Layout::Top => TOP_SIZE_RANGE,
            Layout::Left => SIDE_SIZE_RANGE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clamps_and_repairs() {
        let mut s = Settings {
            top_size_percent: 99.0,
            side_size_percent: f32::NAN,
            theme: "nope".into(),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.top_size_percent, TOP_SIZE_RANGE.1);
        assert_eq!(s.side_size_percent, 14.0);
        assert_eq!(s.theme, crate::theme::CLASSIC);
    }

    #[test]
    fn default_sidebar_puts_music_below_news() {
        // Reddit took News's place by default (the user's choice); Music stays last.
        assert_eq!(Settings::default().sidebar_order, ["weather", "today", "reddit", "music"]);
    }

    #[test]
    fn retired_nebula_theme_becomes_neon() {
        let mut s = Settings { theme: "nebula".into(), ..Default::default() };
        s.normalize();
        assert_eq!(s.theme, "neon");
        assert!(crate::theme::find("coast").is_some());
    }

    #[test]
    fn drag_and_drop_places_cards() {
        let order = |s: &Settings| s.sidebar_order.join(",");
        let mut s = Settings {
            sidebar_order: ["weather", "music", "today", "news"].map(String::from).to_vec(),
            ..Settings::default()
        };
        s.place_card(0, 4);
        assert_eq!(order(&s), "music,today,news,weather", "dragged to the end");
        s.place_card(3, 0);
        assert_eq!(order(&s), "weather,music,today,news", "dragged to the top");
        s.place_card(1, 2);
        assert_eq!(order(&s), "weather,music,today,news", "dropped where it was");
        s.place_card(1, 3);
        assert_eq!(order(&s), "weather,today,music,news", "past one card");
        s.place_card(9, 0);
        s.place_card(0, 9);
        assert_eq!(order(&s), "weather,today,music,news", "out of range is ignored");
        s.insert_card("sports", 1);
        assert_eq!(order(&s), "weather,today,music,news", "full: four is the limit");
        s.remove_card(3);
        s.insert_card("sports", 1);
        assert_eq!(order(&s), "weather,sports,today,music");
        s.remove_card(3);
        s.insert_card("sports", 0);
        s.insert_card("nope", 0);
        s.insert_card("stocks", 99);
        assert_eq!(order(&s), "weather,sports,today,stocks", "no duplicates; clamped to the end");
    }

    #[test]
    fn a_full_hud_swaps_widgets_in() {
        let order = |s: &Settings| s.sidebar_order.join(",");
        let mut s = Settings {
            sidebar_order: ["weather", "music", "today", "news"].map(String::from).to_vec(),
            ..Settings::default()
        };
        assert!(s.hud_full());
        s.replace_card(1, "stocks");
        assert_eq!(order(&s), "weather,stocks,today,news", "music goes back to the extras");
        assert!(s.sidebar_available().contains(&"music"));
        s.replace_card(0, "today");
        s.replace_card(9, "music");
        s.replace_card(2, "nope");
        assert_eq!(order(&s), "weather,stocks,today,news", "no duplicates, out of range, or unknown ids");
    }

    #[test]
    fn sidebar_order_is_repaired_and_movable() {
        let mut s =
            Settings { sidebar_order: vec!["news".into(), "bogus".into(), "news".into()], ..Default::default() };
        s.normalize();
        assert_eq!(s.sidebar_order, ["news"]);
        s.add_card("weather");
        s.add_card("weather");
        s.add_card("bogus");
        assert_eq!(s.sidebar_order, ["news", "weather"]);
        s.move_card(0, 1);
        assert_eq!(s.sidebar_kinds(), [0, 3]);
        s.move_card(0, -1); // out of range: no-op
        assert_eq!(s.sidebar_kinds(), [0, 3]);
        s.remove_card(0);
        s.remove_card(0); // keeps the last card
        assert_eq!(s.sidebar_order, ["news"]);
        assert_eq!(s.sidebar_available()[0], "weather");
        s.sidebar_order.clear();
        s.normalize();
        assert_eq!(s.sidebar_order.len(), 4);
    }

    #[test]
    fn one_list_of_at_most_four_for_both_views() {
        let mut s = Settings {
            sidebar_order: ["news", "x", "stocks", "news", "weather", "music", "today", "reddit"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.sidebar_order, ["news", "stocks", "weather", "music"], "deduped, unknown dropped, cut to 4");
        assert!(s.hud_full());
        s.add_card("reddit");
        s.insert_card("reddit", 0);
        assert_eq!(s.hud_widgets().len(), MAX_WIDGETS, "full: nothing more fits");
        for layout in [Layout::Top, Layout::Left] {
            s.layout = layout;
            assert_eq!(s.visible_widgets(), ["news", "stocks", "weather", "music"], "same widgets, same order");
        }
        s.remove_card(1);
        s.insert_card("reddit", 0);
        assert_eq!(s.hud_widgets(), ["reddit", "news", "weather", "music"]);
    }

    #[test]
    fn old_top_slot_saves_still_load() {
        let json = br#"{ "layout": "top", "sidebarOrder": ["today", "weather", "news", "music"], "topLeft": "sports", "topRight": "stocks" }"#;
        let mut s: Settings = serde_json::from_slice(json).unwrap();
        s.normalize();
        assert_eq!(s.hud_widgets(), ["today", "weather", "news", "music"], "the list wins; old slot keys are ignored");
    }

    #[test]
    fn tolerates_byte_order_mark() {
        let bytes = [&[0xEF, 0xBB, 0xBF][..], br#"{ "theme": "neon" }"#].concat();
        let p = crate::storage::path("bom-test.json");
        std::fs::write(&p, bytes).unwrap();
        let s: Option<Settings> = crate::storage::load_json("bom-test.json");
        let _ = std::fs::remove_file(&p);
        assert_eq!(s.unwrap().theme, "neon");
    }

    #[test]
    fn ai_models_are_repaired() {
        let mut s: Settings =
            serde_json::from_str(r#"{ "ai": { "models": ["", "gpt-6.1-sol", "x/../y"], "tab": 9 } }"#).unwrap();
        s.normalize();
        assert_eq!(s.ai.models, ["claude-opus-5-5", "gpt-6.1-sol", "gemini-flash-latest"]);
        assert_eq!(s.ai.tab, 3);
        assert!(s.ai.show_button);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{ "layout": "left" }"#).unwrap();
        assert_eq!(s.layout, Layout::Left);
        assert!(s.hud_visible);
        assert_eq!(s.top_size_percent, DEFAULT_TOP_SIZE);
    }

    #[test]
    fn reset_layout_restores_arrangement_only() {
        let mut s = Settings {
            layout: Layout::Left,
            monitor: Some(r"\\.\DISPLAY2".into()),
            fit_windows: false,
            sidebar_order: vec!["stocks".into(), "news".into()],
            top_size_percent: 15.0,
            side_size_percent: 22.0,
            ..Settings::default()
        };
        s.reset_layout();
        let d = Settings::default();
        assert_eq!(s.sidebar_order, d.sidebar_order);
        assert_eq!((s.top_size_percent, s.side_size_percent), (d.top_size_percent, d.side_size_percent));
        // The user's layout choice, display and window fitting are kept.
        assert_eq!(s.layout, Layout::Left);
        assert_eq!(s.monitor.as_deref(), Some(r"\\.\DISPLAY2"));
        assert!(!s.fit_windows);
    }
}
