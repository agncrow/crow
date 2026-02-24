use std::collections::BTreeMap;
use std::fmt;

/// Every value in Crow is a struct with status, result, reason.
/// Values auto-unwrap — you use them directly.
#[derive(Debug, Clone)]
pub struct CrowValue {
    pub status: String,   // "ok", "done", or "error"
    pub result: Value,    // the actual value
    pub reason: String,   // error message (empty if not error)
}

impl CrowValue {
    pub fn ok(value: Value) -> Self {
        CrowValue {
            status: "ok".to_string(),
            result: value,
            reason: String::new(),
        }
    }

    pub fn done() -> Self {
        CrowValue {
            status: "done".to_string(),
            result: Value::Done,
            reason: String::new(),
        }
    }

    pub fn error(reason: &str) -> Self {
        CrowValue {
            status: "error".to_string(),
            result: Value::Done,
            reason: reason.to_string(),
        }
    }

    /// Auto-unwrap: get the inner value, propagating errors
    pub fn unwrap_or_propagate(&self) -> Result<&Value, String> {
        if self.status == "error" {
            Err(self.reason.clone())
        } else {
            Ok(&self.result)
        }
    }
}

/// The actual runtime values
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    List(Vec<CrowValue>),
    Map(BTreeMap<String, CrowValue>),
    Fn(FnValue),
    StructDef(StructDef),
    StructInstance(StructInstance),
    Done, // represents no value (from 'done' status)
}

#[derive(Debug, Clone)]
pub struct FnValue {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<crate::ast::Stmt>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StructInstance {
    pub name: String,
    pub fields: BTreeMap<String, CrowValue>,
}

impl Value {
    /// Format a value with quotes around strings (for display inside containers)
    fn fmt_repr(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "'{}'", s),
            other => write!(f, "{}", other),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    item.result.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::Map(map) => {
                write!(f, "[")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "'{}' = ", k)?;
                    v.result.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::StructDef(sd) => write!(f, "<struct {}>", sd.name),
            Value::StructInstance(si) => {
                write!(f, "{}[", si.name)?;
                for (i, (k, v)) in si.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = ", k)?;
                    v.result.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::Done => write!(f, "done"),
        }
    }
}

impl fmt::Display for CrowValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.result)
    }
}

