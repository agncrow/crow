//! Shared display formatting for Crow values.
//!
//! This is the single source of truth for how values print. The interpreter's
//! `Value::Display` and the Rust backend's `log[]` printing must both follow these
//! rules so both execution modes produce identical output:
//!
//! - top-level strings print bare, strings inside containers print single-quoted
//! - arrays print as `[a, b, c]`
//! - maps print as `[key = value]`
//!
//! Part of the shared semantics kernel (see docs/design-review.md, pushback 6).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Crow's canonical value formatting.
///
/// `crow_fmt` is the top-level form (strings bare); `crow_repr` is the
/// inside-a-container form (strings single-quoted). For every other type the
/// two are identical.
pub trait CrowDisplay {
    fn crow_repr(&self) -> String;
    fn crow_fmt(&self) -> String {
        self.crow_repr()
    }
    /// What `crow_fmt` shows, written onto `out` (text as it is, not made
    /// anew first).
    fn crow_write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.crow_fmt().as_bytes());
    }
    /// The value itself when it is text, which `log` hands to the output
    /// where it lies when it is long, rather than copying it first.
    fn as_text(&self) -> Option<&str> {
        None
    }
}

impl CrowDisplay for i64 {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl CrowDisplay for f64 {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl CrowDisplay for crate::moments::Moment {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl CrowDisplay for bool {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl CrowDisplay for String {
    fn crow_repr(&self) -> String {
        format!("'{}'", self)
    }
    fn crow_fmt(&self) -> String {
        self.clone()
    }
    fn crow_write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_bytes());
    }
    fn as_text(&self) -> Option<&str> {
        Some(self)
    }
}

impl CrowDisplay for str {
    fn crow_repr(&self) -> String {
        format!("'{}'", self)
    }
    fn crow_fmt(&self) -> String {
        self.to_string()
    }
    fn crow_write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_bytes());
    }
    fn as_text(&self) -> Option<&str> {
        Some(self)
    }
}

/// A value borrowed shows as the value: an item a loop walks by reference
/// (a matrix's row, a list's record) is logged as itself (a row had not
/// built compiled).
impl<T: CrowDisplay + ?Sized> CrowDisplay for &T {
    fn crow_repr(&self) -> String {
        (**self).crow_repr()
    }
    fn crow_fmt(&self) -> String {
        (**self).crow_fmt()
    }
    fn crow_write(&self, out: &mut Vec<u8>) {
        (**self).crow_write(out)
    }
    fn as_text(&self) -> Option<&str> {
        (**self).as_text()
    }
}

/// A list as both modes show it: `[a, b]`, each item as it shows inside
/// a list (the interpreter's values and the compiled program's alike).
pub fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

/// A map's entries, or a record's fields, as both modes show them:
/// `[key = value, ...]`, in the order given.
pub fn entries<K: AsRef<str>>(pairs: impl IntoIterator<Item = (K, String)>) -> String {
    list(pairs.into_iter().map(|(k, v)| format!("{} = {}", k.as_ref(), v)))
}

/// A record: its name, then its fields in name order (`P[x = 1, y = 2]`).
pub fn record<K: AsRef<str>>(name: &str, fields: impl IntoIterator<Item = (K, String)>) -> String {
    let mut fields: Vec<(K, String)> = fields.into_iter().collect();
    fields.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    format!("{}{}", name, entries(fields))
}

/// A value that failed, shown in place of its value, in both modes: its
/// reason, after the line it failed on — the first place its trace names
/// (`main:4`, or `main.f:9` in a function; a standard module's function
/// names the program's line of the call) — as a stop names its line
/// (spec, Error Messages). It had shown the reason alone, and a program
/// that logged a failed conversion did not say which line had failed.
pub fn failed(reason: &str, trace: &[String]) -> String {
    match trace.first().and_then(|t| t.rsplit(':').next()).and_then(|l| l.parse::<usize>().ok()) {
        Some(line) if line > 0 => format!("error: line {}: {}", line, reason),
        _ => format!("error: {}", reason),
    }
}

/// `log[a, b, …]`: the values as they show, between spaces, a value that
/// failed as its reason — one line, in both modes.
pub fn log(items: &[&dyn CrowDisplay]) {
    use std::io::Write;
    let mut held = OUT.lock().unwrap_or_else(|e| e.into_inner());
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            held.push(b' ');
        }
        match x.as_text() {
            // a long text goes out where it lies, after what is held: copied
            // into the buffer first, fasta's 125 MB section had been held
            // twice, and the buffer kept that size to the end of the run
            Some(t) if t.len() >= BLOCK => {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(&held);
                held.clear();
                let _ = out.write_all(t.as_bytes());
                let _ = out.flush();
            }
            _ => x.crow_write(&mut held),
        }
    }
    held.push(b'\n');
    // a line at a time, unless a script's output goes to a file or a pipe
    // (`buffer_output`): then a block at a time
    if !BLOCKS.load(Ordering::Relaxed) || held.len() >= BLOCK {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(&held);
        let _ = out.flush();
        held.clear();
        // (a buffer one long value grew is let go: it is a block's room)
        if held.capacity() > 4 * BLOCK {
            held.shrink_to(BLOCK);
        }
    }
}

/// What `log` has written and not yet handed to stdout.
static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Whether `log` writes in blocks (`buffer_output`).
static BLOCKS: AtomicBool = AtomicBool::new(false);

/// A block of output: what `log` holds before it writes.
const BLOCK: usize = 64 * 1024;

/// A script's output written in blocks when it goes to a file or a pipe:
/// a line at a time had been a write to the system for every line
/// (revcomp's four million lines, a fifth of its time). What is held is
/// written before the program sleeps, and at least every tenth of a second
/// whatever else it is doing — waiting on a query, computing — so a reader
/// watching it (a job's log, a program reading the pipe) sees each line as
/// a terminal would, or within that (holding it until the end had hidden a
/// long job's progress, and a test that reacts to the lines as they come
/// never saw them); and at the end, before a stop is reported, at `exit`,
/// and before `input` reads (`flush_output`). A terminal sees each line as it is logged, and a
/// served program — which does not ask for this — each line as it happens.
pub fn buffer_output() {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() && !BLOCKS.swap(true, Ordering::Relaxed) {
        let _ = std::thread::Builder::new().name("crow-output".to_string()).spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                flush_output();
            }
        });
    }
}

/// What `log` holds, written out now.
pub fn flush_output() {
    use std::io::Write;
    // (a panic inside `log` leaves its lock to this thread: none is taken
    // twice)
    let Ok(mut held) = OUT.try_lock() else { return };
    if !held.is_empty() {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(&held);
        let _ = out.flush();
        held.clear();
    }
}

/// What a failed `assert[left op right]` says: the comparison and both
/// sides as they show.
pub fn assert_failed(op: &str, left: &str, right: &str) -> String {
    format!("assert failed: {}\n    left:  {}\n    right: {}", op, left, right)
}

/// Nothing, as a value that answers nothing shows (`done`).
impl CrowDisplay for () {
    fn crow_repr(&self) -> String {
        "done".to_string()
    }
}

/// `str[x]`: a value as it shows (text bare, a number as it displays).
pub fn shown<T: CrowDisplay + ?Sized>(x: &T) -> String {
    x.crow_fmt()
}

impl<T: CrowDisplay> CrowDisplay for Vec<T> {
    fn crow_repr(&self) -> String {
        list(self.iter().map(|v| v.crow_repr()))
    }
}

/// A recursive link: the record it holds, or the empty sentinel `0`.
impl<T: CrowDisplay> CrowDisplay for Option<crate::link::Link<T>> {
    fn crow_repr(&self) -> String {
        match self {
            Some(r) => r.crow_repr(),
            None => "0".to_string(),
        }
    }
}

/// A parsed JSON value, as the interpreter shows the value it parsed to:
/// a number as it displays (a whole float as an int), text quoted inside a
/// list, `null` as the error it reads as. (The compiled program had shown a
/// whole float as `2.0`, and `str` of a list its JSON text.)
impl CrowDisplay for serde_json::Value {
    fn crow_repr(&self) -> String {
        match self {
            serde_json::Value::String(s) => s.crow_repr(),
            other => other.crow_fmt(),
        }
    }
    fn crow_fmt(&self) -> String {
        match self {
            serde_json::Value::Null => failed("null", &[]),
            serde_json::Value::Bool(b) => b.crow_repr(),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => i.crow_repr(),
                None => n.as_f64().unwrap_or(f64::NAN).crow_repr(),
            },
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => list(items.iter().map(|v| v.crow_repr())),
            serde_json::Value::Object(m) => entries(m.iter().map(|(k, v)| (k, v.crow_repr()))),
        }
    }
}

impl CrowDisplay for crate::frame::CrowFrame {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl CrowDisplay for crate::sparse::CrowSparse {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

/// A frame column prints like the interpreter's array of its values:
/// numbers bare, strings quoted, a missing cell as the error it is read
/// as, `error: missing` (it had shown as `nan`, in a text column too).
impl CrowDisplay for crate::frame::CrowColumn {
    fn crow_repr(&self) -> String {
        use crate::frame::Column;
        let n = self.len();
        let parts: Vec<String> = (0..n)
            .map(|i| {
                if !self.valid[i] {
                    return failed("missing", &[]);
                }
                match self.data.as_ref() {
                    Column::Int(v) => v[i].crow_repr(),
                    Column::Float(v) => v[i].crow_repr(),
                    Column::Str(v) => v[i].crow_repr(),
                    Column::Bool(v) => v[i].crow_repr(),
                    Column::DateTime(..) => self.data.fmt_cell(i, true),
                }
            })
            .collect();
        format!("[{}]", parts.join(", "))
    }
}

impl<K: std::fmt::Display, T: CrowDisplay> CrowDisplay for std::collections::BTreeMap<K, T> {
    fn crow_repr(&self) -> String {
        entries(self.iter().map(|(k, v)| (k.to_string(), v.crow_repr())))
    }
}

/// fmt[] kernel for numbers — the single implementation both execution modes
/// use. Sentinels mark unset named parameters: decimals < 0, width <= 0,
/// align "" (auto: numbers right, strings left).
pub fn crow_fmt_f64(
    v: f64,
    decimals: i64,
    commas: bool,
    width: i64,
    align: &str,
    fill: &str,
    percent: bool,
) -> Result<String, String> {
    let val = if percent { v * 100.0 } else { v };
    let mut s = if decimals >= 0 {
        format!("{:.*}", decimals as usize, val)
    } else if val == (val as i64) as f64 && val.is_finite() {
        format!("{}", val as i64)
    } else {
        val.to_string()
    };
    if commas {
        let (sign, rest) = if let Some(stripped) = s.strip_prefix('-') {
            ("-", stripped.to_string())
        } else {
            ("", s.clone())
        };
        let (int_part, frac_part) = match rest.split_once('.') {
            Some((i, f)) => (i.to_string(), format!(".{}", f)),
            None => (rest, String::new()),
        };
        let mut grouped = String::new();
        for (i, ch) in int_part.chars().enumerate() {
            if i > 0 && (int_part.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        s = format!("{}{}{}", sign, grouped, frac_part);
    }
    if percent {
        s.push('%');
    }
    apply_width(s, width, if align.is_empty() { "right" } else { align }, fill)
}

/// `fmt[x, decimals =, commas =, width =, align =, fill =, percent =]`:
/// a number by [`crow_fmt_f64`], text by [`crow_fmt_str`] — the one
/// function both modes call, so a value of either kind is written alike;
/// a moment as its ISO 8601 text; any other value is the misuse the program
/// stops on. (A moment's `style =` is `moments::fmt_time_of`, which answers
/// an error value of kind `'time'`.)
pub fn fmt_value(value: &crate::cell::Cell, decimals: i64, commas: bool, width: i64, align: &str, fill: &str, percent: bool) -> Result<String, String> {
    use crate::cell::Cell;
    match value {
        Cell::Int(n) => crow_fmt_f64(*n as f64, decimals, commas, width, align, fill, percent),
        Cell::Float(x) => crow_fmt_f64(*x, decimals, commas, width, align, fill, percent),
        Cell::Text(s) => crow_fmt_str(s, decimals, width, align, fill, percent),
        // a moment with no style: its ISO 8601 text, laid out as text is
        Cell::Moment(m) => crow_fmt_str(&m.to_string(), decimals, width, align, fill, percent),
        other => Err(format!("fmt: cannot format {}", other.kind())),
    }
}

/// fmt[] kernel for strings: width, align and fill only. Decimals or a
/// percent on a string is the misuse both modes stop on, with these words.
pub fn crow_fmt_str(
    s: &str,
    decimals: i64,
    width: i64,
    align: &str,
    fill: &str,
    percent: bool,
) -> Result<String, String> {
    if decimals >= 0 {
        return Err("fmt: decimals applies to numbers, not strings".to_string());
    }
    if percent {
        return Err("fmt: percent applies to numbers, not strings".to_string());
    }
    apply_width(
        s.to_string(),
        width,
        if align.is_empty() { "left" } else { align },
        fill,
    )
}

fn apply_width(s: String, width: i64, align: &str, fill: &str) -> Result<String, String> {
    if align != "left" && align != "right" {
        return Err(format!("fmt: align must be 'left' or 'right', got '{}'", align));
    }
    let Ok(width) = usize::try_from(width) else { return Ok(s) };
    let len = s.chars().count();
    if len >= width {
        return Ok(s);
    }
    let fill_ch = fill.chars().next().unwrap_or(' ');
    let padding: String = std::iter::repeat(fill_ch).take(width - len).collect();
    Ok(if align == "left" { format!("{}{}", s, padding) } else { format!("{}{}", padding, s) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_bare_at_top_level_quoted_in_containers() {
        assert_eq!("hello".to_string().crow_fmt(), "hello");
        assert_eq!(vec!["a".to_string(), "b".to_string()].crow_fmt(), "['a', 'b']");
    }

    #[test]
    fn test_numeric_arrays() {
        assert_eq!(vec![1i64, 2, 3].crow_fmt(), "[1, 2, 3]");
        assert_eq!(vec![1.5f64, 2.0].crow_fmt(), "[1.5, 2]");
    }

    #[test]
    fn test_bool_and_nested_arrays() {
        assert_eq!(vec![true, false].crow_fmt(), "[true, false]");
        assert_eq!(vec![vec![1i64, 2], vec![3, 4]].crow_fmt(), "[[1, 2], [3, 4]]");
    }

    #[test]
    fn test_fmt_decimals_and_commas() {
        assert_eq!(crow_fmt_f64(1234.567, 2, false, 0, "", " ", false).as_deref(), Ok("1234.57"));
        assert_eq!(crow_fmt_f64(1234.567, 2, true, 0, "", " ", false).as_deref(), Ok("1,234.57"));
        assert_eq!(crow_fmt_f64(-1234567.0, 0, true, 0, "", " ", false).as_deref(), Ok("-1,234,567"));
        assert_eq!(crow_fmt_f64(42.0, -1, false, 0, "", " ", false).as_deref(), Ok("42"));
    }

    #[test]
    fn test_fmt_percent_width_fill() {
        assert_eq!(crow_fmt_f64(0.123, 1, false, 0, "", " ", true).as_deref(), Ok("12.3%"));
        assert_eq!(crow_fmt_f64(42.0, -1, false, 8, "", " ", false).as_deref(), Ok("      42"));
        assert_eq!(crow_fmt_f64(42.0, -1, false, 5, "", "0", false).as_deref(), Ok("00042"));
        assert_eq!(crow_fmt_str("alice", -1, 8, "", " ", false).as_deref(), Ok("alice   "));
        assert_eq!(crow_fmt_str("alice", -1, 8, "right", " ", false).as_deref(), Ok("   alice"));
    }

    #[test]
    fn test_map_display() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("age".to_string(), 3i64);
        m.insert("count".to_string(), 7i64);
        assert_eq!(m.crow_fmt(), "[age = 3, count = 7]");
    }
}
