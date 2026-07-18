//! Shared display formatting for Crow values.
//!
//! This is the single source of truth for how values print. The interpreter's
//! `Value::Display` and the transpiler's `log[]` codegen must both follow these
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

impl<T: CrowDisplay> CrowDisplay for std::collections::BTreeMap<String, T> {
    fn crow_repr(&self) -> String {
        let parts: Vec<String> = self
            .iter()
            .map(|(k, v)| format!("{} = {}", k, v.crow_repr()))
            .collect();
        format!("[{}]", parts.join(", "))
    }
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
    fn test_map_display() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("age".to_string(), 3i64);
        m.insert("count".to_string(), 7i64);
        assert_eq!(m.crow_fmt(), "[age = 3, count = 7]");
    }
}
