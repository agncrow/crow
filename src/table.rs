//! One row per builtin (docs/bug-classes.md §4, ir-design.md §3.5).
//!
//! A builtin written twice, once as the interpreter's adapter and once as
//! the compiled program's printing, drifts: one mode checks an argument the
//! other does not, or fails with a value where the other stops. A builtin
//! with a row is written once, as a runtime function taking typed arguments
//! and answering `Result`, and the row says the rest:
//!
//! - the parameters' kinds, from which the interpreter reads its arguments
//!   ([`read_int`] and the others) and the emitter prints them
//!   ([`Kind::print`]);
//! - the answer's kind, from which the interpreter builds its value and the
//!   lowering types the call;
//! - what a failure is: a stop (a misuse no value can carry: the runtime's
//!   reason, on the call's line), an error value of a kind, or impossible.
//!
//! Both modes then agree by construction, for every row: the only code per
//! mode is the reading and the printing of each kind, written once here.
//! The IR names a row by its id (`moments.calendar`); the interpreter calls
//! the row, the emitter prints it, inference types it from it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::type_infer::Ty;
use crate::value::{CrowValue, Value};
use crow_runtime::tensor::CrowTensor;

/// What a parameter or an answer is, as the runtime function takes or
/// gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// `i64`: an int, or a float cut toward zero (as `as i64` does).
    Int,
    /// `i64`: an int only — a row beside a float's row of the same name
    /// (`min` of two ints is an int, of a float a float).
    Whole,
    /// `f64`: a float, or an int.
    Float,
    /// `Moment` taken and given (crow_runtime::moments).
    Moment,
    Bool,
    /// `&str` taken, `String` given.
    Str,
    /// `&CrowTensor` taken, `CrowTensor` given: a tensor, or a list of
    /// numbers.
    Tensor,
    /// `BTreeMap<String, f64>` given: a test's numbers by name (`ttest`'s
    /// `t` and `p`).
    FloatMap,
    /// `&[i64]` taken: the rest of the arguments, each a size (`zeros[2, 3]`,
    /// `poisson[1.5, 4]`); the fixed parameters around it take theirs first.
    Sizes,
    /// `&[CrowTensor]` taken, `Vec<CrowTensor>` given: a decomposition's
    /// parts (`svd`, `qr`), a complex number's `[real, imag]`.
    Tensors,
    /// `&[&CrowTensor]` taken: the rest of the arguments, each a tensor
    /// (`anova[a, b, c]`).
    Groups,
    /// Nothing given: a function run for what it sets (`seed`).
    Unit,
    /// `&CrowSparse` taken, `CrowSparse` given.
    Sparse,
    /// `&CrowFrame` taken, `CrowFrame` given.
    Frame,
    /// `&[String]` taken, `Vec<String>` given: a list of text (a frame's
    /// column names, a column's distinct values).
    Strs,
    /// `&[bool]` taken: a mask, one per row.
    Bools,
    /// `&Cell` taken: a number, text or a bool for a frame's cell (`fill`).
    Cell,
    /// `Cells` taken: a column's values — a list, a tensor, another frame's
    /// column with its missing cells, or a cell for every row (`with`).
    Cells,
    /// `&[&[String]]` taken: the rest of the arguments, each a list of
    /// text (`group`'s `[column, how]`).
    Lists,
    /// `&[String]` taken: a list's items as text, a number as it displays
    /// (`join`'s items, `crow_runtime::cell::Words`).
    Words,
    // A collection's builtin is one generic function over its items: the
    // compiled program calls it with its typed items, the interpreter with
    // its values (crow_runtime::core, "the collections a program calls").
    /// `Vec<T>` taken and given: a list the function makes its answer of,
    /// moved when the statement assigns the variable it reads (`xs =
    /// push[xs, x]` stays in place), copied otherwise.
    List,
    /// `&[T]` taken: a list read.
    Seq,
    /// `T` taken: an item a list or a map keeps (a variable read is copied;
    /// into a list whose items can fail, it is an item that did not).
    Item,
    /// `&T` taken: an item compared (`contains`).
    ItemRef,
    /// `BTreeMap<String, T>` taken and given, moved or copied as a list is.
    MapOf,
    /// `&BTreeMap<String, T>` taken: a map read.
    MapRef,
    /// `&BTreeMap<String, String>` taken: a map's values as text, each as
    /// it shows (a request's headers, query, form and files).
    TextMap,
    /// `Vec<i64>` taken and given, moved or copied as a list is.
    Ints,
    /// `Vec<f64>` taken and given, moved or copied as a list is.
    Floats,
    /// `Vec<String>` taken and given, moved or copied as a list is.
    Texts,
    /// `&serde_json::Value` taken: a parsed JSON value, which only the
    /// compiled program holds as one (the interpreter holds what it parsed
    /// to, and picks the row of that).
    Json,
    /// `&CrowColumn` taken: a frame's column, which only the compiled
    /// program holds as one (the interpreter holds its cells as a list).
    Column,
    /// `&Bytes` taken, `Bytes` given (crow_runtime::sys): what a
    /// connection reads and writes.
    Bytes,
    /// `&CrowStream` taken, `CrowStream` given: a connection `sys.open`
    /// made.
    Stream,
    /// A list of bytes: a connection's chunks, gathered.
    Chunks,
    /// `Answer` given (crow_runtime::sys): a connection's answer, the
    /// record `sys.exchange` and `sys.fetch` make.
    Answer,
    /// `CrowResponse` given (crow_web::core): a response a route answers,
    /// made by a row (`sys.binary`, `msgpack`).
    Response,
    /// `&Socket` taken, `Socket` given (crow_runtime::sys): a WebSocket
    /// client's connection, `socket[url]`'s.
    Socket,
    /// `&Mailer` taken, `Mailer` given (crow_runtime::sys): a mailer
    /// `smtp[…]` made.
    Mailer,
    /// `String` given: markup a view places as markup (`doc.markdown`).
    Markup,
    /// `Vec<BTreeMap<String, String>>` given: rows of text by column
    /// (`io.csv`).
    TextRows,
    /// `crow_runtime::core::Shell` given: a command's status and output,
    /// the compiled program's `CrowShell` and the interpreter's record.
    Shell,
    /// `crow_runtime::sources::Reply` given: a request's status, body and
    /// headers, the compiled program's `CrowReply` and the interpreter's
    /// record.
    Reply,
    /// `&T` taken, any value that shows (`str`): the compiled program's
    /// `CrowDisplay`, the interpreter's value's.
    Shown,
    /// `&T` taken, any value JSON can hold (`encode`): the compiled
    /// program's `ToJson`, the interpreter's value's.
    Encoded,
    /// `Vec<serde_json::Value>` given: a parsed JSON object's values.
    JsonList,
    /// `&[&dyn CrowDisplay]` taken: the rest of the arguments, each any
    /// value that shows (`log`).
    Shows,
    /// Each of `Shows`: `&dyn CrowDisplay`.
    ShownDyn,
    /// `&[crow_runtime::fused::Operand]` taken: the rest of the arguments,
    /// each a tensor or a number — a fused chain's operands.
    Operands,
    /// Each of `Operands`.
    Operand,
    /// `&[crow_runtime::mask::Cell]` taken: the rest of the arguments, each
    /// a number, text or a bool — the values a fused filter's predicate
    /// compares its columns with.
    Scalars,
    /// Each of `Scalars`.
    Scalar,
}

/// What a failure of the function is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fails {
    /// It cannot fail: the function answers its value, not a `Result`.
    Never,
    /// A misuse no value can carry: the program stops with the reason on
    /// the call's line (spec, When a Program Stops).
    Stops,
    /// An error value of this kind (spec §10, the kinds the language
    /// produces).
    Value(&'static str),
}

pub struct Row {
    /// The id the IR names the row by (`Target::Row`, an effect's `row`).
    pub id: &'static str,
    /// The Crow name, and the module it is in ("" for the core).
    pub name: &'static str,
    pub module: &'static str,
    /// The runtime function, as `stringify!` wrote it (`rust_path` is
    /// what the emitter prints).
    pub path: &'static str,
    pub params: &'static [Kind],
    pub ret: Kind,
    pub fails: Fails,
    /// The interpreter's call: the arguments read by kind, the function
    /// applied, the failure dispatched as the row says. `trace` is where an
    /// error value is made.
    pub call: fn(&[CrowValue], &str) -> Result<CrowValue, String>,
    /// The same call, given the arguments to keep: a collection the
    /// function makes its answer of is moved into it, not copied (the
    /// interpreter's `xs = push[xs, x]` grows the list where it lies, as the
    /// compiled program's does).
    pub take: fn(Vec<CrowValue>, &str) -> Result<CrowValue, String>,
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Row({})", self.id)
    }
}

impl Row {
    /// The runtime function's path as the emitter prints it.
    pub fn rust_path(&self) -> String {
        self.path.replace(' ', "")
    }

    /// Whether the row's body is a function of the standard library written
    /// in Crow (`crow!`): the lowering calls that function where it would
    /// call a Rust row's runtime function.
    pub fn is_crow(&self) -> bool {
        self.path == IN_CROW
    }

    /// The call's type, for inference and the lowering: the answer's, able
    /// to fail when a failure is a value.
    pub fn ty(&self) -> Ty {
        let t = self.ret.ty();
        match self.fails {
            Fails::Value(_) => Ty::Result(Box::new(t)),
            _ => t,
        }
    }
}

impl Kind {
    pub fn ty(self) -> Ty {
        match self {
            Kind::Int | Kind::Whole => Ty::Int,
            Kind::Float => Ty::Float,
            Kind::Moment => Ty::Moment,
            Kind::Bool => Ty::Bool,
            Kind::Str => Ty::Str,
            Kind::Tensor => Ty::tensor(Ty::Float),
            Kind::FloatMap => Ty::Map(Box::new(Ty::Float)),
            Kind::Sizes => Ty::List(Box::new(Ty::Int)),
            Kind::Tensors | Kind::Groups => Ty::List(Box::new(Ty::tensor(Ty::Float))),
            Kind::Unit => Ty::Unit,
            Kind::Sparse => Ty::Sparse,
            Kind::Frame => Ty::Frame(None),
            Kind::Strs => Ty::List(Box::new(Ty::Str)),
            Kind::Chunks => Ty::List(Box::new(Ty::Bytes)),
            Kind::Bools => Ty::List(Box::new(Ty::Bool)),
            Kind::Cell | Kind::Cells => Ty::Unknown,
            Kind::Lists => Ty::List(Box::new(Ty::List(Box::new(Ty::Str)))),
            Kind::Words | Kind::Texts => Ty::List(Box::new(Ty::Str)),
            Kind::Ints => Ty::List(Box::new(Ty::Int)),
            Kind::Floats => Ty::List(Box::new(Ty::Float)),
            Kind::Json => Ty::Json,
            Kind::Column => Ty::Column(Box::new(Ty::Unknown)),
            Kind::Bytes => Ty::Bytes,
            Kind::Stream => Ty::Stream,
            Kind::Markup => Ty::Markup,
            Kind::TextRows => Ty::List(Box::new(Ty::Map(Box::new(Ty::Str)))),
            Kind::Shell => Ty::Record("CrowShell".to_string()),
            Kind::Reply => Ty::Record("CrowReply".to_string()),
            Kind::Answer => Ty::Record("CrowAnswer".to_string()),
            Kind::Response => Ty::WebResponse,
            Kind::Socket => Ty::Socket,
            Kind::Mailer => Ty::Mailer,
            Kind::TextMap => Ty::Map(Box::new(Ty::Str)),
            Kind::Shown | Kind::Encoded | Kind::Shows | Kind::ShownDyn | Kind::Operands | Kind::Operand | Kind::Scalars | Kind::Scalar => Ty::Unknown,
            Kind::JsonList => Ty::List(Box::new(Ty::Json)),
            // what the arguments' items are: inference types the call from
            // them (a generic row's answer is the list or map it was given)
            Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef | Kind::MapOf | Kind::MapRef => Ty::Unknown,
        }
    }

    /// Whether the kind is over a collection's items, whatever they are: the
    /// call's type is its arguments', which inference reads.
    pub fn generic(self) -> bool {
        matches!(self, Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef | Kind::MapOf | Kind::MapRef)
    }

    /// Whether the function takes the collection itself, to make its
    /// answer of: the compiled program moves the variable the statement
    /// assigns and copies any other.
    pub fn owned(self) -> bool {
        matches!(self, Kind::List | Kind::MapOf | Kind::Ints | Kind::Floats | Kind::Texts)
    }

    /// An argument printed the way the runtime function takes it (each of
    /// a rest parameter's, as its [`Kind::each`]); `line` is where a stop
    /// in reading it stops (a frame's column's missing cell among words).
    pub fn print(self, arg: &str, line: &str) -> String {
        match self.each() {
            Kind::Int | Kind::Whole => format!("({}) as i64", arg),
            Kind::Float => format!("({}) as f64", arg),
            Kind::Moment => format!("({})", arg),
            Kind::Bool => format!("({})", arg),
            Kind::Str => match (literal_text(arg), borrowed_index(arg)) {
                (Some(lit), _) => lit,
                (None, Some(item)) => format!("&*({})", item),
                _ => format!("&*({})", arg),
            },
            Kind::Unit => format!("({})", arg),
            Kind::Strs | Kind::Bools | Kind::Chunks => format!("&({})[..]", arg),
            Kind::Cell => format!("&crow_runtime::cell::Cell::from(({}).clone())", arg),
            Kind::Words => format!("&crow_runtime::cell::words(&({})).unwrap_or_else(|e| crow_stop({}, e))[..]", arg, line),
            Kind::Cells => format!("crow_runtime::frames::Cells::from(({}).clone())", arg),
            Kind::Seq => format!("&({})[..]", arg),
            Kind::ShownDyn => format!("&({}) as &dyn crow_runtime::display::CrowDisplay", arg),
            Kind::TextMap => format!("&crow_runtime::sources::text_map(&({}))", arg),
            Kind::Operand => format!("crow_runtime::fused::Operand::from(&({}))", arg),
            Kind::Scalar => format!("crow_runtime::mask::Cell::from(&({}))", arg),
            // moved or copied (an item kept) where the call is made
            Kind::List | Kind::MapOf | Kind::Ints | Kind::Floats | Kind::Texts | Kind::Item => format!("({})", arg),
            _ => format!("&({})", arg),
        }
    }

    /// Whether the parameter takes the rest of the arguments.
    pub fn is_rest(self) -> bool {
        matches!(self, Kind::Sizes | Kind::Groups | Kind::Lists | Kind::Shows | Kind::Operands | Kind::Scalars)
    }

    /// The kind of each argument a parameter takes: a size of the sizes, a
    /// tensor of the groups.
    pub fn each(self) -> Kind {
        match self {
            Kind::Sizes => Kind::Int,
            Kind::Groups => Kind::Tensor,
            Kind::Lists => Kind::Strs,
            Kind::Shows => Kind::ShownDyn,
            Kind::Operands => Kind::Operand,
            Kind::Scalars => Kind::Scalar,
            k => k,
        }
    }

    /// The value the compiled program holds for a failed call of this kind.
    pub fn default_rust(self) -> &'static str {
        match self {
            Kind::Int | Kind::Whole => "0_i64",
            Kind::Float => "0.0_f64",
            Kind::Moment => "crow_runtime::moments::Moment::default()",
            Kind::Bool => "false",
            Kind::Str => "String::new()",
            Kind::Tensor => "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])",
            Kind::FloatMap => "std::collections::BTreeMap::new()",
            Kind::Sizes => "Vec::<i64>::new()",
            Kind::Tensors | Kind::Groups => "Vec::<crow_runtime::tensor::CrowTensor>::new()",
            Kind::Unit => "()",
            Kind::Sparse => "crow_runtime::sparse::CrowSparse::from_dense(&crow_runtime::tensor::CrowTensor::zeros(vec![0, 0]))",
            Kind::Frame => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])",
            Kind::Strs => "Vec::<String>::new()",
            Kind::Chunks => "Vec::<crow_runtime::sys::Bytes>::new()",
            Kind::Bools => "Vec::<bool>::new()",
            Kind::Cell => "crow_runtime::cell::Cell::Int(0)",
            Kind::Cells => "crow_runtime::frames::Cells::One(crow_runtime::cell::Cell::Int(0))",
            Kind::Lists => "Vec::<Vec<String>>::new()",
            Kind::Words | Kind::Texts => "Vec::<String>::new()",
            Kind::Ints => "Vec::<i64>::new()",
            Kind::Floats => "Vec::<f64>::new()",
            Kind::Json => "serde_json::Value::Null",
            Kind::Bytes => "crow_runtime::sys::Bytes::default()",
            Kind::Stream => "crow_runtime::sys::CrowStream::closed()",
            // taken, never given
            Kind::Column => "unreachable!()",
            Kind::Markup => "String::new()",
            Kind::TextRows => "Vec::<std::collections::BTreeMap<String, String>>::new()",
            Kind::Shell => "crow_runtime::core::Shell { status: -1, stdout: String::new(), stderr: String::new() }",
            Kind::Reply => "crow_runtime::sources::Reply::default()",
            Kind::Answer => "crow_runtime::sys::Answer::default()",
            Kind::Response => "crow_web::core::CrowResponse::default()",
            Kind::Socket => "crow_runtime::sys::Socket::closed()",
            Kind::Mailer => "crow_runtime::sys::Mailer::default()",
            Kind::Shown | Kind::Encoded | Kind::Shows | Kind::ShownDyn | Kind::Operands | Kind::Operand | Kind::Scalars | Kind::Scalar => "unreachable!()",
            Kind::JsonList => "Vec::<serde_json::Value>::new()",
            // a generic answer never fails as a value (its rows answer or stop)
            Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef => "Vec::new()",
            Kind::MapOf | Kind::MapRef | Kind::TextMap => "std::collections::BTreeMap::new()",
        }
    }
}

/// The arguments grouped by parameter: one each, and a rest parameter the
/// rest (the same grouping both modes use: `groups` for the interpreter's
/// values, [`print`] for the compiled program's text).
pub fn spans(params: &[Kind], argc: usize, name: &str) -> Result<Vec<(usize, usize)>, String> {
    let fixed = params.iter().filter(|k| !k.is_rest()).count();
    let rest = params.iter().any(|k| k.is_rest());
    if (!rest && argc != fixed) || (rest && argc < fixed) {
        let wanted = if rest { format!("at least {}", fixed) } else { fixed.to_string() };
        return Err(miscount(name, &wanted, argc));
    }
    let mut at = 0;
    let mut out = Vec::with_capacity(params.len());
    for k in params {
        let n = if k.is_rest() { argc - fixed } else { 1 };
        out.push((at, at + n));
        at += n;
    }
    Ok(out)
}

/// A text literal as the emitter prints it, `"…".to_string()`, as the
/// literal itself: a parameter that reads text takes `&str`, and the
/// String allocated for it on every call (`split[row, '\t']` a million
/// times) was never kept.
pub fn literal_text(arg: &str) -> Option<String> {
    let inner = arg.strip_prefix('"')?.strip_suffix("\".to_string()")?;
    // one literal, not an expression that begins and ends with one
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return None,
            _ => escaped = false,
        }
    }
    Some(format!("\"{}\"", inner))
}

/// An indexed read of a list as the emitter prints it — `{ let _i = …;
/// let _v = &xs; _v[…].clone() }` — with the item referenced instead of
/// cloned: a parameter that only reads it needs no copy of it. Only of a
/// list a name holds (a local, a field of one): a list an expression
/// makes dies with the block, and a reference into it would not build.
pub fn borrowed_index(arg: &str) -> Option<String> {
    if !arg.starts_with("{ let _i = ") {
        return None;
    }
    let body = arg.strip_suffix(".clone() }")?;
    let at = body.rfind("; _v[")?;
    let held = body[..at].rfind("; let _v = &").map(|k| &body[k + 12..at])?;
    if held.is_empty() || !held.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
        return None;
    }
    Some(format!("{}; &_v[{} }}", &body[..at], &body[at + 5..]))
}

/// The kind each of `argc` arguments is read as by these parameters, or
/// none when the count does not fit them (the lowering converts an
/// argument to its kind: a list of numbers or a column to a tensor).
pub fn kinds_of(params: &[Kind], argc: usize) -> Option<Vec<Kind>> {
    let spans = spans(params, argc, "").ok()?;
    Some(params.iter().zip(spans).flat_map(|(k, (a, b))| std::iter::repeat_n(k.each(), b - a)).collect())
}

/// The interpreter's arguments grouped by the row's parameters.
pub fn groups<'a>(params: &[Kind], args: &'a [CrowValue], name: &str) -> Result<Vec<&'a [CrowValue]>, String> {
    Ok(spans(params, args.len(), name)?.into_iter().map(|(a, b)| &args[a..b]).collect())
}

/// The arguments grouped so, each group the function's to keep.
pub fn groups_mut<'a>(params: &[Kind], args: &'a mut [CrowValue], name: &str) -> Result<Vec<&'a mut [CrowValue]>, String> {
    let spans = spans(params, args.len(), name)?;
    let mut out = Vec::with_capacity(spans.len());
    let mut rest = args;
    for (a, b) in spans {
        let (group, tail) = std::mem::take(&mut rest).split_at_mut(b - a);
        out.push(group);
        rest = tail;
    }
    Ok(out)
}

/// The builtins that read the world outside the program: a value one
/// answers is stamped with its capture moment (`captured[x]`), in both
/// modes — the interpreter stamps what its call answers, the compiled
/// program's printing of a row stamps it (the compiled `rm` had stamped its
/// empty answer while the interpreter did not).
pub const READERS: &[&str] = &["query", "read", "fetch", "recv", "input", "load", "env", "shell", "ls", "csv", "lines", "request"];

/// A row's call as the compiled program prints it: each argument as its
/// kind takes it, the failure as the row says. `line` is the stop's line
/// (a module's code carries `modules::STOP_LINE` for the emitter to fill),
/// `trace` where an error value is made (none for a module's function,
/// whose error values carry no trace in either mode).
pub fn print(row: &Row, args: &[String], line: &str, trace: Option<&str>) -> Result<String, String> {
    let mut printed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans(row.params, args.len(), row.name)?) {
        if k.is_rest() {
            printed.push(format!("&[{}]", args[a..b].iter().map(|x| k.print(x, line)).collect::<Vec<_>>().join(", ")));
        } else {
            printed.push(k.print(&args[a], line));
        }
    }
    if row.is_crow() {
        return Err(format!("a bug in Crow: `{}` is written in Crow, and its call was left to the runtime (an argument's kind not known before the program runs)", row.name));
    }
    let call = format!("{}({})", row.rust_path(), printed.join(", "));
    let captured = if READERS.contains(&row.name) { "crow_runtime::moments::now()" } else { "0.0" };
    Ok(match row.fails {
        Fails::Never => call,
        Fails::Stops => format!("{}.unwrap_or_else(|e| crow_stop({}, e))", call, line),
        Fails::Value(kind) => format!(
            "match {} {{ Ok(_v) => CrowResult {{ ok: true, value: _v, reason: String::new(), kind: String::new(), trace: vec![], captured: {} }}, Err(_e) => CrowResult {{ ok: false, value: {}, reason: _e, kind: \"{}\".to_string(), trace: vec![{}], captured: 0.0 }} }}",
            call,
            captured,
            row.ret.default_rust(),
            kind,
            trace.unwrap_or("")
        ),
    })
}

// ── the interpreter's reading and writing, one per kind ─────────────

fn named(v: &CrowValue) -> &'static str {
    v.value.type_name()
}

pub fn read_int(v: &CrowValue) -> Result<i64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n),
        Value::Float(f) => Ok(*f as i64),
        _ => Err(format!("expected an int, got {}", named(v))),
    }
}

pub fn read_float(v: &CrowValue) -> Result<f64, String> {
    match &v.value {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        _ => Err(format!("expected a number, got {}", named(v))),
    }
}

pub fn read_moment(v: &CrowValue) -> Result<crow_runtime::moments::Moment, String> {
    match &v.value {
        Value::Moment(m) => Ok(*m),
        _ => Err(format!("expected a moment, got {}", named(v))),
    }
}

pub fn write_moment(m: crow_runtime::moments::Moment) -> CrowValue {
    CrowValue::ok(Value::Moment(m))
}

pub fn read_bool(v: &CrowValue) -> Result<bool, String> {
    match &v.value {
        Value::Bool(b) => Ok(*b),
        _ => Err(format!("expected a bool, got {}", named(v))),
    }
}

pub fn read_str(v: &CrowValue) -> Result<&str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("expected a string, got {}", named(v))),
    }
}

pub fn read_tensor(v: &CrowValue) -> Result<Cow<'_, CrowTensor>, String> {
    match &v.value {
        Value::NumArray(t) => Ok(Cow::Borrowed(t.as_ref())),
        // a list of rows is a matrix, as a list of numbers is a vector
        Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.is_ok() && matches!(i.value, Value::NumArray(_))) => {
            let rows: Vec<CrowTensor> = items.iter().filter_map(|i| if let Value::NumArray(t) = &i.value { Some(t.as_ref().clone()) } else { None }).collect();
            CrowTensor::from_elements(&rows).map(Cow::Owned)
        }
        Value::Array(items) => {
            let mut data = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                // a failed item holds a placeholder, never a number to read
                if item.is_error() {
                    return Err(if item.kind() == "missing" { crow_runtime::stops::missing_cell(i) } else { format!("item {} of the list failed: {}", i, item.reason()) });
                }
                data.push(read_float(item)?);
            }
            let n = data.len();
            Ok(Cow::Owned(CrowTensor::from_vec(data, vec![n])))
        }
        _ => Err(format!("expected numbers, got {}", named(v))),
    }
}

pub fn write_int(n: i64) -> CrowValue {
    CrowValue::ok(Value::Int(n))
}

pub fn write_float(f: f64) -> CrowValue {
    CrowValue::ok(Value::Float(f))
}

pub fn write_bool(b: bool) -> CrowValue {
    CrowValue::ok(Value::Bool(b))
}

pub fn write_str(s: String) -> CrowValue {
    CrowValue::ok(Value::Str(s))
}

pub fn write_tensor(t: CrowTensor) -> CrowValue {
    CrowValue::ok(Value::NumArray(Box::new(t)))
}

pub fn write_frame(f: crow_runtime::frame::CrowFrame) -> CrowValue {
    CrowValue::ok(Value::Frame(Box::new(f)))
}

pub fn write_tensors(ts: Vec<CrowTensor>) -> CrowValue {
    CrowValue::ok(Value::Array(ts.into_iter().map(write_tensor).collect()))
}

pub fn write_unit(_: ()) -> CrowValue {
    CrowValue::done()
}

pub fn write_sparse(m: crow_runtime::sparse::CrowSparse) -> CrowValue {
    CrowValue::ok(Value::Sparse(Box::new(m)))
}

pub fn read_sparse(v: &CrowValue) -> Result<&crow_runtime::sparse::CrowSparse, String> {
    match &v.value {
        Value::Sparse(m) => Ok(m.as_ref()),
        _ => Err(format!("expected a sparse matrix, got {}", named(v))),
    }
}

pub fn read_frame(v: &CrowValue) -> Result<&crow_runtime::frame::CrowFrame, String> {
    match &v.value {
        Value::Frame(f) => Ok(f.as_ref()),
        _ => Err(format!("expected a frame, got {}", named(v))),
    }
}

pub fn read_strs(v: &CrowValue) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(|i| read_str(i).map(|s| s.to_string())).collect(),
        _ => Err(format!("expected a list of text, got {}", named(v))),
    }
}

/// A frame's column as the interpreter holds one: its cells as a list, a
/// missing one an error of reason `missing` holding the column's
/// placeholder, as `df.col` reads them (ops::frame_cell).
pub fn write_column(c: crow_runtime::frame::CrowColumn) -> CrowValue {
    CrowValue::ok(Value::Array((0..c.len()).map(|i| crate::ops::column_cell(&c.data, c.valid[i], i)).collect()))
}

/// A mask answered: a list of bools.
pub fn write_bools(bs: Vec<bool>) -> CrowValue {
    CrowValue::ok(Value::Array(bs.into_iter().map(|b| CrowValue::ok(Value::Bool(b))).collect()))
}

pub fn write_strs(ss: Vec<String>) -> CrowValue {
    CrowValue::ok(Value::Array(ss.into_iter().map(write_str).collect()))
}

/// A mask: a list of bools.
pub fn read_bools(v: &CrowValue) -> Result<Vec<bool>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(read_bool).collect(),
        _ => Err(format!("expected a list of bools, got {}", named(v))),
    }
}

/// A scalar, as the compiled program's `Cell::from` reads its typed value:
/// a number, text or a bool; a parsed JSON value's `null`, list or map is
/// named for the builtin to refuse.
pub fn read_cell(v: &CrowValue) -> Result<crow_runtime::cell::Cell, String> {
    use crow_runtime::cell::Cell;
    match &v.value {
        Value::Moment(m) => Ok(crow_runtime::cell::Cell::Moment(*m)),
        Value::Int(n) => Ok(Cell::Int(*n)),
        Value::Float(x) => Ok(Cell::Float(*x)),
        Value::Bool(b) => Ok(Cell::Bool(*b)),
        Value::Str(s) => Ok(Cell::Text(s.clone())),
        Value::Done => Ok(Cell::Null),
        Value::Array(_) => Ok(Cell::Other("a list")),
        Value::Map(_) => Ok(Cell::Other("a map")),
        // a parsed value, as the compiled program reads its JSON
        Value::Json(j) => Ok(match &**j {
            serde_json::Value::Array(_) => Cell::Other("a list"),
            serde_json::Value::Object(_) => Cell::Other("a map"),
            scalar => Cell::from(scalar.clone()),
        }),
        _ => Err(format!("expected a number, text or a bool, got {}", named(v))),
    }
}

/// A list's items as the text `join` writes them, as the compiled
/// program's `Words` reads its typed list.
pub fn read_words(v: &CrowValue) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(n, i)| match &i.value {
                // a failed item holds a placeholder, never a word
                _ if i.is_error() => Err(if i.kind() == "missing" { crow_runtime::stops::missing_cell(n) } else { format!("item {} of the list failed: {}", n, i.reason()) }),
                Value::Str(s) => Ok(s.clone()),
                Value::Int(n) => Ok(n.to_string()),
                Value::Float(x) => Ok(x.to_string()),
                Value::Bool(b) => Ok(b.to_string()),
                Value::Moment(m) => Ok(m.to_string()),
                _ => Err(format!("expected a list of numbers, text, bools or moments, got a list of {}", named(i))),
            })
            .collect(),
        Value::NumArray(t) => Ok(t.to_vec().iter().map(|x| x.to_string()).collect()),
        _ => Err(format!("expected a list, got {}", named(v))),
    }
}

/// An int only.
pub fn read_whole(v: &CrowValue) -> Result<i64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("expected an int, got {}", named(v))),
    }
}

/// A column's values, as the compiled program's `Cells::from` reads its
/// typed value: a list of one kind (ints beside floats make floats), a
/// tensor's numbers, a frame's column — whose missing cells, errors of
/// reason `missing` here, stay missing — or a cell for every row.
pub fn read_cells(v: &CrowValue) -> Result<crow_runtime::frames::Cells, String> {
    use crow_runtime::frame::Column;
    use crow_runtime::frames::Cells;
    let items = match &v.value {
        Value::Array(items) => items,
        Value::NumArray(t) => return Ok(Cells::from(t.as_ref().clone())),
        _ => return read_cell(v).map(Cells::One),
    };
    // every cell says the column's kind, a missing one by the placeholder
    // it holds (ops::frame_cell), so a column all of whose cells are missing
    // keeps its kind, as the compiled program's column does
    let valid: Vec<bool> = items.iter().map(|i| !i.is_error()).collect();
    let all = |f: fn(&Value) -> bool| !items.is_empty() && items.iter().all(|i| f(&i.value));
    let column = if all(|v| matches!(v, Value::Moment(_))) {
        let moments: Vec<crow_runtime::moments::Moment> = items.iter().map(|i| if let Value::Moment(m) = i.value { m } else { crow_runtime::moments::Moment::default() }).collect();
        Column::DateTime(moments.iter().map(|m| m.ms).collect(), moments.iter().map(|m| m.offset).collect())
    } else if all(|v| matches!(v, Value::Int(_))) {
        Column::Int(items.iter().map(|i| if let Value::Int(n) = i.value { n } else { 0 }).collect())
    } else if all(|v| matches!(v, Value::Int(_) | Value::Float(_))) {
        Column::Float(items.iter().map(|i| match i.value { Value::Int(n) => n as f64, Value::Float(x) => x, _ => f64::NAN }).collect())
    } else if all(|v| matches!(v, Value::Bool(_))) {
        Column::Bool(items.iter().map(|i| matches!(i.value, Value::Bool(true))).collect())
    } else if items.iter().all(|i| matches!(i.value, Value::Str(_))) {
        Column::Str(items.iter().map(|i| if let Value::Str(s) = &i.value { s.clone() } else { String::new() }).collect())
    } else {
        let other = items.iter().find(|i| !matches!(i.value, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) | Value::Moment(_)));
        return Err(format!("expected a column's values, got a list of {}", other.map(named).unwrap_or("mixed values")));
    };
    Ok(Cells::Column(column, crow_runtime::frame::Validity::from(valid)))
}

/// The lists of text of a group, each a list.
pub fn read_lists(group: &[CrowValue]) -> Result<Vec<Vec<String>>, String> {
    group.iter().map(read_strs).collect()
}

/// A list of tensors: a decomposition's parts, a complex number.
pub fn read_tensors(v: &CrowValue) -> Result<Vec<CrowTensor>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(|i| read_tensor(i).map(|t| t.into_owned())).collect(),
        _ => Err(format!("expected a list of tensors, got {}", named(v))),
    }
}

/// A list's items, the interpreter's values (a failed item is kept as the
/// item it is, as the compiled program keeps a list's results).
pub fn read_seq(v: &CrowValue) -> Result<&[CrowValue], String> {
    match &v.value {
        Value::Array(items) => Ok(items.as_slice()),
        _ => Err(format!("expected a list, got {}", named(v))),
    }
}

/// A list the function makes its answer of.
pub fn read_list(v: &CrowValue) -> Result<Vec<CrowValue>, String> {
    read_seq(v).map(|items| items.to_vec())
}

/// A map's entries, the interpreter's values.
pub fn read_map_ref(v: &CrowValue) -> Result<&BTreeMap<String, CrowValue>, String> {
    match &v.value {
        Value::Map(m) => Ok(m),
        _ => Err(format!("expected a map, got {}", named(v))),
    }
}

/// A map's values as text, each as it shows.
pub fn read_text_map(v: &CrowValue) -> Result<BTreeMap<String, String>, String> {
    use crow_runtime::display::CrowDisplay;
    Ok(read_map_ref(v)?.iter().map(|(k, v)| (k.clone(), v.crow_fmt())).collect())
}

/// A map the function makes its answer of.
pub fn read_map_of(v: &CrowValue) -> Result<BTreeMap<String, CrowValue>, String> {
    read_map_ref(v).cloned()
}

/// A list's items, each read by `read`: a failed item is the reason the
/// list has no such items (a missing cell's, as a column's).
fn read_items<T>(v: &CrowValue, what: &str, read: fn(&CrowValue) -> Result<T, String>) -> Result<Vec<T>, String> {
    let items = read_seq(v).map_err(|_| format!("expected {}, got {}", what, named(v)))?;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        if item.is_error() {
            return Err(if item.kind() == "missing" { crow_runtime::stops::missing_cell(i) } else { format!("item {} of the list failed: {}", i, item.reason()) });
        }
        out.push(read(item)?);
    }
    Ok(out)
}

/// A list of ints.
pub fn read_ints(v: &CrowValue) -> Result<Vec<i64>, String> {
    read_items(v, "a list of ints", read_whole)
}

/// A list of floats.
pub fn read_floats(v: &CrowValue) -> Result<Vec<f64>, String> {
    read_items(v, "a list of numbers", read_float)
}

/// A parsed JSON value: the interpreter holds what it parsed to, whose
/// own row it picks, so it reads one only for a row it cannot pick.
/// A JSON argument: a parsed value as it is, borrowed; a value the program
/// built (a list, a map) as its JSON.
pub fn read_json(v: &CrowValue) -> Result<std::borrow::Cow<'_, serde_json::Value>, String> {
    match &v.value {
        Value::Json(j) if v.is_ok() => Ok(std::borrow::Cow::Borrowed(&**j)),
        _ => crate::builtins::crow_to_json("", v).map(std::borrow::Cow::Owned),
    }
}

/// A frame's column: the interpreter holds its cells as a list, whose own
/// row it picks.
/// A frame's column as the interpreter holds it — its cells, a missing one
/// an error — read as the column the runtime's rows take (`len` of a
/// column; it had been refused: "expected a frame's column, got array").
pub fn read_column(v: &CrowValue) -> Result<crow_runtime::frame::CrowColumn, String> {
    match read_cells(v)? {
        crow_runtime::frames::Cells::Column(data, valid) => Ok(crow_runtime::frame::CrowColumn { data: std::sync::Arc::new(data), valid: std::sync::Arc::new(valid) }),
        crow_runtime::frames::Cells::One(_) => Err(format!("expected a frame's column, got {}", named(v))),
    }
}

/// Bytes, as `sys` reads and writes them.
pub fn read_bytes(v: &CrowValue) -> Result<crow_runtime::sys::Bytes, String> {
    match &v.value {
        Value::Bytes(b) => Ok(b.clone()),
        _ => Err(format!("expected bytes, got {}", named(v))),
    }
}

/// A list of bytes values, as a connection's chunks are gathered.
pub fn read_chunks(v: &CrowValue) -> Result<Vec<crow_runtime::sys::Bytes>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(read_bytes).collect(),
        _ => Err(format!("expected a list of bytes, got {}", named(v))),
    }
}

pub fn write_bytes(b: crow_runtime::sys::Bytes) -> CrowValue {
    CrowValue::ok(Value::Bytes(b))
}

/// A connection `sys.open` made.
pub fn read_stream(v: &CrowValue) -> Result<crow_runtime::sys::CrowStream, String> {
    match &v.value {
        Value::Stream(s) => Ok(s.clone()),
        _ => Err(format!("expected a stream, got {}", named(v))),
    }
}

pub fn write_stream(s: crow_runtime::sys::CrowStream) -> CrowValue {
    CrowValue::ok(Value::Stream(s))
}

pub fn read_socket(v: &CrowValue) -> Result<crow_runtime::sys::Socket, String> {
    match &v.value {
        Value::Socket(s) => Ok(s.clone()),
        _ => Err(format!("expected a socket, got {}", named(v))),
    }
}

pub fn write_socket(s: crow_runtime::sys::Socket) -> CrowValue {
    CrowValue::ok(Value::Socket(s))
}

pub fn read_mailer(v: &CrowValue) -> Result<crow_runtime::sys::Mailer, String> {
    match &v.value {
        Value::Mailer(m) => Ok(m.clone()),
        _ => Err(format!("expected a mailer, got {}", named(v))),
    }
}

pub fn write_mailer(m: crow_runtime::sys::Mailer) -> CrowValue {
    CrowValue::ok(Value::Mailer(m))
}

pub fn write_list(items: Vec<CrowValue>) -> CrowValue {
    CrowValue::ok(Value::Array(items))
}

pub fn write_map_of(m: BTreeMap<String, CrowValue>) -> CrowValue {
    CrowValue::ok(Value::Map(m))
}

pub fn write_ints(ns: Vec<i64>) -> CrowValue {
    CrowValue::ok(Value::Array(ns.into_iter().map(write_int).collect()))
}

pub fn write_floats(xs: Vec<f64>) -> CrowValue {
    CrowValue::ok(Value::Array(xs.into_iter().map(write_float).collect()))
}

/// A request's answer as the interpreter's record: its status, body and
/// headers, as the compiled program's `CrowReply` holds them.
pub fn write_reply(r: crow_runtime::sources::Reply) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(r.status));
    m.insert("body".to_string(), write_str(r.body));
    let headers: BTreeMap<String, CrowValue> = r.headers.into_iter().map(|(k, v)| (k, write_str(v))).collect();
    m.insert("headers".to_string(), CrowValue::ok(Value::Map(headers)));
    CrowValue::ok(Value::Map(m))
}

/// A connection's answer as the interpreter's record: `status`, `phrase`,
/// `headers`, `body` (bytes).
pub fn write_answer(a: crow_runtime::sys::Answer) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(a.status));
    m.insert("phrase".to_string(), write_str(a.phrase));
    let headers: BTreeMap<String, CrowValue> = a.headers.into_iter().map(|(k, v)| (k, write_str(v))).collect();
    m.insert("headers".to_string(), CrowValue::ok(Value::Map(headers)));
    m.insert("body".to_string(), write_bytes(a.body));
    CrowValue::ok(Value::Map(m))
}

/// A command's answer as the interpreter's record: `status`, `stdout`,
/// `stderr`.
pub fn write_response(r: crow_web::core::CrowResponse) -> CrowValue {
    CrowValue::ok(Value::Response(Box::new(r)))
}

pub fn write_shell(sh: crow_runtime::core::Shell) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(sh.status));
    m.insert("stdout".to_string(), write_str(sh.stdout));
    m.insert("stderr".to_string(), write_str(sh.stderr));
    CrowValue::ok(Value::Map(m))
}

/// A parsed JSON value as the interpreter holds it: arrays, maps and
/// scalars, a `null` inside an error value of reason `null`.
/// A JSON answer, kept as the compiled program keeps it.
pub fn write_json(v: serde_json::Value) -> CrowValue {
    CrowValue::ok(Value::Json(Box::new(v)))
}

pub fn write_json_list(vs: Vec<serde_json::Value>) -> CrowValue {
    CrowValue::ok(Value::Array(vs.into_iter().map(write_json).collect()))
}

pub fn write_markup(s: String) -> CrowValue {
    CrowValue::markup(s)
}

pub fn write_text_rows(rows: Vec<BTreeMap<String, String>>) -> CrowValue {
    CrowValue::ok(Value::Array(rows.into_iter().map(|r| CrowValue::ok(Value::Map(r.into_iter().map(|(k, v)| (k, write_str(v))).collect()))).collect()))
}

/// One argument of a group, the group of a single parameter.
pub fn one(group: &[CrowValue]) -> Result<&CrowValue, String> {
    group.first().ok_or_else(|| "an argument is missing".to_string())
}

/// One argument of a group, to keep: taken out of the arguments.
pub fn one_taken(group: &mut [CrowValue]) -> Result<CrowValue, String> {
    group.first_mut().map(|v| std::mem::replace(v, CrowValue::done())).ok_or_else(|| "an argument is missing".to_string())
}

/// A list the function makes its answer of, moved.
pub fn take_list(v: CrowValue) -> Result<Vec<CrowValue>, String> {
    match v.value {
        Value::Array(items) => Ok(items),
        other => Err(format!("expected a list, got {}", other.type_name())),
    }
}

/// A map the function makes its answer of, moved.
pub fn take_map(v: CrowValue) -> Result<BTreeMap<String, CrowValue>, String> {
    match v.value {
        Value::Map(m) => Ok(m),
        other => Err(format!("expected a map, got {}", other.type_name())),
    }
}

/// The sizes of a group, each an int.
pub fn read_sizes(group: &[CrowValue]) -> Result<Vec<i64>, String> {
    group.iter().map(read_int).collect()
}

/// A fused chain's operands: each a tensor, or a number.
pub fn read_operands(group: &[CrowValue]) -> Result<Vec<crow_runtime::fused::Operand<'_>>, String> {
    group
        .iter()
        .map(|v| match &v.value {
            Value::NumArray(t) => Ok(crow_runtime::fused::Operand::Tensor(t.as_ref())),
            Value::Int(n) => Ok(crow_runtime::fused::Operand::Number(*n as f64)),
            Value::Float(x) => Ok(crow_runtime::fused::Operand::Number(*x)),
            _ => Err(format!("expected a tensor or a number, got {}", named(v))),
        })
        .collect()
}

/// A fused filter's values: each a number, text or a bool.
pub fn read_scalars(group: &[CrowValue]) -> Result<Vec<crow_runtime::mask::Cell<'_>>, String> {
    use crow_runtime::mask::Cell;
    group
        .iter()
        .map(|v| match &v.value {
            Value::Int(n) => Ok(Cell::Number(*n as f64)),
            Value::Float(x) => Ok(Cell::Number(*x)),
            Value::Str(s) => Ok(Cell::Text(s)),
            Value::Bool(b) => Ok(Cell::Bool(*b)),
            _ => Err(format!("expected a number, text or a bool, got {}", named(v))),
        })
        .collect()
}

/// The reading of a parameter's group by its kind, as the function takes it.
macro_rules! read {
    (Int, $g:expr) => { read_int(one($g)?)? };
    (Whole, $g:expr) => { read_whole(one($g)?)? };
    (Words, $g:expr) => { &read_words(one($g)?)? };
    (Float, $g:expr) => { read_float(one($g)?)? };
    (Moment, $g:expr) => { read_moment(one($g)?)? };
    (Bool, $g:expr) => { read_bool(one($g)?)? };
    (Str, $g:expr) => { read_str(one($g)?)? };
    (Tensor, $g:expr) => { &*read_tensor(one($g)?)? };
    (Tensors, $g:expr) => { &read_tensors(one($g)?)? };
    (Sparse, $g:expr) => { read_sparse(one($g)?)? };
    (Frame, $g:expr) => { read_frame(one($g)?)? };
    (Strs, $g:expr) => { &read_strs(one($g)?)? };
    (Chunks, $g:expr) => { &read_chunks(one($g)?)? };
    (Sizes, $g:expr) => { &read_sizes($g)? };
    (Bools, $g:expr) => { &read_bools(one($g)?)? };
    (Cell, $g:expr) => { &read_cell(one($g)?)? };
    (Cells, $g:expr) => { read_cells(one($g)?)? };
    (Lists, $g:expr) => { &read_lists($g)?.iter().map(|l| l.as_slice()).collect::<Vec<&[String]>>() };
    (Operands, $g:expr) => { &read_operands($g)? };
    (Scalars, $g:expr) => { &read_scalars($g)? };
    (Shows, $g:expr) => { &$g.iter().map(|v| v as &dyn crow_runtime::display::CrowDisplay).collect::<Vec<&dyn crow_runtime::display::CrowDisplay>>() };
    (List, $g:expr) => { read_list(one($g)?)? };
    (Seq, $g:expr) => { read_seq(one($g)?)? };
    (Item, $g:expr) => { one($g)?.clone() };
    (ItemRef, $g:expr) => { one($g)? };
    (MapOf, $g:expr) => { read_map_of(one($g)?)? };
    (MapRef, $g:expr) => { read_map_ref(one($g)?)? };
    (TextMap, $g:expr) => { &read_text_map(one($g)?)? };
    (Ints, $g:expr) => { read_ints(one($g)?)? };
    (Floats, $g:expr) => { read_floats(one($g)?)? };
    (Texts, $g:expr) => { read_strs(one($g)?)? };
    (Json, $g:expr) => { &*read_json(one($g)?)? };
    (Column, $g:expr) => { &read_column(one($g)?)? };
    (Bytes, $g:expr) => { &read_bytes(one($g)?)? };
    (Stream, $g:expr) => { &read_stream(one($g)?)? };
    (Socket, $g:expr) => { &read_socket(one($g)?)? };
    (Mailer, $g:expr) => { &read_mailer(one($g)?)? };
    (Shown, $g:expr) => { one($g)? };
    (Encoded, $g:expr) => { one($g)? };
}

/// The value of an answer by its kind.
macro_rules! answer {
    (Int, $x:expr) => { write_int($x) };
    (Float, $x:expr) => { write_float($x) };
    (Moment, $x:expr) => { write_moment($x) };
    (Bool, $x:expr) => { write_bool($x) };
    (Str, $x:expr) => { write_str($x) };
    (Tensor, $x:expr) => { write_tensor($x) };
    (Frame, $x:expr) => { write_frame($x) };
    (Column, $x:expr) => { write_column($x) };
    (Bytes, $x:expr) => { write_bytes($x) };
    (Stream, $x:expr) => { write_stream($x) };
    (Strs, $x:expr) => { write_strs($x) };
    (Bools, $x:expr) => { write_bools($x) };
    (Tensors, $x:expr) => { write_tensors($x) };
    (Unit, $x:expr) => { write_unit($x) };
    (Sparse, $x:expr) => { write_sparse($x) };
    (List, $x:expr) => { write_list($x) };
    (MapOf, $x:expr) => { write_map_of($x) };
    (Ints, $x:expr) => { write_ints($x) };
    (Floats, $x:expr) => { write_floats($x) };
    (Texts, $x:expr) => { write_strs($x) };
    (Markup, $x:expr) => { write_markup($x) };
    (TextRows, $x:expr) => { write_text_rows($x) };
    (Shell, $x:expr) => { write_shell($x) };
    (Reply, $x:expr) => { write_reply($x) };
    (Answer, $x:expr) => { write_answer($x) };
    (Response, $x:expr) => { write_response($x) };
    (Socket, $x:expr) => { write_socket($x) };
    (Mailer, $x:expr) => { write_mailer($x) };
    (Json, $x:expr) => { write_json($x) };
    (JsonList, $x:expr) => { write_json_list($x) };
}

/// The taking of a parameter's group by its kind: a collection the
/// function makes its answer of is moved out of the arguments; any other
/// kind is read as [`read!`] reads it.
macro_rules! take {
    (List, $g:expr) => { take_list(one_taken($g)?)? };
    (MapOf, $g:expr) => { take_map(one_taken($g)?)? };
    (Item, $g:expr) => { one_taken($g)? };
    ($k:ident, $g:expr) => { read!($k, $g) };
}

/// What a failure is, as the row macro writes it.
macro_rules! fails {
    (never) => { Fails::Never };
    (stops) => { Fails::Stops };
    (value $kind:literal) => { Fails::Value($kind) };
}

/// The function's answer as the row says: its value, its reason as a stop
/// (the call's `Err`), or an error value of the row's kind.
macro_rules! respond {
    ($r:ident, $call:expr, $trace:expr, never) => { Ok(answer!($r, $call)) };
    ($r:ident, $call:expr, $trace:expr, stops) => {{
        let got = $call?;
        Ok(answer!($r, got))
    }};
    ($r:ident, $call:expr, $trace:expr, value $kind:literal) => {
        Ok(match $call {
            Ok(got) => answer!($r, got),
            Err(e) => CrowValue::error_with_kind(&e, $kind, $trace),
        })
    };
}

/// A row: its id, Crow name and module, the runtime function, the kinds of
/// its parameters and answer, and what a failure is (`never`, `stops`, or
/// `value "kind"`). The interpreter's call is made from the same tokens the
/// emitter prints, so the two cannot name different functions.
/// The path of a row whose body is Crow (`crow!`).
pub const IN_CROW: &str = "crow";

/// The name of the program's function a Crow row's body is inlined as:
/// the module's path and the name, behind an underscore no program
/// spells (`_math_nn_relu`); `crate::modules::inline` makes them.
pub fn crow_fn_name(row: &Row) -> String {
    format!("_{}_{}", crow_prefix_of(row.module), crow_fn_base(row))
}

/// The prefix a module's functions carry when inlined: the module's path,
/// the core's being `core`.
pub fn crow_prefix_of(module: &str) -> String {
    let module = if module.is_empty() { "core" } else { module };
    module.replace('.', "_")
}

/// The name of the function a Crow row's body is, in the module's source:
/// the row's id after its dot (`read` and `read_asked` share the name
/// `read`, and are two functions).
pub fn crow_fn_base(row: &Row) -> &str {
    row.id.rsplit('.').next().unwrap_or(row.name)
}

/// The Crow row of a module's name that takes these kinds, when it is one.
pub fn crow_row(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    self::module(module, name, args).filter(|r| r.is_crow() && takes(r, args)).or_else(|| {
        // a core row a module's name reaches too (`io.read` is the core's
        // `read`): the row whose id the module prefixes
        let prefix = format!("{}.", module);
        (!module.is_empty()).then(|| ROWS.iter().find(|r| r.module.is_empty() && r.name == name && r.id.starts_with(&prefix) && r.is_crow() && takes(r, args))).flatten()
    })
}

fn written_in_crow(name: &str) -> String {
    format!("a bug in Crow: `{}` is written in Crow, and was called as a runtime function", name)
}

/// A value as a Crow row's parameter takes it, for the interpreter to hand
/// the row's function a value nothing typed before the program ran: the
/// kinds the standard library's Crow is written over.
pub fn value_as(kind: Kind, v: &CrowValue) -> Result<CrowValue, String> {
    Ok(match kind {
        Kind::Tensor => CrowValue::ok(Value::NumArray(Box::new(read_tensor(v)?.into_owned()))),
        Kind::Float => CrowValue::ok(Value::Float(read_float(v)?)),
        Kind::Moment => CrowValue::ok(Value::Moment(read_moment(v)?)),
        Kind::Int | Kind::Whole => CrowValue::ok(Value::Int(read_int(v)?)),
        Kind::Str => CrowValue::ok(Value::Str(read_str(v)?.to_string())),
        Kind::Bool => CrowValue::ok(Value::Bool(read_bool(v)?)),
        Kind::Bytes => CrowValue::ok(Value::Bytes(read_bytes(v)?)),
        Kind::Chunks => CrowValue::ok(Value::Array(read_chunks(v)?.into_iter().map(write_bytes).collect())),
        Kind::TextMap => CrowValue::ok(Value::Map(read_text_map(v)?.into_iter().map(|(k, t)| (k, CrowValue::ok(Value::Str(t)))).collect())),
        Kind::Frame => CrowValue::ok(Value::Frame(Box::new(read_frame(v)?.clone()))),
        Kind::Strs => CrowValue::ok(Value::Array(read_strs(v)?.into_iter().map(|t| CrowValue::ok(Value::Str(t))).collect())),
        // a value JSON can hold reaches the function as its JSON text
        // (`crow_param_type`): the one form a map, a record or a list of
        // either takes in Crow
        Kind::Encoded => CrowValue::ok(Value::Str(crate::builtins::crow_to_encoded(v).to_string())),
        Kind::Socket => CrowValue::ok(Value::Socket(read_socket(v)?)),
        Kind::Mailer => CrowValue::ok(Value::Mailer(read_mailer(v)?)),
        other => return Err(format!("a bug in Crow: a builtin written in Crow takes {} here, which the runtime cannot hand it", other.words())),
    })
}

/// A Crow row's arguments as its function takes them, for the interpreter:
/// each read as the row's parameter takes it (`value_as`), and a parameter
/// that takes the rest of the arguments (`anova[a, b, c]`) handed them as
/// one list.
pub fn crow_arguments(row: &Row, vals: &[CrowValue]) -> Result<Vec<CrowValue>, String> {
    let spans = spans(row.params, vals.len(), row.name)?;
    let mut handed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans) {
        if k.is_rest() {
            let items: Vec<CrowValue> = vals[a..b].iter().map(|v| value_as(k.each(), v)).collect::<Result<_, _>>()?;
            handed.push(CrowValue::ok(Value::Array(items)));
        } else {
            handed.push(value_as(*k, &vals[a])?);
        }
    }
    Ok(handed)
}

/// The type a parameter of a row written in Crow has, from the row's kind:
/// the kind's own, except that a value JSON can hold (`Encoded`: a map, a
/// record, a list of either) reaches the function as its JSON text — the
/// one form every such value takes in Crow, read back with `json[…]`. The
/// lowering hands the call `encode[value]` (`crow_call_args`) and the
/// interpreter the same (`value_as`).
pub fn crow_param_type(k: Kind) -> Ty {
    match k {
        Kind::Encoded => Ty::Str,
        other => other.ty(),
    }
}

/// A row whose body is a function of the standard library written in Crow
/// (src/modules/*.co; design rule 20): the table states its contract — what
/// it takes, what it answers, how it fails — and both backends call the
/// function the module's source is inlined as (`crow_fn_name`). The
/// interpreter's closures here are never reached: a call whose kinds the
/// lowering did not know picks the row as the program runs and hands the
/// function its values through `value_as`.
macro_rules! crow {
    ($id:literal, $module:literal, $name:literal, ($($k:ident),*) -> $r:ident, $($fails:tt)+) => {
        Row {
            id: $id, name: $name, module: $module, path: IN_CROW,
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: fails!($($fails)+),
            call: |_args: &[CrowValue], _trace: &str| Err(written_in_crow($name)),
            take: |_args: Vec<CrowValue>, _trace: &str| Err(written_in_crow($name)),
        }
    };
}

macro_rules! row {
    ($id:literal, $module:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, $($fails:tt)+) => {
        Row {
            id: $id, name: $name, module: $module, path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: fails!($($fails)+),
            call: |args: &[CrowValue], trace: &str| {
                let _ = trace;
                let groups = groups(&[$(Kind::$k),*], args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&[]);
                let _ = &mut next;
                respond!($r, $f($(read!($k, next())),*), trace, $($fails)+)
            },
            take: |mut args: Vec<CrowValue>, trace: &str| {
                let _ = trace;
                let groups = groups_mut(&[$(Kind::$k),*], &mut args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&mut []);
                let _ = &mut next;
                respond!($r, $f($(take!($k, next())),*), trace, $($fails)+)
            },
        }
    };
}

/// The rows. A builtin here is written once, in the runtime; its old arms
/// in `builtins.rs` and `emit_rs/builtins.rs` are gone.
pub static ROWS: &[Row] = &[
    // text: one function each, as both modes already called; a count or a
    // kind no row takes is refused before the program runs
    row!("text.split", "", "split", crow_runtime::text::split, (Str, Str) -> Strs, never),
    row!("text.trim", "", "trim", crow_runtime::text::trim, (Str) -> Str, never),
    row!("text.replace", "", "replace", crow_runtime::text::replace, (Str, Str, Str) -> Str, never),
    row!("text.upper", "", "upper", crow_runtime::text::upper, (Str) -> Str, never),
    row!("text.lower", "", "lower", crow_runtime::text::lower, (Str) -> Str, never),
    row!("text.starts", "", "starts", crow_runtime::text::starts, (Str, Str) -> Bool, never),
    row!("text.ends", "", "ends", crow_runtime::text::ends, (Str, Str) -> Bool, never),
    row!("core.join", "", "join", crow_runtime::core::join_words, (Words, Str) -> Str, never),
    // collections: one generic function each over a list's or a map's
    // items, then text's and a tensor's own, a name's rows in the order
    // they are tried (a list reaches its own row before a tensor's). A kind
    // no row takes is refused before the program runs: compiled, `has` of a
    // list and `len` of a number did not build, and `contains` of a record
    // did not compare.
    row!("core.len", "", "len", crow_runtime::core::len, (Seq) -> Int, never),
    row!("core.len_text", "", "len", crow_runtime::core::len_text, (Str) -> Int, never),
    row!("core.len_map", "", "len", crow_runtime::core::len_map, (MapRef) -> Int, never),
    row!("core.len_column", "", "len", crow_runtime::core::len_column, (Column) -> Int, never),
    row!("core.len_tensor", "", "len", crow_runtime::core::len_tensor, (Tensor) -> Int, never),
    row!("core.len_frame", "", "len", crow_runtime::core::len_frame, (Frame) -> Int, never),
    // a JSON value's kind shows as the program runs: of a number or a bool,
    // the refusal the interpreter gives the value it parsed to (compiled,
    // its length was 0)
    row!("core.len_json", "", "len", crow_runtime::core::len_json, (Json) -> Int, stops),
    row!("core.len_bytes", "", "len", crow_runtime::sys::len_bytes, (Bytes) -> Int, never),
    // an item compared as `eq` compares it: a record field by field, a map
    // entry by entry
    row!("core.contains", "", "contains", crow_runtime::core::contains, (Seq, ItemRef) -> Bool, never),
    row!("core.contains_text", "", "contains", crow_runtime::core::contains_text, (Str, Str) -> Bool, never),
    row!("core.contains_number", "", "contains", crow_runtime::core::contains_number, (Tensor, Float) -> Bool, never),
    row!("core.slice", "", "slice", crow_runtime::core::slice, (Seq, Int, Int) -> List, never),
    row!("core.slice_str", "", "slice", crow_runtime::core::slice_str, (Str, Int, Int) -> Str, never),
    row!("core.slice_tensor", "", "slice", crow_runtime::core::slice_tensor, (Tensor, Int, Int) -> Tensor, never),
    row!("core.push", "", "push", crow_runtime::core::push, (List, Item) -> List, never),
    row!("core.push_row", "", "push", crow_runtime::core::push_row, (Tensor, Tensor) -> Tensor, stops),
    row!("core.push_number", "", "push", crow_runtime::core::push_number, (Tensor, Float) -> Tensor, stops),
    row!("core.pop", "", "pop", crow_runtime::core::pop, (List) -> List, never),
    row!("core.pop_tensor", "", "pop", crow_runtime::core::pop_tensor, (Tensor) -> Tensor, never),
    // numbers ascending with NaN last, text by its characters; a list of
    // anything else has no order (a record's was the interpreter's
    // unchanged list, and the compiled program's missing Ord)
    row!("core.sort_ints", "", "sort", crow_runtime::core::sort_ints, (Ints) -> Ints, never),
    row!("core.sort_floats", "", "sort", crow_runtime::core::sort_floats, (Floats) -> Floats, never),
    row!("core.sort_strs", "", "sort", crow_runtime::core::sort_strs, (Texts) -> Texts, never),
    row!("core.sort_tensor", "", "sort", crow_runtime::core::sort_tensor, (Tensor) -> Tensor, never),
    row!("core.reverse", "", "reverse", crow_runtime::core::reverse, (List) -> List, never),
    row!("core.reverse_text", "", "reverse", crow_runtime::core::reverse_text, (Str) -> Str, never),
    row!("core.reverse_tensor", "", "reverse", crow_runtime::core::reverse_tensor, (Tensor) -> Tensor, never),
    row!("core.concat", "", "concat", crow_runtime::core::concat, (List, Seq) -> List, never),
    row!("core.concat_text", "", "concat", crow_runtime::core::concat_text, (Str, Str) -> Str, never),
    row!("core.concat_tensors", "", "concat", crow_runtime::core::concat_tensors, (Tensor, Tensor) -> Tensor, stops),
    row!("core.concat_axis", "", "concat", crow_runtime::core::concat_axis, (Tensor, Tensor, Int) -> Tensor, stops),
    // ints only (compiled, a float was cut to an int where the interpreter
    // refused it)
    row!("core.range_to", "", "range", crow_runtime::core::range_to, (Whole) -> Ints, stops),
    row!("core.range_from", "", "range", crow_runtime::core::range_from, (Whole, Whole) -> Ints, stops),
    row!("core.range", "", "range", crow_runtime::core::range, (Whole, Whole, Whole) -> Ints, stops),
    row!("core.keys", "", "keys", crow_runtime::core::keys, (MapRef) -> Strs, never),
    row!("core.values", "", "values", crow_runtime::core::values, (MapRef) -> List, never),
    row!("core.has", "", "has", crow_runtime::core::has, (MapRef, Str) -> Bool, never),
    row!("core.set", "", "set", crow_runtime::core::set, (MapOf, Str, Item) -> MapOf, never),
    row!("core.del", "", "del", crow_runtime::core::map_del, (MapOf, Str) -> MapOf, never),
    // a parsed JSON object's keys, values and entries: the compiled program
    // holds the parsed value, the interpreter what it parsed to, whose map
    // rows it picks (compiled, these did not build)
    row!("json.keys", "", "keys", crow_runtime::json::keys, (Json) -> Strs, stops),
    row!("json.values", "", "values", crow_runtime::json::values, (Json) -> JsonList, stops),
    row!("json.has", "", "has", crow_runtime::json::has, (Json, Str) -> Bool, stops),
    // a value's display and its JSON: one display (crow_runtime::display)
    // and one encoding (crow_runtime::json::ToJson) that both modes' values
    // follow — the compiled program had shown a record holding a list, or a
    // list of records, in Rust's debug form, and `str` of a JSON list as its
    // JSON text; the interpreter had refused to encode a record
    // (a column's cells as text, before the form that shows any value as text)
    row!("frame.str_column", "", "str", crow_runtime::frames::str_column, (Column) -> Column, stops),
    row!("display.str", "", "str", crow_runtime::display::shown, (Shown) -> Str, never),
    row!("json.parse", "", "json", crow_runtime::json::parse, (Str) -> Json, value "parse"),
    row!("json.encode", "", "encode", crow_runtime::json::encode, (Encoded) -> Str, never),
    // an element-wise chain of tensors in one pass, which the lowering writes
    // in place of the chain's operators (lower_ir/fuse.rs): a shape that does
    // not meet stops with the operator's words
    row!("tensor.fused", "", "__fused", crow_runtime::fused::run, (Str, Operands) -> Tensor, stops),
    // text written so a header or a URL can carry it, and read back: a
    // scheme encode does not know is a misuse; text decode cannot read is
    // an error value
    row!("text.encode", "", "encode", crow_runtime::text::encode_as, (Str, Str) -> Str, stops),
    row!("text.decode", "", "decode", crow_runtime::text::decode_as, (Str, Str) -> Str, value "parse"),
    // log: the values as they show, one display for both modes (compiled,
    // an error without a value among several did not build)
    row!("display.log", "", "log", crow_runtime::display::log, (Shows) -> Unit, never),
    row!("core.map_new", "", "map", crow_runtime::core::map_new, () -> MapOf, never),
    // a plain number's math (on the dual numbers a function grad
    // differentiates, each keeps the derivative's own form)
    row!("core.abs_int", "", "abs", crow_runtime::core::abs_int, (Whole) -> Int, stops),
    row!("core.abs", "", "abs", crow_runtime::core::abs_float, (Float) -> Float, never),
    row!("core.sqrt", "", "sqrt", crow_runtime::core::sqrt, (Float) -> Float, never),
    row!("core.floor_int", "", "floor", crow_runtime::core::whole, (Whole) -> Int, never),
    row!("core.floor", "", "floor", crow_runtime::core::floor, (Float) -> Int, never),
    row!("core.ceil_int", "", "ceil", crow_runtime::core::whole, (Whole) -> Int, never),
    row!("core.ceil", "", "ceil", crow_runtime::core::ceil, (Float) -> Int, never),
    row!("core.round_int", "", "round", crow_runtime::core::whole, (Whole) -> Int, never),
    row!("core.round", "", "round", crow_runtime::core::round, (Float) -> Int, never),
    // the program and its system: effects, which the session layer
    // performs, and whose call inside is the row (compiled, env's default
    // had been text of any value, and a negative sleep panicked or slept
    // forever in both modes)
    row!("core.args", "", "args", crow_runtime::core::args, () -> Strs, never),
    row!("core.env", "", "env", crow_runtime::core::env, (Str) -> Str, value "env"),
    row!("core.env_or", "", "env", crow_runtime::core::env_or, (Str, Cell) -> Str, value "env"),
    row!("core.input", "", "input", crow_runtime::core::input, () -> Str, value "io"),
    row!("core.input_prompt", "", "input", crow_runtime::core::input_prompt, (Str) -> Str, value "io"),
    row!("core.exit", "", "exit", crow_runtime::core::exit, () -> Unit, never),
    row!("core.exit_code", "", "exit", crow_runtime::core::exit_code, (Whole) -> Unit, never),
    row!("core.halt", "", "halt", crow_runtime::core::halt, (Str) -> Unit, stops),
    row!("core.sleep_ms", "", "sleep", crow_runtime::core::sleep_ms, (Whole) -> Unit, stops),
    row!("core.sleep", "", "sleep", crow_runtime::core::sleep_secs, (Float) -> Unit, stops),
    row!("moments.now", "", "now", crow_runtime::moments::now_moment, () -> Moment, never),
    // the file system: an effect, which the session layer performs (held to
    // a handler's commit, recorded in a rehearsal) and whose call inside is
    // the row; a failure is an error value of kind 'io', a success of a
    // write empty text, as its type says (it had been `done` interpreted)
    crow!("io.read", "", "read", (Str) -> Str, value "io"),
    // a URL read with options: `read[url, headers = map[…], query = map[…],
    // timeout = s]` (the lowering fills in what the call leaves out)
    crow!("io.read_asked", "", "read", (Str, TextMap, TextMap, Float) -> Str, value "io"),
    crow!("io.write", "", "write", (Str, Str) -> Str, value "io"),
    row!("io.exists", "", "exists", crow_runtime::io::exists, (Str) -> Bool, never),
    row!("io.mkdir", "", "mkdir", crow_runtime::io::mkdir, (Str) -> Str, value "io"),
    row!("io.ls", "", "ls", crow_runtime::io::ls, (Str) -> Strs, value "io"),
    row!("io.rm", "", "rm", crow_runtime::io::rm, (Str) -> Str, value "io"),
    row!("io.mv", "", "mv", crow_runtime::io::mv, (Str, Str) -> Str, value "io"),
    row!("io.cp", "", "cp", crow_runtime::io::cp, (Str, Str) -> Str, value "io"),
    // a command that cannot start is an error value of kind 'shell'; a
    // non-zero exit is an answer, read from `.status`
    row!("core.shell", "", "shell", crow_runtime::core::shell, (Str) -> Shell, value "shell"),
    // an HTTP request, answered whatever its status: its URL, method and
    // body, then its headers, query, form, files, save and timeout (the
    // lowering fills in what the call leaves out: `request[url]` is a GET)
    // written in Crow (src/modules/core.co) on the sys layer
    crow!("core.request", "", "request", (Str, Str, Str, TextMap, TextMap, TextMap, TextMap, Str, Float) -> Reply, value "http"),
    // min and max of numbers (text was compared compiled and refused
    // interpreted: min['a', 'b'] is refused before the program runs)
    row!("core.min_ints", "", "min", crow_runtime::core::min_ints, (Whole, Whole) -> Int, never),
    row!("core.min", "", "min", crow_runtime::core::min_floats, (Float, Float) -> Float, never),
    row!("core.max_ints", "", "max", crow_runtime::core::max_ints, (Whole, Whole) -> Int, never),
    row!("core.max", "", "max", crow_runtime::core::max_floats, (Float, Float) -> Float, never),
    row!("hash.hash", "", "hash", crow_runtime::hash::hash, (Str, Str) -> Str, value "hash"),
    // fmt: a number's or a text's options, or a moment's time style, which
    // can fail (kind 'time'; compiled, the failure was the text "[fmt: …]")
    row!("display.fmt", "", "fmt", crow_runtime::display::fmt_value, (Cell, Int, Bool, Int, Str, Str, Bool) -> Str, stops),
    row!("display.fmt_time", "", "__fmt_time", crow_runtime::moments::fmt_time_of, (Moment, Str, Str) -> Str, value "time"),
    // conversions: of a number or a bool, plain; of text or a JSON value,
    // fallible (kind 'parse'), read by one rule (compiled, JSON text was
    // trimmed and a JSON bool refused)
    row!("core.int", "", "int", crow_runtime::cell::int_of_int, (Whole) -> Int, never),
    row!("core.int_of_float", "", "int", crow_runtime::cell::int_of_float, (Float) -> Int, never),
    row!("core.int_of_bool", "", "int", crow_runtime::cell::int_of_bool, (Bool) -> Int, never),
    row!("core.int_of_moment", "", "int", crow_runtime::cell::int_of_moment, (Moment) -> Int, never),
    row!("core.int_of", "", "int", crow_runtime::cell::int_of, (Cell) -> Int, value "parse"),
    row!("core.float_of_int", "", "float", crow_runtime::cell::float_of_int, (Whole) -> Float, never),
    row!("core.float", "", "float", crow_runtime::cell::float_of_float, (Float) -> Float, never),
    row!("core.float_of_bool", "", "float", crow_runtime::cell::float_of_bool, (Bool) -> Float, never),
    row!("core.float_of_moment", "", "float", crow_runtime::cell::float_of_moment, (Moment) -> Float, never),
    row!("core.float_of", "", "float", crow_runtime::cell::float_of, (Cell) -> Float, value "parse"),
    // draws from the program's generator
    row!("core.rand", "", "rand", crow_runtime::core::rand_float, () -> Float, never),
    row!("core.rand_int", "", "rand", crow_runtime::core::rand_int, (Int) -> Int, stops),
    row!("core.rand_between", "", "rand", crow_runtime::core::rand_between, (Int, Int) -> Int, stops),
    row!("moments.moment_text", "", "moment", crow_runtime::moments::moment_text, (Str) -> Moment, value "time"),
    row!("moments.moment_in", "", "moment", crow_runtime::moments::moment_in, (Moment, Str) -> Moment, value "time"),
    row!("moments.moment_text_in", "", "moment", crow_runtime::moments::moment_text_in, (Str, Str) -> Moment, value "time"),
    row!("moments.moment_of_seconds", "", "moment", crow_runtime::moments::moment_of_seconds, (Float) -> Moment, never),
    row!("moments.moment_self", "", "moment", crow_runtime::moments::moment_self, (Moment) -> Moment, never),
    row!("moments.bucket", "", "bucket", crow_runtime::moments::bucket, (Moment, Float) -> Moment, stops),
    row!("moments.min", "", "min", crow_runtime::moments::min_of, (Moment, Moment) -> Moment, never),
    row!("moments.max", "", "max", crow_runtime::moments::max_of, (Moment, Moment) -> Moment, never),
    row!("moments.moment", "", "moment", crow_runtime::moments::moment_parts, (Int, Int, Int, Int, Int, Float, Str) -> Moment, value "time"),
    row!("text.repeat", "", "repeat", crow_runtime::text::repeat, (Str, Int) -> Str, stops),
    // an invalid pattern is an error value of kind 'regex', as find[]'s is
    // (the interpreter answered one while the compiled program stopped)
    row!("text.matches", "", "matches", crow_runtime::text::matches, (Str, Str) -> Bool, value "regex"),
    row!("text.sub", "", "sub", crow_runtime::text::sub, (Str, Str, Str) -> Str, value "regex"),
    // math: the constructors and the distributions (a size, a parameter, a
    // rank the interpreter refused and the compiled program cast or
    // panicked on is one check)
    row!("math.zeros", "math", "zeros", crow_runtime::math::zeros, (Sizes) -> Tensor, stops),
    row!("math.ones", "math", "ones", crow_runtime::math::ones, (Sizes) -> Tensor, stops),
    crow!("math.randn", "math", "randn", (Sizes) -> Tensor, stops),
    row!("math.full", "math", "full", crow_runtime::math::full, (Sizes, Float) -> Tensor, stops),
    row!("math.eye", "math", "eye", crow_runtime::math::eye, (Int) -> Tensor, stops),
    row!("math.onehot", "math", "onehot", crow_runtime::math::onehot, (Tensor, Int) -> Tensor, stops),
    row!("math.linspace", "math", "linspace", crow_runtime::math::linspace, (Float, Float, Int) -> Tensor, stops),
    row!("math.arange", "math", "arange", crow_runtime::math::arange, (Float, Float, Float) -> Tensor, stops),
    row!("math.diag", "math", "diag", crow_runtime::math::diag, (Tensor) -> Tensor, stops),
    crow!("math.normal", "math", "normal", (Float, Float, Sizes) -> Tensor, stops),
    row!("math.uniform", "math", "uniform", crow_runtime::math::uniform, (Float, Float, Sizes) -> Tensor, stops),
    row!("math.poisson", "math", "poisson", crow_runtime::math::poisson, (Float, Sizes) -> Tensor, stops),
    row!("math.bernoulli", "math", "bernoulli", crow_runtime::math::bernoulli, (Float, Sizes) -> Tensor, stops),
    row!("math.gamma_dist", "math", "gamma_dist", crow_runtime::math::gamma_dist, (Float, Float, Sizes) -> Tensor, stops),
    row!("math.beta_dist", "math", "beta_dist", crow_runtime::math::beta_dist, (Float, Float, Sizes) -> Tensor, stops),
    row!("math.exponential", "math", "exponential", crow_runtime::math::exponential, (Float, Sizes) -> Tensor, stops),
    // math: linear algebra (a vector, a rank-3 tensor or a rectangle where a
    // square is needed, a matrix that is not positive definite: the reason,
    // where a kernel panicked)
    row!("math.det", "math", "det", crow_runtime::math::det, (Tensor) -> Float, stops),
    row!("math.inv", "math", "inv", crow_runtime::math::inv, (Tensor) -> Tensor, stops),
    row!("math.svd", "math", "svd", crow_runtime::math::svd, (Tensor) -> Tensors, stops),
    row!("math.qr", "math", "qr", crow_runtime::math::qr, (Tensor) -> Tensors, stops),
    row!("math.cholesky", "math", "cholesky", crow_runtime::math::cholesky, (Tensor) -> Tensor, stops),
    row!("math.eigen", "math", "eigen", crow_runtime::math::eigen, (Tensor) -> Tensors, stops),
    row!("math.norm", "math", "norm", crow_runtime::math::norm, (Tensor) -> Float, never),
    row!("math.norm_p", "math", "norm", crow_runtime::math::norm_p, (Tensor, Float) -> Float, never),
    row!("math.dot", "math", "dot", crow_runtime::math::dot, (Tensor, Tensor) -> Float, stops),
    row!("math.pinv", "math", "pinv", crow_runtime::math::pinv, (Tensor) -> Tensor, stops),
    row!("math.lstsq", "math", "lstsq", crow_runtime::math::lstsq, (Tensor, Tensor) -> Tensor, stops),
    row!("math.triu", "math", "triu", crow_runtime::math::triu, (Tensor) -> Tensor, stops),
    row!("math.tril", "math", "tril", crow_runtime::math::tril, (Tensor) -> Tensor, stops),
    row!("math.rank", "math", "rank", crow_runtime::math::rank, (Tensor) -> Int, stops),
    row!("math.cond", "math", "cond", crow_runtime::math::cond, (Tensor) -> Float, stops),
    // math: reductions (an axis out of range is the reason), sorting and
    // selection, shapes (a reshape that does not fill, a transpose of a
    // vector: the reason, where a kernel asserted)
    row!("math.sum", "math", "sum", crow_runtime::math::sum, (Tensor) -> Float, never),
    row!("math.sum_axis", "math", "sum", crow_runtime::math::sum_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.mean", "math", "mean", crow_runtime::math::mean, (Tensor) -> Float, never),
    row!("math.mean_axis", "math", "mean", crow_runtime::math::mean_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.min", "math", "min", crow_runtime::math::min, (Tensor) -> Float, never),
    row!("math.min_axis", "math", "min", crow_runtime::math::min_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.max", "math", "max", crow_runtime::math::max, (Tensor) -> Float, never),
    row!("math.max_axis", "math", "max", crow_runtime::math::max_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.prod", "math", "prod", crow_runtime::math::prod, (Tensor) -> Float, never),
    row!("math.prod_axis", "math", "prod", crow_runtime::math::prod_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.argmax", "math", "argmax", crow_runtime::math::argmax, (Tensor) -> Int, never),
    row!("math.argmin", "math", "argmin", crow_runtime::math::argmin, (Tensor) -> Int, never),
    row!("math.cumsum", "math", "cumsum", crow_runtime::math::cumsum, (Tensor) -> Tensor, never),
    row!("math.cumprod", "math", "cumprod", crow_runtime::math::cumprod, (Tensor) -> Tensor, never),
    row!("math.argsort", "math", "argsort", crow_runtime::math::argsort, (Tensor) -> Ints, never),
    row!("math.topk", "math", "topk", crow_runtime::math::topk, (Tensor, Int) -> Tensor, stops),
    row!("math.unique", "math", "unique", crow_runtime::math::unique, (Tensor) -> Tensor, never),
    row!("math.where", "math", "where", crow_runtime::math::where_, (Tensor, Tensor, Tensor) -> Tensor, stops),
    row!("math.reshape", "math", "reshape", crow_runtime::math::reshape, (Tensor, Sizes) -> Tensor, stops),
    row!("math.flatten", "math", "flatten", crow_runtime::math::flatten, (Tensor) -> Tensor, never),
    row!("math.transpose", "math", "transpose", crow_runtime::math::transpose, (Tensor) -> Tensor, stops),
    row!("math.clip", "math", "clip", crow_runtime::math::clip, (Tensor, Float, Float) -> Tensor, never),
    row!("math.atan2", "math", "atan2", crow_runtime::math::atan2, (Tensor, Tensor) -> Tensor, stops),
    // element-wise: a number and a tensor share one rule (the number's row
    // first, so a number never reaches the tensor's)
    row!("math.exp_n", "math", "exp", crow_runtime::math::exp_n, (Float) -> Float, never),
    row!("math.exp_t", "math", "exp", crow_runtime::math::exp_t, (Tensor) -> Tensor, never),
    row!("math.ln_n", "math", "ln", crow_runtime::math::ln_n, (Float) -> Float, never),
    row!("math.ln_t", "math", "ln", crow_runtime::math::ln_t, (Tensor) -> Tensor, never),
    row!("math.log2_n", "math", "log2", crow_runtime::math::log2_n, (Float) -> Float, never),
    row!("math.log2_t", "math", "log2", crow_runtime::math::log2_t, (Tensor) -> Tensor, never),
    row!("math.log10_n", "math", "log10", crow_runtime::math::log10_n, (Float) -> Float, never),
    row!("math.log10_t", "math", "log10", crow_runtime::math::log10_t, (Tensor) -> Tensor, never),
    row!("math.sin_n", "math", "sin", crow_runtime::math::sin_n, (Float) -> Float, never),
    row!("math.sin_t", "math", "sin", crow_runtime::math::sin_t, (Tensor) -> Tensor, never),
    row!("math.cos_n", "math", "cos", crow_runtime::math::cos_n, (Float) -> Float, never),
    row!("math.cos_t", "math", "cos", crow_runtime::math::cos_t, (Tensor) -> Tensor, never),
    row!("math.tan_n", "math", "tan", crow_runtime::math::tan_n, (Float) -> Float, never),
    row!("math.tan_t", "math", "tan", crow_runtime::math::tan_t, (Tensor) -> Tensor, never),
    row!("math.asin_n", "math", "asin", crow_runtime::math::asin_n, (Float) -> Float, never),
    row!("math.asin_t", "math", "asin", crow_runtime::math::asin_t, (Tensor) -> Tensor, never),
    row!("math.acos_n", "math", "acos", crow_runtime::math::acos_n, (Float) -> Float, never),
    row!("math.acos_t", "math", "acos", crow_runtime::math::acos_t, (Tensor) -> Tensor, never),
    row!("math.atan_n", "math", "atan", crow_runtime::math::atan_n, (Float) -> Float, never),
    row!("math.atan_t", "math", "atan", crow_runtime::math::atan_t, (Tensor) -> Tensor, never),
    row!("math.sinh_n", "math", "sinh", crow_runtime::math::sinh_n, (Float) -> Float, never),
    row!("math.sinh_t", "math", "sinh", crow_runtime::math::sinh_t, (Tensor) -> Tensor, never),
    row!("math.cosh_n", "math", "cosh", crow_runtime::math::cosh_n, (Float) -> Float, never),
    row!("math.cosh_t", "math", "cosh", crow_runtime::math::cosh_t, (Tensor) -> Tensor, never),
    row!("math.tanh_n", "math", "tanh", crow_runtime::math::tanh_n, (Float) -> Float, never),
    row!("math.tanh_t", "math", "tanh", crow_runtime::math::tanh_t, (Tensor) -> Tensor, never),
    row!("math.sign_n", "math", "sign", crow_runtime::math::sign_n, (Float) -> Float, never),
    row!("math.sign_t", "math", "sign", crow_runtime::math::sign_t, (Tensor) -> Tensor, never),
    row!("math.abs_t", "math", "abs", crow_runtime::math::abs_t, (Tensor) -> Tensor, never),
    row!("math.sqrt_t", "math", "sqrt", crow_runtime::math::sqrt_t, (Tensor) -> Tensor, never),
    row!("math.floor_t", "math", "floor", crow_runtime::math::floor_t, (Tensor) -> Tensor, never),
    row!("math.ceil_t", "math", "ceil", crow_runtime::math::ceil_t, (Tensor) -> Tensor, never),
    row!("math.round_t", "math", "round", crow_runtime::math::round_t, (Tensor) -> Tensor, never),
    // math: signal, special functions, complex numbers, draws (a rank, a
    // window or a count the kernels asserted is the reason)
    row!("math.fft", "math", "fft", crow_runtime::math::fft, (Tensor) -> Tensors, stops),
    row!("math.ifft", "math", "ifft", crow_runtime::math::ifft, (Tensor, Tensor) -> Tensor, stops),
    row!("math.fft2", "math", "fft2", crow_runtime::math::fft2, (Tensor) -> Tensors, stops),
    row!("math.conv", "math", "conv", crow_runtime::math::conv, (Tensor, Tensor) -> Tensor, stops),
    row!("math.conv2d", "math", "conv2d", crow_runtime::math::conv2d, (Tensor, Tensor) -> Tensor, stops),
    row!("math.hamming", "math", "hamming", crow_runtime::math::hamming, (Int) -> Tensor, stops),
    row!("math.hanning", "math", "hanning", crow_runtime::math::hanning, (Int) -> Tensor, stops),
    row!("math.stft", "math", "stft", crow_runtime::math::stft, (Tensor, Int, Int) -> Tensors, stops),
    row!("math.spectrogram", "math", "spectrogram", crow_runtime::math::spectrogram, (Tensor, Int, Int) -> Tensor, stops),
    row!("math.lowpass", "math", "lowpass", crow_runtime::math::lowpass, (Tensor, Float, Float) -> Tensor, stops),
    row!("math.highpass", "math", "highpass", crow_runtime::math::highpass, (Tensor, Float, Float) -> Tensor, stops),
    row!("math.bandpass", "math", "bandpass", crow_runtime::math::bandpass, (Tensor, Float, Float, Float) -> Tensor, stops),
    row!("math.gamma", "math", "gamma", crow_runtime::math::gamma, (Tensor) -> Tensor, never),
    row!("math.erf_n", "math", "erf", crow_runtime::math::erf_n, (Float) -> Float, never),
    row!("math.erf", "math", "erf", crow_runtime::math::erf, (Tensor) -> Tensor, never),
    row!("math.factorial", "math", "factorial", crow_runtime::math::factorial, (Tensor) -> Tensor, never),
    row!("math.bessel", "math", "bessel", crow_runtime::math::bessel, (Tensor, Int) -> Tensor, never),
    row!("math.choose", "math", "choose", crow_runtime::math::choose, (Tensor, Tensor) -> Tensor, stops),
    row!("math.complex", "math", "complex", crow_runtime::math::complex, (Float, Float) -> Tensors, never),
    row!("math.real", "math", "real", crow_runtime::math::real, (Tensors) -> Tensor, stops),
    row!("math.imag", "math", "imag", crow_runtime::math::imag, (Tensors) -> Tensor, stops),
    row!("math.magnitude", "math", "magnitude", crow_runtime::math::magnitude, (Tensors) -> Tensor, stops),
    row!("math.phase", "math", "phase", crow_runtime::math::phase, (Tensors) -> Tensor, stops),
    row!("math.conjugate", "math", "conjugate", crow_runtime::math::conjugate, (Tensors) -> Tensors, stops),
    row!("math.seed", "math", "seed", crow_runtime::math::seed, (Int) -> Unit, never),
    row!("math.shuffle", "math", "shuffle", crow_runtime::math::shuffle, (Tensor) -> Tensor, never),
    row!("math.choice", "math", "choice", crow_runtime::math::choice, (Tensor, Int) -> Tensor, stops),
    row!("math.permutation", "math", "permutation", crow_runtime::math::permutation, (Int) -> Tensor, stops),
    row!("math.mod", "math", "mod", crow_runtime::math::modulo, (Tensor, Tensor) -> Tensor, stops),
    row!("math.mod_by", "math", "mod", crow_runtime::math::modulo_by, (Tensor, Float) -> Tensor, never),
    // math: sparse matrices, solving, stacking, conversion (an entry outside
    // its matrix, a system that does not meet, tensors of two shapes: the
    // reason; a frame that does not convert stopped the interpreter and
    // unwrapped into a panic compiled)
    row!("math.sparse", "math", "sparse", crow_runtime::math::sparse, (Tensor) -> Sparse, stops),
    row!("math.sparse_coo", "math", "sparse", crow_runtime::math::sparse_coo, (Tensor, Tensor, Tensor, Tensor) -> Sparse, stops),
    row!("math.dense", "math", "dense", crow_runtime::math::dense, (Sparse) -> Tensor, never),
    row!("math.dense_of", "math", "dense", crow_runtime::math::dense_of, (Tensor) -> Tensor, never),
    row!("math.nnz", "math", "nnz", crow_runtime::math::nnz, (Sparse) -> Int, never),
    row!("math.spmv", "math", "spmv", crow_runtime::math::spmv, (Sparse, Tensor) -> Tensor, stops),
    row!("math.solve_sparse", "math", "solve", crow_runtime::math::solve_sparse, (Sparse, Tensor) -> Tensor, stops),
    row!("math.solve", "math", "solve", crow_runtime::math::solve, (Tensor, Tensor) -> Tensor, stops),
    row!("math.stack", "math", "stack", crow_runtime::math::stack, (Tensors) -> Tensor, stops),
    row!("math.stack_axis", "math", "stack", crow_runtime::math::stack_axis, (Tensors, Int) -> Tensor, stops),
    row!("math.stack_rows", "math", "stack", crow_runtime::math::stack_rows, (Tensor) -> Tensor, stops),
    row!("math.stack_rows_axis", "math", "stack", crow_runtime::math::stack_rows_axis, (Tensor, Int) -> Tensor, stops),
    row!("math.tensor_of_frame", "math", "tensor", crow_runtime::math::tensor_of_frame, (Frame) -> Tensor, stops),
    row!("math.tensor_of_columns", "math", "tensor", crow_runtime::math::tensor_of_columns, (Frame, Strs) -> Tensor, stops),
    row!("math.tensor", "math", "tensor", crow_runtime::math::tensor, (Tensor) -> Tensor, never),
    // math.stats (series of two lengths, a percent past 100, a distribution
    // no test knows: the reason, where they were answered NaN, 0.5 or an
    // empty map; anova of one group was NaN compiled and refused
    // interpreted)
    // math.stats, written in Crow (src/modules/stats.co): each row the
    // contract, the statistic a formula over math's builtins
    crow!("stats.median", "math.stats", "median", (Tensor) -> Float, never),
    crow!("stats.std", "math.stats", "std", (Tensor) -> Float, never),
    crow!("stats.var", "math.stats", "var", (Tensor) -> Float, never),
    crow!("stats.percentile", "math.stats", "percentile", (Tensor, Float) -> Float, stops),
    crow!("stats.zscore", "math.stats", "zscore", (Tensor) -> Tensor, never),
    crow!("stats.corr", "math.stats", "corr", (Tensor, Tensor) -> Float, stops),
    crow!("stats.cov", "math.stats", "cov", (Tensor, Tensor) -> Float, stops),
    crow!("stats.ttest", "math.stats", "ttest", (Tensor, Tensor) -> FloatMap, never),
    crow!("stats.linreg", "math.stats", "linreg", (Tensor, Tensor) -> FloatMap, stops),
    crow!("stats.chisq", "math.stats", "chisq", (Tensor, Tensor) -> FloatMap, stops),
    crow!("stats.anova", "math.stats", "anova", (Groups) -> FloatMap, stops),
    crow!("stats.kstest", "math.stats", "kstest", (Tensor, Str) -> FloatMap, stops),
    crow!("stats.fit", "math.stats", "fit", (Tensor, Str) -> FloatMap, stops),
    // math.nn, written in Crow (src/modules/nn.co): the rows are its contract
    // (shapes that do not meet, a target that is no class: the
    // reason, where a kernel panicked or a loss read a logit of 0)
    crow!("nn.relu", "math.nn", "relu", (Tensor) -> Tensor, never),
    crow!("nn.sigmoid", "math.nn", "sigmoid", (Tensor) -> Tensor, never),
    crow!("nn.softmax", "math.nn", "softmax", (Tensor) -> Tensor, never),
    crow!("nn.gelu", "math.nn", "gelu", (Tensor) -> Tensor, never),
    crow!("nn.mse", "math.nn", "mse", (Tensor, Tensor) -> Float, stops),
    crow!("nn.crossentropy", "math.nn", "crossentropy", (Tensor, Tensor) -> Float, stops),
    crow!("nn.bce", "math.nn", "bce", (Tensor, Tensor) -> Float, stops),
    crow!("nn.linear", "math.nn", "linear", (Tensor, Tensor, Tensor) -> Tensor, stops),
    // chart (a window of no rows, series of two lengths: the reason, where
    // a negative window was a huge one compiled and one row interpreted,
    // and a shorter series cut the others)
    row!("chart.sma", "chart", "sma", crow_runtime::indicators::sma, (Tensor, Int) -> Tensor, stops),
    row!("chart.ema", "chart", "ema", crow_runtime::indicators::ema, (Tensor, Int) -> Tensor, stops),
    row!("chart.rsi", "chart", "rsi", crow_runtime::indicators::rsi, (Tensor, Int) -> Tensor, stops),
    row!("chart.rsi_buckets", "chart", "rsi", crow_runtime::indicators::rsi_buckets, (Tensor, Tensor, Int, Float) -> Tensor, stops),
    row!("chart.macd", "chart", "macd", crow_runtime::indicators::macd, (Tensor, Int, Int, Int) -> Frame, stops),
    row!("chart.vwap", "chart", "vwap", crow_runtime::indicators::vwap, (Tensor, Tensor) -> Tensor, stops),
    row!("chart.vwap_trailing", "chart", "vwap", crow_runtime::indicators::vwap_trailing, (Tensor, Tensor, Tensor, Float) -> Tensor, stops),
    row!("chart.rolling_sum", "chart", "rolling_sum", crow_runtime::indicators::rolling_sum, (Tensor, Tensor, Float) -> Tensor, stops),
    row!("chart.cvd", "chart", "cvd", crow_runtime::indicators::cvd, (Tensor, Tensor) -> Tensor, stops),
    // `use sys`: the standard library's own layer (crow_runtime::sys) — a
    // program may not import it (spec, The standard library's layer)
    row!("sys.open", "sys", "open", crow_runtime::sys::open, (Str, Int, Bool, Float) -> Stream, value "sys"),
    row!("sys.send", "sys", "send", crow_runtime::sys::send, (Stream, Bytes) -> Int, value "sys"),
    row!("sys.recv", "sys", "recv", crow_runtime::sys::recv, (Stream, Int, Float) -> Bytes, value "sys"),
    row!("sys.close", "sys", "close", crow_runtime::sys::close, (Stream) -> Unit, never),
    row!("sys.bytes", "sys", "bytes", crow_runtime::sys::bytes_of_text, (Str) -> Bytes, never),
    row!("sys.text", "sys", "text", crow_runtime::sys::text_of_bytes, (Bytes) -> Str, value "parse"),
    row!("sys.slice", "sys", "slice", crow_runtime::sys::slice, (Bytes, Int, Int) -> Bytes, stops),
    row!("sys.find", "sys", "find", crow_runtime::sys::find, (Bytes, Bytes) -> Int, never),
    row!("sys.concat", "sys", "concat", crow_runtime::sys::concat, (Bytes, Bytes) -> Bytes, never),
    row!("sys.byte", "sys", "byte", crow_runtime::sys::byte, (Bytes, Int) -> Int, stops),
    row!("sys.gather", "sys", "gather", crow_runtime::sys::gather, (Chunks) -> Bytes, never),
    row!("sys.file", "sys", "file", crow_runtime::sys::file, (Str) -> Bytes, value "sys"),
    row!("sys.store", "sys", "store", crow_runtime::sys::store, (Str, Bytes) -> Unit, value "sys"),
    row!("sys.random", "sys", "random", crow_runtime::sys::random, (Int) -> Bytes, stops),
    row!("sys.hex", "sys", "hex", crow_runtime::sys::hex, (Bytes) -> Str, never),
    row!("sys.text_with", "sys", "text", crow_runtime::sys::text_with, (Bytes, Str) -> Str, never),
    row!("sys.hash", "sys", "hash", crow_runtime::sys::hash, (Bytes, Str) -> Str, stops),
    row!("sys.hmac", "sys", "hmac", crow_runtime::sys::hmac, (Bytes, Bytes, Str) -> Bytes, stops),
    row!("sys.reply", "sys", "reply", crow_runtime::sources::reply, (Int, Str, TextMap) -> Reply, never),
    // bytes as text and back, a signature compared, a number's bytes, and a
    // response of bytes: what a token, a frame's header and a packed value
    // are written over
    row!("sys.base64", "sys", "base64", crow_runtime::sys::base64, (Bytes) -> Str, never),
    row!("sys.base64url", "sys", "base64url", crow_runtime::sys::base64url, (Bytes) -> Str, never),
    row!("sys.unbase64", "sys", "unbase64", crow_runtime::sys::unbase64, (Str) -> Bytes, value "parse"),
    row!("sys.same", "sys", "same", crow_runtime::sys::same, (Bytes, Bytes) -> Bool, never),
    row!("sys.pack", "sys", "pack", crow_runtime::sys::pack, (Int, Int) -> Bytes, stops),
    row!("sys.ieee", "sys", "ieee", crow_runtime::sys::ieee, (Float) -> Bytes, never),
    row!("sys.binary", "sys", "binary", crow_web::core::response::binary_of, (Bytes, Str) -> Response, never),
    // a socket: the stream it talks over, what it holds unread, a frame's
    // mask, a digest as bytes (the handshake's accept key)
    row!("sys.socket_of", "sys", "socket_of", crow_runtime::sys::socket_of, (Stream, Str, Bytes) -> Socket, never),
    row!("sys.stream_of", "sys", "stream_of", crow_runtime::sys::stream_of, (Socket) -> Stream, never),
    row!("sys.leftover", "sys", "leftover", crow_runtime::sys::leftover, (Socket) -> Bytes, never),
    row!("sys.unread", "sys", "unread", crow_runtime::sys::unread, (Socket, Bytes) -> Unit, never),
    row!("sys.mask", "sys", "mask", crow_runtime::sys::mask, (Bytes, Bytes) -> Bytes, stops),
    row!("sys.digest", "sys", "digest", crow_runtime::sys::digest, (Bytes, Str) -> Bytes, stops),
    // the special functions the statistics are written over (crow_runtime::stats)
    row!("sys.betainc", "sys", "betainc", crow_runtime::stats::betainc, (Float, Float, Float) -> Float, never),
    row!("sys.gammainc", "sys", "gammainc", crow_runtime::stats::gammainc, (Float, Float) -> Float, never),
    row!("sys.lgamma", "sys", "lgamma", crow_runtime::stats::lgamma, (Float) -> Float, never),
    row!("sys.shaped", "sys", "shaped", crow_runtime::math::shaped, (Tensor, Ints) -> Tensor, stops),
    row!("sys.note", "sys", "note", crow_runtime::sys::note, (Stream, Str) -> Unit, never),
    row!("sys.noted", "sys", "noted", crow_runtime::sys::noted, (Stream) -> Str, never),
    row!("sys.machine", "sys", "machine", crow_runtime::sys::machine, () -> Str, never),
    row!("sys.process", "sys", "process", crow_runtime::sys::process, () -> Int, never),
    row!("sys.alive", "sys", "alive", crow_runtime::sys::alive, (Int) -> Bool, never),
    row!("sys.rowbinary_of_frame", "sys", "rowbinary_of_frame", crow_runtime::sys::rowbinary_of_frame, (Frame, Texts) -> Bytes, value "frame"),
    row!("sys.xor", "sys", "xor", crow_runtime::sys::xor, (Bytes, Bytes) -> Bytes, stops),
    row!("sys.copy_of_frame", "sys", "copy_of_frame", crow_runtime::sys::copy_of_frame, (Frame) -> Bytes, never),
    // the Postgres driver written in Crow (src/modules/postgres.co)
    crow!("db.postgres.pg_open", "db.postgres", "pg_open", (Encoded) -> Stream, value "db"),
    crow!("db.postgres.pg_check", "db.postgres", "pg_check", (Encoded, Stream) -> Int, value "db"),
    crow!("db.postgres.pg_query", "db.postgres", "pg_query", (Encoded, Stream, Str, Encoded) -> Frame, value "db"),
    crow!("db.postgres.pg_exec", "db.postgres", "pg_exec", (Encoded, Stream, Str, Encoded) -> Int, value "db"),
    crow!("db.postgres.pg_insert", "db.postgres", "pg_insert", (Encoded, Stream, Str, Frame) -> Int, value "db"),
    crow!("db.postgres.pg_describe", "db.postgres", "pg_describe", (Encoded, Stream, Str) -> Frame, value "db"),
    crow!("db.postgres.pg_cancel", "db.postgres", "pg_cancel", (Encoded, Stream) -> Int, value "db"),
    // the ClickHouse driver written in Crow (src/modules/clickhouse.co),
    // as crow_db::driven calls it (docs/db-in-crow-design.md)
    crow!("db.clickhouse.ch_open", "db.clickhouse", "ch_open", (Encoded) -> Stream, value "db"),
    crow!("db.clickhouse.ch_check", "db.clickhouse", "ch_check", (Encoded, Stream) -> Int, value "db"),
    crow!("db.clickhouse.ch_query", "db.clickhouse", "ch_query", (Encoded, Stream, Str, Encoded) -> Frame, value "db"),
    crow!("db.clickhouse.ch_exec", "db.clickhouse", "ch_exec", (Encoded, Stream, Str, Encoded) -> Int, value "db"),
    crow!("db.clickhouse.ch_insert", "db.clickhouse", "ch_insert", (Encoded, Stream, Str, Frame) -> Int, value "db"),
    crow!("db.clickhouse.ch_describe", "db.clickhouse", "ch_describe", (Encoded, Stream, Str) -> Frame, value "db"),
    crow!("db.clickhouse.ch_cancel", "db.clickhouse", "ch_cancel", (Encoded, Stream) -> Int, value "db"),
    // a mailer's fields, and a connection secured (SMTP's STARTTLS)
    row!("sys.mailer_field", "sys", "mailer_field", crow_runtime::sys::mailer_field, (Mailer, Str) -> Str, stops),
    row!("sys.secure", "sys", "secure", crow_runtime::sys::secure, (Stream, Str) -> Stream, value "sys"),
    // the mailer `use web.mail` makes: a handle, its password in no value
    row!("web.mail.smtp", "web.mail", "smtp", crow_runtime::sys::mailer, (Str, Int, Str, Str) -> Mailer, never),
    row!("sys.answer", "sys", "answer", crow_runtime::sys::answer, (Int, Str, TextMap, Bytes) -> Answer, never),
    // the parse kernels the readers written in Crow stand on
    row!("sys.csv_rows", "sys", "csv_rows", crow_runtime::sys::csv_rows, (Str) -> TextRows, value "io"),
    row!("sys.frame_of_csv", "sys", "frame_of_csv", crow_runtime::sys::frame_of_csv, (Str) -> Frame, value "frame"),
    row!("sys.frame_of_csv_bytes", "sys", "frame_of_csv_bytes", crow_runtime::sys::frame_of_csv_bytes, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_of_json", "sys", "frame_of_json", crow_runtime::sys::frame_of_json, (Str) -> Frame, value "frame"),
    row!("sys.frame_of_parquet", "sys", "frame_of_parquet", crow_runtime::sys::frame_of_parquet, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_of_rowbinary", "sys", "frame_of_rowbinary", crow_runtime::sys::frame_of_rowbinary, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_of_postgres", "sys", "frame_of_postgres", crow_runtime::sys::frame_of_postgres, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_csv", "sys", "frame_csv", crow_runtime::sys::frame_csv, (Frame) -> Str, value "frame"),
    row!("sys.frame_json", "sys", "frame_json", crow_runtime::sys::frame_json, (Frame) -> Str, never),
    row!("sys.frame_parquet", "sys", "frame_parquet", crow_runtime::sys::frame_parquet, (Frame) -> Bytes, value "frame"),
    row!("sys.tensor_of_frame", "sys", "tensor_of_frame", crow_runtime::sys::tensor_of_frame, (Frame) -> Tensor, value "load"),
    // the protocol, written in Crow (src/modules/sys.co): one request sent
    // and answered; a remote source read with a reader's options, or an S3
    // object; bytes written to an S3 object
    crow!("sys.exchange", "sys", "exchange", (Str, Str, Strs, Bytes, Float) -> Answer, value "sys"),
    crow!("sys.exchange_on", "sys", "exchange_on", (Stream, Str, Str, Str, Strs, Bytes, Moment, Bool) -> Answer, value "sys"),
    crow!("sys.fetch", "sys", "fetch", (Str, TextMap, TextMap, Float) -> Answer, value "sys"),
    crow!("sys.put", "sys", "put", (Str, Bytes) -> Str, value "sys"),
    crow!("sys.format_of", "sys", "format_of", (Str) -> Str, never),
    crow!("sys.remote", "sys", "remote", (Str) -> Bool, never),
    crow!("sys.with_query", "sys", "with_query", (Str, TextMap) -> Str, never),
    crow!("sys.redirected", "sys", "redirected", (Str, Str) -> Str, never),
    crow!("sys.format_sniffed", "sys", "format_sniffed", (Str, Bytes) -> Str, never),
    crow!("sys.read_head", "sys", "read_head", (Stream, Moment) -> Answer, value "sys"),
    crow!("sys.upgrade", "sys", "upgrade", (Str, Float) -> Socket, value "sys"),
    crow!("sys.civil", "sys", "civil", (Int) -> Ints, never),
    // web's rows written in Crow: a token signed and read (src/modules/auth.co),
    // a value packed as MessagePack (src/modules/web.co)
    crow!("web.auth.sign", "web.auth", "sign", (Encoded, Str) -> Str, value "auth"),
    crow!("web.auth.verify", "web.auth", "verify", (Str, Str) -> Json, value "auth"),
    crow!("web.msgpack", "web", "msgpack", (Encoded) -> Response, never),
    // the WebSocket client written in Crow (src/modules/ws.co): a socket's
    // row is named as the program calls it, its function by the row's id
    crow!("web.ws.socket", "web.ws", "socket", (Str) -> Socket, value "socket"),
    crow!("web.ws.send_text", "web.ws", "send", (Socket, Str) -> Str, value "socket"),
    crow!("web.ws.recv_text", "web.ws", "recv", (Socket) -> Str, value "socket"),
    crow!("web.ws.recv_for", "web.ws", "recv", (Socket, Float) -> Str, value "socket"),
    crow!("web.ws.close_socket", "web.ws", "close", (Socket) -> Str, never),
    // mail written in Crow (src/modules/mail.co): SMTP over the sys layer
    crow!("web.mail.mail", "web.mail", "mail", (Mailer, Str, Str, Str) -> Bool, value "mail"),
    // frame (a column the frame lacks, an aggregation or a join it does not
    // know, a mask or a count that does not fit: the reason the program
    // stops with, as df.col of a missing column is — the interpreter had
    // answered error values its type said could not happen, the compiled
    // program stopped or panicked; values with cannot fit are an error
    // value, as text that is no time is)
    row!("frame.new", "frame", "new", crow_runtime::frames::new, () -> Frame, never),
    row!("frame.with", "frame", "with", crow_runtime::frames::with, (Frame, Str, Cells) -> Frame, value "frame"),
    row!("frame.select", "frame", "select", crow_runtime::frames::select, (Frame, Strs) -> Frame, stops),
    row!("frame.filter", "frame", "filter", crow_runtime::frames::filter, (Frame, Bools) -> Frame, stops),
    row!("frame.filter_where", "", "__filter_where", crow_runtime::mask::filter_where, (Frame, Str, Strs, Scalars) -> Frame, stops),
    row!("frame.sort", "frame", "sort", crow_runtime::frames::sort, (Frame, Str) -> Frame, stops),
    row!("frame.sort_by", "frame", "sort", crow_runtime::frames::sort_by, (Frame, Str, Bool) -> Frame, stops),
    row!("frame.sort_keys", "frame", "sort", crow_runtime::frames::sort_keys, (Frame, Strs) -> Frame, stops),
    row!("frame.sort_keys_by", "frame", "sort", crow_runtime::frames::sort_keys_by, (Frame, Strs, Bool) -> Frame, stops),
    row!("frame.sort_keys_each", "frame", "sort", crow_runtime::frames::sort_keys_each, (Frame, Strs, Bools) -> Frame, stops),
    row!("frame.group", "frame", "group", crow_runtime::frames::group, (Frame, Str, Lists) -> Frame, stops),
    row!("frame.group_keys", "frame", "group", crow_runtime::frames::group_keys, (Frame, Strs, Lists) -> Frame, stops),
    row!("frame.pivot", "frame", "pivot", crow_runtime::frames::pivot, (Frame, Str, Str, Str) -> Frame, stops),
    row!("frame.window", "frame", "window", crow_runtime::frames::window, (Frame, Str, Str, Int, Strs, Float, Str) -> Column, stops),
    row!("frame.window_one", "frame", "window", crow_runtime::frames::window_one, (Frame, Str, Str, Int, Str, Float, Str) -> Column, stops),
    row!("frame.join", "frame", "join", crow_runtime::frames::join, (Frame, Frame, Str) -> Frame, stops),
    row!("frame.join_how", "frame", "join", crow_runtime::frames::join_how, (Frame, Frame, Str, Str) -> Frame, stops),
    row!("frame.join_keys", "frame", "join", crow_runtime::frames::join_keys, (Frame, Frame, Strs) -> Frame, stops),
    row!("frame.join_keys_how", "frame", "join", crow_runtime::frames::join_keys_how, (Frame, Frame, Strs, Str) -> Frame, stops),
    row!("frame.unpivot", "frame", "unpivot", crow_runtime::frames::unpivot, (Frame, Strs, Strs) -> Frame, stops),
    row!("frame.unpivot_one", "frame", "unpivot", crow_runtime::frames::unpivot_one, (Frame, Str, Strs) -> Frame, stops),
    row!("frame.complete", "frame", "complete", crow_runtime::frames::complete, (Frame) -> Frame, stops),
    row!("frame.complete_one", "frame", "complete", crow_runtime::frames::complete_one, (Frame, Str) -> Frame, stops),
    row!("frame.complete_in", "frame", "complete", crow_runtime::frames::complete_in, (Frame, Strs) -> Frame, stops),
    row!("frame.sample", "frame", "sample", crow_runtime::frames::sample, (Frame, Int) -> Frame, stops),
    row!("frame.bucket", "frame", "bucket", crow_runtime::frames::bucket, (Column, Float) -> Column, stops),
    row!("frame.where", "frame", "where", crow_runtime::frames::choose, (Bools, Cells, Cells) -> Column, stops),
    row!("frame.int_column", "", "int", crow_runtime::frames::int_column, (Column) -> Column, stops),
    row!("frame.float_column", "", "float", crow_runtime::frames::float_column, (Column) -> Column, stops),
    row!("frame.moment_column", "", "moment", crow_runtime::frames::moment_column, (Column) -> Column, stops),
    row!("frame.lower_column", "", "lower", crow_runtime::verbs::lower_column, (Column) -> Column, stops),
    row!("frame.upper_column", "", "upper", crow_runtime::verbs::upper_column, (Column) -> Column, stops),
    row!("frame.trim_column", "", "trim", crow_runtime::verbs::trim_column, (Column) -> Column, stops),
    row!("frame.replace_column", "", "replace", crow_runtime::verbs::replace_column, (Column, Str, Str) -> Column, stops),
    row!("frame.sub_column", "", "sub", crow_runtime::verbs::sub_column, (Column, Str, Str) -> Column, stops),
    row!("frame.contains_column", "", "contains", crow_runtime::verbs::contains_column, (Column, Str) -> Bools, stops),
    row!("frame.starts_column", "", "starts", crow_runtime::verbs::starts_column, (Column, Str) -> Bools, stops),
    row!("frame.ends_column", "", "ends", crow_runtime::verbs::ends_column, (Column, Str) -> Bools, stops),
    row!("frame.stack", "frame", "stack", crow_runtime::frames::stack, (Frame, Frame) -> Frame, stops),
    row!("frame.head", "frame", "head", crow_runtime::frames::head, (Frame, Int) -> Frame, stops),
    row!("frame.tail", "frame", "tail", crow_runtime::frames::tail, (Frame, Int) -> Frame, stops),
    row!("frame.fill", "frame", "fill", crow_runtime::frames::fill, (Frame, Str, Cell, Str) -> Frame, stops),
    row!("frame.unique", "frame", "unique", crow_runtime::frames::unique, (Frame, Str) -> Column, stops),
    row!("frame.unique_rows", "frame", "unique", crow_runtime::frames::unique_rows, (Frame, Strs) -> Frame, stops),
    row!("frame.rename", "frame", "rename", crow_runtime::frames::rename, (Frame, Str, Str) -> Frame, stops),
    row!("frame.stats", "frame", "stats", crow_runtime::frames::stats, (Frame) -> Str, never),
    // the modules' files: effects, as the file system's are. `save`'s
    // failure is an error value of kind 'frame', as `load`'s is (compiled,
    // it stopped the program)
    // the readers, written in Crow on the sys layer (src/modules/frame.co,
    // io.co, math.co): a file, a URL or an S3 object
    crow!("frame.load", "frame", "load", (Str) -> Frame, value "frame"),
    crow!("frame.load_asked", "frame", "load", (Str, TextMap, TextMap, Float) -> Frame, value "frame"),
    crow!("frame.save", "frame", "save", (Frame, Str) -> Str, value "frame"),
    crow!("io.csv", "io", "csv", (Str) -> TextRows, value "io"),
    crow!("io.csv_asked", "io", "csv", (Str, TextMap, TextMap, Float) -> TextRows, value "io"),
    crow!("io.lines", "io", "lines", (Str) -> Strs, value "io"),
    crow!("io.lines_asked", "io", "lines", (Str, TextMap, TextMap, Float) -> Strs, value "io"),
    crow!("math.load", "math", "load", (Str) -> Tensor, value "load"),
    crow!("math.load_asked", "math", "load", (Str, TextMap, TextMap, Float) -> Tensor, value "load"),
    // doc: markdown rendered by the kernel
    row!("doc.markdown", "doc", "markdown", crow_runtime::markdown::to_html, (Str) -> Markup, never),
    row!("doc.chapters", "doc", "chapters", crow_runtime::markdown::chapters_frame, (Str) -> Frame, never),
    row!("doc.menu", "doc", "menu", crow_runtime::markdown::menu_from_frame, (Frame, Str) -> Markup, never),
    row!("doc.pager", "doc", "pager", crow_runtime::markdown::pager_from_frame, (Frame, Str) -> Markup, never),
    row!("doc.plain", "doc", "plain", crow_runtime::markdown::plain_text, (Str) -> Str, never),
    row!("doc.document", "doc", "document", crow_runtime::markdown::document, (Str, Str) -> Str, never),
];

/// The row of an id (the tests name rows so).
#[cfg(test)]
pub fn by_id(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The row of an id: what a pass of the lowering writes itself (a fused
/// chain's `tensor.fused`).
pub fn row(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The core builtin's row by its Crow name.
pub fn core(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module.is_empty() && r.name == name)
}

/// A core builtin's row for a call with these arguments: the first whose
/// count and kinds take them (`min` of two ints, or of numbers). The
/// lowering reads the arguments from their types and the IR names the row
/// it picks; a call it leaves by name (an argument of unknown kind, a
/// parsed value) has its row picked from the values as the program runs.
pub fn core_row(name: &str, args: &[Arg]) -> Option<&'static Row> {
    ROWS.iter().filter(|r| r.module.is_empty() && r.name == name).find(|r| takes(r, args))
}

/// Whether a module (`""` the core) answers the name with rows.
pub fn has_rows(module: &str, name: &str) -> bool {
    ROWS.iter().any(|r| r.module == module && r.name == name)
}

/// Whether the core answers the name with rows.
pub fn has_core_rows(name: &str) -> bool {
    core(name).is_some()
}

/// How a parsed JSON value is used where a builtin computes with it: read as
/// one type first — the words arithmetic on one says, and a list's items
/// one by one (the builtins that navigate it take it as it is: `len`,
/// `keys`, `values`, `has`, `str`, `int`, `float`, `encode`).
pub const READ_FIRST: &str = " — a value whose type the program does not know is read as one first: int[…], float[…] or str[…], a list item by item (for x in …)";

/// The refusal of a call that no row of its name — in the core (`""`) and
/// in the modules given — takes, by count or by kind; none when one takes
/// it, or when an argument's kind is not known before the program runs
/// (a JSON value's is known only then).
pub fn refusal(modules: &[&str], name: &str, args: &[Arg]) -> Option<String> {
    let rows: Vec<&Row> = ROWS.iter().filter(|r| r.name == name && modules.contains(&r.module)).collect();
    if rows.is_empty() || rows.iter().any(|r| takes(r, args)) || args.contains(&Arg::Unknown) {
        return None;
    }
    // a parsed JSON value is read as one type before a builtin computes
    // with it (spec, json[]): the words say how
    let read = if args.contains(&Arg::Json) { READ_FIRST } else { "" };
    if !rows.iter().any(|r| spans(r.params, args.len(), name).is_ok()) {
        let mut counts: Vec<String> = Vec::new();
        for r in &rows {
            let fixed = r.params.iter().filter(|k| !k.is_rest()).count();
            let count = if r.params.iter().any(|k| k.is_rest()) { format!("at least {}", fixed) } else { fixed.to_string() };
            if !counts.contains(&count) {
                counts.push(count);
            }
        }
        return Some(miscount(name, &listed(&counts, "or"), args.len()));
    }
    let mut forms: Vec<String> = Vec::new();
    // a JSON value's and a column's own rows are the compiled program's
    // holding of what the interpreter holds as a list, a map or text
    for r in rows.iter().filter(|r| spans(r.params, args.len(), name).is_ok() && !r.params.iter().any(|k| matches!(k, Kind::Json | Kind::Column))) {
        let form = listed(&r.params.iter().map(|k| k.words().to_string()).collect::<Vec<_>>(), "and");
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    let got = listed(&args.iter().map(|a| a.words().to_string()).collect::<Vec<_>>(), "and");
    Some(format!("{}[] takes {}, got {}{}", shown(name), listed(&forms, "or"), got, read))
}

/// `a`, `a and b`, `a, b or c`.
fn listed(items: &[String], last: &str) -> String {
    match items.split_last() {
        Some((tail, rest)) if !rest.is_empty() => format!("{} {} {}", rest.join(", "), last, tail),
        _ => items.join(""),
    }
}

/// What an argument is, as the choice among a name's rows reads it: from
/// the IR's type, where the lowering picks the row the IR names; or, for a
/// call the IR leaves by name, from the interpreter's value and the
/// emitter's type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    Int,
    Float,
    /// A moment, and a list of them.
    Moment,
    Moments,
    Bytes,
    Stream,
    Socket,
    Mailer,
    /// A list of bytes (a connection's chunks).
    Chunks,
    /// A parsed JSON value, whose kind shows only as it runs.
    Json,
    /// A tensor.
    Tensor,
    /// A list of ints.
    Ints,
    /// A list of floats.
    Floats,
    /// A list of tensors, or of lists of numbers.
    Tensors,
    Text,
    /// A list of text.
    Texts,
    Truth,
    /// A list of bools.
    Truths,
    /// A list of records.
    Records,
    /// A list of maps.
    Maps,
    /// A list of anything else: lists of text, functions.
    Items,
    /// A list of numbers whose items can fail: its numbers where numbers
    /// are taken, a failed item stopping the program, as a column's
    /// missing cell does.
    FallibleNumbers,
    /// A list of anything else whose items can fail.
    Fallible,
    /// A frame's column: numbers where numbers are taken, the compiled
    /// program's `CrowColumn` (the interpreter holds its cells as a list).
    Column,
    Map,
    Record,
    Sparse,
    Frame,
    /// A function.
    Fn,
    /// A list whose items are not known here — an empty one, or one of
    /// parsed values: any parameter that takes a list takes it, and no
    /// other (it is a list, whatever it holds).
    AnyList,
    /// Not known here: any parameter takes it.
    Unknown,
}

impl Arg {
    /// What the argument is, as a refusal names it.
    pub fn words(self) -> &'static str {
        match self {
            Arg::Int => "an int",
            Arg::Float => "a number",
            Arg::Moment => "a moment",
            Arg::Moments => "a list of moments",
            Arg::Json => "a JSON value",
            Arg::Tensor => "numbers",
            Arg::Ints => "a list of ints",
            Arg::Floats => "a list of numbers",
            Arg::Tensors => "a list of tensors",
            Arg::Text => "text",
            Arg::Texts => "a list of text",
            Arg::Chunks => "a list of bytes",
            Arg::Truth => "a bool",
            Arg::Truths => "a list of bools",
            Arg::Records => "a list of records",
            Arg::Maps => "a list of maps",
            Arg::Items => "a list",
            Arg::FallibleNumbers => "a list of numbers that can fail",
            Arg::Fallible => "a list whose items can fail",
            Arg::Column => "a frame's column",
            Arg::Map => "a map",
            Arg::Record => "a record",
            Arg::Sparse => "a sparse matrix",
            Arg::Bytes => "bytes",
            Arg::Stream => "a stream",
            Arg::Socket => "a socket",
            Arg::Mailer => "a mailer",
            Arg::Frame => "a frame",
            Arg::Fn => "a function",
            Arg::AnyList => "a list",
            Arg::Unknown => "a value",
        }
    }

    pub fn of_type(t: &Ty) -> Arg {
        match t {
            // a value that can fail is handed on as its error before the
            // call: the call takes what it holds
            Ty::Result(inner) => Arg::of_type(inner),
            Ty::Int => Arg::Int,
            Ty::Float => Arg::Float,
            Ty::Moment => Arg::Moment,
            Ty::Json => Arg::Json,
            Ty::List(inner) => match inner.as_ref() {
                Ty::Unknown => Arg::AnyList,
                Ty::Result(item) if matches!(item.as_ref(), Ty::Int | Ty::Float) => Arg::FallibleNumbers,
                Ty::Result(_) => Arg::Fallible,
                i => Arg::of_items(Arg::of_type(i)),
            },
            Ty::Tensor(..) => Arg::Tensor,
            Ty::Column(_) => Arg::Column,
            Ty::Str | Ty::Markup => Arg::Text,
            Ty::Bool => Arg::Truth,
            Ty::Map(_) => Arg::Map,
            Ty::Record(_) | Ty::OptRecord(_) => Arg::Record,
            Ty::Sparse => Arg::Sparse,
            Ty::Bytes => Arg::Bytes,
            Ty::Stream => Arg::Stream,
            Ty::Socket => Arg::Socket,
            Ty::Mailer => Arg::Mailer,
            Ty::Frame(_) => Arg::Frame,
            Ty::Fn(..) => Arg::Fn,
            // (no row takes a handle a program cannot hand on, a view's
            // node, nothing, or what is not known: each is read by the
            // value as the program runs)
            Ty::Db | Ty::Tx | Ty::WebApp | Ty::WebRequest | Ty::WebResponse | Ty::App | Ty::Node | Ty::Unit | Ty::Unknown => Arg::Unknown,
        }
    }

    /// A list, by what its items are.
    fn of_items(item: Arg) -> Arg {
        match item {
            Arg::Int => Arg::Ints,
            Arg::Float => Arg::Floats,
            Arg::Moment => Arg::Moments,
            Arg::Tensor | Arg::Ints | Arg::Floats => Arg::Tensors,
            Arg::Text => Arg::Texts,
            Arg::Bytes => Arg::Chunks,
            Arg::Truth => Arg::Truths,
            Arg::Record => Arg::Records,
            Arg::Map => Arg::Maps,
            // a list of parsed values (or of cells no description typed yet)
            // is one whose items' kind shows as the program runs
            Arg::Unknown | Arg::Json | Arg::AnyList => Arg::AnyList,
            _ => Arg::Items,
        }
    }

    /// What a value is. A list is what its first item is (an empty one
    /// could be any list); the interpreter's JSON value and frame's column
    /// are what they hold.
    pub fn of_value(v: &CrowValue) -> Arg {
        match &v.value {
            Value::Int(_) => Arg::Int,
            Value::Float(_) => Arg::Float,
            Value::NumArray(_) => Arg::Tensor,
            Value::Array(items) => match items.first() {
                Some(first) => Arg::of_items(Arg::of_value(first)),
                None => Arg::AnyList,
            },
            Value::Str(_) => Arg::Text,
            Value::Bool(_) => Arg::Truth,
            Value::Map(_) => Arg::Map,
            Value::StructInstance(_) => Arg::Record,
            Value::Sparse(_) => Arg::Sparse,
            Value::Bytes(_) => Arg::Bytes,
            Value::Stream(_) => Arg::Stream,
            Value::Socket(_) => Arg::Socket,
            Value::Mailer(_) => Arg::Mailer,
            Value::Frame(_) => Arg::Frame,
            Value::Fn(_) | Value::GradFn(_) => Arg::Fn,
            Value::Json(_) => Arg::Json,
            _ => Arg::Unknown,
        }
    }
}

impl Kind {
    /// The kind as a refusal names what a parameter takes.
    pub fn words(self) -> &'static str {
        match self {
            Kind::Int | Kind::Whole => "an int",
            Kind::Float => "a number",
            Kind::Moment => "a moment",
            Kind::Bool => "a bool",
            Kind::Str => "text",
            Kind::Tensor => "numbers",
            Kind::Tensors => "a list of tensors",
            Kind::Bytes => "bytes",
            Kind::Stream => "a stream",
            Kind::Strs => "a list of text",
            Kind::Chunks => "a list of bytes",
            Kind::Bools => "a list of bools",
            Kind::Sparse => "a sparse matrix",
            Kind::Frame => "a frame",
            Kind::Cell => "a number, text or a bool",
            Kind::Cells => "a column's values",
            Kind::Words => "a list",
            Kind::Sizes => "sizes",
            Kind::Groups => "tensors",
            Kind::Lists => "lists of text",
            Kind::FloatMap | Kind::MapOf | Kind::MapRef | Kind::TextMap => "a map",
            Kind::Unit => "nothing",
            Kind::List | Kind::Seq => "a list",
            Kind::Item | Kind::ItemRef => "a value",
            Kind::Ints => "a list of ints",
            Kind::Floats => "a list of numbers",
            Kind::Texts => "a list of text",
            Kind::Json => "a JSON value",
            Kind::Column => "a frame's column",
            Kind::Markup => "markup",
            Kind::TextRows => "rows of text",
            Kind::Shell => "a command's answer",
            Kind::Reply => "a request's answer",
            Kind::Answer => "a connection's answer",
            Kind::Response => "a response",
            Kind::Socket => "a socket",
            Kind::Mailer => "a mailer",
            Kind::Shown | Kind::Shows | Kind::ShownDyn => "a value",
            Kind::Encoded => "a value JSON can hold",
            Kind::JsonList => "a list of JSON values",
            Kind::Operands | Kind::Operand => "a tensor or a number",
            Kind::Scalars | Kind::Scalar => "a number, text or a bool",
        }
    }

    /// Whether a parameter of this kind takes an argument of that kind.
    fn takes(self, a: Arg) -> bool {
        // a list whose items are not known could be any list
        if a == Arg::AnyList {
            return [Arg::Ints, Arg::Floats, Arg::Texts, Arg::Truths, Arg::Tensors, Arg::Records, Arg::Maps, Arg::Items].iter().any(|l| self.takes(*l));
        }
        a == Arg::Unknown
            || match self.each() {
                Kind::Int | Kind::Float => matches!(a, Arg::Int | Arg::Float),
                Kind::Moment => a == Arg::Moment,
                Kind::Whole => a == Arg::Int,
                // numbers held as a list or a column are converted where
                // they meet the parameter (the lowering's row arguments)
                Kind::Tensor => matches!(a, Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Column | Arg::FallibleNumbers | Arg::Tensors),
                Kind::Tensors => a == Arg::Tensors,
                Kind::Str => a == Arg::Text,
                Kind::Strs | Kind::Texts => a == Arg::Texts,
                Kind::Chunks => a == Arg::Chunks,
                Kind::Bool => a == Arg::Truth,
                Kind::Bools => a == Arg::Truths,
                Kind::Sparse => a == Arg::Sparse,
                Kind::Bytes => a == Arg::Bytes,
                Kind::Stream => a == Arg::Stream,
                Kind::Socket => a == Arg::Socket,
                Kind::Mailer => a == Arg::Mailer,
                Kind::Frame => a == Arg::Frame,
                Kind::Cell => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Json | Arg::Moment),
                Kind::Cells => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Column | Arg::Moment | Arg::Moments),
                Kind::Words => matches!(a, Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Column | Arg::Moments),
                Kind::List | Kind::Seq => matches!(a, Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Tensors | Arg::Records | Arg::Maps | Arg::Items | Arg::Chunks | Arg::Fallible | Arg::FallibleNumbers | Arg::Moments),
                Kind::Item | Kind::ItemRef => true,
                Kind::MapOf | Kind::MapRef | Kind::TextMap => a == Arg::Map,
                Kind::Ints => a == Arg::Ints,
                Kind::Floats => a == Arg::Floats,
                Kind::Json => a == Arg::Json,
                Kind::Column => a == Arg::Column,
                // a function shows as nothing, and JSON holds no function, frame or matrix
                Kind::Shown | Kind::ShownDyn => a != Arg::Fn,
                Kind::Encoded => !matches!(a, Arg::Fn | Arg::Frame | Arg::Sparse | Arg::Column),
                Kind::Operand => matches!(a, Arg::Tensor | Arg::Int | Arg::Float),
                Kind::Scalar => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth),
                _ => true,
            }
    }
}

/// Whether a row takes these arguments: their count, and each one's kind.
/// A frame's column is numbers where a module takes numbers; the core
/// takes one where it says so (`len`), the interpreter's cells being a list.
fn takes(row: &Row, args: &[Arg]) -> bool {
    match spans(row.params, args.len(), row.name) {
        Ok(spans) => row
            .params
            .iter()
            .zip(spans)
            .all(|(k, (a, b))| args[a..b].iter().all(|x| k.takes(*x) && !(row.module.is_empty() && *x == Arg::Column && k.each() == Kind::Tensor))),
        Err(_) => false,
    }
}

/// A module function's row for a call with these arguments: of the rows of
/// its name, the first that takes their count and kinds (`exp` of a number
/// or of a tensor, `norm[t]` or `norm[t, p]`); then the first that takes
/// their count; then the first, whose own check refuses the call.
/// A module's row of a name that takes these arguments, or none: what
/// tells two modules' rows of one name apart (sys's `send` of a stream,
/// a socket's of a socket).
/// The module a bare call reaches, decided once for every pipeline: type
/// inference typing the call and the lowering naming its row read this
/// one answer. (Each had picked by a rule of its own — inference by the
/// first argument's type, the lowering by every argument's kind — and
/// `where` of a mask was math's to the one and the frame's to the other,
/// so the variable holding the column was declared a tensor and the
/// compiled program did not build.) The core's row the kinds pick comes
/// first; else, among the modules in scope that answer the name, the one
/// whose row takes the kinds, where exactly one does; else, where the
/// kinds do not settle it, the first argument's class — a frame or its
/// column the frame's, a tensor or a list math's; else none.
pub fn module_of_bare(name: &str, providers: &[&str], kinds: &[Arg]) -> Option<String> {
    if providers.is_empty() {
        return None;
    }
    let unknown = kinds.iter().any(|k| matches!(k, Arg::Unknown | Arg::AnyList));
    if !unknown && core_row(name, kinds).is_some() {
        return None;
    }
    if providers.len() == 1 {
        return Some(providers[0].to_string());
    }
    let taking: Vec<&&str> = providers.iter().filter(|m| row_taking(m, name, kinds).is_some()).collect();
    if taking.len() == 1 {
        return Some(taking[0].to_string());
    }
    let by_first = match kinds.first() {
        Some(Arg::Frame | Arg::Column) => "frame",
        Some(Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Tensors | Arg::Texts | Arg::Truths | Arg::Records | Arg::Maps | Arg::Items | Arg::AnyList | Arg::FallibleNumbers | Arg::Fallible) => "math",
        _ => return None,
    };
    providers.contains(&by_first).then(|| by_first.to_string())
}

pub fn row_taking(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module == module && r.name == name && takes(r, args))
}

pub fn module(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    let named = || ROWS.iter().filter(move |r| r.module == module && r.name == name);
    named()
        .find(|r| takes(r, args))
        .or_else(|| named().find(|r| spans(r.params, args.len(), r.name).is_ok()))
        .or_else(|| named().next())
}

/// The refusal of a call whose count of arguments no row of its name
/// takes, naming every count the name takes (`rsi[] takes 2 or 4
/// arguments, got 3`); none when a row takes it.
pub fn miscounted(module: &str, name: &str, argc: usize) -> Option<String> {
    let rows: Vec<&Row> = ROWS.iter().filter(|r| r.module == module && r.name == name).collect();
    if rows.is_empty() || rows.iter().any(|r| spans(r.params, argc, name).is_ok()) {
        return None;
    }
    let mut counts: Vec<String> = Vec::new();
    for r in rows {
        let fixed = r.params.iter().filter(|k| !k.is_rest()).count();
        let count = if r.params.iter().any(|k| k.is_rest()) { format!("at least {}", fixed) } else { fixed.to_string() };
        if !counts.contains(&count) {
            counts.push(count);
        }
    }
    let counts = match counts.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {}", rest.join(", "), last),
        _ => counts.join(""),
    };
    Some(miscount(name, &counts, argc))
}

/// The name a refusal shows: an internal one is the builtin the program
/// wrote (`__fmt_time` is `fmt` with a time style).
fn shown(name: &str) -> &str {
    if name == crate::lower::FMT_TIME { "fmt" } else { name }
}

/// `sma[] takes 2 arguments, got 3`, `median[] takes 1 argument, got 2`.
fn miscount(name: &str, counts: &str, argc: usize) -> String {
    let name = shown(name);
    let noun = if counts == "1" || counts == "at least 1" { "argument" } else { "arguments" };
    format!("{}[] takes {} {}, got {}", name, counts, noun, argc)
}

/// The core names with syntax or state of their own, which no row says: a
/// value's own status (`error` makes one, `captured` reads its stamp) and
/// a static type's word compiled beside a value's interpreted (`type`); a
/// statement reporting its operands (`assert`, whose words are the
/// runtime's, `display::assert_failed`); what the session holds (`later`,
/// `running`, `notice`); a function argument the walker calls (`any`,
/// `all`, `find`, `grad`); a resource (`cpu`, `gpu`, `nodes`); a test
/// block; a chart's tree.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const OWN_FORM: &[&str] = &[
    "type", "error", "assert", "captured", "later", "running", "notice", "any", "all", "find", "grad", "cpu", "gpu", "nodes", "test", "chart",
];

/// The core builtins still written twice, each to become a row (docs/
/// bug-classes.md §4). The list only shrinks: a builtin added outside the
/// table fails `every_core_builtin_is_a_row_or_says_why_not`.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const TO_MOVE: &[&str] = &[];

/// The module functions with a form of their own: the database's effects,
/// which the session layer performs (a connection, a transaction, a write
/// held to the handler's commit).
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const MODULE_OWN_FORM: &[(&str, &str)] = &[("db", "connect"), ("db", "query"), ("db", "exec"), ("db", "insert"), ("db", "migrate"), ("db", "transact"), ("db", "describe")];

/// The module functions still written twice, each to become a row (docs/
/// bug-classes.md §4); like TO_MOVE, it only shrinks.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const MODULE_TO_MOVE: &[(&str, &str)] = &[];

#[cfg(test)]
mod tests {
    use super::*;

    fn int(n: i64) -> CrowValue {
        CrowValue::ok(Value::Int(n))
    }
    fn text(s: &str) -> CrowValue {
        CrowValue::ok(Value::Str(s.to_string()))
    }

    #[test]
    fn a_rows_path_is_the_function_it_calls() {
        let r = by_id("text.repeat").expect("the row");
        assert_eq!(r.rust_path(), "crow_runtime::text::repeat");
        assert_eq!(r.params, &[Kind::Str, Kind::Int]);
        assert_eq!(r.ty(), Ty::Str);
        assert_eq!(by_id("moments.moment_text").expect("the row").ty(), Ty::Result(Box::new(Ty::Moment)));
        assert_eq!(by_id("moments.now").expect("the row").ty(), Ty::Moment);
    }

    #[test]
    fn a_failure_is_what_the_row_says() {
        let repeat = by_id("text.repeat").expect("the row");
        assert_eq!((repeat.call)(&[text("ab"), int(2)], "t:1").map(|v| format!("{}", v.value)), Ok("abab".to_string()));
        assert_eq!((repeat.call)(&[text("ab"), int(-1)], "t:1").map(|_| ()), Err("repeat[] count cannot be negative (got -1)".to_string()));
        let parse = by_id("moments.moment_text_in").expect("the row");
        let bad = (parse.call)(&[text("not a time"), text("UTC")], "t:3").expect("an error value, not a stop");
        assert!(bad.is_error());
        assert_eq!(bad.kind(), "time");
        assert_eq!(bad.trace(), &["t:3".to_string()]);
    }

    /// The one row choice for a bare name, as inference and the lowering
    /// both read it: the core's row the kinds fit, else the one module whose
    /// row takes them, else the first argument's class, else none.
    #[test]
    fn a_bare_name_reaches_one_module_by_its_kinds() {
        let both = ["frame", "math"];
        // where of a mask, a column and an int is the frame's; of three tensors math's
        assert_eq!(module_of_bare("where", &both, &[Arg::Truths, Arg::Column, Arg::Int]), Some("frame".to_string()));
        assert_eq!(module_of_bare("where", &both, &[Arg::Tensor, Arg::Tensor, Arg::Tensor]), Some("math".to_string()));
        // sort of a list is the core's (none), of a frame the frame's
        assert_eq!(module_of_bare("sort", &["frame"], &[Arg::Ints]), None);
        assert_eq!(module_of_bare("sort", &["frame"], &[Arg::Frame, Arg::Text]), Some("frame".to_string()));
        // a kind not known yet is taken by any row, so the count decides:
        // unique of a frame and a name fits the frame's rows, of one value
        // math's; of a list the kinds pick math's (the core's unique is a
        // tensor's)
        assert_eq!(module_of_bare("unique", &both, &[Arg::Frame, Arg::Unknown]), Some("frame".to_string()));
        assert_eq!(module_of_bare("unique", &both, &[Arg::Unknown]), Some("math".to_string()));
        assert_eq!(module_of_bare("stack", &both, &[Arg::Unknown, Arg::Unknown]), None, "two rows take two unknowns and the class of nothing is nothing");
        // one provider is reached whatever the kinds say
        assert_eq!(module_of_bare("rank", &["math"], &[Arg::Unknown]), Some("math".to_string()));
        assert_eq!(module_of_bare("nope", &[], &[Arg::Int]), None);
    }

    /// A core builtin is a row, has a form of its own, or is still to move,
    /// and exactly one of these: a builtin added outside the table, or a
    /// moved one left on the list, fails here.
    #[test]
    fn every_core_builtin_is_a_row_or_says_why_not() {
        for name in crate::lower_ir::CORE_RESERVED {
            let places = [core(name).is_some(), OWN_FORM.contains(name), TO_MOVE.contains(name)];
            assert_eq!(places.iter().filter(|p| **p).count(), 1, "'{}' is in {} of: the table, OWN_FORM, TO_MOVE", name, places.iter().filter(|p| **p).count());
        }
        for r in ROWS.iter().filter(|r| r.module.is_empty()) {
            // (an internal name is a core builtin's form: `__fmt_time`)
            assert!(crate::lower_ir::CORE_RESERVED.contains(&r.name) || r.name.starts_with("__"), "row '{}' names no core builtin", r.name);
        }
    }

    /// A module function is a row, has a form of its own, or is still to
    /// move, and exactly one of these.
    #[test]
    fn every_module_function_is_a_row_or_says_why_not() {
        use crate::modules::{chart, db, doc, frame, io, math, nn, stats};
        let modules: [(&str, &[&str]); 8] = [
            ("math", math::MATH_FUNCTIONS),
            ("math.stats", stats::STATS_FUNCTIONS),
            ("math.nn", nn::NN_FUNCTIONS),
            ("frame", frame::FRAME_FUNCTIONS),
            ("db", db::DB_FUNCTIONS),
            ("io", io::IO_FUNCTIONS),
            ("doc", doc::DOC_FUNCTIONS),
            ("chart", chart::CHART_FUNCTIONS),
        ];
        for (m, names) in modules {
            for name in names {
                let row = ROWS.iter().any(|r| r.module == m && r.name == *name);
                let places = [row, MODULE_OWN_FORM.contains(&(m, name)), MODULE_TO_MOVE.contains(&(m, name))];
                assert_eq!(places.iter().filter(|p| **p).count(), 1, "{}.{} is in {} of: the table, MODULE_OWN_FORM, MODULE_TO_MOVE", m, name, places.iter().filter(|p| **p).count());
            }
        }
    }

    /// A rest parameter takes each of the rest as its kind, and a count no
    /// row of a name takes names every count the name takes.
    #[test]
    fn a_rest_takes_the_rest_and_a_miscount_names_the_counts() {
        let anova = module("math.stats", "anova", &[Arg::Tensor; 3]).expect("the row");
        assert_eq!(kinds_of(anova.params, 3), Some(vec![Kind::Tensor; 3]));
        assert_eq!(kinds_of(by_id("math.full").expect("the row").params, 3), Some(vec![Kind::Int, Kind::Int, Kind::Float]));
        assert_eq!(kinds_of(by_id("chart.sma").expect("the row").params, 3), None);
        assert_eq!(miscounted("chart", "rsi", 3), Some("rsi[] takes 2 or 4 arguments, got 3".to_string()));
        assert_eq!(miscounted("chart", "rsi", 4), None);
        assert_eq!(miscounted("math", "zeros", 0), None);
        assert_eq!(miscounted("math.stats", "median", 2), Some("median[] takes 1 argument, got 2".to_string()));
        let zeros = by_id("math.zeros").expect("the row");
        let printed = print(zeros, &["a".to_string(), "b".to_string()], "7", None).expect("prints");
        assert!(printed.starts_with("crow_runtime::math::zeros(&[") && printed.contains("(a)") && printed.contains("(b)"), "{printed}");
        assert!(printed.ends_with("]).unwrap_or_else(|e| crow_stop(7, e))"), "{printed}");
        // a row written in Crow is never printed as a runtime call
        let refused = print(anova, &["a".to_string(), "b".to_string()], "7", None).expect_err("refused");
        assert!(refused.starts_with("a bug in Crow: `anova` is written in Crow"), "{refused}");
    }

    /// A Rust row's text parameter takes a literal as the literal and an
    /// indexed read as a reference: neither allocates or copies per call
    #[test]
    fn a_text_parameter_takes_a_literal_and_an_indexed_read_without_a_copy() {
        assert_eq!(literal_text("\"\\t\".to_string()"), Some("\"\\t\"".to_string()));
        assert_eq!(literal_text("\"a\".to_string() + \"b\".to_string()"), None);
        assert_eq!(literal_text("name.to_string()"), None);
        let indexed = "{ let _i = 2_i64; let _v = &rows; _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))].clone() }";
        let borrowed = "{ let _i = 2_i64; let _v = &rows; &_v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))] }";
        assert_eq!(borrowed_index(indexed).as_deref(), Some(borrowed));
        // an item that is Copy is not cloned, and is not a borrow either
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &xs; _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))] }"), None);
        // a list an expression makes dies with the block: the item is cloned
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &crow_runtime::text::split(&*(s), \" \"); _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))].clone() }"), None);
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &(_t4).value.names; _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))].clone() }"), None);
        assert!(borrowed_index("{ let _i = 0_i64; let _v = &r.names; _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop(5, e))].clone() }").is_some());
        assert_eq!(Kind::Str.print("\"\\t\".to_string()", "3"), "\"\\t\"");
        assert_eq!(Kind::Str.print(indexed, "3"), format!("&*({})", borrowed));
        assert_eq!(Kind::Str.print("name", "3"), "&*(name)");
    }

    #[test]
    fn every_id_is_unique() {
        for (i, r) in ROWS.iter().enumerate() {
            assert!(ROWS.iter().skip(i + 1).all(|o| o.id != r.id), "{} twice", r.id);
        }
    }

    /// A JSON number's length stops the compiled program with the words the
    /// interpreter's refusal gives the number it parsed to.
    #[test]
    fn a_json_value_without_a_length_is_refused_in_the_tables_words() {
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(5)), Err(refusal(&[""], "len", &[Arg::Int]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(2.5)), Err(refusal(&[""], "len", &[Arg::Float]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(true)), Err(refusal(&[""], "len", &[Arg::Truth]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!([1, 2])), Ok(2));
    }

    /// `log` is its row over any values that show, each handed as the one
    /// display takes it; env's default is text, as the variable is; the math
    /// of a plain number is its row, an int's int and a float's float.
    #[test]
    fn a_builtin_once_per_mode_is_a_row() {
        let log = core("log").expect("log's row");
        let printed = print(log, &["a".to_string(), "b".to_string()], "1", None).expect("printed");
        assert!(printed.contains("&(a) as &dyn crow_runtime::display::CrowDisplay"), "{}", printed);
        assert_eq!(crow_runtime::core::env_or("CROW_TABLE_TEST_NO_SUCH_VARIABLE", &crow_runtime::cell::Cell::Int(3000)), Ok("3000".to_string()));
        assert_eq!(core_row("abs", &[Arg::Int]).map(|r| r.id), Some("core.abs_int"));
        assert_eq!(core_row("abs", &[Arg::Float]).map(|r| r.id), Some("core.abs"));
        assert_eq!(crow_runtime::core::round(2.5), 3);
        assert_eq!(crow_runtime::core::round(-2.5), -3);
        assert!(crow_runtime::core::sleep_ms(-1).is_err());
        assert!(!OWN_FORM.contains(&"log") && OWN_FORM.contains(&"assert"));
    }

    /// A list of numbers that can fail is numbers where numbers are taken
    /// (a failed one stops as it is read); a list of parsed values is one
    /// whose items' kind shows as the program runs.
    #[test]
    fn a_list_is_what_its_items_are() {
        let fallible = Ty::List(Box::new(Ty::Result(Box::new(Ty::Float))));
        assert_eq!(Arg::of_type(&fallible), Arg::FallibleNumbers);
        assert!(core_row("sum", &[Arg::FallibleNumbers]).is_none(), "sum is math's");
        assert!(module("math", "sum", &[Arg::FallibleNumbers]).is_some_and(|r| takes(r, &[Arg::FallibleNumbers])));
        // a list of parsed values is a list, whatever its items turn out to
        // be: a list's parameter takes it, a text's does not
        assert_eq!(Arg::of_type(&Ty::List(Box::new(Ty::Json))), Arg::AnyList);
        assert_eq!(Arg::of_type(&Ty::List(Box::new(Ty::Unknown))), Arg::AnyList);
        assert_eq!(core_row("len", &[Arg::AnyList]).map(|r| r.id), Some("core.len"));
        assert!(core_row("upper", &[Arg::AnyList]).is_none());
        assert!(refusal(&[""], "upper", &[Arg::AnyList]).is_some());
        assert!(refusal(&[""], "upper", &[Arg::Json]).is_some_and(|e| e.ends_with(READ_FIRST)));
    }

    /// A collection is taken, not copied, by the call that keeps it: the
    /// interpreter's `take` moves the list into the runtime's push.
    #[test]
    fn a_collection_is_moved_into_its_row() {
        let push = core_row("push", &[Arg::Ints, Arg::Int]).expect("the generic push");
        assert_eq!(push.id, "core.push");
        let grown = (push.take)(vec![CrowValue::ok(Value::Array(vec![int(1)])), int(2)], "t:1").expect("a list");
        assert_eq!(format!("{}", grown.value), "[1, 2]");
        assert_eq!(core_row("sort", &[Arg::Floats]).map(|r| r.id), Some("core.sort_floats"));
        assert!(core_row("sort", &[Arg::Records]).is_none());
        assert_eq!(core_row("len", &[Arg::Column]).map(|r| r.id), Some("core.len_column"));
        assert!(core_row("contains", &[Arg::Column, Arg::Int]).is_none());
    }
}
