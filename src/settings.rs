use serde::{Deserialize, Serialize};

use crate::{locale, storage};

const FILE: &str = "settings.json";

pub const TOP_SIZE_RANGE: (f32, f32) = (6.0, 16.0);
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

    /// News tabs: ids from `services::news::CURATED` and `services::news::TOP`.
    pub news_curated: String,
    pub news_top: String,

    /// Sidebar card order, by id (see [`SIDEBAR_CARDS`]).
    pub sidebar_order: Vec<String>,
}

/// Reorderable sidebar cards: (id, display name). Index = the UI's card kind.
pub const SIDEBAR_CARDS: [(&str, &str); 4] =
    [("weather", "Weather"), ("music", "Music"), ("today", "Tasks & Notes"), ("news", "News")];

impl Default for Settings {
    fn default() -> Self {
        Self {
            layout: Layout::Top,
            top_size_percent: 8.8,
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
            news_curated: crate::services::news::DEFAULT_CURATED.into(),
            news_top: crate::services::news::default_top().into(),
            sidebar_order: SIDEBAR_CARDS.iter().map(|(id, _)| id.to_string()).collect(),
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
        self.top_size_percent = clamp(self.top_size_percent, TOP_SIZE_RANGE, 8.8);
        self.side_size_percent = clamp(self.side_size_percent, SIDE_SIZE_RANGE, 14.0);
        if crate::theme::find(&self.theme).is_none() {
            self.theme = crate::theme::CLASSIC.into();
        }
        self.weather_city = self.weather_city.trim().to_string();
        self.news_curated =
            crate::services::news::find(crate::services::news::Feed::Curated, &self.news_curated).id.into();
        self.news_top = crate::services::news::find(crate::services::news::Feed::Top, &self.news_top).id.into();
        // Keep known cards once each, in the saved order, then append any that are missing.
        let mut order: Vec<String> = Vec::new();
        for id in self.sidebar_order.drain(..) {
            if SIDEBAR_CARDS.iter().any(|(c, _)| *c == id) && !order.contains(&id) {
                order.push(id);
            }
        }
        for (id, _) in SIDEBAR_CARDS {
            if !order.iter().any(|o| o == id) {
                order.push(id.to_string());
            }
        }
        self.sidebar_order = order;
    }

    /// Card kinds (indexes into [`SIDEBAR_CARDS`]) in display order.
    pub fn sidebar_kinds(&self) -> Vec<i32> {
        self.sidebar_order
            .iter()
            .filter_map(|id| SIDEBAR_CARDS.iter().position(|(c, _)| c == id))
            .map(|i| i as i32)
            .collect()
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
    fn sidebar_order_is_repaired_and_movable() {
        let mut s =
            Settings { sidebar_order: vec!["news".into(), "bogus".into(), "news".into()], ..Default::default() };
        s.normalize();
        assert_eq!(s.sidebar_order, ["news", "weather", "music", "today"]);
        s.move_card(0, 1);
        assert_eq!(s.sidebar_kinds(), [0, 3, 1, 2]);
        s.move_card(0, -1); // out of range: no-op
        assert_eq!(s.sidebar_kinds(), [0, 3, 1, 2]);
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
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{ "layout": "left" }"#).unwrap();
        assert_eq!(s.layout, Layout::Left);
        assert!(s.hud_visible);
        assert_eq!(s.top_size_percent, 8.8);
    }
}
