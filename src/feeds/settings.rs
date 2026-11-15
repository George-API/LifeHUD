//! Settings window glue for widget choice (Layout page) and the feed widgets' own pages.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::services::{reddit, sports, stocks, youtube};
use crate::settings::{Settings, widget_name};
use crate::{ArrangeCard, ArrangeChip, FeedSettings, OptionRow, SettingsWindow, app};

const MAX_TEAM_RESULTS: usize = 8;

thread_local! {
    static TEAM_SEARCH: slint::Timer = slint::Timer::default();
}

fn row(id: &str, title: &str, detail: &str, on: bool) -> OptionRow {
    OptionRow { id: id.into(), title: title.into(), detail: detail.into(), on }
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

/// One-line descriptions for widgets not yet in the Sidebar.
pub fn blurb(id: &str) -> &'static str {
    match id {
        "weather" => "Conditions and forecast",
        "music" => "Now playing and controls",
        "today" => "Tasks and a scratchpad",
        "news" => "Headlines from two feeds",
        "sports" => "Live scores for your leagues",
        "youtube" => "New videos from your channels",
        "reddit" => "Top posts from your subreddits",
        "netflix" => "This week's Top 10",
        "stocks" => "Your watchlist and markets",
        _ => "",
    }
}

/// The Sidebar as `ui/hud.slint` lays it out, in DIPs (measured from renders), so the arrange
/// preview shows how much room each card really gets on the user's display.
mod side {
    /// `root.pad + 2px` around the column, with a 16px top.
    pub const PAD: f32 = 12.0;
    pub const PAD_TOP: f32 = 16.0;
    pub const SPACING: f32 = 16.0;
    /// The clock and date header.
    pub const HEADER: f32 = 92.0;
    /// The minimise/search/settings row at the bottom.
    pub const CHROME: f32 = 30.0;
    /// Taller screens get the weather card's hourly row.
    pub const HOURLY_FROM: f32 = 1240.0;

    /// Cards that keep their natural height; the rest share what's left.
    pub fn natural(id: &str, column: f32) -> Option<f32> {
        match id {
            "weather" => Some(if column >= HOURLY_FROM { 262.0 } else { 198.0 }),
            "music" => Some(148.0),
            _ => None,
        }
    }

    /// Layout stretch: Tasks & Notes gets a little more than the feeds.
    pub fn stretch(id: &str) -> f32 {
        if id == "today" { 5.0 } else { 4.0 }
    }

    /// Below this a flexible card shows only a couple of rows.
    pub fn comfortable(id: &str) -> f32 {
        if id == "today" { 230.0 } else { 180.0 }
    }
}

/// One card's place in the Sidebar column.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub id: String,
    /// Top and height, in DIPs from the top of the column.
    pub y: f32,
    pub h: f32,
    /// A flexible card squeezed below a comfortable height.
    pub tight: bool,
}

/// Where each card in `order` lands in a Sidebar `height` DIPs tall.
pub fn sidebar_layout(order: &[String], height: f32) -> Vec<Placed> {
    let top = side::PAD_TOP + side::HEADER + side::SPACING;
    let bottom = height - side::PAD - side::CHROME - side::SPACING;
    let gaps = side::SPACING * order.len().saturating_sub(1) as f32;
    let room = (bottom - top - gaps).max(0.0);
    let fixed: f32 = order.iter().filter_map(|id| side::natural(id, height)).sum();
    let weights: f32 = order.iter().filter(|id| side::natural(id, height).is_none()).map(|id| side::stretch(id)).sum();
    // Fixed cards shrink proportionally only if they alone overflow (a very short display).
    let squeeze = if fixed > room { room / fixed } else { 1.0 };
    let flexible = (room - fixed).max(0.0);
    let mut y = top;
    order
        .iter()
        .map(|id| {
            let (h, tight) = match side::natural(id, height) {
                Some(n) => (n * squeeze, false),
                None => {
                    let h = if weights > 0.0 { flexible * side::stretch(id) / weights } else { 0.0 };
                    (h, h < side::comfortable(id))
                }
            };
            let placed = Placed { id: id.clone(), y, h, tight };
            y += h + side::SPACING;
            placed
        })
        .collect()
}

/// The Sidebar's size on the chosen display, in DIPs (width, height).
fn sidebar_size(s: &Settings) -> (f32, f32) {
    use crate::settings::{MAX_SIDE_DIP, MAX_TOP_DIP, MIN_SIDE_DIP, MIN_TOP_DIP};
    let Some(m) = crate::platform::monitors::find(s.monitor.as_deref()) else { return (340.0, 1040.0) };
    let (mw, mh) = ((m.bounds.right - m.bounds.left) as f32, (m.bounds.bottom - m.bounds.top) as f32);
    let mut h = (m.work_area.bottom - m.work_area.top) as f32;
    if s.layout == crate::settings::Layout::Top {
        // The Top HUD's own reservation would come back to the Sidebar.
        h += (mh * s.top_size_percent / 100.0).clamp(MIN_TOP_DIP * m.scale, MAX_TOP_DIP * m.scale);
        h = h.min(mh);
    }
    let w = (mw * s.side_size_percent / 100.0).clamp(MIN_SIDE_DIP * m.scale, MAX_SIDE_DIP * m.scale);
    (w / m.scale, h / m.scale)
}

/// The arrange editor's data: the Sidebar preview, both palettes and the Top HUD slots.
fn populate_arrange(st: &FeedSettings, s: &Settings) {
    let (w, h) = sidebar_size(s);
    let placed = sidebar_layout(&s.sidebar_order, h);
    st.set_top_active(s.layout == crate::settings::Layout::Top);
    st.set_side_ratio(w / h);
    st.set_side_header((side::PAD_TOP + side::HEADER) / h);
    st.set_side_cards(model(
        placed
            .iter()
            .map(|p| ArrangeCard {
                id: p.id.as_str().into(),
                name: widget_name(&p.id).into(),
                y: p.y / h,
                h: p.h / h,
                tight: p.tight,
            })
            .collect(),
    ));
    let tight = placed.iter().any(|p| p.tight);
    st.set_side_note(
        if tight {
            "Some cards are short on this display; removing one gives the rest more room."
        } else {
            "Weather and Music keep their natural height; the other cards share the rest of the column."
        }
        .into(),
    );
    st.set_top_cards(model(
        top_layout(&s.sidebar_order)
            .into_iter()
            .map(|(id, x, w)| ArrangeCard {
                id: id.as_str().into(),
                name: widget_name(&id).into(),
                y: x,
                h: w,
                tight: false,
            })
            .collect(),
    ));
    st.set_hud_full(s.hud_full());
    st.set_side_palette(model(
        s.sidebar_available()
            .into_iter()
            .map(|id| ArrangeChip {
                id: id.into(),
                name: widget_name(id).into(),
                detail: blurb(id).into(),
                place: SharedString::new(),
            })
            .collect(),
    ));
}

/// The Top HUD preview: each widget's (id, left, width) as fractions of the bar. The first two sit
/// left of the clock, the rest right of it; Weather and Music are narrower, as on the real bar.
pub fn top_layout(order: &[String]) -> Vec<(String, f32, f32)> {
    const LEFT: (f32, f32) = (0.0, 0.405);
    const RIGHT: (f32, f32) = (0.595, 1.0);
    const GAP: f32 = 0.012;
    let weight = |id: &str| match id {
        "weather" => 0.7,
        "music" => 0.85,
        _ => 1.0,
    };
    let split = order.len().min(2);
    let mut out = Vec::new();
    for (ids, (from, to)) in [(&order[..split], LEFT), (&order[split..], RIGHT)] {
        if ids.is_empty() {
            continue;
        }
        let total: f32 = ids.iter().map(|id| weight(id)).sum();
        let room = to - from - GAP * (ids.len() - 1) as f32;
        let mut x = from;
        for id in ids {
            let w = room * weight(id) / total;
            out.push((id.clone(), x, w));
            x += w + GAP;
        }
    }
    out
}

/// Writes every feed-related property from the current settings.
pub fn populate(sw: &SettingsWindow, s: &Settings) {
    let st = sw.global::<FeedSettings>();
    // Any change to the lists makes old notes ("Added r/rust.") stale. Flows that report a result
    // set their note after this runs.
    st.set_team_status(SharedString::new());
    st.set_channel_status(SharedString::new());
    st.set_subreddit_status(SharedString::new());
    st.set_stock_status(SharedString::new());
    st.set_sidebar(model(s.sidebar_order.iter().map(|id| row(id, widget_name(id), "", true)).collect()));
    st.set_available(model(
        s.sidebar_available().into_iter().map(|id| row(id, widget_name(id), blurb(id), false)).collect(),
    ));
    populate_arrange(&st, s);

    st.set_leagues(model(
        sports::LEAGUES.iter().map(|l| row(l.id, l.name, "", s.sports.leagues.iter().any(|id| id == l.id))).collect(),
    ));
    st.set_favorites(model(
        s.sports
            .favorites
            .iter()
            .filter(|f| s.sports.leagues.contains(&f.league))
            .map(|f| {
                let league = sports::league(&f.league).map(|l| l.name).unwrap_or("");
                row(&format!("{}:{}", f.league, f.id), &f.name, league, true)
            })
            .collect(),
    ));

    st.set_channels(model(s.youtube.channels.iter().map(|c| row(&c.id, &c.name, &c.handle, true)).collect()));
    st.set_popout(s.youtube.popout);
    st.set_play_mode(if s.youtube.in_widget {
        0
    } else if s.youtube.popout {
        1
    } else {
        2
    });
    st.set_netflix_in_widget(s.netflix.in_widget);

    st.set_subreddits(model(s.reddit.subreddits.iter().map(|n| row(n, &format!("r/{n}"), "", true)).collect()));
    let followed = |n: &str| s.reddit.subreddits.iter().any(|f| f.eq_ignore_ascii_case(n));
    st.set_reddit_presets(model(
        reddit::PRESETS
            .iter()
            .filter(|(n, _)| !followed(n))
            .map(|(n, d)| row(n, &format!("r/{n}"), d, false))
            .collect(),
    ));
    st.set_reddit_sort(reddit::Sort::ALL.iter().position(|x| *x == s.reddit.sort).unwrap_or(0) as i32);
    st.set_netflix_non_english(s.netflix.non_english);

    st.set_watchlist(model(
        s.stocks
            .watchlist
            .iter()
            .map(|h| {
                let detail =
                    if h.exchange.is_empty() { h.name.clone() } else { format!("{} · {}", h.name, h.exchange) };
                row(&h.symbol, &h.symbol, &detail, true)
            })
            .collect(),
    ));
    st.set_indices(model(
        stocks::INDICES
            .iter()
            .map(|i| row(i.symbol, i.name, "", s.stocks.indices.iter().any(|x| x == i.symbol)))
            .collect(),
    ));
}

/// Connects the pages' callbacks. Called once per Settings window.
pub fn wire(sw: &SettingsWindow) {
    let st = sw.global::<FeedSettings>();
    st.on_edit_on_hud(|| with_app(|a| a.set_editing(true)));
    st.on_move_card(|i, delta| with_app(move |a| a.update_settings(|s| s.move_card(i.max(0) as usize, delta))));
    st.on_remove_card(|i| with_app(move |a| a.update_settings(|s| s.remove_card(i.max(0) as usize))));
    st.on_add_card(|id| with_app(move |a| a.update_settings(|s| s.add_card(&id))));
    st.on_insert_card(|id, at| with_app(move |a| a.update_settings(|s| s.insert_card(&id, at.max(0) as usize))));
    st.on_place_card(|from, to| {
        with_app(move |a| a.update_settings(|s| s.place_card(from.max(0) as usize, to.max(0) as usize)))
    });
    // Where a card dragged to `fraction` of the column's height would land: before the first card
    // whose middle is below the pointer.
    let weak = sw.as_weak();
    st.on_side_drop_index(move |fraction| {
        let Some(sw) = weak.upgrade() else { return 0 };
        let cards = sw.global::<FeedSettings>().get_side_cards();
        cards.iter().take_while(|c| c.y + c.h / 2.0 < fraction).count() as i32
    });
    // The same along the Top HUD bar.
    let weak = sw.as_weak();
    st.on_top_drop_index(move |fraction| {
        let Some(sw) = weak.upgrade() else { return 0 };
        let cards = sw.global::<FeedSettings>().get_top_cards();
        cards.iter().take_while(|c| c.y + c.h / 2.0 < fraction).count() as i32
    });

    st.on_toggle_league(|id| {
        with_app(move |a| {
            a.update_settings(|s| {
                let leagues = &mut s.sports.leagues;
                if let Some(i) = leagues.iter().position(|l| *l == id.as_str()) {
                    leagues.remove(i);
                } else {
                    // Keep the catalogue's order so the label reads consistently.
                    leagues.push(id.to_string());
                    leagues.sort_by_key(|l| sports::LEAGUES.iter().position(|x| x.id == l));
                }
            })
        })
    });
    st.on_remove_favorite(|key| {
        with_app(move |a| {
            a.update_settings(|s| s.sports.favorites.retain(|f| format!("{}:{}", f.league, f.id) != key.as_str()))
        })
    });
    // Debounced: typing "toronto" makes one lookup, not seven parallel roster downloads.
    st.on_search_teams(|query| {
        let query = query.to_string();
        TEAM_SEARCH.with(|t| {
            t.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(300), move || {
                search_teams(query.clone())
            })
        });
    });
    wire_youtube(&st);
    wire_reddit(&st);
    wire_stocks(&st);
    st.on_set_netflix_non_english(|on| with_app(move |a| a.update_settings(|s| s.netflix.non_english = on)));
    st.on_add_favorite(|key| {
        let Some((league, id)) = key.split_once(':') else { return };
        let Some(team) = cached_teams(league).into_iter().find(|t| t.id == id) else { return };
        with_app(move |a| {
            if a.settings().sports.favorites.len() >= sports::MAX_FAVORITES {
                if let Some(sw) = a.settings_window() {
                    sw.global::<FeedSettings>()
                        .set_team_status(format!("You can follow up to {} teams.", sports::MAX_FAVORITES).into());
                }
                return;
            }
            a.update_settings(|s| {
                if !s.sports.favorites.iter().any(|f| f.league == team.league && f.id == team.id) {
                    s.sports.favorites.push(sports::Favorite {
                        league: team.league.into(),
                        id: team.id.clone(),
                        abbr: team.abbr.clone(),
                        name: team.name.clone(),
                    });
                }
            });
            if let Some(sw) = a.settings_window() {
                let st = sw.global::<FeedSettings>();
                st.set_team_query(SharedString::new());
                st.set_team_results(model(Vec::new()));
            }
        })
    });
}

/// YouTube page callbacks.
fn wire_youtube(st: &FeedSettings) {
    st.on_remove_channel(|id| {
        with_app(move |a| a.update_settings(|s| s.youtube.channels.retain(|c| c.id != id.as_str())))
    });
    st.on_set_popout(|on| with_app(move |a| a.update_settings(|s| s.youtube.popout = on)));
    st.on_set_play_mode(|mode| {
        with_app(move |a| {
            a.update_settings(|s| {
                s.youtube.in_widget = mode == 0;
                s.youtube.popout = mode == 1;
            })
        })
    });
    st.on_set_netflix_in_widget(|on| with_app(move |a| a.update_settings(|s| s.netflix.in_widget = on)));
    st.on_add_channel(|input| {
        let input = input.to_string();
        if let Err(e) = youtube::channel_page(&input) {
            return set_channel_status(e.to_string());
        }
        set_channel_status("Looking up the channelâ€¦".into());
        std::thread::spawn(move || {
            let result = youtube::resolve_channel(&input);
            app::post(move |a| match result {
                Ok(ch) => {
                    let name = ch.name.clone();
                    let mut added = false;
                    a.update_settings(|s| {
                        if s.youtube.channels.len() >= youtube::MAX_CHANNELS {
                            return;
                        }
                        if !s.youtube.channels.iter().any(|c| c.id == ch.id) {
                            s.youtube.channels.push(ch);
                            added = true;
                        }
                    });
                    if let Some(sw) = a.settings_window() {
                        let st = sw.global::<FeedSettings>();
                        st.set_channel_status(
                            if added {
                                format!("Added {name}.")
                            } else if a.settings().youtube.channels.len() >= youtube::MAX_CHANNELS {
                                format!("You can follow up to {} channels.", youtube::MAX_CHANNELS)
                            } else {
                                format!("{name} is already in your list.")
                            }
                            .into(),
                        );
                        st.set_channel_input(SharedString::new());
                    }
                }
                Err(e) => {
                    if let Some(sw) = a.settings_window() {
                        sw.global::<FeedSettings>().set_channel_status(e.into());
                    }
                }
            });
        });
    });
}

/// Reddit page callbacks.
fn wire_reddit(st: &FeedSettings) {
    st.on_remove_subreddit(|name| {
        with_app(move |a| a.update_settings(|s| s.reddit.subreddits.retain(|n| *n != name.as_str())))
    });
    st.on_set_reddit_sort(|i| {
        let sort = reddit::Sort::ALL.get(i.max(0) as usize).copied().unwrap_or_default();
        with_app(move |a| a.update_settings(|s| s.reddit.sort = sort));
    });
    st.on_add_subreddit(|input| {
        let status = |text: String| {
            with_app(move |a| {
                if let Some(sw) = a.settings_window() {
                    sw.global::<FeedSettings>().set_subreddit_status(text.into());
                }
            })
        };
        let Some(name) = reddit::name_from(&input) else {
            return status("Use a subreddit's name, like r/AskHistorians.".into());
        };
        status(format!("Checking r/{name}â€¦"));
        std::thread::spawn(move || {
            let result = reddit::exists(&name);
            app::post(move |a| {
                let text = match result {
                    Ok(true) => {
                        let mut note = format!("Added r/{name}.");
                        a.update_settings(|s| {
                            let subs = &mut s.reddit.subreddits;
                            if subs.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                                note = format!("r/{name} is already in your list.");
                            } else if subs.len() >= reddit::MAX_SUBREDDITS {
                                note = format!("You can follow up to {} subreddits.", reddit::MAX_SUBREDDITS);
                            } else {
                                subs.push(name.clone());
                            }
                        });
                        if let Some(sw) = a.settings_window() {
                            sw.global::<FeedSettings>().set_subreddit_input(SharedString::new());
                        }
                        note
                    }
                    Ok(false) => format!("r/{name} doesn't exist or is private."),
                    Err(e) => e,
                };
                if let Some(sw) = a.settings_window() {
                    sw.global::<FeedSettings>().set_subreddit_status(text.into());
                }
            });
        });
    });
}

/// Stocks page callbacks. Search results are kept so "+" adds the exact listing picked.
fn wire_stocks(st: &FeedSettings) {
    st.on_remove_stock(|sym| {
        with_app(move |a| a.update_settings(|s| s.stocks.watchlist.retain(|h| h.symbol != sym.as_str())))
    });
    st.on_move_stock(|i, delta| {
        with_app(move |a| {
            a.update_settings(|s| {
                let list = &mut s.stocks.watchlist;
                let j = i + delta;
                if i >= 0 && (i as usize) < list.len() && j >= 0 && (j as usize) < list.len() {
                    list.swap(i as usize, j as usize);
                }
            })
        })
    });
    st.on_toggle_index(|sym| {
        with_app(move |a| {
            a.update_settings(|s| {
                let list = &mut s.stocks.indices;
                if let Some(i) = list.iter().position(|x| *x == sym.as_str()) {
                    list.remove(i);
                } else {
                    list.push(sym.to_string());
                    list.sort_by_key(|x| stocks::INDICES.iter().position(|i| i.symbol == x));
                }
            })
        })
    });
    st.on_search_stocks(|query| {
        let query = query.trim().to_string();
        if query.is_empty() {
            return set_stock_status(String::new());
        }
        set_stock_status("Searchingâ€¦".into());
        std::thread::spawn(move || {
            let (status, found) = match stocks::search(&query) {
                Ok(m) if m.is_empty() => (format!("Nothing matches â€œ{query}â€."), Vec::new()),
                Ok(m) => (String::new(), m),
                Err(e) => (e, Vec::new()),
            };
            let rows: Vec<(String, String)> = found
                .iter()
                .map(|x| {
                    let detail = [x.name.as_str(), x.exchange.as_str(), x.kind.as_str()]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    (x.symbol.clone(), detail)
                })
                .collect();
            app::post(move |a| {
                if let Some(sw) = a.settings_window() {
                    let st = sw.global::<FeedSettings>();
                    if st.get_stock_query().trim() != query {
                        return; // a newer search is on its way
                    }
                    if let Ok(mut last) = last_matches().lock() {
                        *last = found;
                    }
                    st.set_stock_status(status.into());
                    st.set_stock_results(model(rows.iter().map(|(sym, d)| row(sym, sym, d, false)).collect()));
                }
            });
        });
    });
    st.on_add_stock(|sym| {
        let found = last_matches().lock().ok().and_then(|m| m.iter().find(|x| x.symbol == sym.as_str()).cloned());
        let Some(m) = found else { return };
        with_app(move |a| {
            let mut note = format!("Added {}.", m.symbol);
            a.update_settings(|s| {
                let list = &mut s.stocks.watchlist;
                if list.iter().any(|h| h.symbol.eq_ignore_ascii_case(&m.symbol)) {
                    note = format!("{} is already on your watchlist.", m.symbol);
                } else if list.len() >= stocks::MAX_WATCHLIST {
                    note = format!("Your watchlist can hold up to {} symbols.", stocks::MAX_WATCHLIST);
                } else {
                    list.push(stocks::Holding {
                        symbol: m.symbol.clone(),
                        name: m.name.clone(),
                        exchange: m.exchange.clone(),
                    });
                }
            });
            if let Some(sw) = a.settings_window() {
                let st = sw.global::<FeedSettings>();
                st.set_stock_status(note.into());
                st.set_stock_results(model(Vec::new()));
                st.set_stock_query(SharedString::new());
            }
        });
    });
}

/// Shows a note under the watchlist and clears any old search results.
fn set_stock_status(text: String) {
    with_app(move |a| {
        if let Some(sw) = a.settings_window() {
            let st = sw.global::<FeedSettings>();
            st.set_stock_status(text.into());
            st.set_stock_results(model(Vec::new()));
        }
    });
}

fn last_matches() -> &'static Mutex<Vec<stocks::Match>> {
    static LAST: OnceLock<Mutex<Vec<stocks::Match>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

fn set_channel_status(text: String) {
    with_app(move |a| {
        if let Some(sw) = a.settings_window() {
            sw.global::<FeedSettings>().set_channel_status(text.into());
        }
    });
}

fn with_app(f: impl FnOnce(&std::rc::Rc<app::App>) + Send + 'static) {
    app::post(f);
}

fn team_cache() -> &'static Mutex<HashMap<&'static str, Vec<sports::Team>>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Vec<sports::Team>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cached_teams(league: &str) -> Vec<sports::Team> {
    team_cache().lock().ok().and_then(|m| m.get(league).cloned()).unwrap_or_default()
}

/// Looks the query up across the followed leagues' rosters (fetched once, then cached) on a
/// worker thread, and shows the matches unless the user has typed something newer meanwhile.
fn search_teams(query: String) {
    let q = query.trim().to_lowercase();
    if q.chars().count() < 2 {
        with_app(|a| {
            if let Some(sw) = a.settings_window() {
                let st = sw.global::<FeedSettings>();
                st.set_team_results(model(Vec::new()));
                st.set_team_status(SharedString::new());
            }
        });
        return;
    }
    with_app(move |a| {
        let leagues = a.settings().sports.leagues.clone();
        let followed: std::collections::HashSet<String> =
            a.settings().sports.favorites.iter().map(|f| format!("{}:{}", f.league, f.id)).collect();
        if leagues.is_empty() {
            if let Some(sw) = a.settings_window() {
                sw.global::<FeedSettings>().set_team_status("Turn on a league first.".into());
            }
            return;
        }
        std::thread::spawn(move || {
            let lgs: Vec<&sports::League> = leagues.iter().filter_map(|id| sports::league(id)).collect();
            // A cold search over 13 leagues is 13 roster requests; overlap them.
            let rosters = crate::services::poller::fan_out(&lgs, 6, |lg| {
                let teams = cached_teams(lg.id);
                if !teams.is_empty() {
                    return Some(teams);
                }
                match sports::teams(lg.id) {
                    Ok(list) => {
                        // Only real rosters are cached, so a failure is retried next search.
                        if let Ok(mut m) = team_cache().lock() {
                            m.insert(lg.id, list.clone());
                        }
                        Some(list)
                    }
                    Err(e) => {
                        crate::storage::log(format!("team search failed ({}): {e}", lg.name));
                        None
                    }
                }
            });
            let failed = rosters.iter().any(Option::is_none);
            let matches: Vec<(String, String, &str)> = lgs
                .iter()
                .zip(rosters)
                .flat_map(|(lg, teams)| teams.unwrap_or_default().into_iter().map(move |t| (lg.name, t)))
                .filter(|(_, t)| t.name.to_lowercase().contains(&q) || t.abbr.to_lowercase() == q)
                .map(|(name, t)| (format!("{}:{}", t.league, t.id), t.name, name))
                .filter(|(key, ..)| !followed.contains(key))
                .take(MAX_TEAM_RESULTS)
                .collect();
            app::post(move |a| {
                let Some(sw) = a.settings_window() else { return };
                let st = sw.global::<FeedSettings>();
                if st.get_team_query().trim().to_lowercase() != q {
                    return; // stale
                }
                // Re-checked here too: a team may have been followed while the search ran.
                let favorites: std::collections::HashSet<String> =
                    a.settings().sports.favorites.iter().map(|f| format!("{}:{}", f.league, f.id)).collect();
                let rows: Vec<OptionRow> = matches
                    .iter()
                    .filter(|(key, ..)| !favorites.contains(key))
                    .map(|(key, name, league)| row(key, name, league, false))
                    .collect();
                st.set_team_status(
                    match (rows.is_empty(), failed) {
                        (true, true) => "Couldn't reach ESPN. Check your connection and try again.",
                        (true, false) => "No matching teams in your leagues.",
                        (false, true) => "Some leagues couldn't be searched right now.",
                        (false, false) => "",
                    }
                    .into(),
                );
                st.set_team_results(model(rows));
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::sidebar_layout;

    fn order(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sidebar_preview_matches_the_real_column() {
        // A 1080p display with a taskbar: ~1032 DIPs of column.
        let cards = sidebar_layout(&order(&["weather", "music", "today", "news"]), 1032.0);
        assert_eq!(cards[0].h, 198.0, "weather keeps its natural height");
        assert!(cards.windows(2).all(|w| w[1].y > w[0].y + w[0].h), "stacked without overlap");
        let last = cards.last().unwrap();
        assert!((last.y + last.h - (1032.0 - 12.0 - 30.0 - 16.0)).abs() < 0.5, "fills down to the chrome row");
        assert!(cards[2].h > cards[3].h, "Tasks & Notes gets a bigger share");
        assert!(cards.iter().all(|c| !c.tight), "the default fits comfortably");
    }

    #[test]
    fn top_bar_splits_the_list_around_the_clock() {
        let ids = |v: &[(String, f32, f32)]| v.iter().map(|c| c.0.clone()).collect::<Vec<_>>();
        let four = super::top_layout(&order(&["weather", "today", "reddit", "music"]));
        assert_eq!(ids(&four), ["weather", "today", "reddit", "music"]);
        assert!(four[1].1 + four[1].2 <= 0.41 && four[2].1 >= 0.59, "two each side of the clock");
        assert!(four[0].2 < four[1].2, "Weather is narrower than a feed");
        let one = super::top_layout(&order(&["news"]));
        assert_eq!((one[0].1, one.len()), (0.0, 1), "a single widget takes the left side");
        let three = super::top_layout(&order(&["a", "b", "c"]));
        assert!(three[2].1 >= 0.59 && (three[2].1 + three[2].2 - 1.0).abs() < 1e-4, "the third fills the right");
    }

    #[test]
    fn crowding_is_flagged() {
        // Four flexible cards on a short (768p) display.
        let crowded = order(&["today", "news", "sports", "youtube"]);
        let cards = sidebar_layout(&crowded, 720.0);
        assert!(cards.iter().all(|c| c.tight));
        assert!(cards.iter().all(|c| c.h >= 0.0));
        // A tiny display squeezes even the fixed cards rather than overflowing.
        let tiny = sidebar_layout(&order(&["weather", "music"]), 300.0);
        let last = tiny.last().unwrap();
        assert!(last.y + last.h <= 300.0 - 12.0 - 30.0 - 16.0 + 0.5);
        assert!(sidebar_layout(&[], 1000.0).is_empty());
    }
}
