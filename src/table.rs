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

use crate::type_infer::CrowType;
use crate::value::{CrowValue, Value};
use crow_runtime::tensor::CrowTensor;

/// What a parameter or an answer is, as the runtime function takes or
/// gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// `i64`: an int, or a float cut toward zero (as `as i64` does).
    Int,
    /// `f64`: a float, or an int.
    Float,
    Bool,
    /// `&str` taken, `String` given.
    Str,
    /// `&CrowTensor` taken, `CrowTensor` given: a tensor, or a list of
    /// numbers.
    Tensor,
    /// `BTreeMap<String, i64>` given.
    IntMap,
    /// `&[i64]` taken: the rest of the arguments, each a size (`zeros[2, 3]`,
    /// `poisson[1.5, 4]`); the fixed parameters around it take theirs first.
    Sizes,
    /// `Vec<CrowTensor>` given: a decomposition's parts (`svd`, `qr`).
    Tensors,
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
    /// The id the IR names the row by, as `Target::Runtime`.
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
}

impl Row {
    /// The runtime function's path as the emitter prints it.
    pub fn rust_path(&self) -> String {
        self.path.replace(' ', "")
    }

    /// The call's type, for inference and the lowering: the answer's, able
    /// to fail when a failure is a value.
    pub fn crow_type(&self) -> CrowType {
        let t = self.ret.crow_type();
        match self.fails {
            Fails::Value(_) => CrowType::Result(Box::new(t)),
            _ => t,
        }
    }
}

impl Kind {
    pub fn crow_type(self) -> CrowType {
        match self {
            Kind::Int => CrowType::Int,
            Kind::Float => CrowType::Float,
            Kind::Bool => CrowType::Bool,
            Kind::Str => CrowType::Str,
            Kind::Tensor => CrowType::NumArray(Box::new(CrowType::Float)),
            Kind::IntMap => CrowType::Map(Box::new(CrowType::Int)),
            Kind::Sizes => CrowType::Array(Box::new(CrowType::Int)),
            Kind::Tensors => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
        }
    }

    /// An argument printed the way the runtime function takes it.
    pub fn print(self, arg: &str) -> String {
        match self {
            Kind::Int => format!("({}) as i64", arg),
            Kind::Float => format!("({}) as f64", arg),
            Kind::Bool => format!("({})", arg),
            Kind::Str => format!("&*({})", arg),
            Kind::Tensor => format!("&({})", arg),
            Kind::IntMap => format!("&({})", arg),
            Kind::Sizes => format!("({}) as i64", arg),
            Kind::Tensors => format!("&({})", arg),
        }
    }

    /// The value the compiled program holds for a failed call of this kind.
    pub fn default_rust(self) -> &'static str {
        match self {
            Kind::Int => "0_i64",
            Kind::Float => "0.0_f64",
            Kind::Bool => "false",
            Kind::Str => "String::new()",
            Kind::Tensor => "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])",
            Kind::IntMap => "std::collections::BTreeMap::new()",
            Kind::Sizes => "Vec::<i64>::new()",
            Kind::Tensors => "Vec::<crow_runtime::tensor::CrowTensor>::new()",
        }
    }
}

/// The arguments grouped by parameter: one each, and the sizes the rest
/// (the same grouping both modes use: `groups` for the interpreter's
/// values, [`print`] for the compiled program's text).
fn spans(params: &[Kind], argc: usize, name: &str) -> Result<Vec<(usize, usize)>, String> {
    let fixed = params.iter().filter(|k| **k != Kind::Sizes).count();
    let rest = params.contains(&Kind::Sizes);
    if (!rest && argc != fixed) || (rest && argc < fixed) {
        let wanted = if rest { format!("at least {}", fixed) } else { fixed.to_string() };
        return Err(format!("{}[] takes {} arguments, got {}", name, wanted, argc));
    }
    let mut at = 0;
    let mut out = Vec::with_capacity(params.len());
    for k in params {
        let n = if *k == Kind::Sizes { argc - fixed } else { 1 };
        out.push((at, at + n));
        at += n;
    }
    Ok(out)
}

/// The interpreter's arguments grouped by the row's parameters.
pub fn groups<'a>(params: &[Kind], args: &'a [CrowValue], name: &str) -> Result<Vec<&'a [CrowValue]>, String> {
    Ok(spans(params, args.len(), name)?.into_iter().map(|(a, b)| &args[a..b]).collect())
}

/// A row's call as the compiled program prints it: each argument as its
/// kind takes it, the failure as the row says. `line` is the stop's line
/// (a module's code carries `modules::STOP_LINE` for the emitter to fill),
/// `trace` where an error value is made (none for a module's function,
/// whose error values carry no trace in either mode).
pub fn print(row: &Row, args: &[String], line: &str, trace: Option<&str>) -> Result<String, String> {
    let mut printed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans(row.params, args.len(), row.name)?) {
        match k {
            Kind::Sizes => printed.push(format!("&[{}]", args[a..b].iter().map(|x| k.print(x)).collect::<Vec<_>>().join(", "))),
            _ => printed.push(k.print(&args[a])),
        }
    }
    let call = format!("{}({})", row.rust_path(), printed.join(", "));
    Ok(match row.fails {
        Fails::Never => call,
        Fails::Stops => format!("{}.unwrap_or_else(|e| crow_stop({}, e))", call, line),
        Fails::Value(kind) => format!(
            "match {} {{ Ok(_v) => CrowResult {{ ok: true, value: _v, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: {}, reason: _e, kind: \"{}\".to_string(), trace: vec![{}], captured: 0.0 }} }}",
            call,
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
        Value::Array(items) => {
            let mut data = Vec::with_capacity(items.len());
            for item in items {
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

pub fn write_int_map(m: BTreeMap<String, i64>) -> CrowValue {
    CrowValue::ok(Value::Map(m.into_iter().map(|(k, v)| (k, write_int(v))).collect()))
}

pub fn write_tensors(ts: Vec<CrowTensor>) -> CrowValue {
    CrowValue::ok(Value::Array(ts.into_iter().map(write_tensor).collect()))
}

/// One argument of a group, the group of a single parameter.
pub fn one(group: &[CrowValue]) -> Result<&CrowValue, String> {
    group.first().ok_or_else(|| "an argument is missing".to_string())
}

/// The sizes of a group, each an int.
pub fn read_sizes(group: &[CrowValue]) -> Result<Vec<i64>, String> {
    group.iter().map(read_int).collect()
}

/// The reading of a parameter's group by its kind, as the function takes it.
macro_rules! read {
    (Int, $g:expr) => { read_int(one($g)?)? };
    (Float, $g:expr) => { read_float(one($g)?)? };
    (Bool, $g:expr) => { read_bool(one($g)?)? };
    (Str, $g:expr) => { read_str(one($g)?)? };
    (Tensor, $g:expr) => { &*read_tensor(one($g)?)? };
    (Sizes, $g:expr) => { &read_sizes($g)? };
}

/// The value of an answer by its kind.
macro_rules! answer {
    (Int, $x:expr) => { write_int($x) };
    (Float, $x:expr) => { write_float($x) };
    (Bool, $x:expr) => { write_bool($x) };
    (Str, $x:expr) => { write_str($x) };
    (Tensor, $x:expr) => { write_tensor($x) };
    (IntMap, $x:expr) => { write_int_map($x) };
    (Tensors, $x:expr) => { write_tensors($x) };
}

/// A row: its id, Crow name and module, the runtime function, the kinds of
/// its parameters and answer, and what a failure is (`never`, `stops`, or
/// `value "kind"`). The interpreter's call is made from the same tokens the
/// emitter prints, so the two cannot name different functions.
macro_rules! row {
    ($id:literal, $module:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, never) => {
        Row {
            id: $id, name: $name, module: $module, path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: Fails::Never,
            call: |args: &[CrowValue], _trace: &str| {
                let groups = groups(&[$(Kind::$k),*], args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&[]);
                let _ = &mut next;
                Ok(answer!($r, $f($(read!($k, next())),*)))
            },
        }
    };
    ($id:literal, $module:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, stops) => {
        Row {
            id: $id, name: $name, module: $module, path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: Fails::Stops,
            call: |args: &[CrowValue], _trace: &str| {
                let groups = groups(&[$(Kind::$k),*], args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&[]);
                let _ = &mut next;
                let got = $f($(read!($k, next())),*)?;
                Ok(answer!($r, got))
            },
        }
    };
    ($id:literal, $module:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, value $kind:literal) => {
        Row {
            id: $id, name: $name, module: $module, path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: Fails::Value($kind),
            call: |args: &[CrowValue], trace: &str| {
                let groups = groups(&[$(Kind::$k),*], args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&[]);
                let _ = &mut next;
                Ok(match $f($(read!($k, next())),*) {
                    Ok(got) => answer!($r, got),
                    Err(e) => CrowValue::error_with_kind(&e, $kind, trace),
                })
            },
        }
    };
}

/// The rows. A builtin here is written once, in the runtime; its old arms
/// in `builtins.rs` and `emit_rs/builtins.rs` are gone.
pub static ROWS: &[Row] = &[
    row!("moments.moment", "", "moment", crow_runtime::moments::moment, (Int, Int, Int, Int, Int, Float, Str) -> Float, value "time"),
    row!("moments.calendar", "", "calendar", crow_runtime::moments::calendar_map, (Float, Str) -> IntMap, value "time"),
    row!("moments.parse_time", "", "parse_time", crow_runtime::moments::parse_time, (Str, Str) -> Float, value "time"),
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
    row!("math.randn", "math", "randn", crow_runtime::math::randn, (Sizes) -> Tensor, stops),
    row!("math.full", "math", "full", crow_runtime::math::full, (Sizes, Float) -> Tensor, stops),
    row!("math.eye", "math", "eye", crow_runtime::math::eye, (Int) -> Tensor, stops),
    row!("math.linspace", "math", "linspace", crow_runtime::math::linspace, (Float, Float, Int) -> Tensor, stops),
    row!("math.arange", "math", "arange", crow_runtime::math::arange, (Float, Float, Float) -> Tensor, stops),
    row!("math.diag", "math", "diag", crow_runtime::math::diag, (Tensor) -> Tensor, stops),
    row!("math.normal", "math", "normal", crow_runtime::math::normal, (Float, Float, Sizes) -> Tensor, stops),
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
];

/// The row the IR names by `id`.
pub fn by_id(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The core builtin's row by its Crow name.
pub fn core(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module.is_empty() && r.name == name)
}

/// What an argument is, as the choice among a name's rows reads it: from
/// the interpreter's value, or from the IR's type (the verifier keeps the
/// two agreeing, so both modes choose the same row).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    Number,
    Tensor,
    Text,
    Truth,
    /// Not known here: any parameter takes it.
    Unknown,
}

impl Arg {
    pub fn of_type(t: &CrowType) -> Arg {
        match t {
            CrowType::Result(inner) => Arg::of_type(inner),
            CrowType::Int | CrowType::Float => Arg::Number,
            CrowType::NumArray(_) | CrowType::Array(_) | CrowType::FrameCol => Arg::Tensor,
            CrowType::Str | CrowType::Markup => Arg::Text,
            CrowType::Bool => Arg::Truth,
            _ => Arg::Unknown,
        }
    }

    pub fn of_value(v: &CrowValue) -> Arg {
        match &v.value {
            Value::Int(_) | Value::Float(_) => Arg::Number,
            Value::NumArray(_) | Value::Array(_) => Arg::Tensor,
            Value::Str(_) => Arg::Text,
            Value::Bool(_) => Arg::Truth,
            _ => Arg::Unknown,
        }
    }
}

impl Kind {
    /// Whether a parameter of this kind takes an argument of that kind.
    fn takes(self, a: Arg) -> bool {
        a == Arg::Unknown
            || match self {
                Kind::Int | Kind::Float | Kind::Sizes => a == Arg::Number,
                Kind::Tensor => a == Arg::Tensor,
                Kind::Str => a == Arg::Text,
                Kind::Bool => a == Arg::Truth,
                Kind::IntMap | Kind::Tensors => true,
            }
    }
}

/// Whether a row takes these arguments: their count, and each one's kind.
fn takes(row: &Row, args: &[Arg]) -> bool {
    match spans(row.params, args.len(), row.name) {
        Ok(spans) => row.params.iter().zip(spans).all(|(k, (a, b))| args[a..b].iter().all(|x| k.takes(*x))),
        Err(_) => false,
    }
}

/// A module function's row for a call with these arguments: of the rows of
/// its name, the first that takes their count and kinds (`exp` of a number
/// or of a tensor, `norm[t]` or `norm[t, p]`); then the first that takes
/// their count; then the first, whose own check refuses the call.
pub fn module(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    let named = || ROWS.iter().filter(move |r| r.module == module && r.name == name);
    named()
        .find(|r| takes(r, args))
        .or_else(|| named().find(|r| spans(r.params, args.len(), r.name).is_ok()))
        .or_else(|| named().next())
}

/// The core names with syntax or state of their own, which no row says: a
/// statement (`log`), a value's own status (`error`, `captured`), what the
/// interpreter holds (`later`, `running`, `notice`), a function argument
/// (`any`, `all`, `find`, `grad`), a resource (`cpu`, `gpu`, `nodes`), a
/// test block, a chart's tree.
pub const OWN_FORM: &[&str] = &[
    "log", "type", "map", "error", "assert", "captured", "later", "running", "notice", "any", "all", "find", "grad", "cpu", "gpu", "nodes", "test",
    "chart", "args", "exit", "sleep", "time", "now", "input", "env",
];

/// The core builtins still written twice, each to become a row (docs/
/// bug-classes.md §4). The list only shrinks: a builtin added outside the
/// table fails `every_core_builtin_is_a_row_or_says_why_not`.
pub const TO_MOVE: &[&str] = &[
    "abs", "sqrt", "floor", "ceil", "round", "min", "max", "split", "trim", "replace", "upper", "lower", "join", "starts", "ends", "fmt", "int", "float",
    "str", "len", "contains", "slice", "push", "pop", "sort", "reverse", "concat", "range", "keys", "values", "has", "set", "del", "read", "write",
    "exists", "mkdir", "ls", "rm", "mv", "cp", "rand", "json", "encode", "shell", "hash",
];

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
        assert_eq!(r.crow_type(), CrowType::Str);
        assert_eq!(by_id("moments.calendar").expect("the row").crow_type(), CrowType::Result(Box::new(CrowType::Map(Box::new(CrowType::Int)))));
    }

    #[test]
    fn a_failure_is_what_the_row_says() {
        let repeat = by_id("text.repeat").expect("the row");
        assert_eq!((repeat.call)(&[text("ab"), int(2)], "t:1").map(|v| format!("{}", v.value)), Ok("abab".to_string()));
        assert_eq!((repeat.call)(&[text("ab"), int(-1)], "t:1").map(|_| ()), Err("repeat[] count cannot be negative (got -1)".to_string()));
        let parse = by_id("moments.parse_time").expect("the row");
        let bad = (parse.call)(&[text("not a time"), text("UTC")], "t:3").expect("an error value, not a stop");
        assert!(bad.is_error());
        assert_eq!(bad.kind(), "time");
        assert_eq!(bad.trace(), &["t:3".to_string()]);
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
            assert!(crate::lower_ir::CORE_RESERVED.contains(&r.name), "row '{}' names no core builtin", r.name);
        }
    }

    #[test]
    fn every_id_is_unique() {
        for (i, r) in ROWS.iter().enumerate() {
            assert!(ROWS.iter().skip(i + 1).all(|o| o.id != r.id), "{} twice", r.id);
        }
    }
}
