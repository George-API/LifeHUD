//! UI-thread controllers for the feed widgets (Sports, YouTube, Reddit, Netflix, Stocks). Each owns
//! its background poller and the Slint models it feeds; `Hub` fans the app's events out to them.

pub mod netflix;
pub mod player;
pub mod reddit;
pub mod settings;
pub mod sports;
pub mod stocks;
pub mod youtube;

use std::collections::HashMap;

use slint::{ComponentHandle, Image, Model, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::services::images;
use crate::settings::Settings;
use crate::{HudWindow, Slots};

/// Settings pages for the feed widgets (index into the Settings sidebar).
pub const PAGE_SPORTS: i32 = 7;
pub const PAGE_YOUTUBE: i32 = 8;
pub const PAGE_REDDIT: i32 = 9;
pub const PAGE_STOCKS: i32 = 11;

pub struct Hub {
    hud: slint::Weak<HudWindow>,
    pub sports: sports::Sports,
    pub youtube: youtube::YouTube,
    pub reddit: reddit::Reddit,
    pub netflix: netflix::Netflix,
    pub stocks: stocks::Stocks,
    /// A YouTube or Netflix video docked into its widget.
    pub player: player::Player,
}

impl Hub {
    pub fn new(hud: &HudWindow) -> Self {
        Self {
            hud: hud.as_weak(),
            sports: sports::Sports::new(hud),
            youtube: youtube::YouTube::new(hud),
            reddit: reddit::Reddit::new(hud),
            netflix: netflix::Netflix::new(hud),
            stocks: stocks::Stocks::new(hud),
            player: player::Player::new(hud),
        }
    }

    pub fn start(&self, settings: &Settings) {
        self.sports.start(settings);
        self.youtube.start(settings);
        self.reddit.start(settings);
        self.netflix.start(settings);
        self.stocks.start(settings);
        self.apply_layout(settings);
    }

    /// Slots and visibility follow the layout, the slot choices and whether the HUD is shown.
    pub fn apply_layout(&self, settings: &Settings) {
        if let Some(hud) = self.hud.upgrade() {
            // The Top HUD splits the one list around the clock: two left, the rest right.
            let slots = hud.global::<Slots>();
            let kinds = settings.sidebar_kinds();
            let split = kinds.len().min(2);
            slots.set_left(slint::ModelRc::new(VecModel::from(kinds[..split].to_vec())));
            slots.set_right(slint::ModelRc::new(VecModel::from(kinds[split..].to_vec())));
        }
        let visible = settings.visible_widgets();
        self.player.set_on_hud(&visible);
        self.sports.set_active(visible.contains(&"sports"));
        self.youtube.set_active(visible.contains(&"youtube"));
        self.reddit.set_active(visible.contains(&"reddit"));
        self.netflix.set_active(visible.contains(&"netflix"));
        self.stocks.set_active(visible.contains(&"stocks"));
    }

    /// Settings changed: each widget refetches only if its own configuration did.
    pub fn configure(&self, settings: &Settings) {
        self.sports.configure(settings);
        self.youtube.configure(settings);
        self.reddit.configure(settings);
        self.netflix.configure(settings);
        self.stocks.configure(settings);
    }

    /// Once a minute: relative times ("12m", "Tomorrow") move on.
    pub fn minute(&self, settings: &Settings) {
        self.sports.render(settings.use_24h);
        self.youtube.render();
        self.reddit.render();
        self.stocks.render(settings.use_24h);
    }
}

/// Converts decoded pixels to Slint images once per URL, and forgets the ones no longer shown.
#[derive(Default)]
pub struct ImageStore {
    map: HashMap<String, Image>,
}

impl ImageStore {
    pub fn get(&mut self, url: &str, px: Option<&images::Shared>) -> Option<Image> {
        if let Some(img) = self.map.get(url) {
            return Some(img.clone());
        }
        let px = px?;
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&px.rgba, px.width, px.height);
        let img = Image::from_rgba8(buffer);
        self.map.insert(url.to_string(), img.clone());
        Some(img)
    }

    pub fn retain<'a>(&mut self, keep: impl IntoIterator<Item = &'a str>) {
        let keep: std::collections::HashSet<&str> = keep.into_iter().collect();
        self.map.retain(|k, _| keep.contains(k.as_str()));
    }
}

/// Replaces a model's rows only where they changed, so a 30-second live refresh doesn't rebuild
/// (and redraw) every tile.
pub(crate) fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).is_some_and(|r| r != row) {
            model.set_row_data(i, row);
        }
    }
}

/// Hands only web links to the shell.
pub fn open_link(url: &str) {
    if url.starts_with("https://") || url.starts_with("http://") {
        crate::platform::shell_open(url);
    }
}

/// "NBA · NHL", or "NBA · NHL +2" when there are many.
pub fn join_names(names: &[&str], max: usize) -> String {
    let shown = names.iter().take(max).copied().collect::<Vec<_>>().join(" · ");
    if names.len() > max { format!("{shown} +{}", names.len() - max) } else { shown }
}

#[cfg(test)]
mod tests {
    #[test]
    fn joins_names_with_overflow() {
        assert_eq!(super::join_names(&["NBA"], 3), "NBA");
        assert_eq!(super::join_names(&["A", "B", "C", "D", "E"], 3), "A · B · C +2");
        assert_eq!(super::join_names(&[], 3), "");
    }
}
