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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_counts_items_keys_and_characters() {
        assert_eq!(len(&serde_json::json!([1, 2, 3])), 3);
        assert_eq!(len(&serde_json::json!({"a": 1, "b": 2})), 2);
        assert_eq!(len(&serde_json::json!("héllo")), 5);
        assert_eq!(len(&serde_json::json!(7)), 0);
        assert_eq!(len(&serde_json::Value::Null), 0);
    }
}
