//! What both modes agree a parsed JSON value is, where the compiled
//! program keeps it as `serde_json::Value` and the interpreter has
//! already turned it into arrays, maps and strings.

/// `len[]` of a parsed JSON value: items of an array, keys of an object,
/// characters of a string; anything else has no length and counts 0, as
/// the interpreter's value of the same text would.
pub fn len(v: &serde_json::Value) -> i64 {
    match v {
        serde_json::Value::Array(a) => a.len() as i64,
        serde_json::Value::Object(o) => o.len() as i64,
        serde_json::Value::String(s) => s.chars().count() as i64,
        _ => 0,
    }
}

/// A JSON value, as a generated program names it without naming serde.
pub type Json = serde_json::Value;

/// JSON's `null`: what a value that failed encodes as.
pub fn null() -> Json {
    serde_json::Value::Null
}

/// A record's JSON: an object of its fields.
pub fn object(fields: Vec<(&str, Json)>) -> Json {
    serde_json::Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `json[text]`: the value the text is. A top-level `null` is an error of
/// reason `null` (spec, json[]); text that is no JSON is the parser's
/// reason.
pub fn parse(s: &str) -> Result<serde_json::Value, String> {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(serde_json::Value::Null) => Err("null".to_string()),
        Ok(v) => Ok(v),
        Err(e) => Err(format!("invalid JSON: {}", e)),
    }
}

/// What a loop walks of a parsed JSON value, as the interpreter walks the
/// value it parsed to: a list's items — a list of numbers' numbers, whole
/// ones as ints — each still a parsed value; anything else is no list.
/// (Compiled, a loop over a parsed value did not build.)
pub fn items(v: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    use serde_json::Value;
    match v {
        Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.is_number()) => Ok(items
            .iter()
            .map(|n| {
                let f = n.as_f64().unwrap_or(0.0);
                if f == (f as i64) as f64 && f.is_finite() { serde_json::json!(f as i64) } else { float_to_json(f) }
            })
            .collect()),
        Value::Array(items) => Ok(items.clone()),
        _ => Err("for loop requires an array".to_string()),
    }
}

/// `type[j]` of a parsed JSON value: the word the interpreter's value of it
/// says (a list of numbers is an array there too, `null` an error).
pub fn type_word(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Object(_) => "map",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::String(_) => "str",
        serde_json::Value::Number(n) if n.is_i64() => "int",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Null => "error",
    }
}

/// `encode[x]`: a value's JSON text, by the one rule every type follows.
pub fn encode<T: ToJson + ?Sized>(x: &T) -> String {
    x.to_json().to_string()
}

/// The refusal a JSON value that is no object is given, in the words the
/// interpreter gives the value it parsed to (a list of numbers is a tensor
/// there): the table's, naming the forms (`form`), or — for a list whose
/// kind no item says (an empty one, one starting with `null`) — the words
/// of reading one as a map.
fn not_a_map(name: &str, form: &str, rest: &str, v: &serde_json::Value) -> String {
    use serde_json::Value;
    let got = match v {
        Value::Number(n) if n.is_i64() => "an int",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Bool(_) => "a bool",
        Value::Object(_) => "a map",
        Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.is_number()) => "numbers",
        Value::Array(items) => match items.first() {
            Some(Value::Number(n)) if n.is_i64() => "a list of ints",
            Some(Value::Number(_)) => "a list of numbers",
            Some(Value::String(_)) => "a list of text",
            Some(Value::Bool(_)) => "a list of bools",
            Some(Value::Object(_)) => "a list of maps",
            Some(Value::Array(inner)) => match inner.first() {
                Some(Value::Number(_)) => "a list of tensors",
                Some(Value::Null) | None => return "expected a map, got array".to_string(),
                Some(_) => "a list",
            },
            _ => return "expected a map, got array".to_string(),
        },
        Value::Null => "null",
    };
    format!("{}[] takes {}, got {}{}", name, form, got, rest)
}

/// `keys[j]` of a parsed JSON object, in order.
pub fn keys(j: &serde_json::Value) -> Result<Vec<String>, String> {
    match j {
        serde_json::Value::Object(m) => Ok(m.keys().cloned().collect()),
        other => Err(not_a_map("keys", "a map", "", other)),
    }
}

/// `values[j]` of a parsed JSON object, in its keys' order.
pub fn values(j: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    match j {
        serde_json::Value::Object(m) => Ok(m.values().cloned().collect()),
        other => Err(not_a_map("values", "a map", "", other)),
    }
}

/// `key in j` of a parsed JSON object.
pub fn has(j: &serde_json::Value, key: &str) -> Result<bool, String> {
    match j {
        serde_json::Value::Object(m) => Ok(m.contains_key(key)),
        other => Err(not_a_map("has", "a map and text", " and text", other)),
    }
}

/// How a float encodes: JSON has no NaN or infinity, so those encode as
/// `null`, as serde writes them; anything else is the number.
pub fn float_to_json(f: f64) -> serde_json::Value {
    if f.is_finite() { serde_json::json!(f) } else { serde_json::Value::Null }
}

/// How a tensor encodes: a list, whole numbers as ints (a tensor holds
/// floats, but `[1, 2, 3]` must come back as `[1,2,3]`), the rest as
/// floats, non-finite cells as `null`.
pub fn tensor_to_json(t: &crate::tensor::CrowTensor) -> serde_json::Value {
    if t.ndim() >= 2 {
        // a matrix is a list of its rows
        return serde_json::Value::Array(t.rows().iter().map(tensor_to_json).collect());
    }
    serde_json::Value::Array(
        t.to_vec()
            .iter()
            .map(|&f| if f == (f as i64) as f64 && f.is_finite() { serde_json::json!(f as i64) } else { float_to_json(f) })
            .collect(),
    )
}

/// What `encode[]` writes for a typed value: one rule for every type the
/// compiled program keeps natively, so a tensor inside a map is a list
/// there too, never its internal form.
pub trait ToJson {
    fn to_json(&self) -> serde_json::Value;
}

impl ToJson for i64 {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(*self) }
}
impl ToJson for f64 {
    fn to_json(&self) -> serde_json::Value { float_to_json(*self) }
}
impl ToJson for crate::moments::Moment {
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::String(self.to_string())
    }
}
impl ToJson for bool {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(*self) }
}
impl ToJson for String {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(self) }
}
impl ToJson for str {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(self) }
}
/// A parsed value encodes as the interpreter's value of it does: a list of
/// numbers is a tensor there, whose whole numbers encode as ints (the
/// compiled program had written `[1,2.0]` back where the interpreter wrote
/// `[1,2]`).
impl ToJson for serde_json::Value {
    fn to_json(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.is_number()) => Value::Array(
                items
                    .iter()
                    .map(|n| {
                        let f = n.as_f64().unwrap_or(0.0);
                        if f == (f as i64) as f64 && f.is_finite() { serde_json::json!(f as i64) } else { float_to_json(f) }
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|i| i.to_json()).collect()),
            Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
            other => other.clone(),
        }
    }
}
impl ToJson for crate::tensor::CrowTensor {
    fn to_json(&self) -> serde_json::Value { tensor_to_json(self) }
}
impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> serde_json::Value { serde_json::Value::Array(self.iter().map(|v| v.to_json()).collect()) }
}
/// A recursive link: the record it holds, or the empty sentinel `0`.
impl<T: ToJson> ToJson for Option<Box<T>> {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Some(r) => r.to_json(),
            None => serde_json::json!(0),
        }
    }
}
// (a map keyed by ints encodes its keys as text: JSON's objects have no
// other kind; the spec, Maps)
impl<K: std::fmt::Display, T: ToJson> ToJson for std::collections::BTreeMap<K, T> {
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(self.iter().map(|(k, v)| (k.to_string(), v.to_json())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_and_tensors_encode_whole_numbers_bare_and_non_finite_as_null() {
        assert_eq!(float_to_json(2.5).to_string(), "2.5");
        assert_eq!(float_to_json(f64::NAN).to_string(), "null");
        assert_eq!(float_to_json(f64::INFINITY).to_string(), "null");
        let t = crate::tensor::CrowTensor::from_vec(vec![1.0, 2.5, f64::NAN], vec![3]);
        assert_eq!(tensor_to_json(&t).to_string(), "[1,2.5,null]");
        let m = crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        assert_eq!(tensor_to_json(&m).to_string(), "[[1,2],[3,4]]");
        let mut map = std::collections::BTreeMap::new();
        map.insert("xs".to_string(), crate::tensor::CrowTensor::from_vec(vec![1.0, 2.0], vec![2]));
        assert_eq!(map.to_json().to_string(), "{\"xs\":[1,2]}");
        assert_eq!(vec!["a".to_string()].to_json().to_string(), "[\"a\"]");
    }

    #[test]
    fn len_counts_items_keys_and_characters() {
        assert_eq!(len(&serde_json::json!([1, 2, 3])), 3);
        assert_eq!(len(&serde_json::json!({"a": 1, "b": 2})), 2);
        assert_eq!(len(&serde_json::json!("héllo")), 5);
        assert_eq!(len(&serde_json::json!(7)), 0);
        assert_eq!(len(&serde_json::Value::Null), 0);
    }
}

/// What a parsed value holds, as the stop that reads it as another kind
/// names it.
fn holds(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => format!("text '{}'", s),
        serde_json::Value::Number(n) => format!("a number {}", n),
        serde_json::Value::Bool(b) => format!("a bool {}", b),
        serde_json::Value::Array(_) => "a list".to_string(),
        serde_json::Value::Object(_) => "a map".to_string(),
        serde_json::Value::Null => "nothing".to_string(),
    }
}

/// The stop's words: what the value holds, what the program reads, and
/// the read that asked (`a position in xs at line 3`).
fn not_read(v: &serde_json::Value, what: &str, how: &str) -> String {
    let asked = if how.is_empty() { String::new() } else { format!(" ({})", how) };
    format!("a parsed value holds {} where the program reads {}{}", holds(v), what, asked)
}

/// A parsed value read as an int, where its use takes one (a position, a
/// row that takes an int): a whole number, a number with nothing after
/// the point, or text that reads as one; anything else stops with what it
/// holds and the read that asked. (`__read_int`; spec, JSON.)
pub fn read_int(v: &serde_json::Value, how: &str) -> Result<i64, String> {
    if let Some(n) = v.as_i64() {
        return Ok(n);
    }
    if let Some(x) = v.as_f64() {
        if x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0 {
            return Ok(x as i64);
        }
    }
    if let Some(n) = v.as_str().and_then(|s| s.trim().parse::<i64>().ok()) {
        return Ok(n);
    }
    Err(not_read(v, "an int", how))
}

/// A parsed value read as a number, where its use meets one (arithmetic,
/// an order, a variable holding numbers): a number, or text that reads
/// as one; else the stop. (`__read_float`.)
pub fn read_float(v: &serde_json::Value, how: &str) -> Result<f64, String> {
    if let Some(x) = v.as_f64() {
        return Ok(x);
    }
    if let Some(x) = v.as_str().and_then(|s| s.trim().parse::<f64>().ok()) {
        return Ok(x);
    }
    Err(not_read(v, "a number", how))
}

/// A parsed value read as text, where its use takes text: text itself, a
/// number or a bool as `str[…]` shows it; a list or a map stops.
/// (`__read_str`.)
pub fn read_str(v: &serde_json::Value, how: &str) -> Result<String, String> {
    match v {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        _ => Err(not_read(v, "text", how)),
    }
}

/// A parsed value read as a bool, where the program decides on it,
/// negates it or joins it with and/or: a bool, nothing else.
/// (`__read_bool`.)
pub fn read_bool(v: &serde_json::Value, how: &str) -> Result<bool, String> {
    match v {
        serde_json::Value::Bool(b) => Ok(*b),
        _ => Err(not_read(v, "a bool", how)),
    }
}

/// A parsed value read as a moment, where it meets one: ISO 8601 text, or
/// a number of seconds; else the stop. (`__read_moment`.)
pub fn read_moment(v: &serde_json::Value, how: &str) -> Result<crate::moments::Moment, String> {
    match v {
        serde_json::Value::String(s) => crate::moments::Moment::parse(s).map_err(|_| not_read(v, "a moment", how)),
        serde_json::Value::Number(n) => n.as_f64().map(crate::moments::Moment::of_seconds).ok_or_else(|| not_read(v, "a moment", how)),
        _ => Err(not_read(v, "a moment", how)),
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    /// Each read takes what its kind can hold and stops on the rest, naming
    /// what the value holds and the read that asked.
    #[test]
    fn a_parsed_value_is_read_as_its_use_implies_or_stops_naming_both() {
        let j: serde_json::Value = serde_json::from_str(r#"{"i": 1, "f": 2.0, "x": 2.5, "s": "7", "k": "b", "flag": true, "t": "2026-07-14T09:30:00Z", "l": [1]}"#).unwrap_or(serde_json::Value::Null);
        assert_eq!(read_int(&j["i"], "a position in xs at line 3"), Ok(1));
        assert_eq!(read_int(&j["f"], ""), Ok(2));
        assert_eq!(read_int(&j["s"], ""), Ok(7));
        assert_eq!(read_int(&j["x"], "a position in xs at line 3"), Err("a parsed value holds a number 2.5 where the program reads an int (a position in xs at line 3)".to_string()));
        assert_eq!(read_float(&j["i"], ""), Ok(1.0));
        assert_eq!(read_float(&j["s"], ""), Ok(7.0));
        assert_eq!(read_float(&j["k"], "met in add at line 4"), Err("a parsed value holds text 'b' where the program reads a number (met in add at line 4)".to_string()));
        assert_eq!(read_str(&j["k"], ""), Ok("b".to_string()));
        assert_eq!(read_str(&j["i"], ""), Ok("1".to_string()));
        assert_eq!(read_str(&j["l"], "handed to upper at line 5"), Err("a parsed value holds a list where the program reads text (handed to upper at line 5)".to_string()));
        assert_eq!(read_bool(&j["flag"], ""), Ok(true));
        assert_eq!(read_bool(&j["i"], "decided on at line 6"), Err("a parsed value holds a number 1 where the program reads a bool (decided on at line 6)".to_string()));
        assert!(read_moment(&j["t"], "").is_ok());
        assert_eq!(read_moment(&j["k"], "met in eq at line 7"), Err("a parsed value holds text 'b' where the program reads a moment (met in eq at line 7)".to_string()));
    }
}
