use std::collections::BTreeMap;
use std::fmt;

use crow_runtime::frame::CrowFrame;
use crow_runtime::tensor::CrowTensor;
use crow_runtime::sparse::CrowSparse;
use crow_db::DbHandle;

/// Every value in Crow carries error metadata: ok, reason, kind, trace.
/// Values auto-unwrap — you use them directly.
#[derive(Debug, Clone)]
pub struct CrowValue {
    pub value: Value,              // the actual value
    pub ok: bool,                  // true for normal values, false for errors
    pub reason: String,            // error message ("" if not error)
    pub kind: String,              // error category ("" if not error)
    pub trace: Vec<String>,        // call trace, built during propagation
    /// When this value was captured from the outside world (epoch seconds),
    /// or None for values computed here and now. Flows like error metadata:
    /// a derived value is as old as its oldest input (see age[]).
    pub captured: Option<f64>,
    /// Markup a view, an element or `doc` made: the one kind of string a
    /// view places as markup. Every other string is text, whatever it holds.
    pub markup: bool,
}

impl CrowValue {
    pub fn ok(value: Value) -> Self {
        CrowValue {
            value,
            ok: true,
            reason: String::new(),
            kind: String::new(),
            trace: Vec::new(),
            captured: None,
            markup: false,
        }
    }

    pub fn done() -> Self {
        CrowValue {
            value: Value::Done,
            ok: true,
            reason: String::new(),
            kind: String::new(),
            trace: Vec::new(),
            captured: None,
            markup: false,
        }
    }

    pub fn error(reason: &str, trace_entry: &str) -> Self {
        CrowValue {
            value: Value::Done,
            ok: false,
            reason: reason.to_string(),
            kind: String::new(),
            trace: vec![trace_entry.to_string()],
            captured: None,
            markup: false,
        }
    }

    pub fn error_with_kind(reason: &str, kind: &str, trace_entry: &str) -> Self {
        // An empty trace entry means "no location" — module builtins have
        // none, and compiled mode emits an empty trace for them too.
        let trace = if trace_entry.is_empty() { vec![] } else { vec![trace_entry.to_string()] };
        CrowValue {
            value: Value::Done,
            ok: false,
            reason: reason.to_string(),
            kind: kind.to_string(),
            trace,
            captured: None,
            markup: false,
        }
    }

    /// Markup a view, an element or `doc` made — placed as markup when it
    /// is a child of an element.
    pub fn markup(html: String) -> Self {
        let mut v = CrowValue::ok(Value::Str(html));
        v.markup = true;
        v
    }

    #[inline(always)]
    pub fn is_error(&self) -> bool {
        !self.ok
    }

    #[inline(always)]
    #[allow(dead_code)]
    pub fn is_ok(&self) -> bool {
        self.ok
    }

    /// Auto-unwrap: get the inner value, propagating errors
    pub fn unwrap_or_propagate(&self) -> Result<&Value, String> {
        if !self.ok {
            Err(if self.reason.is_empty() { "unknown error".to_string() } else { self.reason.clone() })
        } else {
            Ok(&self.value)
        }
    }

    /// Append a trace entry during auto-propagation: a place the trace
    /// already ends at is not named twice (a refusal on the line the error
    /// came from)
    pub fn append_trace(&mut self, location: &str) {
        if self.trace.last().map(String::as_str) != Some(location) {
            self.trace.push(location.to_string());
        }
    }

    /// Stamp this value as captured from the outside world now.
    pub fn stamp_captured(mut self) -> Self {
        self.captured = Some(crow_runtime::moments::now());
        self
    }

}

/// The actual runtime values
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Array(Vec<CrowValue>),
    NumArray(CrowTensor),
    Map(BTreeMap<String, CrowValue>),
    Fn(FnValue),
    StructInstance(StructInstance),
    Sparse(CrowSparse), // sparse matrix (CSR format)
    Frame(CrowFrame), // column-oriented data frame
    DbConn(DbHandle), // database connection handle
    /// A mailer `smtp[…]` made: its place among the interpreter's mailers
    /// and its host. The password stays with the mailer, out of every value.
    Mailer(usize, String),
    Dual(f64, f64), // dual number for autograd: (real, derivative)
    App(FnValue, f64, bool, u64),   // headless app session for test blocks (the view, its viewport width, the wide look, which session)
    GradFn(FnValue), // gradient function wrapping a user function
    Done, // represents no value (from 'done' status)
}

/// A function as a value: its name and parameters; the walker holds the
/// body under the name.
#[derive(Debug, Clone)]
pub struct FnValue {
    pub name: String,
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

    /// Return the Crow type name for this value
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::App(..) => "app",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Bool(_) => "bool",
            Value::Array(_) => "array",
            Value::NumArray(_) => "array",
            Value::Map(_) => "map",
            Value::Fn(_) => "fn",
            Value::StructInstance(_) => "struct_instance",
            Value::Sparse(_) => "sparse",
            Value::Frame(_) => "frame",
            Value::DbConn(_) => "db",
            Value::Mailer(..) => "mailer",
            Value::Dual(_, _) => "float", // Dual appears as float to user
            Value::GradFn(_) => "fn",
            Value::Done => "done",
        }
    }

}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::App(a, ..) => write!(f, "app[{}]", a.name),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    item.fmt_item(f)?;
                }
                write!(f, "]")
            }
            // Delegate to the shared display kernel (crow_runtime::display) so
            // interpreter and compiled output are identical, including nesting
            Value::NumArray(tensor) => write!(f, "{}", tensor),
            Value::Map(map) => {
                write!(f, "[")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = ", k)?;
                    v.fmt_item(f)?;
                }
                write!(f, "]")
            }
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::StructInstance(si) => {
                write!(f, "{}[", si.name)?;
                for (i, (k, v)) in si.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = ", k)?;
                    v.fmt_item(f)?;
                }
                write!(f, "]")
            }
            Value::Sparse(s) => write!(f, "{}", s),
            Value::Frame(frame) => write!(f, "{}", frame),
            Value::DbConn(h) => write!(f, "<db:{}>", h.backend_name()),
            Value::Mailer(_, host) => write!(f, "<mailer:{}>", host),
            Value::GradFn(func) => write!(f, "<grad {}>", func.name),
            Value::Dual(real, _) => {
                // Display as float (user never sees the derivative)
                if *real == (*real as i64) as f64 && real.is_finite() {
                    write!(f, "{}", *real as i64)
                } else {
                    write!(f, "{}", real)
                }
            }
            Value::Done => write!(f, "done"),
        }
    }
}

impl CrowValue {
    /// An item of a list, a map or a struct as the container shows it: a
    /// string quoted, an error by its reason
    fn fmt_item(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ok {
            self.value.fmt_repr(f)
        } else if self.kind == "missing" && self.reason == "nan" {
            // a frame column's missing cell, as the display kernel shows it
            write!(f, "nan")
        } else {
            write!(f, "error: {}", self.reason)
        }
    }
}

impl fmt::Display for CrowValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ok {
            write!(f, "{}", self.value)
        } else {
            write!(f, "error: {}", self.reason)
        }
    }
}

