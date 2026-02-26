use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Status of a CrowValue — avoids heap-allocating a String for every value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Done,
    Error,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Ok => write!(f, "ok"),
            Status::Done => write!(f, "done"),
            Status::Error => write!(f, "error"),
        }
    }
}

/// Every value in Crow is a struct with status, result, reason, line.
/// Values auto-unwrap — you use them directly.
#[derive(Debug, Clone)]
pub struct CrowValue {
    pub status: Status,
    pub result: Value,    // the actual value
    pub reason: Option<String>,   // error message (None if not error)
    pub line: usize,              // line number where error originated (0 = not an error)
}

impl CrowValue {
    pub fn ok(value: Value) -> Self {
        CrowValue {
            status: Status::Ok,
            result: value,
            reason: None,
            line: 0,
        }
    }

    pub fn done() -> Self {
        CrowValue {
            status: Status::Done,
            result: Value::Done,
            reason: None,
            line: 0,
        }
    }

    pub fn error(reason: &str, line: usize) -> Self {
        CrowValue {
            status: Status::Error,
            result: Value::Done,
            reason: Some(reason.to_string()),
            line,
        }
    }

    #[inline(always)]
    pub fn is_error(&self) -> bool {
        self.status == Status::Error
    }

    #[inline(always)]
    pub fn is_ok(&self) -> bool {
        self.status == Status::Ok
    }

    /// Auto-unwrap: get the inner value, propagating errors
    pub fn unwrap_or_propagate(&self) -> Result<&Value, String> {
        if self.status == Status::Error {
            Err(self.reason.as_deref().unwrap_or("unknown error").to_string())
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
    Module(ModuleValue),
    Done, // represents no value (from 'done' status)
}

#[derive(Debug, Clone)]
pub struct FnValue {
    pub name: String,
    pub params: Vec<String>,
    pub body: Arc<Vec<crate::ast::Stmt>>,
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

#[derive(Debug, Clone)]
pub struct ModuleValue {
    pub name: String,
    pub bindings: BTreeMap<String, CrowValue>,
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
            Value::Module(m) => write!(f, "<module {}>", m.name),
            Value::Done => write!(f, "done"),
        }
    }
}

impl fmt::Display for CrowValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.result)
    }
}

