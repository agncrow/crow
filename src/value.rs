use std::collections::BTreeMap;
use std::fmt;

use crow_runtime::frame::CrowFrame;
use crow_runtime::tensor::CrowTensor;
use crow_runtime::sparse::CrowSparse;
use crow_db::DbHandle;

/// A value, and what it carries besides itself: whether it failed (with
/// its reason, kind and trace), when it was captured from the outside
/// world, and whether it is markup. Most values carry none of that, so it
/// lives apart, absent on a plain value (typed-values-design.md §2): a
/// value is as big as what it holds, not as its failure could be.
#[derive(Debug, Clone)]
pub struct CrowValue {
    pub value: Value,
    status: Option<Box<Status>>,
}

/// What a value carries besides itself (`CrowValue`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// An error value: `ok` is false.
    pub failed: bool,
    /// Its message ("" if not an error).
    pub reason: String,
    /// Its category ("" if not an error).
    pub kind: String,
    /// Where it passed, built during propagation.
    pub trace: Vec<String>,
    /// When it was captured from the outside world (epoch seconds), or
    /// None for values computed here and now. Flows like error metadata: a
    /// derived value is as old as its oldest input (see age[]).
    pub captured: Option<f64>,
    /// Markup a view, an element or `doc` made: the one kind of string a
    /// view places as markup. Every other string is text, whatever it holds.
    pub markup: bool,
}

impl CrowValue {
    pub fn ok(value: Value) -> Self {
        CrowValue { value, status: None }
    }

    pub fn done() -> Self {
        CrowValue { value: Value::Done, status: None }
    }

    pub fn error(reason: &str, trace_entry: &str) -> Self {
        CrowValue {
            value: Value::Done,
            status: Some(Box::new(Status { failed: true, reason: reason.to_string(), trace: vec![trace_entry.to_string()], ..Status::default() })),
        }
    }

    pub fn error_with_kind(reason: &str, kind: &str, trace_entry: &str) -> Self {
        // An empty trace entry means "no location" — module builtins have
        // none, and compiled mode emits an empty trace for them too.
        let trace = if trace_entry.is_empty() { vec![] } else { vec![trace_entry.to_string()] };
        CrowValue {
            value: Value::Done,
            status: Some(Box::new(Status { failed: true, reason: reason.to_string(), kind: kind.to_string(), trace, ..Status::default() })),
        }
    }

    /// Markup a view, an element or `doc` made — placed as markup when it
    /// is a child of an element.
    pub fn markup(html: String) -> Self {
        let mut v = CrowValue::ok(Value::Str(html));
        v.set_markup(true);
        v
    }

    #[inline(always)]
    pub fn is_error(&self) -> bool {
        self.status.as_ref().map_or(false, |s| s.failed)
    }

    #[inline(always)]
    pub fn is_ok(&self) -> bool {
        !self.is_error()
    }

    /// Its message, "" if it is no error.
    pub fn reason(&self) -> &str {
        self.status.as_ref().map_or("", |s| s.reason.as_str())
    }

    /// Its category, "" if it is no error.
    pub fn kind(&self) -> &str {
        self.status.as_ref().map_or("", |s| s.kind.as_str())
    }

    /// Where it passed.
    pub fn trace(&self) -> &[String] {
        self.status.as_ref().map_or(&[], |s| s.trace.as_slice())
    }

    /// When it was captured from the outside world.
    pub fn captured(&self) -> Option<f64> {
        self.status.as_ref().and_then(|s| s.captured)
    }

    /// Whether it is markup.
    pub fn is_markup(&self) -> bool {
        self.status.as_ref().map_or(false, |s| s.markup)
    }

    /// What it carries besides itself, made room for when it has none.
    pub fn status_mut(&mut self) -> &mut Status {
        self.status.get_or_insert_with(Default::default)
    }

    pub fn set_captured(&mut self, at: Option<f64>) {
        if at.is_some() || self.status.is_some() {
            self.status_mut().captured = at;
        }
    }

    pub fn set_markup(&mut self, markup: bool) {
        if markup || self.status.is_some() {
            self.status_mut().markup = markup;
        }
    }

    /// Auto-unwrap: get the inner value, propagating errors
    pub fn unwrap_or_propagate(&self) -> Result<&Value, String> {
        if self.is_error() {
            Err(if self.reason().is_empty() { "unknown error".to_string() } else { self.reason().to_string() })
        } else {
            Ok(&self.value)
        }
    }

    /// Append a trace entry during auto-propagation: a place the trace
    /// already ends at is not named twice (a refusal on the line the error
    /// came from)
    pub fn append_trace(&mut self, location: &str) {
        if self.trace().last().map(String::as_str) != Some(location) {
            self.status_mut().trace.push(location.to_string());
        }
    }

    /// Stamp this value as captured from the outside world now.
    pub fn stamp_captured(mut self) -> Self {
        self.set_captured(Some(crow_runtime::moments::now()));
        self
    }

}

/// Two values compared as `eq` compares them (`ops::values_equal`): what
/// `contains` asks of a list's items, in both modes alike.
impl PartialEq for CrowValue {
    fn eq(&self, other: &Self) -> bool {
        crate::ops::values_equal(self, other)
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
    // the large ones are held apart, so a value is as big as the small
    // ones (typed-values-design.md §5, B)
    NumArray(Box<CrowTensor>),
    Map(BTreeMap<String, CrowValue>),
    Fn(FnValue),
    StructInstance(Box<StructInstance>),
    Sparse(Box<CrowSparse>), // sparse matrix (CSR format)
    Frame(Box<CrowFrame>), // column-oriented data frame
    DbConn(DbHandle), // database connection handle
    /// A mailer `smtp[…]` made: its place among the interpreter's mailers
    /// and its host. The password stays with the mailer, out of every value.
    Mailer(usize, std::sync::Arc<str>),
    /// What a route's handler answers: `text[…]`, `html[…]`, `json[…]`,
    /// `respond[status, body]`, `redirect[url]`, `bytes[…]` (use web).
    Response(Box<crow_web::core::CrowResponse>),
    /// A parsed JSON value, held as the compiled program holds it: the rows
    /// that take JSON read it as it is, and a field or an item read from it
    /// is JSON too (typed-values-design.md, `Json`). It had been converted
    /// to lists, maps and tensors, which JSON's own rows then converted back.
    Json(Box<serde_json::Value>),
    Dual(f64, f64), // dual number for autograd: (real, derivative)
    App(Box<AppSession>), // headless app session for test blocks
    GradFn(FnValue), // gradient function wrapping a user function
    Done, // represents no value (from 'done' status)
}

/// A headless app session a test block opened (`app[view]`).
#[derive(Debug, Clone)]
pub struct AppSession {
    /// The view it renders.
    pub view: FnValue,
    /// Its viewport width.
    pub width: f64,
    /// The wide look.
    pub wide: bool,
    /// Which session.
    pub id: u64,
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
            Value::Response(_) => "response",
            Value::Json(j) => crow_runtime::json::type_word(j),
            Value::Dual(_, _) => "float", // Dual appears as float to user
            Value::GradFn(_) => "fn",
            Value::Done => "done",
        }
    }

}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::App(a) => write!(f, "app[{}]", a.view.name),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            // a list, a map and a record as the one display writes them
            // (crow_runtime::display), the compiled program's typed values alike
            Value::Array(items) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", crow_runtime::display::list(items.iter().map(|i| i.crow_repr())))
            }
            // Delegate to the shared display kernel (crow_runtime::display) so
            // interpreter and compiled output are identical, including nesting
            Value::NumArray(tensor) => write!(f, "{}", tensor),
            Value::Map(map) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", crow_runtime::display::entries(map.iter().map(|(k, v)| (k, v.crow_repr()))))
            }
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::StructInstance(si) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", crow_runtime::display::record(&si.name, si.fields.iter().map(|(k, v)| (k, v.crow_repr()))))
            }
            Value::Sparse(s) => write!(f, "{}", s),
            Value::Frame(frame) => write!(f, "{}", frame),
            Value::DbConn(h) => write!(f, "<db:{}>", h.backend_name()),
            Value::Mailer(_, host) => write!(f, "<mailer:{}>", host),
            Value::Response(r) => write!(f, "<response {}>", r.status),
            Value::Json(j) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", j.crow_fmt())
            }
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

/// A value as both modes show it (crow_runtime::display): inside a list
/// (`crow_repr`, text quoted) or alone (`crow_fmt`), a failed one as its
/// reason. `str` is this display, called through its row.
impl crow_runtime::display::CrowDisplay for CrowValue {
    fn crow_repr(&self) -> String {
        if self.is_ok() {
            match &self.value {
                Value::Str(s) => s.crow_repr(),
                Value::Json(j) => j.crow_repr(),
                other => other.to_string(),
            }
        } else if self.kind() == "missing" && self.reason() == "nan" {
            // a frame column's missing cell, as the display kernel shows it
            "nan".to_string()
        } else {
            crow_runtime::display::failed(&self.reason())
        }
    }
    fn crow_fmt(&self) -> String {
        self.to_string()
    }
}

/// A value's JSON (`encode`), by the rule the compiled program's values
/// follow (crow_runtime::json::ToJson): a failed value is `null`, a record
/// an object of its fields, what JSON holds no form of `null`.
impl crow_runtime::json::ToJson for CrowValue {
    fn to_json(&self) -> serde_json::Value {
        crate::builtins::crow_to_encoded(self)
    }
}

impl fmt::Display for CrowValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_ok() {
            write!(f, "{}", self.value)
        } else {
            write!(f, "error: {}", self.reason())
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A value is as big as what it holds: what a failed or captured value
    /// carries lives apart (typed-values-design.md §2), where every value
    /// once carried it all (192 bytes).
    #[test]
    fn a_plain_value_carries_nothing_beside_itself() {
        assert!(std::mem::size_of::<CrowValue>() <= std::mem::size_of::<Value>() + 8);
        // and the large ones are held apart: a value is as big as a string
        assert!(std::mem::size_of::<Value>() <= 32, "{}", std::mem::size_of::<Value>());
        let v = CrowValue::ok(Value::Int(1));
        assert!(v.is_ok() && v.reason().is_empty() && v.trace().is_empty() && v.captured().is_none() && !v.is_markup());
        let mut e = CrowValue::error_with_kind("no", "parse", "line 1");
        e.append_trace("line 1");
        e.append_trace("line 2");
        assert_eq!((e.is_error(), e.reason(), e.kind(), e.trace().len()), (true, "no", "parse", 2));
    }
}
