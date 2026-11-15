//! Locale-aware formatting straight from Windows, so day/month names, date order and the
//! 12/24-hour preference match the user's regional settings without pulling in a date crate.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_IMEASURE, LOCALE_S1159,
    LOCALE_S2359, LOCALE_SDECIMAL, LOCALE_SMONTHDAY, LOCALE_STHOUSAND, LOCALE_STIMEFORMAT,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::core::{HSTRING, PCWSTR};

pub fn now() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

fn locale_info(kind: u32) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut buf)) };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(len as usize - 1)])
}

/// Formats a date with a Windows date picture (e.g. "dddd", "MMMM d").
pub fn format_date(st: &SYSTEMTIME, picture: &str) -> String {
    let picture = HSTRING::from(picture);
    let mut buf = [0u16; 128];
    let len = unsafe {
        GetDateFormatEx(PCWSTR::null(), ENUM_DATE_FORMATS_FLAGS(0), Some(st), &picture, Some(&mut buf), PCWSTR::null())
    };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(len as usize - 1)])
}

/// Two-letter region of the user's locale ("en-CA" → "CA"), or empty.
pub fn region() -> String {
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len <= 1 {
        return String::new();
    }
    let name = String::from_utf16_lossy(&buf[..(len as usize - 1)]);
    name.rsplit('-').next().filter(|r| r.len() == 2).unwrap_or("").to_string()
}

/// The user's decimal and thousands separators ("." and "," in en-US; "," and "." in de-DE).
pub fn number_separators() -> (String, String) {
    let decimal = locale_info(LOCALE_SDECIMAL);
    let thousands = locale_info(LOCALE_STHOUSAND);
    (if decimal.is_empty() { ".".into() } else { decimal }, thousands)
}

pub fn prefers_24h() -> bool {
    locale_info(LOCALE_STIMEFORMAT).contains('H')
}

pub fn is_metric() -> bool {
    locale_info(LOCALE_IMEASURE) != "1"
}

pub fn am_pm(hour: u16) -> String {
    let s = locale_info(if hour < 12 { LOCALE_S1159 } else { LOCALE_S2359 });
    if s.is_empty() { if hour < 12 { "AM".into() } else { "PM".into() } } else { s }
}

pub fn weekday(st: &SYSTEMTIME) -> String {
    format_date(st, "dddd")
}

pub fn weekday_short(st: &SYSTEMTIME) -> String {
    format_date(st, "ddd")
}

/// "November 14" or "14 November", following the regional month/day pattern.
pub fn month_day(st: &SYSTEMTIME) -> String {
    let pattern = locale_info(LOCALE_SMONTHDAY);
    format_date(st, if pattern.is_empty() { "MMMM d" } else { &pattern })
}

pub fn hour_label(hour: u32, use_24h: bool) -> String {
    if use_24h {
        format!("{hour:02}")
    } else {
        let h = if hour.is_multiple_of(12) { 12 } else { hour % 12 };
        format!("{h}{}", if hour < 12 { "AM" } else { "PM" })
    }
}

/// Builds a SYSTEMTIME for a calendar date ("2026-11-14").
pub fn date_from_iso(s: &str) -> Option<SYSTEMTIME> {
    let mut parts = s.get(..10)?.split('-');
    let y: u16 = parts.next()?.parse().ok()?;
    let m: u16 = parts.next()?.parse().ok()?;
    let d: u16 = parts.next()?.parse().ok()?;
    Some(SYSTEMTIME { wYear: y, wMonth: m, wDay: d, ..Default::default() })
}

/// A Unix timestamp as local wall-clock time, honouring the zone's daylight-saving rules.
pub fn local_from_unix(t: i64) -> Option<SYSTEMTIME> {
    let ticks = (t.checked_mul(10_000_000)?).checked_add(116_444_736_000_000_000)?;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

/// "7:05 PM" or "19:05".
pub fn time_label(st: &SYSTEMTIME, use_24h: bool) -> String {
    if use_24h {
        format!("{:02}:{:02}", st.wHour, st.wMinute)
    } else {
        let h = if st.wHour.is_multiple_of(12) { 12 } else { st.wHour % 12 };
        format!("{h}:{:02} {}", st.wMinute, am_pm(st.wHour))
    }
}

/// Days from `today` to `st`, both local calendar dates.
pub fn days_between(today: &SYSTEMTIME, st: &SYSTEMTIME) -> i64 {
    let day = |s: &SYSTEMTIME| civil_days(s.wYear as i64, s.wMonth as i64, s.wDay as i64);
    day(st) - day(today)
}

/// "Today", "Tomorrow", "Sun" within the week, else "Nov 20" in the user's date order.
pub fn day_label(st: &SYSTEMTIME, today: &SYSTEMTIME) -> String {
    match days_between(today, st) {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => weekday_short(st),
        _ => short_date(st),
    }
}

/// "Nov 20" or "20 Nov".
pub fn short_date(st: &SYSTEMTIME) -> String {
    let pattern = locale_info(LOCALE_SMONTHDAY);
    let day_first = pattern.trim_start().starts_with('d');
    format_date(st, if day_first { "d MMM" } else { "MMM d" })
}

fn civil_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// Sortable local timestamp for the log.
pub fn timestamp() -> String {
    let t = now();
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_labels() {
        assert_eq!(hour_label(0, false), "12AM");
        assert_eq!(hour_label(13, false), "1PM");
        assert_eq!(hour_label(7, true), "07");
    }

    #[test]
    fn local_times_and_day_distances() {
        let st = local_from_unix(1_794_600_000).unwrap();
        assert!(st.wYear == 2026 && st.wMonth == 11);
        let a = date_from_iso("2026-12-31").unwrap();
        let b = date_from_iso("2027-01-02").unwrap();
        assert_eq!(days_between(&a, &b), 2);
        let t = SYSTEMTIME { wHour: 19, wMinute: 5, ..Default::default() };
        assert_eq!(time_label(&t, true), "19:05");
        assert!(time_label(&t, false).starts_with("7:05"));
    }

    #[test]
    fn iso_dates() {
        let st = date_from_iso("2026-11-14").unwrap();
        assert_eq!((st.wYear, st.wMonth, st.wDay), (2026, 11, 14));
        assert!(date_from_iso("bad").is_none());
    }
}
