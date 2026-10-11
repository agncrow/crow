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

    /// The int it is, when it is a plain one (no failure): what a typed
    /// instruction reads (ir_interp/prepared.rs).
    #[inline(always)]
    /// A whole number, in 64 bits or past them, of a plain value.
    pub fn whole(&self) -> Option<crow_runtime::int::Int> {
        if !self.is_ok() {
            return None;
        }
        Value::int_of(&self.value)
    }

    pub fn plain_int(&self) -> Option<i64> {
        match &self.value {
            Value::Int(n) if self.is_ok() => Some(*n),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn plain_float(&self) -> Option<f64> {
        match &self.value {
            Value::Float(x) if self.is_ok() => Some(*x),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn plain_bool(&self) -> Option<bool> {
        match &self.value {
            Value::Bool(b) if self.is_ok() => Some(*b),
            _ => None,
        }
    }

    /// A plain number or truth written over this value: into its payload
    /// when it is one of the same kind already (nothing dropped, nothing
    /// built), else replacing it.
    #[inline(always)]
    pub fn set_plain(&mut self, v: Value) {
        match v {
            Value::Int(n) => self.set_int(n),
            Value::Float(x) => self.set_float(x),
            Value::Bool(b) => self.set_bool(b),
            other => *self = CrowValue::ok(other),
        }
    }

    #[inline(always)]
    pub fn set_int(&mut self, n: i64) {
        if self.status.is_none()
            && let Value::Int(d) = &mut self.value
        {
            *d = n;
            return;
        }
        *self = CrowValue::ok(Value::Int(n));
    }

    #[inline(always)]
    pub fn set_float(&mut self, x: f64) {
        if self.status.is_none()
            && let Value::Float(d) = &mut self.value
        {
            *d = x;
            return;
        }
        *self = CrowValue::ok(Value::Float(x));
    }

    #[inline(always)]
    pub fn set_bool(&mut self, b: bool) {
        if self.status.is_none()
            && let Value::Bool(d) = &mut self.value
        {
            *d = b;
            return;
        }
        *self = CrowValue::ok(Value::Bool(b));
    }

    /// A value written over this one: a plain number over a plain number of
    /// its kind into the payload — the old one has nothing to drop and the
    /// new one nothing to keep — anything else replacing it (a register's
    /// write in a tight loop had been a call to the general drop each time).
    #[inline(always)]
    pub fn assign(&mut self, v: CrowValue) {
        let fast = self.status.is_none()
            && v.status.is_none()
            && match (&mut self.value, &v.value) {
                (Value::Int(d), Value::Int(s)) => {
                    *d = *s;
                    true
                }
                (Value::Float(d), Value::Float(s)) => {
                    *d = *s;
                    true
                }
                (Value::Bool(d), Value::Bool(s)) => {
                    *d = *s;
                    true
                }
                _ => false,
            };
        if fast {
            // (a plain number holds nothing to free)
            std::mem::forget(v);
        } else {
            *self = v;
        }
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
    /// A whole number past 64 bits (spec, Numbers That Keep Every Digit): an
    /// int within them is always `Int`, so equal ints are equal values.
    Big(std::sync::Arc<crow_runtime::int::IBig>),
    /// An exact base-ten number (spec, Numbers That Keep Every Digit).
    Decimal(std::sync::Arc<crow_runtime::decimal::Dec>),
    /// An exact ratio of two ints.
    Fraction(std::sync::Arc<crow_runtime::fraction::Frac>),
    Float(f64),
    Str(String),
    Bool(bool),
    /// A moment (crow_runtime::moments::Moment).
    Moment(crow_runtime::moments::Moment),
    /// A list: its items behind an `Arc`, so a copy of a list is a count
    /// and a write copies the items only when another value shares them
    /// (`Arc::make_mut`). A list read had been copied item by item: a
    /// parallel loop's chunks each copied a file's 400,000 lines to slice
    /// 2,000 of them.
    Array(std::sync::Arc<Vec<CrowValue>>),
    // the large ones are held apart, so a value is as big as the small
    // ones (typed-values-design.md §5, B)
    NumArray(Box<CrowTensor>),
    Map(BTreeMap<String, CrowValue>),
    /// A map keyed by ints: `map[[1, 2], …]`, `set[m, 2, v]` (spec, Maps).
    IntMap(BTreeMap<i64, CrowValue>),
    Fn(FnValue),
    StructInstance(std::sync::Arc<StructInstance>),
    Sparse(Box<CrowSparse>), // sparse matrix (CSR format)
    Frame(Box<CrowFrame>), // column-oriented data frame
    DbConn(DbHandle), // database connection handle
    /// A mailer `smtp[…]` made (crow_runtime::sys): the password stays with
    /// the mailer, out of every value a program prints.
    Mailer(crow_runtime::sys::Mailer),
    /// A WebSocket client's connection (`socket[url]`, written in Crow over
    /// the sys layer), shared by every value that holds it.
    Socket(crow_runtime::sys::Socket),
    /// Bytes (`use sys`): what a connection reads and writes.
    Bytes(crow_runtime::sys::Bytes),
    /// A connection `sys.open` made, shared by every value that holds it.
    Stream(crow_runtime::sys::CrowStream),
    /// What a route's handler answers: `text[…]`, `html[…]`, `json[…]`,
    /// `respond[status, body]`, `redirect[url]`, `bytes[…]` (use web).
    Response(Box<crow_web::core::CrowResponse>),
    /// A parsed JSON value, held as the compiled program holds it: the rows
    /// that take JSON read it as it is, and a field or an item read from it
    /// is JSON too (typed-values-design.md, `Json`). It had been converted
    /// to lists, maps and tensors, which JSON's own rows then converted back.
    Json(Box<serde_json::Value>),
    Dual(f64, f64), // dual number for autograd: (real, derivative)
    /// A tensor a function `grad` differentiates computes with: its value
    /// and, where it depends on a parameter, its node on the tape
    /// (crow_runtime::autograd). A number inside such a function is a
    /// tensor of no dimensions.
    Traced(Box<crow_runtime::autograd::Traced>),
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

/// A record's declaration as every value of it shares it: its names, and
/// its fields' names in the order a record shows them (by name).
#[derive(Debug, PartialEq)]
pub struct Layout {
    /// Its declaration's name in the program, unique (a module's record is
    /// prefixed with the module's path)
    pub name: String,
    /// The name it was declared by, which it shows (`Day`, not
    /// `sessions_Day`)
    pub shown: String,
    pub fields: Vec<String>,
}

impl Layout {
    pub fn new(name: String, shown: String, mut fields: Vec<String>) -> Self {
        fields.sort();
        fields.dedup();
        Layout { name, shown, fields }
    }

    /// The one layout of these names, for as long as the process runs: a
    /// record refers to it with no count to keep (a count every thread
    /// wrote at every record it made had made sixteen threads slower than
    /// one). There are as many as the programs loaded declare.
    pub fn intern(name: String, shown: String, fields: Vec<String>) -> &'static Layout {
        static TABLE: std::sync::Mutex<BTreeMap<(String, String, Vec<String>), &'static Layout>> = std::sync::Mutex::new(BTreeMap::new());
        let layout = Layout::new(name, shown, fields);
        let key = (layout.name.clone(), layout.shown.clone(), layout.fields.clone());
        let mut table = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = table.get(&key) {
            return found;
        }
        let kept: &'static Layout = Box::leak(Box::new(layout));
        table.insert(key, kept);
        kept
    }

    /// Where a field's value lies in a record of this layout.
    pub fn slot(&self, field: &str) -> Option<usize> {
        self.fields.binary_search_by(|f| f.as_str().cmp(field)).ok()
    }
}

/// A record: its declaration's layout (`Layout::intern`), and one value per
/// field in the layout's order. A record value holds it behind an `Arc`
/// (`Value::StructInstance`), so a copy of a record is a count and a write
/// copies it only when another value shares it (`Arc::make_mut`): each
/// record had held its names twice and a map of its fields by name, and a
/// field read copied the record below it whole (binarytrees: 4.6 GB).
#[derive(Debug, Clone)]
pub struct StructInstance {
    pub layout: &'static Layout,
    pub values: Vec<CrowValue>,
}

impl StructInstance {
    /// A record of these fields, named with their values, in a layout of
    /// its own (the interpreter shares one per declaration: `with_layout`).
    pub fn new(name: String, shown: String, fields: BTreeMap<String, CrowValue>) -> Self {
        let layout = Layout::intern(name, shown, fields.keys().cloned().collect());
        StructInstance { layout, values: fields.into_values().collect() }
    }

    /// A record of a shared layout, its values in the layout's order.
    pub fn with_layout(layout: &'static Layout, values: Vec<CrowValue>) -> Self {
        StructInstance { layout, values }
    }

    pub fn name(&self) -> &str {
        &self.layout.name
    }

    pub fn shown(&self) -> &str {
        &self.layout.shown
    }

    pub fn get(&self, field: &str) -> Option<&CrowValue> {
        self.layout.slot(field).and_then(|i| self.values.get(i))
    }

    pub fn get_mut(&mut self, field: &str) -> Option<&mut CrowValue> {
        let i = self.layout.slot(field)?;
        self.values.get_mut(i)
    }

    /// Its fields, by name, as a record shows them.
    pub fn fields(&self) -> impl Iterator<Item = (&String, &CrowValue)> {
        self.layout.fields.iter().zip(self.values.iter())
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl Value {
    /// An int as the runtime's whole number (in 64 bits or past them).
    pub fn int_of(v: &Value) -> Option<crow_runtime::int::Int> {
        match v {
            Value::Int(n) => Some(crow_runtime::int::Int::Small(*n)),
            Value::Big(b) => Some(crow_runtime::int::Int::Big(b.clone())),
            _ => None,
        }
    }

    /// The value of a whole number: `Int` where it fits 64 bits.
    pub fn of_int(n: crow_runtime::int::Int) -> Value {
        match n {
            crow_runtime::int::Int::Small(n) => Value::Int(n),
            crow_runtime::int::Int::Big(b) => Value::Big(b),
        }
    }

    /// Return the Crow type name for this value
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::App(..) => "app",
            Value::Int(_) | Value::Big(_) => "int",
            Value::Decimal(_) => "decimal",
            Value::Fraction(_) => "fraction",
            Value::Float(_) => "float",
            Value::Moment(_) => "moment",
            Value::Str(_) => "string",
            Value::Bool(_) => "bool",
            Value::Array(_) => "array",
            Value::NumArray(_) => "array",
            Value::Map(_) | Value::IntMap(_) => "map",
            Value::Fn(_) => "fn",
            Value::StructInstance(_) => "struct_instance",
            Value::Sparse(_) => "sparse",
            Value::Frame(_) => "frame",
            Value::DbConn(_) => "db",
            Value::Mailer(..) => "mailer",
            Value::Socket(_) => "socket",
            Value::Bytes(_) => "bytes",
            Value::Stream(_) => "stream",
            Value::Response(_) => "response",
            Value::Json(j) => crow_runtime::json::type_word(j),
            Value::Dual(_, _) => "float", // Dual appears as float to user
            Value::Traced(t) => if t.value.ndim() == 0 { "float" } else { "array" },
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
            Value::Big(n) => write!(f, "{}", n),
            Value::Decimal(d) => write!(f, "{}", d),
            Value::Fraction(q) => write!(f, "{}", q),
            Value::Float(n) => write!(f, "{}", n),
            Value::Moment(m) => write!(f, "{}", m),
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
            Value::IntMap(map) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", crow_runtime::display::entries(map.iter().map(|(k, v)| (k.to_string(), v.crow_repr()))))
            }
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::StructInstance(si) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", crow_runtime::display::record(si.shown(), si.fields().map(|(k, v)| (k, v.crow_repr()))))
            }
            Value::Sparse(s) => write!(f, "{}", s),
            Value::Frame(frame) => write!(f, "{}", frame),
            Value::DbConn(h) => write!(f, "<db:{}>", h.backend_name()),
            Value::Mailer(m) => write!(f, "{}", m),
            Value::Socket(s) => write!(f, "<{:?}>", s),
            Value::Bytes(b) => write!(f, "{}", b),
            Value::Stream(s) => write!(f, "<{:?}>", s),
            Value::Response(r) => write!(f, "<response {}>", r.status),
            Value::Json(j) => {
                use crow_runtime::display::CrowDisplay as _;
                write!(f, "{}", j.crow_fmt())
            }
            Value::GradFn(func) => write!(f, "<grad {}>", func.name),
            // a traced value shows as its value: a number, or the tensor
            Value::Traced(t) => match t.as_number().filter(|_| t.value.ndim() == 0) {
                Some(x) if x == (x as i64) as f64 && x.is_finite() => write!(f, "{}", x as i64),
                Some(x) => write!(f, "{}", x),
                None => write!(f, "{}", t.value),
            },
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
        } else {
            // (a frame's missing cell is an error of reason `missing`, shown
            // as one: it had shown as `nan`, as the display kernel did)
            crow_runtime::display::failed(self.reason(), self.trace())
        }
    }
    fn crow_fmt(&self) -> String {
        self.to_string()
    }
    fn crow_write(&self, out: &mut Vec<u8>) {
        match &self.value {
            Value::Str(s) if self.is_ok() => out.extend_from_slice(s.as_bytes()),
            _ => out.extend_from_slice(self.to_string().as_bytes()),
        }
    }
    fn as_text(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) if self.is_ok() => Some(s),
            _ => None,
        }
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
            write!(f, "{}", crow_runtime::display::failed(self.reason(), self.trace()))
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
