//! Moments: a moment is a kind of its own — an instant, in milliseconds
//! since the Unix epoch, and the offset it was written in — the same in
//! both modes (a float of seconds in the core, ISO text in the frame
//! module and a float again in a database's column had been three ways to
//! hold one). `now[]` answers one; `moment[…]` makes one from text, from
//! its parts in a zone, or from a moment in another zone; its fields read
//! in its own zone; `t2 - t1` is elapsed seconds and `t + 60` a moment;
//! `str[t]` is its ISO 8601 text and `fmt[t, style = …]` any other.
//! Zones are IANA names or offsets and always explicit; the default is
//! UTC — machine-local time does not exist in this vocabulary, because
//! "works on my laptop, wrong on the UTC server" is the classic deploy-day
//! bug. The clock below is kept in seconds for the harnesses that hold it.
//!
//! Shared kernel: the interpreter and generated code both call these.

use chrono::{Datelike, LocalResult, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

fn zone(tz: &str) -> Result<Tz, String> {
    tz.parse::<Tz>()
        .map_err(|_| format!("unknown timezone '{}' — use an IANA name like 'America/New_York' or 'UTC'", tz))
}

thread_local! {
    /// The moment the current event began, while one runs on this thread.
    static EVENT: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// The wall clock: seconds since the epoch — or, in a replay of recordings
/// (`crow test --fixtures`), the moment `CROW_CLOCK` names, still, as the
/// recording's clock stood, so what a program stamps and asks is what was
/// kept.
pub fn clock() -> f64 {
    static HELD: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    if let Some(at) = *HELD.get_or_init(|| std::env::var("CROW_CLOCK").ok().and_then(|s| s.trim().parse::<f64>().ok())) {
        return at;
    }
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

/// A moment: the instant, in milliseconds since the epoch, and the offset
/// (minutes east of UTC) it was written in. Two moments are equal, and
/// ordered, by their instant; the offset says how its fields read and how
/// it is written.
#[derive(Clone, Copy, Debug, Default)]
pub struct Moment {
    pub ms: i64,
    pub offset: i16,
}

impl PartialEq for Moment {
    fn eq(&self, other: &Moment) -> bool {
        self.ms == other.ms
    }
}
impl Eq for Moment {}
impl PartialOrd for Moment {
    fn partial_cmp(&self, other: &Moment) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Moment {
    fn cmp(&self, other: &Moment) -> std::cmp::Ordering {
        self.ms.cmp(&other.ms)
    }
}
impl std::hash::Hash for Moment {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ms.hash(state);
    }
}
impl std::fmt::Display for Moment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", crate::frame::format_datetime(self.ms, self.offset))
    }
}

/// A moment is one text wherever it is carried as JSON — a session's
/// state, a handler's argument, a derivation's answer, an API's reply —
/// the text it shows as (`2026-07-14T09:30:00-04:00`), which reads back
/// to the same instant and offset; the interpreter and a compiled program
/// hand each other that text, and a test compares what came back with a
/// moment (as `{ms, offset}` a fact had come back as a map).
impl serde::Serialize for Moment {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Moment {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        Moment::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// `t + s` and `t - s`: the moment `s` seconds on or back, in its zone.
impl std::ops::Add<f64> for Moment {
    type Output = Moment;
    fn add(self, s: f64) -> Moment {
        self.plus_seconds(s)
    }
}
impl std::ops::Add<i64> for Moment {
    type Output = Moment;
    fn add(self, s: i64) -> Moment {
        self.plus_seconds(s as f64)
    }
}
impl std::ops::Add<Moment> for f64 {
    type Output = Moment;
    fn add(self, t: Moment) -> Moment {
        t.plus_seconds(self)
    }
}
impl std::ops::Add<Moment> for i64 {
    type Output = Moment;
    fn add(self, t: Moment) -> Moment {
        t.plus_seconds(self as f64)
    }
}
impl std::ops::Sub<f64> for Moment {
    type Output = Moment;
    fn sub(self, s: f64) -> Moment {
        self.plus_seconds(-s)
    }
}
impl std::ops::Sub<i64> for Moment {
    type Output = Moment;
    fn sub(self, s: i64) -> Moment {
        self.plus_seconds(-(s as f64))
    }
}
/// `t2 - t1`: the seconds between, as a number.
impl std::ops::Sub<Moment> for Moment {
    type Output = f64;
    fn sub(self, other: Moment) -> f64 {
        self.minus(&other)
    }
}

impl Moment {
    /// The moment `secs` seconds after the epoch, in UTC.
    pub fn of_seconds(secs: f64) -> Moment {
        Moment { ms: (secs * 1000.0).round() as i64, offset: 0 }
    }

    /// The current moment (see `now`), in UTC.
    pub fn now() -> Moment {
        Moment::of_seconds(now())
    }

    /// Seconds since the epoch, as a number: `float[t]`.
    pub fn seconds(&self) -> f64 {
        self.ms as f64 / 1000.0
    }

    /// Whole seconds since the epoch: `int[t]`.
    pub fn whole_seconds(&self) -> i64 {
        self.ms.div_euclid(1000)
    }

    /// A moment read from ISO 8601 text — a date, a date and time, with
    /// `Z`, an offset or neither (UTC then), a fraction of a second kept to
    /// the millisecond — or from a date and time parted by a space, as a
    /// database or a log writes one.
    pub fn parse(text: &str) -> Result<Moment, String> {
        let s = text.trim();
        if let Some((ms, offset)) = crate::frame::parse_datetime(s) {
            return Ok(Moment { ms, offset });
        }
        // a date and time parted by a space, and a time without seconds
        let joined = if s.len() > 10 && s.as_bytes()[10] == b' ' { format!("{}T{}", &s[..10], &s[11..]) } else { s.to_string() };
        let joined = with_seconds(&joined);
        if let Some((ms, offset)) = crate::frame::parse_datetime(&joined) {
            return Ok(Moment { ms, offset });
        }
        Err(format!("cannot read '{}' as a moment — write ISO 8601: '2026-07-14', '2026-07-14T13:30:00Z' or '2026-07-14T13:30:00+09:00'", s))
    }

    /// A moment from its parts in a zone: the zone's offset at that
    /// instant is kept. A fraction of a second is allowed; a date that
    /// does not exist, or a time in a DST gap, is the reason.
    pub fn from_parts(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: f64, tz: &str) -> Result<Moment, String> {
        let whole = second.floor();
        let millis = ((second - whole) * 1000.0).round() as i64;
        let naive = chrono::NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
            .and_then(|d| d.and_hms_opt(hour as u32, minute as u32, whole as u32))
            .ok_or_else(|| format!("invalid date or time: {}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hour, minute, whole as i64))?;
        if let Some(offset) = fixed_offset(tz) {
            let fixed = chrono::FixedOffset::east_opt(i32::from(offset) * 60).ok_or_else(|| format!("'{}' is no zone", tz))?;
            let dt = fixed.from_local_datetime(&naive).single().ok_or_else(|| format!("'{}' is no zone", tz))?;
            return Ok(Moment { ms: dt.timestamp_millis() + millis, offset });
        }
        let z = zone(tz)?;
        let dt = match z.from_local_datetime(&naive) {
            LocalResult::Single(d) => d,
            LocalResult::Ambiguous(a, _) => a, // fall-back DST hour: earlier offset
            LocalResult::None => return Err(format!("{}-{:02}-{:02} {:02}:{:02} does not exist in {} (DST gap)", year, month, day, hour, minute, tz)),
        };
        let offset = (dt.offset().fix().local_minus_utc() / 60) as i16;
        Ok(Moment { ms: dt.timestamp_millis() + millis, offset })
    }

    /// Text read in a zone: a date and time written without an offset are
    /// that zone's (an offset in the text wins).
    pub fn parse_in(text: &str, tz: &str) -> Result<Moment, String> {
        let m = Moment::parse(text)?;
        let s = text.trim();
        let explicit = s.ends_with('Z') || s.ends_with('z') || (s.len() > 10 && s[10..].contains(['+', '-']));
        if explicit {
            return Ok(m);
        }
        // the civil time it wrote, as that zone's
        let civil = m.local();
        Moment::from_parts(i64::from(civil.year()), i64::from(civil.month()), i64::from(civil.day()), i64::from(civil.hour()), i64::from(civil.minute()), f64::from(civil.second()) + (m.ms.rem_euclid(1000) as f64) / 1000.0, tz)
    }

    /// The same instant in another zone: an IANA name, an offset
    /// (`'+09:00'`) or `'UTC'`.
    pub fn in_zone(&self, tz: &str) -> Result<Moment, String> {
        Ok(Moment { ms: self.ms, offset: offset_of(tz, self.ms)? })
    }

    /// The instant in its own zone, for its fields.
    fn local(&self) -> chrono::DateTime<chrono::FixedOffset> {
        let fixed = chrono::FixedOffset::east_opt(i32::from(self.offset) * 60).unwrap_or_else(|| Utc.fix());
        match fixed.timestamp_millis_opt(self.ms) {
            LocalResult::Single(d) | LocalResult::Ambiguous(d, _) => d,
            LocalResult::None => chrono::DateTime::<chrono::FixedOffset>::from(chrono::DateTime::<Utc>::UNIX_EPOCH),
        }
    }

    pub fn year(&self) -> i64 {
        i64::from(self.local().year())
    }
    pub fn month(&self) -> i64 {
        i64::from(self.local().month())
    }
    pub fn day(&self) -> i64 {
        i64::from(self.local().day())
    }
    pub fn hour(&self) -> i64 {
        i64::from(self.local().hour())
    }
    pub fn minute(&self) -> i64 {
        i64::from(self.local().minute())
    }
    pub fn second(&self) -> i64 {
        i64::from(self.local().second())
    }
    /// ISO: 1 is Monday, 7 Sunday.
    pub fn weekday(&self) -> i64 {
        i64::from(self.local().weekday().number_from_monday())
    }
    /// Minutes east of UTC the moment was written in.
    pub fn offset_minutes(&self) -> i64 {
        i64::from(self.offset)
    }

    /// A field by name, where the name is one.
    pub fn field(&self, name: &str) -> Option<i64> {
        Some(match name {
            "year" => self.year(),
            "month" => self.month(),
            "day" => self.day(),
            "hour" => self.hour(),
            "minute" => self.minute(),
            "second" => self.second(),
            "weekday" => self.weekday(),
            "offset" => self.offset_minutes(),
            _ => return None,
        })
    }

    pub fn plus_seconds(&self, s: f64) -> Moment {
        Moment { ms: self.ms + (s * 1000.0).round() as i64, offset: self.offset }
    }

    pub fn minus(&self, other: &Moment) -> f64 {
        (self.ms - other.ms) as f64 / 1000.0
    }

    /// `fmt[t, style = …, tz = …]`: the moment in a style — `'iso'` (as
    /// `str[t]` writes it), `'date'`, `'time'`, `'datetime'` — in its own
    /// zone, or in `tz` where one is named; a style or a zone it does not
    /// know is the reason.
    pub fn fmt(&self, style: &str, tz: &str) -> Result<String, String> {
        let m = if tz.is_empty() { *self } else { self.in_zone(tz)? };
        let dt = m.local();
        Ok(match style {
            "" | "iso" => m.to_string(),
            "date" => dt.format("%Y-%m-%d").to_string(),
            "time" => dt.format("%H:%M:%S").to_string(),
            "datetime" => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            other => return Err(format!("unknown time style '{}' — use 'iso', 'date', 'time', or 'datetime'", other)),
        })
    }
}

/// `2026-07-14T09:30` with `:00` added: a time written without seconds,
/// before its zone if one is written.
fn with_seconds(s: &str) -> String {
    let Some(t) = s.find('T') else { return s.to_string() };
    let rest = &s[t + 1..];
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let time = &rest[..zone_at];
    if time.len() == 5 && time.as_bytes()[2] == b':' {
        format!("{}T{}:00{}", &s[..t], time, &rest[zone_at..])
    } else {
        s.to_string()
    }
}

/// An offset written as a zone: `'UTC'`, `'Z'`, `'+09:00'`, `'-05:30'`,
/// `'+0900'`, `'+09'`; none for an IANA name.
fn fixed_offset(tz: &str) -> Option<i16> {
    if matches!(tz, "UTC" | "Z" | "utc") {
        return Some(0);
    }
    let (sign, rest) = match tz.as_bytes().first() {
        Some(b'+') => (1i16, &tz[1..]),
        Some(b'-') => (-1i16, &tz[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) || !(matches!(digits.len(), 2 | 4)) {
        return None;
    }
    let hours: i16 = digits[..2].parse().ok()?;
    let minutes: i16 = if digits.len() == 4 { digits[2..].parse().ok()? } else { 0 };
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

/// The offset of a zone at an instant: a written offset as it is, an IANA
/// zone's at that instant.
fn offset_of(tz: &str, ms: i64) -> Result<i16, String> {
    if let Some(offset) = fixed_offset(tz) {
        return Ok(offset);
    }
    let z = zone(tz)?;
    let utc = match Utc.timestamp_millis_opt(ms) {
        LocalResult::Single(d) => d,
        _ => return Err(format!("moment {} is out of range", ms)),
    };
    Ok((z.offset_from_utc_datetime(&utc.naive_utc()).fix().local_minus_utc() / 60) as i16)
}

/// `now[]`.
pub fn now_moment() -> Moment {
    Moment::now()
}

/// `moment[text]`.
pub fn moment_text(text: &str) -> Result<Moment, String> {
    Moment::parse(text)
}

/// `moment[t, tz = zone]`.
pub fn moment_in(t: Moment, tz: &str) -> Result<Moment, String> {
    t.in_zone(tz)
}

/// `moment[text, tz = zone]`: text without an offset read as that zone's.
pub fn moment_text_in(text: &str, tz: &str) -> Result<Moment, String> {
    Moment::parse_in(text, tz)
}

/// `moment[seconds]`: the moment `seconds` after the epoch, in UTC — the
/// reverse of `float[t]`, for a time a program kept as a number.
pub fn moment_of_seconds(seconds: f64) -> Moment {
    Moment::of_seconds(seconds)
}

/// `moment[t]` of a moment: itself, so `moment[x]` is the moment of a
/// number, of text or of a moment alike.
pub fn moment_self(t: Moment) -> Moment {
    t
}

/// `min[a, b]` of two moments: the earlier.
/// `bucket[t, seconds]`: the start of the bucket `t` falls in, `seconds`
/// wide, the buckets aligned to midnight in the moment's own offset (a
/// day's bucket starts at the local midnight; from UTC, at the epoch's).
pub fn bucket(t: Moment, span: f64) -> Result<Moment, String> {
    let span_ms = crate::verbs::span_ms("bucket", span)?;
    Ok(Moment { ms: crate::verbs::bucket_ms(t.ms, t.offset, span_ms), offset: t.offset })
}

pub fn min_of(a: Moment, b: Moment) -> Moment {
    if b < a { b } else { a }
}

/// `max[a, b]` of two moments: the later.
pub fn max_of(a: Moment, b: Moment) -> Moment {
    if b > a { b } else { a }
}

/// `moment[year, month, day, hour = 0, minute = 0, second = 0, tz = 'UTC']`.
pub fn moment_parts(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: f64, tz: &str) -> Result<Moment, String> {
    Moment::from_parts(year, month, day, hour, minute, second, tz)
}

/// `fmt[t, style = …, tz = …]`.
pub fn fmt_time_of(t: Moment, style: &str, tz: &str) -> Result<String, String> {
    t.fmt(style, tz)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A moment made from its parts in a zone keeps the zone's offset, reads
    /// its fields in it, and is the same instant in every zone.
    #[test]
    fn a_moment_keeps_the_zone_it_was_written_in() {
        let t = Moment::from_parts(2026, 7, 14, 9, 30, 0.0, "America/New_York").unwrap();
        assert_eq!((t.year(), t.month(), t.day(), t.hour(), t.minute(), t.weekday(), t.offset_minutes()), (2026, 7, 14, 9, 30, 2, -240));
        assert_eq!(t.to_string(), "2026-07-14T09:30:00-04:00");
        let utc = t.in_zone("UTC").unwrap();
        assert_eq!(utc, t);
        assert_eq!(utc.hour(), 13);
        assert_eq!(utc.to_string(), "2026-07-14T13:30:00Z");
        let tokyo = t.in_zone("+09:00").unwrap();
        assert_eq!((tokyo.hour(), tokyo.day()), (22, 14));
        assert_eq!(t.in_zone("Asia/Tokyo").unwrap().offset_minutes(), 540);
        assert_eq!(Moment::from_parts(2026, 1, 5, 0, 0, 0.0, "+05:30").unwrap().to_string(), "2026-01-05T00:00:00+05:30");
    }

    /// Subtraction is elapsed seconds, and comparison is by instant.
    #[test]
    fn subtraction_is_elapsed_and_comparison_is_free() {
        let open = Moment::from_parts(2026, 7, 14, 9, 30, 0.0, "America/New_York").unwrap();
        let print_ts = Moment::from_parts(2026, 7, 14, 0, 20, 0.0, "America/New_York").unwrap();
        assert!(print_ts < open - 10.0);
        assert_eq!(open - print_ts, 9.0 * 3600.0 + 10.0 * 60.0);
        assert_eq!((open + 60).minute(), 31);
        assert_eq!((60.0 + open).minute(), 31);
        assert_eq!(open.seconds(), 1_784_035_800.0);
        assert_eq!(open.whole_seconds(), 1_784_035_800);
        assert_eq!(Moment::of_seconds(1_784_035_800.25).to_string(), "2026-07-14T13:30:00.250Z");
    }

    /// Text is read as ISO 8601, with or without a time, a zone or a
    /// fraction, and with a space between the date and the time.
    #[test]
    fn text_is_read_as_iso_8601() {
        let a = Moment::parse("2026-07-14T04:09:00-04:00").unwrap();
        let b = Moment::parse("2026-07-14T08:09:00Z").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.offset_minutes(), -240);
        assert_eq!(Moment::parse("2026-07-14").unwrap().to_string(), "2026-07-14");
        assert_eq!(Moment::parse("2026-07-14").unwrap().ms, 1_783_987_200_000);
        assert_eq!(Moment::parse("2026-07-14 04:09:00").unwrap().to_string(), "2026-07-14T04:09:00Z");
        assert_eq!(Moment::parse("2026-07-14 09:30").unwrap().to_string(), "2026-07-14T09:30:00Z");
        assert_eq!(Moment::parse("2026-07-14T09:30+09:00").unwrap().to_string(), "2026-07-14T09:30:00+09:00");
        assert_eq!(Moment::parse(" 2026-07-14T04:09:00.5+01:00 ").unwrap().to_string(), "2026-07-14T04:09:00.500+01:00");
        assert!(Moment::parse("not a time").is_err());
        // read in a zone: naive text is that zone's, an offset in the text wins
        let ny = Moment::parse_in("2026-07-14 04:09:00", "America/New_York").unwrap();
        assert_eq!(ny.to_string(), "2026-07-14T04:09:00-04:00");
        assert_eq!(ny, Moment::parse("2026-07-14T08:09:00Z").unwrap());
        assert_eq!(Moment::parse_in("2026-07-14T08:09:00Z", "America/New_York").unwrap().to_string(), "2026-07-14T08:09:00Z");
        assert_eq!(Moment::parse_in("2026-07-14", "+05:30").unwrap().to_string(), "2026-07-14T00:00:00+05:30");
    }

    #[test]
    fn fmt_styles_and_errors() {
        let t = Moment::from_parts(2026, 7, 14, 9, 30, 5.0, "UTC").unwrap();
        assert_eq!(t.fmt("date", "").unwrap(), "2026-07-14");
        assert_eq!(t.fmt("time", "").unwrap(), "09:30:05");
        assert_eq!(t.fmt("datetime", "America/New_York").unwrap(), "2026-07-14 05:30:05");
        assert_eq!(t.fmt("iso", "+09:00").unwrap(), "2026-07-14T18:30:05+09:00");
        assert_eq!(t.fmt("", "").unwrap(), t.to_string());
        assert!(t.fmt("wat", "").is_err());
        assert!(t.fmt("date", "Mars/Olympus").is_err());
        assert!(Moment::from_parts(2026, 2, 30, 0, 0, 0.0, "UTC").is_err());
        assert!(Moment::from_parts(2026, 3, 8, 2, 30, 0.0, "America/New_York").is_err(), "a DST gap");
        assert!(zone("Mars/Olympus").is_err());
    }
}
