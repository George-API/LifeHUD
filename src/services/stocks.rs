//! Quotes and intraday sparklines for a watchlist and market indices, from Yahoo Finance's public
//! chart data: one batched `spark` request covers every symbol. Refreshes every minute while any
//! of those markets is open, every 15 minutes otherwise. Prices are as Yahoo reports them: real
//! time for most US listings, delayed (often 15 minutes) for many other exchanges.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::services::{http, poller};
use crate::storage;

const CACHE: &str = "stocks-cache.json";
const SPARK: &str = "https://query1.finance.yahoo.com/v7/finance/spark";
const SEARCH: &str = "https://query2.finance.yahoo.com/v1/finance/search";
/// Yahoo's spark endpoint accepts up to 20 symbols per request.
const BATCH: usize = 20;
const OPEN_REFRESH: Duration = Duration::from_secs(60);
const CLOSED_REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(3 * 60);
pub const MAX_WATCHLIST: usize = 30;
const SPARK_POINTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    /// Yahoo symbol, exchange suffix included ("SHOP.TO", "VOD.L").
    pub symbol: String,
    pub name: String,
    /// "NASDAQ", "Toronto"
    #[serde(default)]
    pub exchange: String,
}

pub struct Index {
    pub symbol: &'static str,
    pub name: &'static str,
}

const fn ix(symbol: &'static str, name: &'static str) -> Index {
    Index { symbol, name }
}

/// The Markets tab's catalogue.
pub const INDICES: &[Index] = &[
    ix("^GSPC", "S&P 500"),
    ix("^DJI", "Dow Jones"),
    ix("^IXIC", "Nasdaq Composite"),
    ix("^RUT", "Russell 2000"),
    ix("^VIX", "VIX"),
    ix("^GSPTSE", "S&P/TSX Composite"),
    ix("^FTSE", "FTSE 100"),
    ix("^GDAXI", "DAX"),
    ix("^FCHI", "CAC 40"),
    ix("^STOXX50E", "Euro Stoxx 50"),
    ix("^N225", "Nikkei 225"),
    ix("^HSI", "Hang Seng"),
    ix("^AXJO", "S&P/ASX 200"),
    ix("^BSESN", "BSE Sensex"),
    ix("BTC-USD", "Bitcoin"),
    ix("GC=F", "Gold"),
    ix("CL=F", "Crude Oil"),
    ix("EURUSD=X", "EUR/USD"),
];

pub fn index_name(symbol: &str) -> Option<&'static str> {
    INDICES.iter().find(|i| i.symbol == symbol).map(|i| i.name)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub watchlist: Vec<Holding>,
    pub indices: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        let region = crate::locale::region();
        let h = |symbol: &str, name: &str, exchange: &str| Holding {
            symbol: symbol.into(),
            name: name.into(),
            exchange: exchange.into(),
        };
        let us = vec![
            h("AAPL", "Apple", "NASDAQ"),
            h("MSFT", "Microsoft", "NASDAQ"),
            h("NVDA", "NVIDIA", "NASDAQ"),
            h("AMZN", "Amazon", "NASDAQ"),
        ];
        let watchlist = match region.as_str() {
            "CA" => vec![
                h("SHOP.TO", "Shopify", "Toronto"),
                h("RY.TO", "Royal Bank of Canada", "Toronto"),
                us[0].clone(),
                us[2].clone(),
            ],
            "GB" => {
                vec![h("AZN.L", "AstraZeneca", "London"), h("HSBA.L", "HSBC", "London"), us[0].clone(), us[2].clone()]
            }
            _ => us,
        };
        let indices: &[&str] = match region.as_str() {
            "CA" => &["^GSPTSE", "^GSPC", "^IXIC"],
            "GB" | "IE" => &["^FTSE", "^GSPC", "^STOXX50E"],
            "DE" | "AT" => &["^GDAXI", "^STOXX50E", "^GSPC"],
            "FR" => &["^FCHI", "^STOXX50E", "^GSPC"],
            "JP" => &["^N225", "^GSPC", "^IXIC"],
            "AU" => &["^AXJO", "^GSPC", "^IXIC"],
            "IN" => &["^BSESN", "^GSPC", "^IXIC"],
            "HK" => &["^HSI", "^GSPC", "^IXIC"],
            _ => &["^GSPC", "^DJI", "^IXIC"],
        };
        Self { watchlist, indices: indices.iter().map(|s| s.to_string()).collect() }
    }
}

impl Prefs {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.watchlist.retain(|h| is_symbol(&h.symbol) && seen.insert(h.symbol.to_ascii_uppercase()));
        self.watchlist.truncate(MAX_WATCHLIST);
        let mut seen = std::collections::HashSet::new();
        self.indices.retain(|s| index_name(s).is_some() && seen.insert(s.clone()));
    }

    fn symbols(&self) -> Vec<String> {
        self.watchlist.iter().map(|h| h.symbol.clone()).chain(self.indices.iter().cloned()).collect()
    }
}

/// Yahoo symbols: letters, digits and . - ^ = (e.g. "BRK-B", "^GSPC", "GC=F", "EURUSD=X").
pub fn is_symbol(s: &str) -> bool {
    (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b".-^=".contains(&b))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub previous_close: f64,
    pub currency: String,
    pub exchange: String,
    /// Decimal places Yahoo suggests for this instrument.
    pub decimals: u8,
    /// Unix seconds of the last trade.
    pub time: i64,
    /// Today's regular session, Unix seconds.
    pub session: (i64, i64),
    /// Intraday closes, oldest first (downsampled).
    pub points: Vec<f64>,
}

impl Quote {
    pub fn change(&self) -> f64 {
        self.price - self.previous_close
    }

    pub fn percent(&self) -> f64 {
        if self.previous_close.abs() < f64::EPSILON { 0.0 } else { self.change() / self.previous_close * 100.0 }
    }

    pub fn is_open(&self, now: i64) -> bool {
        self.session.0 <= now && now < self.session.1
    }
}

pub enum Update {
    Quotes(Vec<Quote>),
    Offline,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    quotes: Vec<Quote>,
}

pub fn cached() -> Vec<Quote> {
    storage::load_json::<Cache>(CACHE).map(|c| c.quotes).unwrap_or_default()
}

pub fn spawn(prefs: Prefs, sink: impl Fn(Update) + Send + 'static) -> poller::Poller<Prefs> {
    poller::spawn("stocks", prefs, move |prefs: &Prefs| fetch(prefs, &sink))
}

pub fn fetch(prefs: &Prefs, sink: &dyn Fn(Update)) -> Duration {
    let symbols = prefs.symbols();
    if symbols.is_empty() {
        sink(Update::Quotes(Vec::new()));
        return CLOSED_REFRESH;
    }
    let mut quotes = Vec::new();
    let mut failed = false;
    for chunk in symbols.chunks(BATCH) {
        let list: Vec<String> = chunk.iter().map(|s| http::encode(s)).collect();
        let url = format!("{SPARK}?symbols={}&range=1d&interval=5m", list.join(","));
        match http::get(&url) {
            // No "spark" object at all means Yahoo changed its format: a failure, not "no data".
            Ok(body) if !serde_json::from_str::<Value>(&body).is_ok_and(|v| v["spark"].is_object()) => {
                storage::log("stocks refresh failed: unexpected response");
                failed = true;
            }
            Ok(body) => quotes.extend(parse_spark(&body)),
            Err(e) => {
                storage::log(format!("stocks refresh failed: {e}"));
                failed = true;
            }
        }
    }
    if failed && quotes.is_empty() {
        sink(Update::Offline);
        return RETRY;
    }
    // Merge with what we had, so one failed batch doesn't blank half the list.
    let mut cache: Cache = storage::load_json(CACHE).unwrap_or_default();
    cache.quotes.retain(|q| symbols.contains(&q.symbol) && !quotes.iter().any(|n| n.symbol == q.symbol));
    cache.quotes.extend(quotes);
    storage::save_json(CACHE, &cache);
    let now = crate::services::weather::unix_now();
    let open = cache.quotes.iter().any(|q| q.is_open(now));
    sink(Update::Quotes(cache.quotes));
    if failed {
        RETRY
    } else if open {
        OPEN_REFRESH
    } else {
        CLOSED_REFRESH
    }
}

pub fn parse_spark(body: &str) -> Vec<Quote> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    v["spark"]["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let x = &r["response"][0];
            let m = &x["meta"];
            let price = m["regularMarketPrice"].as_f64()?;
            let previous_close = m["chartPreviousClose"].as_f64().or_else(|| m["previousClose"].as_f64())?;
            let symbol = m["symbol"].as_str().or_else(|| r["symbol"].as_str())?.to_string();
            let name = m["shortName"].as_str().or_else(|| m["longName"].as_str()).unwrap_or(&symbol).trim().to_string();
            let closes: Vec<f64> = x["indicators"]["quote"][0]["close"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_f64())
                .collect();
            let regular = &m["currentTradingPeriod"]["regular"];
            Some(Quote {
                name,
                price,
                previous_close,
                currency: m["currency"].as_str().unwrap_or("").into(),
                exchange: m["fullExchangeName"].as_str().unwrap_or("").into(),
                decimals: m["priceHint"].as_u64().unwrap_or(2).min(6) as u8,
                time: m["regularMarketTime"].as_i64().unwrap_or(0),
                session: (regular["start"].as_i64().unwrap_or(0), regular["end"].as_i64().unwrap_or(0)),
                points: downsample(&closes, SPARK_POINTS),
                symbol,
            })
        })
        .collect()
}

fn downsample(values: &[f64], max: usize) -> Vec<f64> {
    if values.len() <= max {
        return values.to_vec();
    }
    let step = (values.len() - 1) as f64 / (max - 1) as f64;
    (0..max).map(|i| values[((i as f64 * step).round() as usize).min(values.len() - 1)]).collect()
}

/// A sparkline as SVG path commands in a 100×`height` box, scaled so the previous close (the
/// baseline) is always in view. Callers pick `height` to match the element's aspect ratio, since
/// Slint's Path keeps the viewbox's proportions. Returns (line, filled area, baseline as a 0..1
/// fraction of the height).
pub fn sparkline(points: &[f64], baseline: f64, height: f64) -> Option<(String, String, f32)> {
    if points.len() < 2 {
        return None;
    }
    let lo = points.iter().copied().fold(baseline, f64::min);
    let hi = points.iter().copied().fold(baseline, f64::max);
    let span = (hi - lo).max(hi.abs() * 1e-4).max(1e-9);
    let pad = height * 0.07;
    let y = |v: f64| height - pad - (v - lo) / span * (height - 2.0 * pad);
    let x = |i: usize| i as f64 / (points.len() - 1) as f64 * 100.0;
    let mut line = String::new();
    for (i, v) in points.iter().enumerate() {
        line.push_str(&format!("{} {:.2} {:.2} ", if i == 0 { "M" } else { "L" }, x(i), y(*v)));
    }
    let area = format!("{line}L 100 {height} L 0 {height} Z");
    Some((line.trim_end().to_string(), area, (y(baseline) / height) as f32))
}

/// "1,234.56" with the instrument's decimals.
/// "1,234.56" with the instrument's decimals, using the user's regional separators.
pub fn format_price(v: f64, decimals: u8) -> String {
    let (decimal, thousands) = crate::locale::number_separators();
    format_number(v, decimals, &decimal, &thousands)
}

/// `format_price` with explicit separators.
pub fn format_number(v: f64, decimals: u8, decimal: &str, thousands: &str) -> String {
    let s = format!("{:.*}", decimals as usize, v.abs());
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            grouped.push_str(thousands);
        }
        grouped.push(c);
    }
    // "-0.00" reads as a loss; only show the sign when the rounded value isn't zero.
    let sign = if v < 0.0 && s.chars().any(|c| ('1'..='9').contains(&c)) { "-" } else { "" };
    match frac {
        Some(f) => format!("{sign}{grouped}{decimal}{f}"),
        None => format!("{sign}{grouped}"),
    }
}

pub fn quote_url(symbol: &str) -> String {
    format!("https://finance.yahoo.com/quote/{}", http::encode(symbol))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    pub kind: String,
}

/// Ticker search for adding to the watchlist. Blocking.
pub fn search(query: &str) -> Result<Vec<Match>, String> {
    let url = format!("{SEARCH}?q={}&quotesCount=8&newsCount=0&listsCount=0", http::encode(query.trim()));
    let body = http::get(&url).map_err(|_| "Couldn't reach Yahoo Finance. Check your connection.".to_string())?;
    Ok(parse_search(&body))
}

pub fn parse_search(body: &str) -> Vec<Match> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    v["quotes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| {
            let symbol = q["symbol"].as_str()?;
            let kind = q["quoteType"].as_str().unwrap_or("");
            if !is_symbol(symbol)
                || !matches!(kind, "EQUITY" | "ETF" | "INDEX" | "MUTUALFUND" | "CRYPTOCURRENCY" | "CURRENCY" | "FUTURE")
            {
                return None;
            }
            let name = q["shortname"].as_str().or_else(|| q["longname"].as_str()).unwrap_or(symbol);
            Some(Match {
                symbol: symbol.into(),
                name: name.split_whitespace().collect::<Vec<_>>().join(" "),
                exchange: q["exchDisp"].as_str().unwrap_or("").into(),
                kind: match kind {
                    "EQUITY" => "Stock",
                    "ETF" => "ETF",
                    "INDEX" => "Index",
                    "MUTUALFUND" => "Fund",
                    "CRYPTOCURRENCY" => "Crypto",
                    "CURRENCY" => "Currency",
                    _ => "Future",
                }
                .into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPARK_BODY: &str = r#"{"spark":{"result":[
      {"symbol":"AAPL","response":[{"meta":{"currency":"USD","symbol":"AAPL","fullExchangeName":"NasdaqGS","regularMarketTime":1794603601,
        "regularMarketPrice":356.27,"chartPreviousClose":354.88,"shortName":"Apple Inc.","priceHint":2,
        "currentTradingPeriod":{"regular":{"start":1794580200,"end":1794603600}}},
        "timestamp":[1,2,3],"indicators":{"quote":[{"close":[355.0,null,356.27]}]}}]},
      {"symbol":"NOPE","response":[{"meta":{"symbol":"NOPE"}}]}
    ],"error":null}}"#;

    #[test]
    fn parses_spark_quotes() {
        let q = parse_spark(SPARK_BODY);
        assert_eq!(q.len(), 1, "symbols without a price are skipped");
        let a = &q[0];
        assert_eq!((a.symbol.as_str(), a.name.as_str(), a.exchange.as_str()), ("AAPL", "Apple Inc.", "NasdaqGS"));
        assert_eq!(a.points, [355.0, 356.27], "nulls dropped");
        assert!((a.change() - 1.39).abs() < 1e-9);
        assert!((a.percent() - 0.3917).abs() < 1e-3);
        assert!(a.is_open(1794590000) && !a.is_open(1794603600));
    }

    #[test]
    fn sparklines_keep_the_baseline_in_view() {
        let (line, area, base) = sparkline(&[10.0, 12.0, 11.0], 9.0, 100.0).unwrap();
        assert!(line.starts_with("M 0.00 "));
        assert!(line.ends_with("L 100.00 35.67"), "{line}");
        assert!(area.ends_with("L 100 100 L 0 100 Z"));
        assert!((base - 0.93).abs() < 1e-6, "previous close below every point sits at the bottom");
        assert!(sparkline(&[1.0], 1.0, 30.0).is_none());
        let flat = sparkline(&[5.0, 5.0], 5.0, 30.0).unwrap();
        assert!(!flat.0.contains("NaN"));
        assert_eq!(downsample(&(0..100).map(f64::from).collect::<Vec<_>>(), 5), [0.0, 25.0, 50.0, 74.0, 99.0]);
    }

    #[test]
    fn formats_prices() {
        let us = |v, d| format_number(v, d, ".", ",");
        assert_eq!(us(36105.9, 2), "36,105.90");
        assert_eq!(us(356.27, 2), "356.27");
        assert_eq!(us(1234567.0, 0), "1,234,567");
        assert_eq!(us(-1.5, 2), "-1.50");
        assert_eq!(us(1.08345, 4), "1.0835");
        assert_eq!(us(-0.001, 2), "0.00", "no negative zero");
        assert_eq!(format_number(1234567.891, 2, ",", "."), "1.234.567,89", "German");
        assert_eq!(format_number(1234567.891, 2, ",", "\u{a0}"), "1\u{a0}234\u{a0}567,89", "French");
        assert_eq!(format_number(999.0, 2, ",", ""), "999,00", "no grouping");
    }

    #[test]
    fn symbols_and_prefs() {
        for s in ["AAPL", "SHOP.TO", "BRK-B", "^GSPC", "GC=F", "EURUSD=X"] {
            assert!(is_symbol(s), "{s}");
        }
        assert!(!is_symbol("AA PL") && !is_symbol(""));
        let mut p = Prefs {
            watchlist: vec![
                Holding { symbol: "AAPL".into(), name: "Apple".into(), exchange: String::new() },
                Holding { symbol: "aapl".into(), name: "dupe".into(), exchange: String::new() },
            ],
            indices: vec!["^GSPC".into(), "^BOGUS".into(), "^GSPC".into()],
        };
        p.normalize();
        assert_eq!(p.watchlist.len(), 1);
        assert_eq!(p.indices, ["^GSPC"]);
        assert_eq!(quote_url("^GSPC"), "https://finance.yahoo.com/quote/%5EGSPC");
    }

    #[test]
    fn parses_search_results() {
        let body = r#"{"quotes":[{"symbol":"VOD.L","exchDisp":"London","shortname":"VODAFONE GROUP PLC   ORD","quoteType":"EQUITY"},
            {"symbol":"X","quoteType":"OPTION"}]}"#;
        let m = parse_search(body);
        assert_eq!(m.len(), 1);
        assert_eq!(
            (m[0].symbol.as_str(), m[0].name.as_str(), m[0].kind.as_str()),
            ("VOD.L", "VODAFONE GROUP PLC ORD", "Stock")
        );
    }

    #[test]
    #[ignore = "network"]
    fn live_yahoo() {
        let prefs = Prefs::default();
        let got = std::cell::RefCell::new(Vec::new());
        fetch(&prefs, &|u| {
            if let Update::Quotes(q) = u {
                *got.borrow_mut() = q;
            }
        });
        let quotes = got.into_inner();
        println!("{:?}", quotes.iter().map(|q| (&q.symbol, q.price, q.points.len())).collect::<Vec<_>>());
        assert_eq!(quotes.len(), prefs.watchlist.len() + prefs.indices.len());
        assert!(search("shopify").unwrap().iter().any(|m| m.symbol == "SHOP.TO"));
    }
}
