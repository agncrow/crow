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
impl ToJson for bool {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(*self) }
}
impl ToJson for String {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(self) }
}
impl ToJson for str {
    fn to_json(&self) -> serde_json::Value { serde_json::json!(self) }
}
impl ToJson for serde_json::Value {
    fn to_json(&self) -> serde_json::Value { self.clone() }
}
impl ToJson for crate::tensor::CrowTensor {
    fn to_json(&self) -> serde_json::Value { tensor_to_json(self) }
}
impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> serde_json::Value { serde_json::Value::Array(self.iter().map(|v| v.to_json()).collect()) }
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
