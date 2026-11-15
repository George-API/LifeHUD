//! Weather from Open-Meteo (free, no API key) on a background thread, using the OS HTTP stack
//! (WinRT HttpClient) so TLS, proxies and certificates follow Windows settings.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
use windows::Foundation::TimeSpan;

use crate::settings::ResolvedCity;
use crate::storage;

const CACHE: &str = "weather-cache.json";
const REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(2 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub location_name: String,
    /// Which location source produced this ("device" or "city:<query>").
    pub location_key: String,
    pub celsius: bool,
    /// Unix seconds.
    pub fetched_at: i64,
    pub temperature: f64,
    pub feels_like: f64,
    pub code: i32,
    pub is_day: bool,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hour {
    pub hour: u32,
    pub temperature: f64,
    pub code: i32,
    pub is_day: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Day {
    /// ISO date, e.g. "2026-11-14".
    pub date: String,
    pub code: i32,
    pub high: f64,
    pub low: f64,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub city: String,
    pub celsius: bool,
    pub resolved: Option<ResolvedCity>,
}

impl Config {
    fn key(&self) -> String {
        if self.city.is_empty() { "device".into() } else { format!("city:{}", self.city.to_lowercase()) }
    }
}

pub enum Command {
    Configure(Config),
}

pub enum Update {
    Snapshot {
        snapshot: Snapshot,
        stale: bool,
    },
    /// No data to show; explain why.
    Unavailable(String),
    /// A city was geocoded; persist it.
    Resolved(ResolvedCity),
    /// Human-readable description of the location source (for Settings).
    Status(String),
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Cached snapshot for an instant first paint, if it matches the current configuration.
pub fn cached(config: &Config) -> Option<Snapshot> {
    storage::load_json::<Snapshot>(CACHE).filter(|s| s.celsius == config.celsius && s.location_key == config.key())
}

pub fn spawn(config: Config, sink: impl Fn(Update) + Send + 'static) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("weather".into())
        .spawn(move || {
            crate::platform::init_worker_thread();
            Worker { config, sink: Box::new(sink), device: None, last_good: None }.run(rx);
        })
        .expect("spawn weather thread");
    tx
}

struct Worker {
    config: Config,
    sink: Box<dyn Fn(Update) + Send>,
    device: Option<(f64, f64, Instant)>,
    last_good: Option<Snapshot>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        self.last_good = cached(&self.config);
        let mut wait = Duration::ZERO;
        loop {
            match rx.recv_timeout(wait) {
                Ok(Command::Configure(c)) => self.config = c,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Coalesce bursts (e.g. typing a city, then switching units).
            while let Ok(cmd) = rx.try_recv() {
                let Command::Configure(c) = cmd;
                self.config = c;
            }
            wait = if self.refresh() { REFRESH } else { RETRY };
        }
    }

    /// Returns true on success.
    fn refresh(&mut self) -> bool {
        let key = self.config.key();
        let Some((lat, lon, name)) = self.location() else {
            if self.last_good.as_ref().is_none_or(|s| s.location_key != key) {
                self.last_good = None;
            }
            return false;
        };
        match fetch(lat, lon, &name, &key, self.config.celsius) {
            Ok(snapshot) => {
                storage::save_json(CACHE, &snapshot);
                (self.sink)(Update::Status(format!("Showing weather for {name}.")));
                (self.sink)(Update::Snapshot { snapshot: snapshot.clone(), stale: false });
                self.last_good = Some(snapshot);
                true
            }
            Err(e) => {
                storage::log(format!("weather refresh failed: {e}"));
                match self.last_good.clone().filter(|s| s.location_key == key && s.celsius == self.config.celsius) {
                    Some(snapshot) => (self.sink)(Update::Snapshot { snapshot, stale: true }),
                    None => {
                        (self.sink)(Update::Unavailable("Offline. Weather will update when you're back online.".into()))
                    }
                }
                false
            }
        }
    }

    fn location(&mut self) -> Option<(f64, f64, String)> {
        let city = self.config.city.trim().to_string();
        if !city.is_empty() {
            if let Some(r) = self.config.resolved.as_ref().filter(|r| r.query.eq_ignore_ascii_case(&city)) {
                return Some((r.latitude, r.longitude, r.name.clone()));
            }
            return match geocode(&city) {
                Ok(Some(r)) => {
                    self.config.resolved = Some(r.clone());
                    (self.sink)(Update::Resolved(r.clone()));
                    Some((r.latitude, r.longitude, r.name))
                }
                Ok(None) => {
                    (self.sink)(Update::Unavailable(format!("Couldn't find “{city}”.")));
                    (self.sink)(Update::Status(format!("No match for “{city}”. Try adding the country.")));
                    None
                }
                Err(e) => {
                    // Network trouble: keep showing the last snapshot and retry later.
                    storage::log(format!("geocoding failed: {e}"));
                    (self.sink)(Update::Status(format!(
                        "Couldn't look up “{city}”. Check your connection; Life HUD will try again."
                    )));
                    None
                }
            };
        }

        if let Some((lat, lon, at)) = self.device
            && at.elapsed() < Duration::from_secs(3600)
        {
            return Some((lat, lon, "My Location".into()));
        }
        match device_location() {
            Some((lat, lon)) => {
                self.device = Some((lat, lon, Instant::now()));
                Some((lat, lon, "My Location".into()))
            }
            None => {
                (self.sink)(Update::Unavailable("Set a location in Settings.".into()));
                (self.sink)(Update::Status("Windows location is off. Enter a city instead.".into()));
                self.device.map(|(lat, lon, _)| (lat, lon, "My Location".into()))
            }
        }
    }
}

fn device_location() -> Option<(f64, f64)> {
    let run = || -> windows::core::Result<Option<(f64, f64)>> {
        if Geolocator::RequestAccessAsync()?.join()? != GeolocationAccessStatus::Allowed {
            return Ok(None);
        }
        let locator = Geolocator::new()?;
        let max_age = TimeSpan { Duration: 30 * 60 * 10_000_000 };
        let timeout = TimeSpan { Duration: 10 * 10_000_000 };
        let pos = locator.GetGeopositionAsyncWithAgeAndTimeout(max_age, timeout)?.join()?;
        let p = pos.Coordinate()?.Point()?.Position()?;
        Ok(Some((p.Latitude, p.Longitude)))
    };
    run().unwrap_or_else(|e| {
        storage::log(format!("device location unavailable: {e}"));
        None
    })
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn geocode(city: &str) -> Result<Option<ResolvedCity>, String> {
    let url =
        format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json", encode(city));
    let body = crate::services::http::get(&url).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let Some(r) = v.get("results").and_then(|r| r.get(0)) else { return Ok(None) };
    let mut name = r["name"].as_str().unwrap_or(city).to_string();
    if let Some(country) = r["country"].as_str().filter(|c| !c.is_empty()) {
        name = format!("{name}, {country}");
    }
    match (r["latitude"].as_f64(), r["longitude"].as_f64()) {
        (Some(latitude), Some(longitude)) => {
            Ok(Some(ResolvedCity { query: city.to_string(), name, latitude, longitude }))
        }
        _ => Ok(None),
    }
}

fn fetch(lat: f64, lon: f64, name: &str, key: &str, celsius: bool) -> Result<Snapshot, String> {
    let units = if celsius {
        "&temperature_unit=celsius&wind_speed_unit=kmh"
    } else {
        "&temperature_unit=fahrenheit&wind_speed_unit=mph"
    };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,apparent_temperature,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days=6{units}"
    );
    let body = crate::services::http::get(&url).map_err(|e| e.to_string())?;
    parse(&body, name, key, celsius)
}

/// Parses an Open-Meteo forecast response. Separate from I/O so it can be unit-tested.
#[allow(clippy::needless_range_loop)] // parallel arrays are indexed together
pub fn parse(body: &str, name: &str, key: &str, celsius: bool) -> Result<Snapshot, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let cur = &v["current"];
    let num = |x: &Value| x.as_f64();
    let now_time = cur["time"].as_str().ok_or("missing current.time")?;
    let now_hour_key = now_time.get(..13).ok_or("bad current.time")?; // "2026-11-14T15"

    let mut hours = Vec::new();
    let h = &v["hourly"];
    if let (Some(times), Some(temps), Some(codes), Some(days)) =
        (h["time"].as_array(), h["temperature_2m"].as_array(), h["weather_code"].as_array(), h["is_day"].as_array())
    {
        let start = times.iter().position(|t| t.as_str().and_then(|s| s.get(..13)) == Some(now_hour_key)).unwrap_or(0);
        for i in start..times.len().min(start + 24) {
            let (Some(t), Some(temp)) = (times[i].as_str(), temps.get(i).and_then(num)) else { continue };
            hours.push(Hour {
                hour: t.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(0),
                temperature: temp,
                code: codes.get(i).and_then(|c| c.as_i64()).unwrap_or(3) as i32,
                is_day: days.get(i).and_then(|d| d.as_i64()).unwrap_or(1) == 1,
            });
        }
    }

    let mut days = Vec::new();
    let d = &v["daily"];
    if let (Some(times), Some(codes), Some(maxs), Some(mins)) = (
        d["time"].as_array(),
        d["weather_code"].as_array(),
        d["temperature_2m_max"].as_array(),
        d["temperature_2m_min"].as_array(),
    ) {
        for i in 0..times.len() {
            let (Some(date), Some(high), Some(low)) =
                (times[i].as_str(), maxs.get(i).and_then(num), mins.get(i).and_then(num))
            else {
                continue;
            };
            days.push(Day {
                date: date.into(),
                code: codes.get(i).and_then(|c| c.as_i64()).unwrap_or(3) as i32,
                high,
                low,
            });
        }
    }
    if days.is_empty() {
        return Err("forecast had no daily data".into());
    }

    Ok(Snapshot {
        location_name: name.into(),
        location_key: key.into(),
        celsius,
        fetched_at: unix_now(),
        temperature: num(&cur["temperature_2m"]).ok_or("missing temperature")?,
        feels_like: num(&cur["apparent_temperature"]).unwrap_or(f64::NAN),
        code: cur["weather_code"].as_i64().unwrap_or(3) as i32,
        is_day: cur["is_day"].as_i64().unwrap_or(1) == 1,
        hours,
        days,
    })
}

/// WMO weather interpretation codes → our icon set.
pub fn condition(code: i32) -> crate::Condition {
    use crate::Condition as C;
    match code {
        0 => C::Clear,
        1 | 2 => C::PartlyCloudy,
        3 => C::Cloudy,
        45 | 48 => C::Fog,
        51..=57 => C::Drizzle,
        61..=67 | 80..=82 => C::Rain,
        71..=77 | 85 | 86 => C::Snow,
        95.. => C::Thunder,
        _ => C::Cloudy,
    }
}

pub fn describe(code: i32, is_day: bool) -> &'static str {
    match code {
        0 => {
            if is_day {
                "Sunny"
            } else {
                "Clear"
            }
        }
        1 => {
            if is_day {
                "Mostly Sunny"
            } else {
                "Mostly Clear"
            }
        }
        2 => "Partly Cloudy",
        3 => "Cloudy",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing Drizzle",
        61 => "Light Rain",
        63 => "Rain",
        65 => "Heavy Rain",
        66 | 67 => "Freezing Rain",
        71 => "Light Snow",
        73 => "Snow",
        75 => "Heavy Snow",
        77 => "Snow Grains",
        80 => "Showers",
        81 => "Rain Showers",
        82 => "Heavy Showers",
        85 | 86 => "Snow Showers",
        95 => "Thunderstorms",
        96 | 99 => "Hail",
        _ => "—",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "current": {"time":"2026-11-14T15:00","temperature_2m":6.2,"apparent_temperature":3.1,"weather_code":3,"is_day":1},
        "hourly": {"time":["2026-11-14T14:00","2026-11-14T15:00","2026-11-14T16:00"],
                   "temperature_2m":[6.0,6.2,5.8],"weather_code":[3,3,61],"is_day":[1,1,0]},
        "daily": {"time":["2026-11-14","2026-11-15"],"weather_code":[3,61],
                  "temperature_2m_max":[7.0,10.0],"temperature_2m_min":[1.0,4.0]}
    }"#;

    #[test]
    fn parses_forecast_starting_at_current_hour() {
        let s = parse(SAMPLE, "Here", "device", true).unwrap();
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].hour, 15);
        assert!(!s.hours[1].is_day);
        assert_eq!(s.days.len(), 2);
        assert_eq!(s.days[1].high, 10.0);
    }

    #[test]
    fn rejects_empty_forecast() {
        assert!(parse(r#"{"current":{"time":"2026-11-14T15:00","temperature_2m":1}}"#, "x", "k", true).is_err());
    }

    #[test]
    fn url_encoding() {
        assert_eq!(encode("São Paulo"), "S%C3%A3o%20Paulo");
    }

    #[test]
    fn codes_map_to_conditions() {
        assert!(matches!(condition(0), crate::Condition::Clear));
        assert!(matches!(condition(81), crate::Condition::Rain));
        assert!(matches!(condition(99), crate::Condition::Thunder));
    }
}
