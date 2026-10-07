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

/// `has[j, key]` of a parsed JSON object.
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
impl<T: ToJson> ToJson for std::collections::BTreeMap<String, T> {
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(self.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
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
