//! Stocks widget controller: orders quotes as the user listed them and formats prices, changes and
//! sparklines.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::{PAGE_STOCKS, sync};
use crate::services::poller::Poller;
use crate::services::stocks::{self, Prefs, Quote};
use crate::services::weather::unix_now;
use crate::settings::Settings;
use crate::{HudWindow, QuoteData, StocksState, app, locale};

/// Sparkline viewbox heights for a width of 100, matching each element's aspect ratio.
const TILE_SPARK: f64 = 30.0;
const ROW_SPARK: f64 = 46.0;

pub struct Stocks {
    hud: slint::Weak<HudWindow>,
    poller: RefCell<Option<Poller<Prefs>>>,
    prefs: RefCell<Prefs>,
    quotes: RefCell<Vec<Quote>>,
    loaded: Cell<bool>,
    offline: Cell<bool>,
    use_24h: Cell<bool>,
    watch_rows: Rc<VecModel<QuoteData>>,
    market_rows: Rc<VecModel<QuoteData>>,
}

impl Stocks {
    pub fn new(hud: &HudWindow) -> Self {
        let s = Self {
            hud: hud.as_weak(),
            poller: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            quotes: RefCell::new(Vec::new()),
            loaded: Cell::new(false),
            offline: Cell::new(false),
            use_24h: Cell::new(false),
            watch_rows: Rc::new(VecModel::default()),
            market_rows: Rc::new(VecModel::default()),
        };
        let state = hud.global::<StocksState>();
        state.set_watchlist(ModelRc::from(s.watch_rows.clone()));
        state.set_markets(ModelRc::from(s.market_rows.clone()));
        state.on_open_quote(|url| {
            if url.starts_with("https://finance.yahoo.com/") {
                super::open_link(&url);
            }
        });
        state.on_customize(|| app::post(|a| a.open_settings(Some(PAGE_STOCKS))));
        s
    }

    pub fn start(&self, settings: &Settings) {
        *self.prefs.borrow_mut() = settings.stocks.clone();
        let cached = stocks::cached();
        self.loaded.set(!cached.is_empty());
        *self.quotes.borrow_mut() = cached;
        self.render(settings.use_24h);
        let poller = stocks::spawn(settings.stocks.clone(), |u| app::post(move |a| a.feeds.stocks.on_update(u)));
        *self.poller.borrow_mut() = Some(poller);
    }

    pub fn set_active(&self, active: bool) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.set_active(active);
        }
    }

    pub fn configure(&self, settings: &Settings) {
        if settings.stocks == *self.prefs.borrow() {
            return;
        }
        *self.prefs.borrow_mut() = settings.stocks.clone();
        self.render(settings.use_24h);
        if let Some(p) = self.poller.borrow().as_ref() {
            p.configure(settings.stocks.clone());
        }
    }

    pub fn on_update(&self, update: stocks::Update) {
        match update {
            stocks::Update::Quotes(q) => {
                *self.quotes.borrow_mut() = q;
                self.loaded.set(true);
                self.offline.set(false);
            }
            stocks::Update::Offline => self.offline.set(true),
        }
        self.render(self.use_24h.get());
    }

    pub fn render(&self, use_24h: bool) {
        self.use_24h.set(use_24h);
        let Some(hud) = self.hud.upgrade() else { return };
        let state = hud.global::<StocksState>();
        let prefs = self.prefs.borrow();
        let quotes = self.quotes.borrow();
        let find = |symbol: &str| quotes.iter().find(|q| q.symbol.eq_ignore_ascii_case(symbol));
        let loaded = self.loaded.get();
        // Symbols with no quote yet show as pending until the first refresh, then as unavailable.
        let watch: Vec<QuoteData> = prefs
            .watchlist
            .iter()
            .filter_map(|h| match find(&h.symbol) {
                Some(q) => Some(quote_data(q, &h.symbol, &h.name)),
                None => loaded.then(|| missing(&h.symbol, &h.name)),
            })
            .collect();
        let markets: Vec<QuoteData> = prefs
            .indices
            .iter()
            .filter_map(|s| {
                let name = stocks::index_name(s).unwrap_or(s);
                match find(s) {
                    Some(q) => Some(quote_data(q, name, "")),
                    None => loaded.then(|| missing(name, "")),
                }
            })
            .collect();

        let now = unix_now();
        let shown: Vec<&Quote> = prefs
            .watchlist
            .iter()
            .map(|h| h.symbol.as_str())
            .chain(prefs.indices.iter().map(|s| s.as_str()))
            .filter_map(find)
            .collect();
        let open = shown.iter().any(|q| q.is_open(now));
        state.set_open(open);
        state.set_status(status(&shown, open, use_24h).into());
        let waiting = if self.offline.get() && !loaded {
            Some("Quotes will appear when you're online.")
        } else if !loaded {
            Some("Loading quotes…")
        } else {
            None
        };
        state.set_placeholder(
            if prefs.watchlist.is_empty() { "Your watchlist is empty." } else { waiting.unwrap_or("No quotes yet.") }
                .into(),
        );
        state.set_markets_placeholder(
            if prefs.indices.is_empty() { "No markets chosen." } else { waiting.unwrap_or("No quotes yet.") }.into(),
        );
        state.set_has_watchlist(!prefs.watchlist.is_empty());
        state.set_has_markets(!prefs.indices.is_empty());
        sync(&self.watch_rows, watch);
        sync(&self.market_rows, markets);
    }
}

/// "Market open", or when the latest quote is from: "At close · Fri 4:00 PM".
fn status(quotes: &[&Quote], open: bool, use_24h: bool) -> String {
    if open {
        return "Market open".into();
    }
    let Some(latest) = quotes.iter().map(|q| q.time).max().filter(|t| *t > 0) else { return String::new() };
    let Some(st) = locale::local_from_unix(latest) else { return String::new() };
    let day = locale::day_label(&st, &locale::now());
    format!("At close · {day} {}", locale::time_label(&st, use_24h))
}

pub(crate) fn quote_data(q: &Quote, symbol: &str, name: &str) -> QuoteData {
    let (change, pct) = (q.change(), q.percent());
    let up = change >= 0.0;
    let sign = if up { "+" } else { "−" };
    let chart = stocks::sparkline(&q.points, q.previous_close, TILE_SPARK);
    let (line, area, baseline) = chart.clone().unwrap_or_default();
    let (row_line, row_area, _) = stocks::sparkline(&q.points, q.previous_close, ROW_SPARK).unwrap_or_default();
    QuoteData {
        symbol: symbol.into(),
        // Mixed watchlists (CAD, USD, GBp in pence) aren't comparable at a glance without it.
        name: {
            let name = if name.is_empty() { q.name.as_str() } else { name };
            if q.currency.is_empty() || name == q.currency {
                name.into()
            } else {
                format!("{name} · {}", q.currency).into()
            }
        },
        price: stocks::format_price(q.price, q.decimals).into(),
        change: format!("{sign}{}", stocks::format_price(change.abs(), q.decimals)).into(),
        percent: format!("{sign}{}%", stocks::format_price(pct.abs(), 2)).into(),
        up,
        line: line.into(),
        area: area.into(),
        row_line: row_line.into(),
        row_area: row_area.into(),
        baseline,
        has_chart: chart.is_some(),
        missing: false,
        url: stocks::quote_url(&q.symbol).into(),
    }
}

fn missing(symbol: &str, name: &str) -> QuoteData {
    QuoteData {
        symbol: symbol.into(),
        name: if name.is_empty() { "No data".into() } else { name.into() },
        missing: true,
        up: true,
        baseline: 0.5,
        url: stocks::quote_url(symbol).into(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use crate::services::stocks::Quote;

    #[test]
    fn quote_rows_sign_and_format() {
        let q = Quote {
            symbol: "^GSPTSE".into(),
            name: "S&P/TSX".into(),
            price: 36105.9,
            previous_close: 36137.2,
            currency: "CAD".into(),
            exchange: String::new(),
            decimals: 2,
            time: 0,
            session: (0, 0),
            points: vec![36137.0, 36100.0],
        };
        let r = super::quote_data(&q, "S&P/TSX Composite", "CAD");
        assert_eq!((r.price.as_str(), r.change.as_str(), r.percent.as_str()), ("36,105.90", "−31.30", "−0.09%"));
        assert!(!r.up && r.has_chart);
        assert_eq!(r.url, "https://finance.yahoo.com/quote/%5EGSPTSE");
        let m = super::missing("ZZZZ", "");
        assert!(m.missing && m.name == "No data");
    }
}
