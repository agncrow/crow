//! One scalar a program hands a builtin, read one way in both modes
//! (docs/bug-classes.md §4). The interpreter reads it from its value; the
//! compiled program builds it from its typed value by `Cell::from`, so a
//! number, a text, a bool or a parsed JSON value means one thing to
//! `fmt`, `int`, `float`, `fill` and `with` whichever mode runs them.

/// A number, a text, a bool — or, from a JSON value, `null` or a list or
/// an object, which no scalar builtin takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    /// JSON's `null`.
    Null,
    /// A JSON list or object: what it is, as a refusal names it.
    Other(&'static str),
}

impl From<i64> for Cell {
    fn from(n: i64) -> Self {
        Cell::Int(n)
    }
}
impl From<f64> for Cell {
    fn from(x: f64) -> Self {
        Cell::Float(x)
    }
}
impl From<bool> for Cell {
    fn from(b: bool) -> Self {
        Cell::Bool(b)
    }
}
impl From<String> for Cell {
    fn from(s: String) -> Self {
        Cell::Text(s)
    }
}
impl From<&str> for Cell {
    fn from(s: &str) -> Self {
        Cell::Text(s.to_string())
    }
}
impl From<serde_json::Value> for Cell {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Cell::Int(i),
                None => Cell::Float(n.as_f64().unwrap_or(f64::NAN)),
            },
            serde_json::Value::String(s) => Cell::Text(s),
            serde_json::Value::Bool(b) => Cell::Bool(b),
            serde_json::Value::Null => Cell::Null,
            serde_json::Value::Array(_) => Cell::Other("a list"),
            serde_json::Value::Object(_) => Cell::Other("a map"),
        }
    }
}

impl Cell {
    /// What the cell is, as a refusal names it.
    pub fn shown(&self) -> String {
        match self {
            Cell::Int(n) => n.to_string(),
            Cell::Float(x) => x.to_string(),
            Cell::Bool(b) => b.to_string(),
            Cell::Text(s) => format!("'{}'", s),
            Cell::Null => "null".to_string(),
            Cell::Other(what) => what.to_string(),
        }
    }

    /// The kind of value it is: `a bool`, `null`.
    pub fn kind(&self) -> &'static str {
        match self {
            Cell::Int(_) => "an int",
            Cell::Float(_) => "a number",
            Cell::Bool(_) => "a bool",
            Cell::Text(_) => "text",
            Cell::Null => "null",
            Cell::Other(what) => what,
        }
    }
}

/// `int[x]` of text or a JSON value: the int it names, or the reason it
/// names none (an error value of kind `'parse'`). A number is cut toward
/// zero and a bool is 1 or 0, as `int` of either is.
pub fn int_of(c: &Cell) -> Result<i64, String> {
    match c {
        Cell::Int(n) => Ok(*n),
        Cell::Float(x) => Ok(*x as i64),
        Cell::Bool(b) => Ok(i64::from(*b)),
        Cell::Text(s) => s.parse::<i64>().map_err(|_| format!("cannot convert '{}' to int", s)),
        other => Err(format!("cannot convert {} to int", other.shown())),
    }
}

/// `float[x]` of text or a JSON value, as `int_of` reads it.
pub fn float_of(c: &Cell) -> Result<f64, String> {
    match c {
        Cell::Int(n) => Ok(*n as f64),
        Cell::Float(x) => Ok(*x),
        Cell::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Cell::Text(s) => s.parse::<f64>().map_err(|_| format!("cannot convert '{}' to float", s)),
        other => Err(format!("cannot convert {} to float", other.shown())),
    }
}

/// `int[f]` of a float: cut toward zero, the largest int past its range.
pub fn int_of_float(x: f64) -> i64 {
    x as i64
}

/// `int[b]` of a bool: 1 or 0.
pub fn int_of_bool(b: bool) -> i64 {
    i64::from(b)
}

/// `int[n]` of an int: itself.
pub fn int_of_int(n: i64) -> i64 {
    n
}

/// `float[n]` of an int.
pub fn float_of_int(n: i64) -> f64 {
    n as f64
}

/// `float[x]` of a float: itself.
pub fn float_of_float(x: f64) -> f64 {
    x
}

/// `float[b]` of a bool: 1.0 or 0.0.
pub fn float_of_bool(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// A list's items as the text `join` writes: a number as it displays (a
/// whole float as an int), a bool as `true` or `false`.
pub trait Words {
    fn words(&self) -> Vec<String>;
}

impl Words for Vec<String> {
    fn words(&self) -> Vec<String> {
        self.clone()
    }
}
impl Words for Vec<i64> {
    fn words(&self) -> Vec<String> {
        self.iter().map(|n| n.to_string()).collect()
    }
}
impl Words for Vec<f64> {
    fn words(&self) -> Vec<String> {
        self.iter().map(|x| x.to_string()).collect()
    }
}
impl Words for Vec<bool> {
    fn words(&self) -> Vec<String> {
        self.iter().map(|b| b.to_string()).collect()
    }
}
impl Words for crate::tensor::CrowTensor {
    fn words(&self) -> Vec<String> {
        self.to_vec().iter().map(|x| x.to_string()).collect()
    }
}

/// The words of a list, for the compiled program's `join`.
pub fn words<T: Words + ?Sized>(items: &T) -> Vec<String> {
    items.words()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One reading of a scalar: text is not trimmed, a JSON bool is 1 or 0
    /// (the compiled program trimmed JSON text and refused a JSON bool).
    #[test]
    fn a_cell_converts_one_way() {
        assert_eq!(int_of(&Cell::Text(" 12 ".to_string())), Err("cannot convert ' 12 ' to int".to_string()));
        assert_eq!(int_of(&Cell::from(serde_json::json!(true))), Ok(1));
        assert_eq!(int_of(&Cell::from(serde_json::json!(2.9))), Ok(2));
        assert_eq!(float_of(&Cell::from(serde_json::json!("2.5"))), Ok(2.5));
        assert_eq!(int_of(&Cell::from(serde_json::Value::Null)), Err("cannot convert null to int".to_string()));
        assert_eq!(words(&vec![2.0, 2.5]), vec!["2".to_string(), "2.5".to_string()]);
    }
}
