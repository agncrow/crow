//! Moments: a moment is a number — f64 seconds since the Unix epoch.
//!
//! Comparison and arithmetic come free from numbers: `end - start` is
//! elapsed seconds, `t lt open - 10.0` is a staleness check. Timezones are
//! named (IANA) and explicit; the default everywhere is UTC — machine-local
//! time does not exist in this vocabulary, because "works on my laptop,
//! wrong on the UTC server" is the classic deploy-day bug.
//!
//! Shared kernel: the interpreter and generated code both call these.

use chrono::{Datelike, LocalResult, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

fn zone(tz: &str) -> Result<Tz, String> {
    tz.parse::<Tz>()
        .map_err(|_| format!("unknown timezone '{}' — use an IANA name like 'America/New_York' or 'UTC'", tz))
}

thread_local! {
    /// The moment the current event began, while one runs on this thread.
    static EVENT: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// The wall clock: seconds since the epoch.
pub fn clock() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Current moment: inside an event of a live program (a click, a render, a
/// tick, a scheduled function) the moment that event began, the same for
/// every read inside it; anywhere else the wall clock.
pub fn now() -> f64 {
    EVENT.with(|e| e.get()).unwrap_or_else(clock)
}

/// Run `f` as one event that began at `moment`: every `now()` inside it
/// reads `moment` (core-design.md §3).
pub fn at<T>(moment: f64, f: impl FnOnce() -> T) -> T {
    let saved = EVENT.with(|e| e.replace(Some(moment)));
    struct Restore(Option<f64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let saved = self.0;
            EVENT.with(|e| e.set(saved));
        }
    }
    let _restore = Restore(saved);
    f()
}

/// Hold this thread's clock at `moment` until told otherwise (`None`
/// releases it): the test harness, whose clock is the test's for a whole
/// test block (`change[a, 'now', t]` moves it).
pub fn hold(moment: Option<f64>) {
    EVENT.with(|e| e.set(moment));
}

/// The moment of the event running on this thread, if one is.
pub fn event_moment() -> Option<f64> {
    EVENT.with(|e| e.get())
}

/// Build a moment from calendar components in a named timezone.
/// Fractional seconds are allowed. DST gaps and ambiguities are loud.
pub fn moment(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: f64,
    tz: &str,
) -> Result<f64, String> {
    let z = zone(tz)?;
    let whole = second.floor();
    let nanos = ((second - whole) * 1e9).round() as u32;
    let dt = z.with_ymd_and_hms(
        year as i32,
        month as u32,
        day as u32,
        hour as u32,
        minute as u32,
        whole as u32,
    );
    let dt = match dt {
        LocalResult::Single(d) => d,
        LocalResult::Ambiguous(a, _) => a, // fall-back DST hour: earlier offset
        LocalResult::None => {
            return Err(format!(
                "{}-{:02}-{:02} {:02}:{:02} does not exist in {} (DST gap)",
                year, month, day, hour, minute, tz
            ))
        }
    };
    let dt = dt
        .with_nanosecond(nanos)
        .ok_or_else(|| "invalid fractional second".to_string())?;
    if dt.year() as i64 != year || dt.month() as i64 != month || dt.day() as i64 != day {
        return Err(format!("invalid date: {}-{}-{}", year, month, day));
    }
    Ok(dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1e9)
}

/// Calendar components of a moment in a named timezone.
/// Returns (year, month, day, hour, minute, second, weekday) —
/// weekday is ISO: 1 = Monday … 7 = Sunday.
pub fn calendar(t: f64, tz: &str) -> Result<[(&'static str, i64); 7], String> {
    let z = zone(tz)?;
    let secs = t.floor() as i64;
    let nanos = ((t - t.floor()) * 1e9) as u32;
    let dt = match Utc.timestamp_opt(secs, nanos) {
        LocalResult::Single(d) => d.with_timezone(&z),
        _ => return Err(format!("moment {} is out of range", t)),
    };
    Ok([
        ("year", dt.year() as i64),
        ("month", dt.month() as i64),
        ("day", dt.day() as i64),
        ("hour", dt.hour() as i64),
        ("minute", dt.minute() as i64),
        ("second", dt.second() as i64),
        ("weekday", dt.weekday().number_from_monday() as i64),
    ])
}

/// Parse a timestamp string into a moment. Accepts RFC3339 (offset wins over
/// tz), then naive datetime / date forms interpreted in the named timezone.
pub fn parse_time(text: &str, tz: &str) -> Result<f64, String> {
    let s = text.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1e9);
    }
    let z = zone(tz)?;
    for pat in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M", "%Y-%m-%d"] {
        let naive = if *pat == *"%Y-%m-%d" {
            chrono::NaiveDate::parse_from_str(s, pat)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        } else {
            chrono::NaiveDateTime::parse_from_str(s, pat).ok()
        };
        if let Some(naive) = naive {
            return match z.from_local_datetime(&naive) {
                LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => {
                    Ok(dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1e9)
                }
                LocalResult::None => Err(format!("'{}' does not exist in {} (DST gap)", s, tz)),
            };
        }
    }
    Err(format!(
        "cannot parse '{}' as a time — expected RFC3339 or 'YYYY-MM-DD[ HH:MM[:SS]]'",
        s
    ))
}

/// Render a moment. Styles are named words, not a mini-language:
/// 'iso' (RFC3339), 'date', 'time', 'datetime'.
pub fn fmt_time(t: f64, style: &str, tz: &str) -> Result<String, String> {
    let z = zone(tz)?;
    let secs = t.floor() as i64;
    let nanos = ((t - t.floor()) * 1e9) as u32;
    let dt = match Utc.timestamp_opt(secs, nanos) {
        LocalResult::Single(d) => d.with_timezone(&z),
        _ => return Err(format!("moment {} is out of range", t)),
    };
    Ok(match style {
        "iso" => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "date" => dt.format("%Y-%m-%d").to_string(),
        "time" => dt.format("%H:%M:%S").to_string(),
        "datetime" => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        other => {
            return Err(format!(
                "unknown time style '{}' — use 'iso', 'date', 'time', or 'datetime'",
                other
            ))
        }
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_event_reads_one_moment() {
        let t = 1_788_000_000.25;
        at(t, || {
            assert_eq!(now(), t);
            std::thread::sleep(std::time::Duration::from_millis(2));
            assert_eq!(now(), t, "time does not move inside an event");
        });
        assert!(now() > t, "outside the event the clock runs");
    }

    use super::*;

    #[test]
    fn moment_round_trips_through_calendar() {
        let t = moment(2026, 7, 14, 9, 30, 0.0, "America/New_York").unwrap();
        let p = calendar(t, "America/New_York").unwrap();
        let get = |k: &str| p.iter().find(|(n, _)| *n == k).unwrap().1;
        assert_eq!(get("year"), 2026);
        assert_eq!(get("month"), 7);
        assert_eq!(get("day"), 14);
        assert_eq!(get("hour"), 9);
        assert_eq!(get("minute"), 30);
        assert_eq!(get("weekday"), 2); // 2026-07-14 is a Tuesday
        // Same instant reads as 13:30 UTC (EDT = UTC-4)
        let pu = calendar(t, "UTC").unwrap();
        assert_eq!(pu.iter().find(|(n, _)| *n == "hour").unwrap().1, 13);
    }

    #[test]
    fn subtraction_is_elapsed_and_comparison_is_free() {
        let open = moment(2026, 7, 14, 9, 30, 0.0, "America/New_York").unwrap();
        let print_ts = moment(2026, 7, 14, 0, 20, 0.0, "America/New_York").unwrap();
        // The finch stale-print question, as arithmetic:
        assert!(print_ts < open - 10.0);
        assert_eq!(open - print_ts, 9.0 * 3600.0 + 10.0 * 60.0);
    }

    #[test]
    fn parse_accepts_rfc3339_and_naive_in_tz() {
        let a = parse_time("2026-07-14T04:09:00-04:00", "UTC").unwrap();
        let b = parse_time("2026-07-14 04:09:00", "America/New_York").unwrap();
        assert_eq!(a, b);
        let d = parse_time("2026-07-14", "UTC").unwrap();
        assert_eq!(fmt_time(d, "iso", "UTC").unwrap(), "2026-07-14T00:00:00Z");
        assert!(parse_time("not a time", "UTC").is_err());
    }

    #[test]
    fn fmt_styles_and_errors() {
        let t = moment(2026, 7, 14, 9, 30, 5.0, "UTC").unwrap();
        assert_eq!(fmt_time(t, "date", "UTC").unwrap(), "2026-07-14");
        assert_eq!(fmt_time(t, "time", "UTC").unwrap(), "09:30:05");
        assert_eq!(
            fmt_time(t, "datetime", "America/New_York").unwrap(),
            "2026-07-14 05:30:05"
        );
        assert!(fmt_time(t, "wat", "UTC").is_err());
        assert!(moment(2026, 2, 30, 0, 0, 0.0, "UTC").is_err());
        assert!(zone("Mars/Olympus").is_err());
    }
}
