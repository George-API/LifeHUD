//! Locale-aware formatting straight from Windows, so day/month names, date order and the
//! 12/24-hour preference match the user's regional settings without pulling in a date crate.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_IMEASURE, LOCALE_S1159,
    LOCALE_S2359, LOCALE_SMONTHDAY, LOCALE_STIMEFORMAT,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
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
    fn iso_dates() {
        let st = date_from_iso("2026-11-14").unwrap();
        assert_eq!((st.wYear, st.wMonth, st.wDay), (2026, 11, 14));
        assert!(date_from_iso("bad").is_none());
    }
}
