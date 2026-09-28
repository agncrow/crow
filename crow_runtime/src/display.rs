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
}

impl CrowDisplay for &str {
    fn crow_repr(&self) -> String {
        format!("'{}'", self)
    }
    fn crow_fmt(&self) -> String {
        self.to_string()
    }
}

impl<T: CrowDisplay> CrowDisplay for Vec<T> {
    fn crow_repr(&self) -> String {
        let parts: Vec<String> = self.iter().map(|v| v.crow_repr()).collect();
        format!("[{}]", parts.join(", "))
    }
}

/// A frame column prints like the interpreter's array of its values:
/// numbers bare, strings quoted, missing cells as nan.
impl CrowDisplay for crate::frame::CrowColumn {
    fn crow_repr(&self) -> String {
        use crate::frame::Column;
        let n = self.len();
        let parts: Vec<String> = (0..n)
            .map(|i| {
                if !self.valid[i] {
                    return "nan".to_string();
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

impl<T: CrowDisplay> CrowDisplay for std::collections::BTreeMap<String, T> {
    fn crow_repr(&self) -> String {
        let parts: Vec<String> = self
            .iter()
            .map(|(k, v)| format!("{} = {}", k, v.crow_repr()))
            .collect();
        format!("[{}]", parts.join(", "))
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
