//! The IR: one typed form of a program, decided once (docs/ir-design.md).
//!
//! The parser's tree says what was written; the IR says what it means.
//! Every node carries its type, every conversion is a node, every call
//! names the runtime function it means, every effect is marked answered
//! or held, and a variable's last read is a move. Two backends consume
//! it and decide nothing: the IR interpreter walks it, the Rust backend
//! prints it. `crow build --ir file.co` shows a program in the printer's
//! notation, which is the notation of the design record's worked cases.

use std::collections::HashMap;
use std::fmt::Write as _;

// ── Types ─────────────────────────────────────────────────────────────

/// A map's keys are one kind: text — the literal's names, JSON's keys —
/// or ints (`map[[1, 2], …]`, `set[m, 2, v]`); open is an empty map before
/// its first key, text in the compiled program if it is never keyed.
/// `2` and `'2'` are different keys, and a map holds one or the other
/// (spec, Maps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Text,
    Int,
    Open,
}

impl Key {
    /// The kind a key of this map is read as: ints or text (an open map's
    /// first key decides; until then, text).
    pub fn ty(self) -> Ty {
        if self == Key::Int { Ty::Int } else { Ty::Str }
    }
    /// The two agree where one is open or both are one kind.
    pub fn unify(self, other: Key) -> Option<Key> {
        match (self, other) {
            (Key::Open, k) | (k, Key::Open) => Some(k),
            (a, b) if a == b => Some(a),
            _ => None,
        }
    }
    /// The Rust type of the compiled program's key.
    pub fn rust(self) -> &'static str {
        if self == Key::Int { "i64" } else { "String" }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int,
    /// An int held as the runtime's growing int (crow_runtime::int::Int):
    /// one the width pass (int_width) could not bound to 64 bits. The
    /// language has one int; this is how a backend holds a wide one.
    Big,
    Float,
    Bool,
    Str,
    /// A moment (crow_runtime::moments::Moment).
    Moment,
    /// An exact base-ten number (crow_runtime::decimal::Dec).
    Decimal,
    /// An exact ratio of two ints (crow_runtime::fraction::Frac).
    Fraction,
    /// Bytes: what a connection reads and writes (`use sys`).
    Bytes,
    Unit,
    /// Text that is markup (a view's, an element's, `doc`'s): a string
    /// everywhere a string is read, placed as markup by a view.
    Markup,
    List(Box<Ty>),
    /// A map: its keys' kind and its values' type.
    Map(Key, Box<Ty>),
    /// A tensor of its element type (Int or Float), with its shape where
    /// the program states it (its dimensions, each a size or unknown; none
    /// where its rank shows only as it runs); the storage is f64 today.
    Tensor(Box<Ty>, Option<crate::type_infer::Dims>),
    /// A frame, with its columns and their types when `describe` or a
    /// literal knew them.
    /// A frame, with its columns when they are known.
    Frame(Option<std::sync::Arc<crate::type_infer::Cols>>),
    /// A frame column read.
    Column(Box<Ty>),
    /// A parsed JSON value, dynamic until read through int[], float[], str[].
    Json,
    /// A struct, or a runtime record (shell, find).
    Record(String),
    /// A struct field that holds either a struct of its own kind or the
    /// empty sentinel 0: a recursive link.
    OptRecord(String),
    /// A value that can fail: status lives only here (ir-design.md §3.1).
    Result(Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    /// A database connection (`db.connect`), and a transaction within one.
    Db,
    Tx,
    /// A sparse matrix (crow_runtime::sparse).
    Sparse,
    /// A web application, a request a route reads, a response it answers.
    WebApp,
    WebRequest,
    WebResponse,
    /// A mailer, a socket, a byte stream `sys` opened, a test's app session.
    Mailer,
    Socket,
    Stream,
    App,
    /// A view tree.
    Node,
    /// The checker did not know; the backends treat it as a plain value.
    Unknown,
}

impl Ty {
    /// A tensor's item — what a loop walks and an index reads — by its
    /// dimensions: a row when it has two or more, else a number
    /// (type_infer's `tensor_item`, at the IR's types; a tensor's element
    /// is always a number).
    pub fn tensor_item(elem: &Ty, dims: &Option<crate::type_infer::Dims>) -> Ty {
        match dims {
            Some(d) if d.len() >= 2 => Ty::Tensor(Box::new(elem.clone()), Some(d[1..].to_vec())),
            _ => elem.clone(),
        }
    }

    /// A tensor of `elem`s whose shape nothing states.
    pub fn tensor(elem: Ty) -> Ty {
        Ty::Tensor(Box::new(elem), None)
    }

    /// A vector of `elem`s: one dimension, its size unstated.
    pub fn vector(elem: Ty) -> Ty {
        Ty::Tensor(Box::new(elem), Some(vec![None]))
    }

    /// A mask of any depth: a list of bools, or a list of masks (a matrix
    /// compared with a number answers its rows of bools).
    pub fn is_truths(&self) -> bool {
        match self {
            Ty::List(item) => matches!(item.as_ref(), Ty::Bool) || item.is_truths(),
            _ => false,
        }
    }

    /// How many lists deep a mask is (a list of bools: 1).
    pub fn truths_depth(&self) -> usize {
        match self {
            Ty::List(item) if item.is_truths() => 1 + item.truths_depth(),
            Ty::List(_) => 1,
            _ => 0,
        }
    }

    /// The mask a tensor compared with a number answers: of the tensor's
    /// shape — a list of bools for a vector (and for a tensor whose rank the
    /// type does not say: the lowering refuses that comparison), a list of
    /// rows of bools for a matrix, deeper for more axes.
    pub fn mask_of(&self) -> Ty {
        let rank = match self.payload() {
            Ty::Tensor(_, Some(dims)) => dims.len().max(1),
            _ => 1,
        };
        (0..rank).fold(Ty::Bool, |m, _| Ty::List(Box::new(m)))
    }

    /// A tensor's shape, of a tensor or of one that can fail, when its type
    /// knows it.
    pub fn tensor_dims(&self) -> Option<&crate::type_infer::Dims> {
        match self.payload() {
            Ty::Tensor(_, dims) => dims.as_ref(),
            _ => None,
        }
    }

    pub fn is_result(&self) -> bool {
        matches!(self, Ty::Result(_))
    }
    /// The payload type of a result, or the type itself.
    /// A frame's columns, of a frame or of one that can fail, as the
    /// lowering's types carry them.
    pub fn frame_cols_ty(&self) -> Option<Option<std::sync::Arc<crate::type_infer::Cols>>> {
        match self.payload() {
            Ty::Frame(c) => Some(c.clone()),
            _ => None,
        }
    }

    pub fn payload(&self) -> &Ty {
        match self {
            Ty::Result(inner) => inner,
            t => t,
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Int => write!(f, "Int"),
            Ty::Big => write!(f, "Big"),
            Ty::Decimal => write!(f, "Decimal"),
            Ty::Fraction => write!(f, "Fraction"),
            Ty::Float => write!(f, "Float"),
            Ty::Moment => write!(f, "Moment"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Str => write!(f, "Str"),
            Ty::Bytes => write!(f, "Bytes"),
            Ty::Markup => write!(f, "Markup"),
            Ty::Unit => write!(f, "Unit"),
            Ty::List(t) => write!(f, "List({})", t),
            Ty::Map(Key::Int, t) => write!(f, "Map(Int, {})", t),
            Ty::Map(_, t) => write!(f, "Map({})", t),
            Ty::Tensor(t, None) => write!(f, "Tensor({})", t),
            Ty::Tensor(t, Some(dims)) => write!(f, "Tensor({}, {})", t, crate::type_infer::shown_dims(dims)),
            Ty::Frame(None) => write!(f, "Frame"),
            Ty::Frame(Some(cols)) => {
                write!(f, "Frame{{")?;
                for (i, (n, t)) in cols.cols.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, t)?;
                }
                if cols.open {
                    write!(f, "{}…", if cols.cols.is_empty() { "" } else { ", " })?;
                }
                write!(f, "}}")
            }
            Ty::Column(t) => write!(f, "Column({})", t),
            Ty::Json => write!(f, "Json"),
            Ty::Record(n) => write!(f, "Record({})", n),
            Ty::OptRecord(n) => write!(f, "OptRecord({})", n),
            Ty::Result(t) => write!(f, "Result({})", t),
            Ty::Fn(ps, r) => {
                write!(f, "Fn(")?;
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", r)
            }
            Ty::Db => write!(f, "Handle(db)"),
            Ty::Tx => write!(f, "Handle(tx)"),
            Ty::Sparse => write!(f, "Handle(sparse)"),
            Ty::WebApp => write!(f, "Handle(web.app)"),
            Ty::WebRequest => write!(f, "Handle(web.request)"),
            Ty::WebResponse => write!(f, "Handle(web.response)"),
            Ty::Mailer => write!(f, "Handle(mailer)"),
            Ty::Socket => write!(f, "Handle(socket)"),
            Ty::Stream => write!(f, "Handle(stream)"),
            Ty::App => write!(f, "Handle(app)"),
            Ty::Node => write!(f, "Node"),
            Ty::Unknown => write!(f, "?"),
        }
    }
}

// ── Declarations ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Program {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone)]
pub struct Part {
    /// The part's name: the entry file's stem, or the module prefix.
    pub name: String,
    pub uses: Vec<String>,
    pub cpu: Option<i64>,
    pub gpu: Option<i64>,
    pub structs: Vec<StructDef>,
    pub facts: Vec<Fact>,
    pub derives: Vec<Derive>,
    pub fns: Vec<Fn>,
    pub tests: Vec<Test>,
    /// The statements of a script, or a ui program's top level, in order.
    pub main: Block,
    /// Every effect the top level reaches, by the part its statements
    /// belong to (the part's prefix; "" for the entry's own): what a part
    /// does before anything runs (a connection it opens, a page it serves).
    pub main_effects: std::collections::BTreeMap<String, Vec<String>>,
    /// The functions `grad` differentiates over tensors, and those they
    /// reach: run on traced values, copied over them when compiled, left
    /// alone by the fusion pass (lower_ir::grad).
    pub traced: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    /// The name the program declared it by, as a value shows it: a
    /// module's `Day`, which the expansion prefixed `sessions_Day`.
    pub shown: String,
    pub fields: Vec<(String, Ty)>,
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub name: String,
    pub ty: Ty,
    pub default: Expr,
    /// Written by a handler, so held per session.
    pub session: bool,
    /// The line it is declared on.
    pub line: usize,
    /// Its declared value can fail: a failure stops the program where the
    /// facts are set (`cannot assign an error value to 'name' — reason`),
    /// and the fact holds the payload, `ty` — a fact never holds an error.
    pub refuses: bool,
    /// A page's data source (`fetch`, `poll`): the columns the program's
    /// reads say it has — each name, its kind and the read that decided it
    /// — held where its answer arrives, in the session (uses.rs).
    pub held: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadSet {
    pub facts: Vec<String>,
    pub derives: Vec<String>,
    pub tables: Vec<String>,
    pub clock: bool,
}

#[derive(Debug, Clone)]
pub struct Derive {
    pub name: String,
    pub ty: Ty,
    pub body: Block,
    pub reads: ReadSet,
    pub fresh: f64,
    /// What the runtime memoizes it by, as the live layer reads it: the
    /// facts, derivations and tables it reads, the clock, whether a table
    /// is outside the app, and whether one value serves every session.
    pub spec: crate::derive::ReadSet,
    /// Every effect it reaches, its own and through what it calls (a
    /// query, the clock): what it can touch.
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FnKind {
    /// Reads its parameters, returns a value, touches nothing.
    Pure,
    /// Writes a fact for the handler that calls it; runs in its transaction.
    Helper,
    /// Wired to a control or a page: the only writer of facts.
    Handler,
    /// A pure function of state to a tree.
    View,
}

impl FnKind {
    pub fn word(self) -> &'static str {
        match self {
            FnKind::Pure => "pure",
            FnKind::Helper => "helper",
            FnKind::Handler => "handler",
            FnKind::View => "view",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: Block,
    pub kind: FnKind,
    /// What the function can do, transitively: the effect names it reaches.
    pub effects: Vec<String>,
    /// The tables it writes, itself or through what it calls: a handler
    /// wakes their readers when it commits.
    pub tables_written: Vec<String>,
    /// Whether it can call itself, directly or through what it calls: its
    /// calls count toward the recursion limit, in both modes
    /// (crow_runtime::stops::DEPTH).
    pub recursive: bool,
}

/// A body's first source line: where a recursion past the limit stops,
/// the same line in both modes.
pub fn first_line(b: &[Stmt]) -> usize {
    b.iter().find_map(|s| if let Stmt::Line(n) = s { Some(*n) } else { None }).unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Test {
    pub name: Option<String>,
    /// Its inputs, typed by their use in the body: the runner generates
    /// values of these types.
    pub params: Vec<(String, Ty)>,
    pub body: Block,
    /// How many of its bare lines are truths, each lowered as an assertion.
    pub truths: usize,
}

// ── Statements ────────────────────────────────────────────────────────

pub type Block = Vec<Stmt>;

#[allow(dead_code)] // Stop and Line are read by the backends
#[derive(Debug, Clone)]
pub enum Stmt {
    /// First binding of a local.
    Let { var: String, ty: Ty, expr: Expr },
    /// Rebinding a local.
    Set { var: String, expr: Expr },
    /// A field or an index of a local, assigned in place.
    SetPath { var: String, path: Vec<PathStep>, expr: Expr },
    /// The only write to a fact.
    Store { fact: String, expr: Expr },
    If { cond: Expr, then: Block, els: Option<Block> },
    While { cond: Expr, body: Block },
    /// `parallel` when nothing the body does can show the order its
    /// iterations ran in (spec §14, For Loops): decided once by the
    /// lowering, read by both backends.
    For { var: String, iter: Expr, body: Block, parallel: bool },
    Return(Option<Expr>),
    Break,
    Continue,
    /// An expression for its effect; an effect here is held to commit.
    Do(Expr),
    /// The spec's auto-propagation, made visible: if any named value is an
    /// error, a function returns it, a handler stops, the top level halts.
    /// `at` is where the error was refused: a branch, a loop's list, or a
    /// variable whose first value could not fail.
    Propagate { values: Vec<String>, at: Decision },
    /// A source line, for traces and the checker; not executed.
    Line(usize),
}

/// Where an error value was refused (spec, Errors in Conditions): an error
/// has no truth value to branch on and no items to walk, and a variable
/// whose first value could not fail holds no error (spec, Reassignment).
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Branch,
    Loop,
    /// Assigned into the named variable.
    Assign(String),
    /// A field read from it: a frame's column, a record's field, a map's
    /// key, which a value that failed does not have.
    Read(String),
    /// An item read from it: a list's, a text's, a map's by its key, a
    /// column's cell, a parsed value's — none of which a failed value has.
    Item,
    /// An item read or written at it: a failed value is no position and no
    /// key (`xs[int[ch]]`).
    Index,
    /// Dropped by its line: a value that can fail, called or computed as a
    /// statement of its own, which nothing reads (`write[path, text]` alone)
    /// — its failure had gone by unseen in both modes.
    Dropped,
}

impl Decision {
    /// How a halt names it: `cannot branch on an error value`.
    pub fn refusal(&self) -> String {
        match self {
            Decision::Branch => "cannot branch on an error value".to_string(),
            Decision::Loop => "cannot loop over an error value".to_string(),
            Decision::Assign(var) => format!("cannot assign an error value to '{}'", var),
            Decision::Read(field) => format!("cannot read '{}' of an error value", field),
            Decision::Item => "cannot read an item of an error value".to_string(),
            Decision::Index => "cannot index by an error value".to_string(),
            Decision::Dropped => "cannot drop an error value".to_string(),
        }
    }
}

/// The builtins that grow a list or a map in place when the variable they
/// read is the one the statement assigns (`x = push[x, v]`).
pub fn in_place_builtin(path: &str) -> bool {
    let name = path.rsplit('.').next().unwrap_or(path);
    let name = match name {
        "map_set" => "set",
        "map_del" => "del",
        other => other,
    };
    crate::ast::IN_PLACE_BUILTINS.contains(&name)
}

#[derive(Debug, Clone)]
pub enum PathStep {
    Field(String),
    Index(Expr),
}

// ── Expressions ───────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Const {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// A builtin's row (crate::table): the runtime function both backends
    /// call, picked once by the lowering from its arguments' types.
    Row(&'static crate::table::Row),
    /// A builtin by its name or table path (`core.push`): one with a form
    /// of its own (the table's OWN_FORM, the harness's words, a ui
    /// control), or one whose row its values pick as the program runs —
    /// an argument whose kind the lowering does not know.
    Runtime(String),
    /// A user function, by its (specialised) name.
    User(String),
    /// A module function with a form of its own: `db.connect`.
    Module(String, String),
}

#[allow(dead_code)] // the conversions the backends emit
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Convert {
    IntToFloat,
    /// An int held in 64 bits as a growing one (int_width).
    ToBig,
    /// An int as the decimal it is (an int meeting a decimal), or a float
    /// literal as the decimal it writes.
    ToDecimal,
    /// An int or a decimal as the fraction it is (meeting a fraction).
    ToFraction,
    /// `+`, `-`, `*` or a negation of 64-bit ints the width pass proves
    /// cannot pass 64 bits (int_width): computed without the check.
    Proven,
    /// A growing int handed where a 64-bit one is taken: its 64 bits, or
    /// refused past them, naming what took it (int_width).
    ToSmall(Who),
    /// The payload of a result, after a Propagate covered the error.
    Unwrap,
    /// The payload of a result a guard made known good (spec, Errors in
    /// Conditions): no error reaches here, so nothing propagates from it.
    Known,
    /// A list of numbers to a tensor, or back.
    ToTensor,
    ToList,
    /// A list or a map of ints as one of floats: a holder handed a float
    /// holds floats from then on (spec §1).
    IntsToFloats,
}

/// What took a growing int where 64 bits are taken (`Convert::ToSmall`):
/// the refusal names it, in one wording for both modes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Who {
    /// A builtin's argument, by the builtin's name.
    Row(&'static str),
    /// A position in a list, a key of a map.
    Position,
    /// A place the compiled program holds in 64 bits.
    Held,
    /// The answer of a function handed as a value (its signature is the
    /// function type of what takes it).
    Handed,
    /// An argument of a function handed as a value, called through it.
    HandedArg,
}

impl Who {
    /// A builtin's argument, as `Who::Row` says it (the interpreter's rows
    /// refuse by the name they hold, table::no_big_taken).
    pub fn row_text(name: &str) -> String {
        format!("{} takes an int of 64 bits", crate::table::form(name))
    }

    pub fn text(self) -> String {
        match self {
            Who::Row(name) => Who::row_text(name),
            Who::Position => "a position or a key is an int of 64 bits".to_string(),
            Who::Held => "an int is held in 64 bits".to_string(),
            Who::Handed => "a function handed as a value answers an int of 64 bits".to_string(),
            Who::HandedArg => "a function handed as a value takes an int of 64 bits".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Meta {
    Ok,
    Reason,
    Kind,
    Trace,
    /// `.ok` of a frame column: its rows' validity, a missing cell false.
    /// Every other value's `.ok` is its own status, a list's included.
    Valid,
}

#[derive(Debug, Clone)]
pub enum Piece {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Const(Const, Ty),
    /// Read a local.
    Local(String, Ty),
    /// Read a local for the last time: the value moves.
    Move(String, Ty),
    /// Read a fact.
    Load(String, Ty),
    /// Read a derivation.
    Derived(String, Ty),
    /// A module constant: pi, e, inf, nan.
    Constant(String, Ty),
    /// A user function as a value.
    FnRef(String, Ty),
    /// `propagates`: an error among the arguments is the call's value
    /// (spec §10, auto-propagation); false for log, type, error, assert,
    /// and for a view, which renders an error argument as `error: reason`.
    Call { target: Target, args: Vec<Expr>, ty: Ty, propagates: bool },
    /// A call that reaches outside the program's values; `answered` when
    /// its value is used (runs when asked), held to commit otherwise.
    /// `row`: the builtin's row the call inside is, picked by the lowering
    /// as a call's is (none: a form of its own, or a row its values pick).
    Effect { name: String, row: Option<&'static crate::table::Row>, args: Vec<Expr>, ty: Ty, answered: bool, propagates: bool },
    Convert { how: Convert, expr: Box<Expr>, ty: Ty },
    Binary { op: String, l: Box<Expr>, r: Box<Expr>, ty: Ty },
    Unary { op: String, expr: Box<Expr>, ty: Ty },
    Index { base: Box<Expr>, indices: Vec<Expr>, ty: Ty },
    Field { base: Box<Expr>, name: String, ty: Ty },
    /// `.ok`, `.reason`, `.kind`, `.trace`: constants on a non-result.
    Meta { base: Box<Expr>, which: Meta },
    Interp(Vec<Piece>),
    ListLit(Vec<Expr>, Ty),
    MapLit(Vec<(String, Expr)>, Ty),
    RecordLit { name: String, fields: Vec<Expr> },
    /// A view node: element, attributes, children.
    Tree { element: String, attrs: Vec<(String, Expr)>, children: Vec<Expr> },
    /// The collecting loop; the body's last expression is what each
    /// iteration yields, an iteration that yields nothing is skipped.
    /// `parallel` as for the statement form.
    ForExpr { var: String, iter: Box<Expr>, body: Block, ty: Ty, parallel: bool },
    /// A call held for someone else to make: a control's handler with its
    /// arguments (`click = set_speed[10.0]`), or the harness's
    /// `click[a, set_speed[10.0]]`. The arguments are evaluated where the
    /// call is written; the call happens when the event does.
    Deferred { name: String, args: Vec<Expr> },
}

impl Expr {
    /// The type as stored on the node, borrowed — for the interpreter's
    /// hot paths, where `ty()`'s clone of a boxed type was an allocation
    /// per read; `None` for the few variants that compute theirs.
    pub fn ty_ref(&self) -> Option<&Ty> {
        match self {
            Expr::Const(_, t) | Expr::Local(_, t) | Expr::Move(_, t) | Expr::Load(_, t) | Expr::Derived(_, t) | Expr::Constant(_, t) | Expr::FnRef(_, t) => Some(t),
            Expr::Call { ty, .. } | Expr::Effect { ty, .. } | Expr::Convert { ty, .. } | Expr::Binary { ty, .. } | Expr::Unary { ty, .. } | Expr::Index { ty, .. } | Expr::Field { ty, .. } | Expr::ListLit(_, ty) | Expr::MapLit(_, ty) | Expr::ForExpr { ty, .. } => Some(ty),
            _ => None,
        }
    }

    pub fn ty(&self) -> Ty {
        match self {
            Expr::Const(_, t)
            | Expr::Local(_, t)
            | Expr::Move(_, t)
            | Expr::Load(_, t)
            | Expr::Derived(_, t)
            | Expr::Constant(_, t)
            | Expr::FnRef(_, t) => t.clone(),
            Expr::Call { ty, .. }
            | Expr::Effect { ty, .. }
            | Expr::Convert { ty, .. }
            | Expr::Binary { ty, .. }
            | Expr::Unary { ty, .. }
            | Expr::Index { ty, .. }
            | Expr::Field { ty, .. }
            | Expr::ListLit(_, ty)
            | Expr::MapLit(_, ty)
            | Expr::ForExpr { ty, .. } => ty.clone(),
            Expr::Meta { which, .. } => match which {
                Meta::Ok => Ty::Bool,
                Meta::Reason | Meta::Kind => Ty::Str,
                Meta::Trace => Ty::List(Box::new(Ty::Str)),
                Meta::Valid => Ty::List(Box::new(Ty::Bool)),
            },
            // text with a span that can fail is itself fallible: a failed
            // span is the string's value (spec: errors propagate through
            // interpolation)
            Expr::Interp(pieces) => {
                if pieces.iter().any(|p| matches!(p, Piece::Expr(x) if x.ty().is_result())) {
                    Ty::Result(Box::new(Ty::Str))
                } else {
                    Ty::Str
                }
            }
            Expr::RecordLit { name, .. } => Ty::Record(name.clone()),
            Expr::Tree { .. } => Ty::Node,
            Expr::Deferred { .. } => Ty::Unit,
        }
    }
}

// ── The runtime table ─────────────────────────────────────────────────

/// The builtins that live in `crow_runtime` under one name for both
/// backends. A builtin not listed is called by its own name until it
/// moves (core-runtime-builtins.md is the worklist).
pub const RUNTIME_TABLE: &[(&str, &str)] = &[
    ("range", "core.range"),
    ("push", "core.push"),
    ("pop", "core.pop"),
    ("reverse", "core.reverse"),
    ("concat", "core.concat"),
    ("slice", "core.slice"),
    ("sort", "core.sort"),
    ("set", "core.map_set"),
    ("del", "core.map_del"),
    ("rand", "core.rand"),
    ("join", "core.join"),
    ("read", "io.fs_read"),
    ("write", "io.fs_write"),
    ("exists", "io.fs_exists"),
    ("mkdir", "io.fs_mkdir"),
    ("ls", "io.fs_ls"),
    ("rm", "io.fs_rm"),
    ("mv", "io.fs_mv"),
    ("cp", "io.fs_cp"),
    ("encode", "json.encode"),
    ("json", "json.parse"),
    ("matches", "text.matches"),
    ("find", "text.find"),
    ("sub", "text.sub"),
    ("split", "text.split"),
    ("trim", "text.trim"),
    ("replace", "text.replace"),
    ("upper", "text.upper"),
    ("lower", "text.lower"),
    ("starts", "text.starts"),
    ("ends", "text.ends"),
    ("repeat", "text.repeat"),
    ("fmt", "display.fmt"),
    ("str", "display.str"),
    ("log", "display.log"),
    ("now", "moments.now"),
    ("moment", "moments.moment"),
];

pub fn runtime_name(builtin: &str) -> String {
    for (b, r) in RUNTIME_TABLE {
        if *b == builtin {
            return (*r).to_string();
        }
    }
    builtin.to_string()
}

/// The builtins that reach outside the program's values. Their calls
/// lower to `Expr::Effect`; the session layer performs them.
pub const EFFECTS: &[&str] = &[
    "query", "exec", "insert", "transact", "migrate", "connect", "read", "write", "mkdir", "ls",
    "rm", "mv", "cp", "fetch", "poll", "shell", "request", "mail", "later", "notice", "sign_in", "sign_out", "download", "now",
    "input", "env", "sleep", "exit", "log", "csv", "lines", "load", "save", "send", "recv",
    "serve", "page", "route", "args",
];

pub fn is_effect(name: &str) -> bool {
    EFFECTS.contains(&name)
}

/// The functions that answer without waiting on anything or looping: no
/// effect, of their own or through what they call (`Fn::effects`: a
/// query, a read, a sleep, a log), no recursion, and no loop and no call
/// through a function value in them or in what they call. A route's
/// handler among them is answered on the server's own thread, as the
/// request is read; any other runs off the server's threads, so a slow one
/// delays only its own request (spec, Routes). Both modes ask it.
pub fn quick_fns<'a>(fns: impl IntoIterator<Item = &'a Fn>) -> std::collections::BTreeSet<String> {
    let mut own: Vec<(&str, Vec<String>)> = Vec::new();
    for f in fns {
        if !f.effects.is_empty() || f.recursive || f.kind != FnKind::Pure {
            continue;
        }
        let mut ok = true;
        visit_stmts(&f.body, &mut |s| ok &= !matches!(s, Stmt::While { .. } | Stmt::For { .. }));
        let mut callees = Vec::new();
        visit_block(&f.body, &mut |e| match e {
            // (a derivation's read can wait on its computation: a query)
            Expr::ForExpr { .. } | Expr::Effect { .. } | Expr::Deferred { .. } | Expr::Derived(..) => ok = false,
            Expr::Index { base, .. } if matches!(base.ty(), Ty::Fn(..)) => ok = false,
            Expr::Call { target: Target::User(g), .. } | Expr::FnRef(g, _) => callees.push(g.clone()),
            Expr::Call { .. } if call_name(e).is_some_and(|n| is_effect(&n)) => ok = false,
            _ => {}
        });
        if ok {
            own.push((f.name.as_str(), callees));
        }
    }
    // the least set closed under calls: quick when everything it calls is
    let mut quick = std::collections::BTreeSet::new();
    loop {
        let before = quick.len();
        for (name, callees) in &own {
            if !quick.contains(*name) && callees.iter().all(|c| quick.contains(c)) {
                quick.insert(name.to_string());
            }
        }
        if quick.len() == before {
            return quick;
        }
    }
}

/// The derivations a block reads, by name: what a view, or a route's
/// handler, reads once as it starts — the interpreter and the emitter read
/// them in this order.
pub fn derivations_read(block: &[Stmt]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    visit_block(block, &mut |e| {
        if let Expr::Derived(d, _) = e {
            out.insert(d.clone());
        }
    });
    out
}

/// The builtins that draw from the program's random generator, or seed
/// it: a seeded program's draws repeat only in the order written, so a
/// loop that draws runs in order (spec §14, For Loops).
pub const DRAWS: &[&str] = &[
    "rand", "seed", "randn", "normal", "uniform", "poisson", "bernoulli", "gamma_dist", "beta_dist", "exponential", "shuffle", "choice",
    "permutation",
];

// ── walking the IR ──────────────────────────────────────────────────

/// The functions a part can run: what its top level, its facts' defaults,
/// its derivations and its tests call or hold as values, the views and
/// handlers a page runs by itself, and what each of those reaches in turn.
/// A function nothing reaches (a shared part's, in another part's program)
/// never runs there.
pub fn reached_fns(part: &Part) -> std::collections::BTreeSet<String> {
    let mut todo: Vec<String> = Vec::new();
    let note = |e: &Expr, todo: &mut Vec<String>| match e {
        Expr::Call { target: Target::User(n), .. } | Expr::FnRef(n, _) => todo.push(n.clone()),
        Expr::Deferred { name, .. } => todo.push(name.clone()),
        _ => {}
    };
    visit_block(&part.main, &mut |e| note(e, &mut todo));
    for fact in &part.facts {
        visit_expr(&fact.default, &mut |e| note(e, &mut todo));
    }
    for d in &part.derives {
        visit_block(&d.body, &mut |e| note(e, &mut todo));
    }
    for t in &part.tests {
        visit_block(&t.body, &mut |e| note(e, &mut todo));
    }
    for f in &part.fns {
        if !matches!(f.kind, FnKind::Pure | FnKind::Helper) {
            todo.push(f.name.clone());
        }
    }
    let fns: std::collections::HashMap<&str, &Fn> = part.fns.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut reached = std::collections::BTreeSet::new();
    while let Some(n) = todo.pop() {
        if !reached.insert(n.clone()) {
            continue;
        }
        if let Some(f) = fns.get(n.as_str()) {
            visit_block(&f.body, &mut |e| note(e, &mut todo));
        }
    }
    reached
}

pub fn visit_block(b: &[Stmt], f: &mut dyn FnMut(&Expr)) {
    for s in b {
        visit_stmt(s, f);
    }
}

pub fn visit_stmt(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => visit_expr(expr, f),
        Stmt::SetPath { path, expr, .. } => {
            for p in path {
                if let PathStep::Index(i) = p {
                    visit_expr(i, f);
                }
            }
            visit_expr(expr, f);
        }
        Stmt::If { cond, then, els } => {
            visit_expr(cond, f);
            visit_block(then, f);
            if let Some(e) = els {
                visit_block(e, f);
            }
        }
        Stmt::While { cond, body } => {
            visit_expr(cond, f);
            visit_block(body, f);
        }
        Stmt::For { iter, body, .. } => {
            visit_expr(iter, f);
            visit_block(body, f);
        }
        Stmt::Return(Some(e)) => visit_expr(e, f),
        _ => {}
    }
}

pub fn visit_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
            for a in args {
                visit_expr(a, f);
            }
        }
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => visit_expr(expr, f),
        Expr::Binary { l, r, .. } => {
            visit_expr(l, f);
            visit_expr(r, f);
        }
        Expr::Index { base, indices, .. } => {
            visit_expr(base, f);
            for i in indices {
                visit_expr(i, f);
            }
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => visit_expr(base, f),
        Expr::Interp(pieces) => {
            for p in pieces {
                if let Piece::Expr(x) = p {
                    visit_expr(x, f);
                }
            }
        }
        Expr::ListLit(items, _) => {
            for i in items {
                visit_expr(i, f);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (_, v) in pairs {
                visit_expr(v, f);
            }
        }
        Expr::RecordLit { fields, .. } => {
            for x in fields {
                visit_expr(x, f);
            }
        }
        Expr::Tree { attrs, children, .. } => {
            for (_, a) in attrs {
                visit_expr(a, f);
            }
            for c in children {
                visit_expr(c, f);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            visit_expr(iter, f);
            visit_block(body, f);
        }
        _ => {}
    }
}

/// Every expression in a block, each given to `f` to change before its
/// own parts are visited (`visit_expr`'s order).
pub fn visit_block_mut(b: &mut [Stmt], f: &mut dyn FnMut(&mut Expr)) {
    for s in b {
        match s {
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => visit_expr_mut(expr, f),
            Stmt::SetPath { path, expr, .. } => {
                for p in path.iter_mut() {
                    if let PathStep::Index(i) = p {
                        visit_expr_mut(i, f);
                    }
                }
                visit_expr_mut(expr, f);
            }
            Stmt::If { cond, then, els } => {
                visit_expr_mut(cond, f);
                visit_block_mut(then, f);
                if let Some(e) = els {
                    visit_block_mut(e, f);
                }
            }
            Stmt::While { cond, body } => {
                visit_expr_mut(cond, f);
                visit_block_mut(body, f);
            }
            Stmt::For { iter, body, .. } => {
                visit_expr_mut(iter, f);
                visit_block_mut(body, f);
            }
            Stmt::Return(Some(e)) => visit_expr_mut(e, f),
            _ => {}
        }
    }
}

pub fn visit_expr_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    f(e);
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
            for a in args.iter_mut() {
                visit_expr_mut(a, f);
            }
        }
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => visit_expr_mut(expr, f),
        Expr::Binary { l, r, .. } => {
            visit_expr_mut(l, f);
            visit_expr_mut(r, f);
        }
        Expr::Index { base, indices, .. } => {
            visit_expr_mut(base, f);
            for i in indices.iter_mut() {
                visit_expr_mut(i, f);
            }
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => visit_expr_mut(base, f),
        Expr::Interp(pieces) => {
            for p in pieces.iter_mut() {
                if let Piece::Expr(x) = p {
                    visit_expr_mut(x, f);
                }
            }
        }
        Expr::ListLit(items, _) => {
            for i in items.iter_mut() {
                visit_expr_mut(i, f);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (_, v) in pairs.iter_mut() {
                visit_expr_mut(v, f);
            }
        }
        Expr::RecordLit { fields, .. } => {
            for x in fields.iter_mut() {
                visit_expr_mut(x, f);
            }
        }
        Expr::Tree { attrs, children, .. } => {
            for (_, a) in attrs.iter_mut() {
                visit_expr_mut(a, f);
            }
            for c in children.iter_mut() {
                visit_expr_mut(c, f);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            visit_expr_mut(iter, f);
            visit_block_mut(body, f);
        }
        _ => {}
    }
}

/// A body whose lent parameters (`lendable_params`) are read where a move
/// of them stood: a call lends the caller's own value, which goes back.
pub fn reading_lent(b: &[Stmt], lent: &[&str]) -> Block {
    let mut out = b.to_vec();
    visit_block_mut(&mut out, &mut |e| {
        if let Expr::Move(n, t) = e
            && lent.contains(&n.as_str())
        {
            *e = Expr::Local(n.clone(), t.clone());
        }
    });
    out
}

/// Every statement in a block, nested blocks included.
pub fn visit_stmts(b: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for s in b {
        f(s);
        match s {
            Stmt::If { then, els, .. } => {
                visit_stmts(then, f);
                if let Some(e) = els {
                    visit_stmts(e, f);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => visit_stmts(body, f),
            _ => {}
        }
        // a collecting loop's body is statements too
        let mut inner: Vec<Block> = Vec::new();
        visit_stmt(s, &mut |e| {
            if let Expr::ForExpr { body, .. } = e {
                inner.push(body.clone());
            }
        });
        for b in inner {
            visit_stmts(&b, f);
        }
    }
}

/// The arguments of a call or an effect.
pub fn call_args(e: &Expr) -> &[Expr] {
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args,
        _ => &[],
    }
}

/// A call's name as the program wrote it: a runtime table path reads as
/// its builtin (`core.push` → `push`), an effect as its name.
/// The parameters of each function a call may lend the caller's value
/// instead of a copy, in both backends: ones the body never assigns (nor an
/// item of), never binds anew (a `let`, a loop's variable), and moves —
/// the lowering moves a variable at its last read — only where the move
/// borrows: an index's base, a builtin's argument its row reads where it
/// lies, a loop's list, or a call's argument the callee may be lent in turn
/// (decided together, to a fixed point). A lent parameter's move compiles
/// as a read.
/// Whether `l and r` (or `or`) decides on two truths, the right side
/// computed only when the left does not decide (spec, Operators): false for
/// `and`, true for `or`, an error on the left the value. A list of truths
/// on either side is a mask joined item by item, both sides computed. The
/// one rule every backend reads.
pub fn short_circuits(l: &Expr, r: &Expr) -> bool {
    let truth = |e: &Expr| !matches!(e.ty().payload(), Ty::List(_) | Ty::Tensor(..) | Ty::Column(_) | Ty::Frame(_));
    truth(l) && truth(r)
}

pub fn lendable_params<'a>(fns: &[&'a Fn]) -> HashMap<String, Vec<bool>> {
    let mut lend: HashMap<String, Vec<bool>> = fns.iter().map(|f| (f.name.clone(), f.params.iter().map(|(p, _)| never_rebound(&f.body, p)).collect())).collect();
    loop {
        let mut changed = false;
        for f in fns {
            for (i, (p, _)) in f.params.iter().enumerate() {
                if lend[&f.name][i] && !moves_borrow(&f.body, p, &lend) {
                    if let Some(v) = lend.get_mut(&f.name) {
                        v[i] = false;
                    }
                    changed = true;
                }
            }
        }
        if !changed {
            return lend;
        }
    }
}

/// Whether a body never assigns a name, an item of it, or binds it anew.
fn never_rebound(b: &[Stmt], p: &str) -> bool {
    let mut never = true;
    visit_stmts(b, &mut |s| match s {
        Stmt::Set { var, .. } | Stmt::SetPath { var, .. } | Stmt::Let { var, .. } | Stmt::For { var, .. } if var == p => never = false,
        _ => {}
    });
    visit_block(b, &mut |e| {
        if matches!(e, Expr::ForExpr { var, .. } if var == p) {
            never = false;
        }
    });
    never
}

/// Whether every move of `p` in a body is in a position that borrows.
fn moves_borrow(b: &[Stmt], p: &str, lend: &HashMap<String, Vec<bool>>) -> bool {
    let mut w = Borrows { p, lend, ok: true };
    w.block(b);
    w.ok
}

struct Borrows<'a> {
    p: &'a str,
    lend: &'a HashMap<String, Vec<bool>>,
    ok: bool,
}

impl Borrows<'_> {
    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) | Stmt::Return(Some(expr)) => self.expr(expr, false),
            Stmt::SetPath { path, expr, .. } => {
                for step in path {
                    if let PathStep::Index(i) = step {
                        self.expr(i, false);
                    }
                }
                self.expr(expr, false);
            }
            Stmt::If { cond, then, els } => {
                self.expr(cond, false);
                self.block(then);
                if let Some(e) = els {
                    self.block(e);
                }
            }
            Stmt::While { cond, body } => {
                self.expr(cond, false);
                self.block(body);
            }
            Stmt::For { iter, body, .. } => {
                self.expr(iter, true);
                self.block(body);
            }
            _ => {}
        }
    }

    /// An expression in a position that borrows (`borrowing`) or one that
    /// may keep what it is given.
    fn expr(&mut self, e: &Expr, borrowing: bool) {
        match e {
            Expr::Move(n, _) if n == self.p => self.ok &= borrowing,
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => self.expr(expr, borrowing),
            // (an index's key is read too: `m[k]`)
            Expr::Index { base, indices, .. } => {
                self.expr(base, true);
                for i in indices {
                    self.expr(i, true);
                }
            }
            Expr::Call { target: Target::Row(row), args, .. } => {
                for (k, a) in args.iter().enumerate() {
                    let reads = row.params.get(k).is_some_and(|kind| !kind.owned() && !kind.is_rest() && !matches!(kind, crate::table::Kind::Item));
                    self.expr(a, reads);
                }
            }
            // a comparison reads both sides
            Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "eq" | "ne") => {
                self.expr(l, true);
                self.expr(r, true);
            }
            Expr::Call { target: Target::User(c), args, .. } => {
                for (k, a) in args.iter().enumerate() {
                    let lent = self.lend.get(c).and_then(|v| v.get(k)).copied().unwrap_or(false);
                    self.expr(a, lent);
                }
            }
            Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
                for a in args {
                    self.expr(a, false);
                }
            }
            Expr::ForExpr { iter, body, .. } => {
                self.expr(iter, true);
                self.block(body);
            }
            Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => self.expr(expr, false),
            Expr::Binary { l, r, .. } => {
                self.expr(l, false);
                self.expr(r, false);
            }
            Expr::Field { base, .. } | Expr::Meta { base, .. } => self.expr(base, false),
            // text made of pieces reads each
            Expr::Interp(pieces) => {
                for piece in pieces {
                    if let Piece::Expr(x) = piece {
                        self.expr(x, true);
                    }
                }
            }
            Expr::ListLit(items, _) => {
                for x in items {
                    self.expr(x, false);
                }
            }
            Expr::MapLit(pairs, _) => {
                for (_, x) in pairs {
                    self.expr(x, false);
                }
            }
            Expr::RecordLit { fields, .. } => {
                for x in fields {
                    self.expr(x, false);
                }
            }
            Expr::Tree { attrs, children, .. } => {
                for (_, x) in attrs {
                    self.expr(x, false);
                }
                for x in children {
                    self.expr(x, false);
                }
            }
            Expr::Local(..) | Expr::Move(..) | Expr::Const(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => {}
        }
    }
}

/// Whether a body calls `cpu[…]`: its count holds until the function
/// returns, so a call of it puts the caller's back (both modes;
/// crow_runtime::cores).
pub fn sets_cores(b: &[Stmt]) -> bool {
    let mut found = false;
    visit_block(b, &mut |e| {
        if call_name(e).as_deref() == Some("cpu") {
            found = true;
        }
    });
    found
}

pub fn call_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Call { target: Target::Runtime(path), .. } => {
            Some(RUNTIME_TABLE.iter().find(|(_, r)| *r == path.as_str()).map(|(b, _)| b.to_string()).unwrap_or_else(|| path.clone()))
        }
        Expr::Call { target: Target::Row(row), .. } => Some(row.name.to_string()),
        Expr::Call { target: Target::User(n), .. } => Some(n.clone()),
        Expr::Call { target: Target::Module(_, n), .. } => Some(n.clone()),
        Expr::Effect { name, .. } => Some(name.rsplit('.').next().unwrap_or(name).to_string()),
        _ => None,
    }
}

/// Sets every loop's `parallel` flag in a block, nested loops included,
/// to what `decide` says of its variable and body.
pub fn set_loops(b: &mut Block, decide: &dyn std::ops::Fn(&str, &Block) -> bool) {
    for s in b.iter_mut() {
        set_loops_stmt(s, decide);
    }
}

fn set_loops_stmt(s: &mut Stmt, decide: &dyn std::ops::Fn(&str, &Block) -> bool) {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => set_loops_expr(expr, decide),
        Stmt::SetPath { path, expr, .. } => {
            for p in path.iter_mut() {
                if let PathStep::Index(i) = p {
                    set_loops_expr(i, decide);
                }
            }
            set_loops_expr(expr, decide);
        }
        Stmt::If { cond, then, els } => {
            set_loops_expr(cond, decide);
            set_loops(then, decide);
            if let Some(e) = els {
                set_loops(e, decide);
            }
        }
        Stmt::While { cond, body } => {
            set_loops_expr(cond, decide);
            set_loops(body, decide);
        }
        Stmt::For { var, iter, body, parallel } => {
            set_loops_expr(iter, decide);
            *parallel = decide(var, body);
            set_loops(body, decide);
        }
        Stmt::Return(Some(e)) => set_loops_expr(e, decide),
        _ => {}
    }
}

/// `set_loops` inside an expression: a collecting loop anywhere in it.
pub fn set_loops_expr(e: &mut Expr, decide: &dyn std::ops::Fn(&str, &Block) -> bool) {
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
            for a in args.iter_mut() {
                set_loops_expr(a, decide);
            }
        }
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => set_loops_expr(expr, decide),
        Expr::Binary { l, r, .. } => {
            set_loops_expr(l, decide);
            set_loops_expr(r, decide);
        }
        Expr::Index { base, indices, .. } => {
            set_loops_expr(base, decide);
            for i in indices.iter_mut() {
                set_loops_expr(i, decide);
            }
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => set_loops_expr(base, decide),
        Expr::Interp(pieces) => {
            for p in pieces.iter_mut() {
                if let Piece::Expr(x) = p {
                    set_loops_expr(x, decide);
                }
            }
        }
        Expr::ListLit(items, _) => {
            for i in items.iter_mut() {
                set_loops_expr(i, decide);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (_, v) in pairs.iter_mut() {
                set_loops_expr(v, decide);
            }
        }
        Expr::RecordLit { fields, .. } => {
            for x in fields.iter_mut() {
                set_loops_expr(x, decide);
            }
        }
        Expr::Tree { attrs, children, .. } => {
            for (_, a) in attrs.iter_mut() {
                set_loops_expr(a, decide);
            }
            for c in children.iter_mut() {
                set_loops_expr(c, decide);
            }
        }
        Expr::ForExpr { var, iter, body, parallel, .. } => {
            set_loops_expr(iter, decide);
            *parallel = decide(var, body);
            set_loops(body, decide);
        }
        _ => {}
    }
}

// ── The printer ───────────────────────────────────────────────────────

/// The notation of the design record's worked cases, one statement per
/// line, types after a colon, `←` a store, `!now` / `!held` an effect's
/// timing, `move` a last read.
pub fn print(program: &Program) -> String {
    let mut out = String::new();
    for part in &program.parts {
        let _ = writeln!(out, "part {}", part.name);
        if !part.uses.is_empty() {
            let _ = writeln!(out, "  use {}", part.uses.join(", "));
        }
        if let Some(n) = part.cpu {
            let _ = writeln!(out, "  cpu {}", n);
        }
        if let Some(n) = part.gpu {
            let _ = writeln!(out, "  gpu {}", n);
        }
        for s in &part.structs {
            let fields: Vec<String> = s.fields.iter().map(|(n, t)| format!("{}: {}", n, t)).collect();
            let _ = writeln!(out, "  struct {} {{ {} }}", s.name, fields.join(", "));
        }
        for f in &part.facts {
            let _ = writeln!(
                out,
                "  fact {}: {} = {}{}",
                f.name,
                f.ty,
                expr(&f.default),
                if f.session { "            session" } else { "" }
            );
        }
        for d in &part.derives {
            let mut reads = Vec::new();
            if !d.reads.facts.is_empty() {
                reads.push(format!("facts: {}", d.reads.facts.join(" ")));
            }
            if !d.reads.derives.is_empty() {
                reads.push(format!("derives: {}", d.reads.derives.join(" ")));
            }
            if !d.reads.tables.is_empty() {
                reads.push(format!("tables: {}", d.reads.tables.join(" ")));
            }
            if d.reads.clock {
                reads.push("clock".to_string());
            }
            let fresh = if d.fresh > 0.0 { format!("  fresh {}", d.fresh) } else { String::new() };
            let _ = writeln!(out, "  derive {}: {}  reads {{{}}}{}", d.name, d.ty, reads.join(", "), fresh);
            block(&mut out, &d.body, 2);
        }
        for f in &part.fns {
            let params: Vec<String> = f.params.iter().map(|(n, t)| format!("{}: {}", n, t)).collect();
            let effects = if f.effects.is_empty() { String::new() } else { format!("  effects {{{}}}", f.effects.join(" ")) };
            let _ = writeln!(out, "  {} {}({}) -> {}{}", f.kind.word(), f.name, params.join(", "), f.ret, effects);
            block(&mut out, &f.body, 2);
        }
        for t in &part.tests {
            let params: Vec<String> = t.params.iter().map(|(n, ty)| format!("{}: {}", n, ty)).collect();
            let _ = writeln!(out, "  test {}({})", t.name.clone().unwrap_or_default(), params.join(", "));
            block(&mut out, &t.body, 2);
        }
        if part.main.iter().any(|s| !matches!(s, Stmt::Line(_))) {
            let _ = writeln!(out, "  main");
            block(&mut out, &part.main, 2);
        }
    }
    out
}

fn block(out: &mut String, b: &Block, depth: usize) {
    let pad = "  ".repeat(depth);
    for s in b {
        match s {
            Stmt::Let { var, ty, expr: e } => {
                let _ = writeln!(out, "{}let {}: {} = {}", pad, var, ty, expr(e));
            }
            Stmt::Set { var, expr: e } => {
                let _ = writeln!(out, "{}{} = {}", pad, var, expr(e));
            }
            Stmt::SetPath { var, path, expr: e } => {
                let mut p = var.clone();
                for step in path {
                    match step {
                        PathStep::Field(f) => {
                            p.push('.');
                            p.push_str(f);
                        }
                        PathStep::Index(i) => {
                            let _ = write!(p, "[{}]", expr(i));
                        }
                    }
                }
                let _ = writeln!(out, "{}{} = {}", pad, p, expr(e));
            }
            Stmt::Store { fact, expr: e } => {
                let _ = writeln!(out, "{}store {} ← {}", pad, fact, expr(e));
            }
            Stmt::If { cond, then, els } => {
                let _ = writeln!(out, "{}if {}", pad, expr(cond));
                block(out, then, depth + 1);
                if let Some(e) = els {
                    let _ = writeln!(out, "{}else", pad);
                    block(out, e, depth + 1);
                }
            }
            Stmt::While { cond, body } => {
                let _ = writeln!(out, "{}while {}", pad, expr(cond));
                block(out, body, depth + 1);
            }
            Stmt::For { var, iter, body, parallel } => {
                let _ = writeln!(out, "{}for {} in {}{}", pad, var, expr(iter), if *parallel { "  parallel" } else { "" });
                block(out, body, depth + 1);
            }
            Stmt::Return(None) => {
                let _ = writeln!(out, "{}return", pad);
            }
            Stmt::Return(Some(e)) => {
                let _ = writeln!(out, "{}return {}", pad, expr(e));
            }
            Stmt::Break => {
                let _ = writeln!(out, "{}break", pad);
            }
            Stmt::Continue => {
                let _ = writeln!(out, "{}continue", pad);
            }
            Stmt::Do(e) => {
                let _ = writeln!(out, "{}{}", pad, expr(e));
            }
            Stmt::Propagate { values, at } => {
                let at = match at {
                    Decision::Branch => String::new(),
                    Decision::Loop => " (a loop's list)".to_string(),
                    Decision::Assign(var) => format!(" (into {})", var),
                    Decision::Read(field) => format!(" (reading {})", field),
                    Decision::Item => " (reading an item)".to_string(),
                    Decision::Index => " (as a position or a key)".to_string(),
                    Decision::Dropped => " (dropped by its line)".to_string(),
                };
                let _ = writeln!(out, "{}propagate {}{}", pad, values.join(", "), at);
            }
            Stmt::Line(_) => {}
        }
    }
}

fn const_text(c: &Const) -> String {
    match c {
        Const::Int(n) => n.to_string(),
        Const::Float(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                format!("{:.1}", f)
            } else {
                f.to_string()
            }
        }
        Const::Bool(b) => b.to_string(),
        Const::Str(s) => format!("{:?}", s),
        Const::Unit => "unit".to_string(),
    }
}

pub fn expr(e: &Expr) -> String {
    match e {
        Expr::Const(c, _) => const_text(c),
        Expr::Local(v, _) => v.clone(),
        Expr::Move(v, _) => format!("move {}", v),
        Expr::Load(f, _) => format!("load {}", f),
        Expr::Derived(d, _) => format!("derived {}", d),
        Expr::Constant(c, _) => format!("const {}", c),
        Expr::FnRef(f, _) => format!("fn {}", f),
        Expr::Call { target, args, .. } => {
            let t = match target {
                Target::Row(row) => row.id.to_string(),
                Target::Runtime(n) => n.clone(),
                Target::User(n) => format!("call {}", n),
                Target::Module(m, n) => format!("{}.{}", m, n),
            };
            format!("{}({})", t, args.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::Effect { name, row, args, answered, .. } => {
            format!(
                "{}!{}({})",
                row.map_or(name.as_str(), |r| r.id),
                if *answered { "now" } else { "held" },
                args.iter().map(expr).collect::<Vec<_>>().join(", ")
            )
        }
        Expr::Convert { how, expr: inner, .. } => match how {
            Convert::IntToFloat => format!("float({})", expr(inner)),
            Convert::Unwrap => format!("unwrap {}", expr(inner)),
            Convert::Known => format!("known {}", expr(inner)),
            Convert::ToTensor => format!("tensor({})", expr(inner)),
            Convert::ToList => format!("list({})", expr(inner)),
            Convert::IntsToFloats => format!("floats({})", expr(inner)),
            Convert::ToBig => format!("big({})", expr(inner)),
            Convert::ToSmall(_) => format!("small({})", expr(inner)),
            Convert::ToDecimal => format!("decimal({})", expr(inner)),
            Convert::ToFraction => format!("fraction({})", expr(inner)),
            Convert::Proven => format!("proven({})", expr(inner)),
        },
        Expr::Binary { op, l, r, .. } => format!("({} {} {})", expr(l), op, expr(r)),
        Expr::Unary { op, expr: inner, .. } => format!("({} {})", op, expr(inner)),
        Expr::Index { base, indices, .. } => {
            format!("{}[{}]", expr(base), indices.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::Field { base, name, .. } => format!("{}.{}", expr(base), name),
        Expr::Meta { base, which } => {
            let w = match which {
                Meta::Ok => "ok",
                Meta::Reason => "reason",
                Meta::Kind => "kind",
                Meta::Trace => "trace",
                Meta::Valid => "valid",
            };
            format!("{}.{}", expr(base), w)
        }
        Expr::Interp(pieces) => {
            let parts: Vec<String> = pieces
                .iter()
                .map(|p| match p {
                    Piece::Text(t) => format!("{:?}", t),
                    Piece::Expr(e) => expr(e),
                })
                .collect();
            format!("interp({})", parts.join(", "))
        }
        Expr::ListLit(items, _) => format!("[{}]", items.iter().map(expr).collect::<Vec<_>>().join(", ")),
        Expr::MapLit(pairs, _) => {
            format!("{{{}}}", pairs.iter().map(|(k, v)| format!("{}: {}", k, expr(v))).collect::<Vec<_>>().join(", "))
        }
        Expr::RecordLit { name, fields } => {
            format!("{}{{{}}}", name, fields.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::Tree { element, attrs, children } => {
            let a = if attrs.is_empty() {
                String::new()
            } else {
                format!("({})", attrs.iter().map(|(k, v)| format!("{}: {}", k, expr(v))).collect::<Vec<_>>().join(", "))
            };
            format!("tree {}{}[{}]", element, a, children.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::ForExpr { var, iter, body, parallel, .. } => {
            let mut inner = String::new();
            block(&mut inner, body, 0);
            let lines: Vec<&str> = inner.lines().collect();
            format!("for {} in {} yield {{ {} }}{}", var, expr(iter), lines.join("; "), if *parallel { "  parallel" } else { "" })
        }
        Expr::Deferred { name, args } => {
            format!("defer {}({})", name, args.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
    }
}
