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
    /// `i64`: an int only — a row beside a float's row of the same name
    /// (`min` of two ints is an int, of a float a float).
    Whole,
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
            Kind::Int | Kind::Whole => CrowType::Int,
            Kind::Float => CrowType::Float,
            Kind::Bool => CrowType::Bool,
            Kind::Str => CrowType::Str,
            Kind::Tensor => CrowType::NumArray(Box::new(CrowType::Float)),
            Kind::IntMap => CrowType::Map(Box::new(CrowType::Int)),
            Kind::FloatMap => CrowType::Map(Box::new(CrowType::Float)),
            Kind::Sizes => CrowType::Array(Box::new(CrowType::Int)),
            Kind::Tensors | Kind::Groups => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
            Kind::Unit => CrowType::Void,
            Kind::Sparse => CrowType::Sparse,
            Kind::Frame => CrowType::Frame,
            Kind::Strs => CrowType::Array(Box::new(CrowType::Str)),
            Kind::Bools => CrowType::Array(Box::new(CrowType::Bool)),
            Kind::Cell | Kind::Cells => CrowType::Unknown,
            Kind::Lists => CrowType::Array(Box::new(CrowType::Array(Box::new(CrowType::Str)))),
            Kind::Words => CrowType::Array(Box::new(CrowType::Str)),
        }
    }

    /// An argument printed the way the runtime function takes it (each of
    /// a rest parameter's, as its [`Kind::each`]).
    pub fn print(self, arg: &str) -> String {
        match self.each() {
            Kind::Int | Kind::Whole => format!("({}) as i64", arg),
            Kind::Float => format!("({}) as f64", arg),
            Kind::Bool => format!("({})", arg),
            Kind::Str => format!("&*({})", arg),
            Kind::Unit => format!("({})", arg),
            Kind::Strs | Kind::Bools => format!("&({})[..]", arg),
            Kind::Cell => format!("&crow_runtime::cell::Cell::from(({}).clone())", arg),
            Kind::Words => format!("&crow_runtime::cell::words(&({}))[..]", arg),
            Kind::Cells => format!("crow_runtime::frames::Cells::from(({}).clone())", arg),
            _ => format!("&({})", arg),
        }
    }

    /// Whether the parameter takes the rest of the arguments.
    pub fn is_rest(self) -> bool {
        matches!(self, Kind::Sizes | Kind::Groups | Kind::Lists)
    }

    /// The kind of each argument a parameter takes: a size of the sizes, a
    /// tensor of the groups.
    pub fn each(self) -> Kind {
        match self {
            Kind::Sizes => Kind::Int,
            Kind::Groups => Kind::Tensor,
            Kind::Lists => Kind::Strs,
            k => k,
        }
    }

    /// The value the compiled program holds for a failed call of this kind.
    pub fn default_rust(self) -> &'static str {
        match self {
            Kind::Int | Kind::Whole => "0_i64",
            Kind::Float => "0.0_f64",
            Kind::Bool => "false",
            Kind::Str => "String::new()",
            Kind::Tensor => "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])",
            Kind::IntMap | Kind::FloatMap => "std::collections::BTreeMap::new()",
            Kind::Sizes => "Vec::<i64>::new()",
            Kind::Tensors | Kind::Groups => "Vec::<crow_runtime::tensor::CrowTensor>::new()",
            Kind::Unit => "()",
            Kind::Sparse => "crow_runtime::sparse::CrowSparse::from_dense(&crow_runtime::tensor::CrowTensor::zeros(vec![0, 0]))",
            Kind::Frame => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])",
            Kind::Strs => "Vec::<String>::new()",
            Kind::Bools => "Vec::<bool>::new()",
            Kind::Cell => "crow_runtime::cell::Cell::Int(0)",
            Kind::Cells => "crow_runtime::frames::Cells::One(crow_runtime::cell::Cell::Int(0))",
            Kind::Lists => "Vec::<Vec<String>>::new()",
            Kind::Words => "Vec::<String>::new()",
        }
    }
}

/// The arguments grouped by parameter: one each, and a rest parameter the
/// rest (the same grouping both modes use: `groups` for the interpreter's
/// values, [`print`] for the compiled program's text).
fn spans(params: &[Kind], argc: usize, name: &str) -> Result<Vec<(usize, usize)>, String> {
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

/// A row's call as the compiled program prints it: each argument as its
/// kind takes it, the failure as the row says. `line` is the stop's line
/// (a module's code carries `modules::STOP_LINE` for the emitter to fill),
/// `trace` where an error value is made (none for a module's function,
/// whose error values carry no trace in either mode).
pub fn print(row: &Row, args: &[String], line: &str, trace: Option<&str>) -> Result<String, String> {
    let mut printed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans(row.params, args.len(), row.name)?) {
        if k.is_rest() {
            printed.push(format!("&[{}]", args[a..b].iter().map(|x| k.print(x)).collect::<Vec<_>>().join(", ")));
        } else {
            printed.push(k.print(&args[a]));
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

pub fn write_float_map(m: BTreeMap<String, f64>) -> CrowValue {
    CrowValue::ok(Value::Map(m.into_iter().map(|(k, v)| (k, write_float(v))).collect()))
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
        Value::Int(n) => Ok(Cell::Int(*n)),
        Value::Float(x) => Ok(Cell::Float(*x)),
        Value::Bool(b) => Ok(Cell::Bool(*b)),
        Value::Str(s) => Ok(Cell::Text(s.clone())),
        Value::Done => Ok(Cell::Null),
        Value::Array(_) => Ok(Cell::Other("a list")),
        Value::Map(_) => Ok(Cell::Other("a map")),
        _ => Err(format!("expected a number, text or a bool, got {}", named(v))),
    }
}

/// A list's items as the text `join` writes them, as the compiled
/// program's `Words` reads its typed list.
pub fn read_words(v: &CrowValue) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => items
            .iter()
            .map(|i| match &i.value {
                Value::Str(s) => Ok(s.clone()),
                Value::Int(n) => Ok(n.to_string()),
                Value::Float(x) => Ok(x.to_string()),
                Value::Bool(b) => Ok(b.to_string()),
                _ => Err(format!("expected a list of numbers, text or bools, got a list of {}", named(i))),
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
    let present = || items.iter().filter(|i| !i.is_error());
    let valid: Vec<bool> = items.iter().map(|i| !i.is_error()).collect();
    let column = if present().all(|i| matches!(i.value, Value::Int(_))) && present().next().is_some() {
        Column::Int(items.iter().map(|i| if let Value::Int(n) = i.value { n } else { 0 }).collect())
    } else if present().all(|i| matches!(i.value, Value::Int(_) | Value::Float(_))) && present().next().is_some() {
        Column::Float(items.iter().map(|i| read_float(i).unwrap_or(f64::NAN)).collect())
    } else if present().all(|i| matches!(i.value, Value::Bool(_))) && present().next().is_some() {
        Column::Bool(items.iter().map(|i| matches!(i.value, Value::Bool(true))).collect())
    } else if present().all(|i| matches!(i.value, Value::Str(_))) {
        Column::Str(items.iter().map(|i| if let Value::Str(s) = &i.value { s.clone() } else { String::new() }).collect())
    } else {
        let other = present().find(|i| !matches!(i.value, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_)));
        return Err(format!("expected a column's values, got a list of {}", other.map(named).unwrap_or("mixed values")));
    };
    Ok(Cells::Column(column, valid))
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

/// One argument of a group, the group of a single parameter.
pub fn one(group: &[CrowValue]) -> Result<&CrowValue, String> {
    group.first().ok_or_else(|| "an argument is missing".to_string())
}

/// The sizes of a group, each an int.
pub fn read_sizes(group: &[CrowValue]) -> Result<Vec<i64>, String> {
    group.iter().map(read_int).collect()
}

/// The groups of a group, each a tensor.
pub fn read_groups(group: &[CrowValue]) -> Result<Vec<Cow<'_, CrowTensor>>, String> {
    group.iter().map(read_tensor).collect()
}

/// The reading of a parameter's group by its kind, as the function takes it.
macro_rules! read {
    (Int, $g:expr) => { read_int(one($g)?)? };
    (Whole, $g:expr) => { read_whole(one($g)?)? };
    (Words, $g:expr) => { &read_words(one($g)?)? };
    (Float, $g:expr) => { read_float(one($g)?)? };
    (Bool, $g:expr) => { read_bool(one($g)?)? };
    (Str, $g:expr) => { read_str(one($g)?)? };
    (Tensor, $g:expr) => { &*read_tensor(one($g)?)? };
    (Tensors, $g:expr) => { &read_tensors(one($g)?)? };
    (Sparse, $g:expr) => { read_sparse(one($g)?)? };
    (Frame, $g:expr) => { read_frame(one($g)?)? };
    (Strs, $g:expr) => { &read_strs(one($g)?)? };
    (Sizes, $g:expr) => { &read_sizes($g)? };
    (Groups, $g:expr) => { &read_groups($g)?.iter().map(|t| &**t).collect::<Vec<&CrowTensor>>() };
    (Bools, $g:expr) => { &read_bools(one($g)?)? };
    (Cell, $g:expr) => { &read_cell(one($g)?)? };
    (Cells, $g:expr) => { read_cells(one($g)?)? };
    (Lists, $g:expr) => { &read_lists($g)?.iter().map(|l| l.as_slice()).collect::<Vec<&[String]>>() };
}

/// The value of an answer by its kind.
macro_rules! answer {
    (Int, $x:expr) => { write_int($x) };
    (Float, $x:expr) => { write_float($x) };
    (Bool, $x:expr) => { write_bool($x) };
    (Str, $x:expr) => { write_str($x) };
    (Tensor, $x:expr) => { write_tensor($x) };
    (IntMap, $x:expr) => { write_int_map($x) };
    (FloatMap, $x:expr) => { write_float_map($x) };
    (Frame, $x:expr) => { write_frame($x) };
    (Strs, $x:expr) => { write_strs($x) };
    (Tensors, $x:expr) => { write_tensors($x) };
    (Unit, $x:expr) => { write_unit($x) };
    (Sparse, $x:expr) => { write_sparse($x) };
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
    row!("display.fmt_time", "", "__fmt_time", crow_runtime::moments::fmt_moment, (Float, Str, Str) -> Str, value "time"),
    // conversions: of a number or a bool, plain; of text or a JSON value,
    // fallible (kind 'parse'), read by one rule (compiled, JSON text was
    // trimmed and a JSON bool refused)
    row!("core.int", "", "int", crow_runtime::cell::int_of_int, (Whole) -> Int, never),
    row!("core.int_of_float", "", "int", crow_runtime::cell::int_of_float, (Float) -> Int, never),
    row!("core.int_of_bool", "", "int", crow_runtime::cell::int_of_bool, (Bool) -> Int, never),
    row!("core.int_of", "", "int", crow_runtime::cell::int_of, (Cell) -> Int, value "parse"),
    row!("core.float_of_int", "", "float", crow_runtime::cell::float_of_int, (Whole) -> Float, never),
    row!("core.float", "", "float", crow_runtime::cell::float_of_float, (Float) -> Float, never),
    row!("core.float_of_bool", "", "float", crow_runtime::cell::float_of_bool, (Bool) -> Float, never),
    row!("core.float_of", "", "float", crow_runtime::cell::float_of, (Cell) -> Float, value "parse"),
    // draws from the program's generator
    row!("core.rand", "", "rand", crow_runtime::core::rand_float, () -> Float, never),
    row!("core.rand_int", "", "rand", crow_runtime::core::rand_int, (Int) -> Int, stops),
    row!("core.rand_between", "", "rand", crow_runtime::core::rand_between, (Int, Int) -> Int, stops),
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
    row!("math.argsort", "math", "argsort", crow_runtime::math::argsort, (Tensor) -> Tensor, never),
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
    row!("stats.median", "math.stats", "median", crow_runtime::stats::median, (Tensor) -> Float, never),
    row!("stats.std", "math.stats", "std", crow_runtime::stats::std, (Tensor) -> Float, never),
    row!("stats.var", "math.stats", "var", crow_runtime::stats::var, (Tensor) -> Float, never),
    row!("stats.percentile", "math.stats", "percentile", crow_runtime::stats::percentile, (Tensor, Float) -> Float, stops),
    row!("stats.zscore", "math.stats", "zscore", crow_runtime::stats::zscore, (Tensor) -> Tensor, never),
    row!("stats.corr", "math.stats", "corr", crow_runtime::stats::corr, (Tensor, Tensor) -> Float, stops),
    row!("stats.cov", "math.stats", "cov", crow_runtime::stats::cov, (Tensor, Tensor) -> Float, stops),
    row!("stats.ttest", "math.stats", "ttest", crow_runtime::stats::ttest, (Tensor, Tensor) -> FloatMap, never),
    row!("stats.linreg", "math.stats", "linreg", crow_runtime::stats::linreg, (Tensor, Tensor) -> FloatMap, stops),
    row!("stats.chisq", "math.stats", "chisq", crow_runtime::stats::chisq, (Tensor, Tensor) -> FloatMap, stops),
    row!("stats.anova", "math.stats", "anova", crow_runtime::stats::anova, (Groups) -> FloatMap, stops),
    row!("stats.kstest", "math.stats", "kstest", crow_runtime::stats::kstest, (Tensor, Str) -> FloatMap, stops),
    row!("stats.fit", "math.stats", "fit", crow_runtime::stats::fit, (Tensor, Str) -> FloatMap, stops),
    // math.nn (shapes that do not meet, a target that is no class: the
    // reason, where a kernel panicked or a loss read a logit of 0)
    row!("nn.relu", "math.nn", "relu", crow_runtime::nn::relu, (Tensor) -> Tensor, never),
    row!("nn.sigmoid", "math.nn", "sigmoid", crow_runtime::nn::sigmoid, (Tensor) -> Tensor, never),
    row!("nn.softmax", "math.nn", "softmax", crow_runtime::nn::softmax, (Tensor) -> Tensor, never),
    row!("nn.gelu", "math.nn", "gelu", crow_runtime::nn::gelu, (Tensor) -> Tensor, never),
    row!("nn.mse", "math.nn", "mse", crow_runtime::nn::mse, (Tensor, Tensor) -> Float, stops),
    row!("nn.crossentropy", "math.nn", "crossentropy", crow_runtime::nn::crossentropy, (Tensor, Tensor) -> Float, stops),
    row!("nn.bce", "math.nn", "bce", crow_runtime::nn::bce, (Tensor, Tensor) -> Float, stops),
    row!("nn.linear", "math.nn", "linear", crow_runtime::nn::linear, (Tensor, Tensor, Tensor) -> Tensor, stops),
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
    row!("frame.sort", "frame", "sort", crow_runtime::frames::sort, (Frame, Str) -> Frame, stops),
    row!("frame.sort_by", "frame", "sort", crow_runtime::frames::sort_by, (Frame, Str, Bool) -> Frame, stops),
    row!("frame.group", "frame", "group", crow_runtime::frames::group, (Frame, Str, Lists) -> Frame, stops),
    row!("frame.join", "frame", "join", crow_runtime::frames::join, (Frame, Frame, Str) -> Frame, stops),
    row!("frame.join_how", "frame", "join", crow_runtime::frames::join_how, (Frame, Frame, Str, Str) -> Frame, stops),
    row!("frame.stack", "frame", "stack", crow_runtime::frames::stack, (Frame, Frame) -> Frame, stops),
    row!("frame.head", "frame", "head", crow_runtime::frames::head, (Frame, Int) -> Frame, stops),
    row!("frame.tail", "frame", "tail", crow_runtime::frames::tail, (Frame, Int) -> Frame, stops),
    row!("frame.fill", "frame", "fill", crow_runtime::frames::fill, (Frame, Str, Cell) -> Frame, stops),
    row!("frame.unique", "frame", "unique", crow_runtime::frames::unique, (Frame, Str) -> Strs, stops),
    row!("frame.rename", "frame", "rename", crow_runtime::frames::rename, (Frame, Str, Str) -> Frame, stops),
    row!("frame.stats", "frame", "stats", crow_runtime::frames::stats, (Frame) -> Str, never),
    row!("frame.datetime", "frame", "datetime", crow_runtime::frames::datetime, (Str) -> Str, value "time"),
    row!("frame.tz", "frame", "tz", crow_runtime::frames::tz, (Str, Str) -> Str, value "time"),
];

/// The row the IR names by `id`.
pub fn by_id(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The core builtin's row by its Crow name.
pub fn core(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module.is_empty() && r.name == name)
}

/// A core builtin's row for a call with these arguments: the first whose
/// count and kinds take them (`min` of two ints, or of numbers). The
/// interpreter reads the arguments from its values, the emitter and
/// inference from their types, so the modes pick one row.
pub fn core_row(name: &str, args: &[Arg]) -> Option<&'static Row> {
    ROWS.iter().filter(|r| r.module.is_empty() && r.name == name).find(|r| takes(r, args))
}

/// Whether the core answers the name with rows.
pub fn has_core_rows(name: &str) -> bool {
    core(name).is_some()
}

/// The refusal of a call that no row of its name — in the core (`""`) and
/// in the modules given — takes, by count or by kind; none when one takes
/// it, or when an argument's kind is not known before the program runs
/// (a JSON value's is known only then).
pub fn refusal(modules: &[&str], name: &str, args: &[Arg]) -> Option<String> {
    let rows: Vec<&Row> = ROWS.iter().filter(|r| r.name == name && modules.contains(&r.module)).collect();
    if rows.is_empty() || rows.iter().any(|r| takes(r, args)) || args.iter().any(|a| matches!(a, Arg::Unknown | Arg::Json)) {
        return None;
    }
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
    for r in rows.iter().filter(|r| spans(r.params, args.len(), name).is_ok()) {
        let form = listed(&r.params.iter().map(|k| k.words().to_string()).collect::<Vec<_>>(), "and");
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    let got = listed(&args.iter().map(|a| a.words().to_string()).collect::<Vec<_>>(), "and");
    Some(format!("{}[] takes {}, got {}", shown(name), listed(&forms, "or"), got))
}

/// `a`, `a and b`, `a, b or c`.
fn listed(items: &[String], last: &str) -> String {
    match items.split_last() {
        Some((tail, rest)) if !rest.is_empty() => format!("{} {} {}", rest.join(", "), last, tail),
        _ => items.join(""),
    }
}

/// What an argument is, as the choice among a name's rows reads it: from
/// the interpreter's value, or from the IR's type (the verifier keeps the
/// two agreeing, so both modes choose the same row).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    Int,
    Float,
    /// A parsed JSON value, whose kind shows only as it runs.
    Json,
    /// A tensor, or a list of numbers.
    Tensor,
    /// A list of tensors.
    Tensors,
    Text,
    /// A list of text.
    Texts,
    Truth,
    /// A list of bools.
    Truths,
    Sparse,
    Frame,
    /// Not known here: any parameter takes it.
    Unknown,
}

impl Arg {
    /// What the argument is, as a refusal names it.
    pub fn words(self) -> &'static str {
        match self {
            Arg::Int => "an int",
            Arg::Float => "a number",
            Arg::Json => "a JSON value",
            Arg::Tensor => "numbers",
            Arg::Tensors => "a list of tensors",
            Arg::Text => "text",
            Arg::Texts => "a list of text",
            Arg::Truth => "a bool",
            Arg::Truths => "a list of bools",
            Arg::Sparse => "a sparse matrix",
            Arg::Frame => "a frame",
            Arg::Unknown => "a value",
        }
    }

    pub fn of_type(t: &CrowType) -> Arg {
        match t {
            CrowType::Result(inner) => Arg::of_type(inner),
            CrowType::Int => Arg::Int,
            CrowType::Float => Arg::Float,
            CrowType::Json => Arg::Json,
            CrowType::Array(inner) => match Arg::of_type(inner) {
                Arg::Int | Arg::Float => Arg::Tensor,
                Arg::Tensor => Arg::Tensors,
                Arg::Text => Arg::Texts,
                Arg::Truth => Arg::Truths,
                _ => Arg::Unknown,
            },
            CrowType::NumArray(_) | CrowType::FrameCol => Arg::Tensor,
            CrowType::Str | CrowType::Markup => Arg::Text,
            CrowType::Bool => Arg::Truth,
            CrowType::Sparse => Arg::Sparse,
            CrowType::Frame => Arg::Frame,
            _ => Arg::Unknown,
        }
    }

    pub fn of_value(v: &CrowValue) -> Arg {
        match &v.value {
            Value::Int(_) => Arg::Int,
            Value::Float(_) => Arg::Float,
            Value::NumArray(_) => Arg::Tensor,
            Value::Array(items) => match items.first().map(Arg::of_value) {
                Some(Arg::Tensor) => Arg::Tensors,
                Some(Arg::Text) => Arg::Texts,
                Some(Arg::Int | Arg::Float) => Arg::Tensor,
                Some(Arg::Truth) => Arg::Truths,
                _ => Arg::Unknown,
            },
            Value::Str(_) => Arg::Text,
            Value::Bool(_) => Arg::Truth,
            Value::Sparse(_) => Arg::Sparse,
            Value::Frame(_) => Arg::Frame,
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
            Kind::Bool => "a bool",
            Kind::Str => "text",
            Kind::Tensor => "numbers",
            Kind::Tensors => "a list of tensors",
            Kind::Strs => "a list of text",
            Kind::Bools => "a list of bools",
            Kind::Sparse => "a sparse matrix",
            Kind::Frame => "a frame",
            Kind::Cell => "a number, text or a bool",
            Kind::Cells => "a column's values",
            Kind::Words => "a list",
            Kind::Sizes => "sizes",
            Kind::Groups => "tensors",
            Kind::Lists => "lists of text",
            Kind::IntMap | Kind::FloatMap => "a map",
            Kind::Unit => "nothing",
        }
    }

    /// Whether a parameter of this kind takes an argument of that kind.
    fn takes(self, a: Arg) -> bool {
        a == Arg::Unknown
            || match self.each() {
                Kind::Int | Kind::Float => matches!(a, Arg::Int | Arg::Float),
                Kind::Whole => a == Arg::Int,
                Kind::Tensor => a == Arg::Tensor,
                Kind::Tensors => a == Arg::Tensors,
                Kind::Str => a == Arg::Text,
                Kind::Strs => a == Arg::Texts,
                Kind::Bool => a == Arg::Truth,
                Kind::Bools => a == Arg::Truths,
                Kind::Sparse => a == Arg::Sparse,
                Kind::Frame => a == Arg::Frame,
                Kind::Cell => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Json),
                Kind::Cells => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Tensor | Arg::Texts | Arg::Truths),
                Kind::Words => matches!(a, Arg::Tensor | Arg::Texts | Arg::Truths),
                _ => true,
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
/// statement (`log`), a value's own status (`error`, `captured`), what the
/// interpreter holds (`later`, `running`, `notice`), a function argument
/// (`any`, `all`, `find`, `grad`), a resource (`cpu`, `gpu`, `nodes`), a
/// test block, a chart's tree.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const OWN_FORM: &[&str] = &[
    "log", "type", "map", "error", "assert", "captured", "later", "running", "notice", "any", "all", "find", "grad", "cpu", "gpu", "nodes", "test",
    "chart", "args", "exit", "sleep", "time", "now", "input", "env",
    // taken on dual numbers too, inside a function grad differentiates
    "abs", "sqrt", "floor", "ceil", "round",
];

/// The core builtins still written twice, each to become a row (docs/
/// bug-classes.md §4). The list only shrinks: a builtin added outside the
/// table fails `every_core_builtin_is_a_row_or_says_why_not`.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const TO_MOVE: &[&str] = &[
    "str", "len", "contains", "slice", "push", "pop", "sort", "reverse", "concat", "range", "keys", "values", "has", "set", "del", "read", "write",
    "exists", "mkdir", "ls", "rm", "mv", "cp", "json", "encode", "shell",
];

/// The module functions with a form of their own: the database's effects,
/// which the session layer performs (a connection, a transaction, a write
/// held to the handler's commit).
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const MODULE_OWN_FORM: &[(&str, &str)] = &[("db", "connect"), ("db", "query"), ("db", "exec"), ("db", "insert"), ("db", "migrate"), ("db", "transact")];

/// The module functions still written twice, each to become a row (docs/
/// bug-classes.md §4); like TO_MOVE, it only shrinks.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const MODULE_TO_MOVE: &[(&str, &str)] = &[
    // math
    ("math", "load"),
    // frame
    ("frame", "load"), ("frame", "save"),
    // io
    ("io", "csv"), ("io", "lines"),
    // doc
    ("doc", "markdown"), ("doc", "chapters"), ("doc", "menu"), ("doc", "pager"), ("doc", "plain"), ("doc", "document"),
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
        let printed = print(anova, &["a".to_string(), "b".to_string()], "7", None).expect("prints");
        assert_eq!(printed, "crow_runtime::stats::anova(&[&(a), &(b)]).unwrap_or_else(|e| crow_stop(7, e))");
    }

    #[test]
    fn every_id_is_unique() {
        for (i, r) in ROWS.iter().enumerate() {
            assert!(ROWS.iter().skip(i + 1).all(|o| o.id != r.id), "{} twice", r.id);
        }
    }
}
