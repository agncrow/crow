//! The IR: one typed form of a program, decided once (docs/ir-design.md).
//!
//! The parser's tree says what was written; the IR says what it means.
//! Every node carries its type, every conversion is a node, every call
//! names the runtime function it means, every effect is marked answered
//! or held, and a variable's last read is a move. Two backends consume
//! it and decide nothing: the IR interpreter walks it, the Rust backend
//! prints it. `crow build --ir file.co` shows a program in the printer's
//! notation, which is the notation of the design record's worked cases.

use std::fmt::Write as _;

// ── Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    List(Box<Ty>),
    Map(Box<Ty>),
    /// A tensor; rank is known at run time, the dtype is f64 today.
    Tensor,
    /// A frame, with its columns and their types when `describe` or a
    /// literal knew them.
    Frame(Option<Vec<(String, Ty)>>),
    /// A frame column read.
    Column(Box<Ty>),
    /// A parsed JSON value, dynamic until read through int[], float[], str[].
    Json,
    /// A struct, or a runtime record (shell, find).
    Record(String),
    /// A value that can fail: status lives only here (ir-design.md §3.1).
    Result(Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    /// A connection or another handle the runtime owns: "db", "web", "tx", "sparse".
    Handle(String),
    /// A view tree.
    Node,
    /// The checker did not know; the backends treat it as a plain value.
    Unknown,
}

impl Ty {
    pub fn is_result(&self) -> bool {
        matches!(self, Ty::Result(_))
    }
    /// The payload type of a result, or the type itself.
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
            Ty::Float => write!(f, "Float"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Str => write!(f, "Str"),
            Ty::Unit => write!(f, "Unit"),
            Ty::List(t) => write!(f, "List({})", t),
            Ty::Map(t) => write!(f, "Map({})", t),
            Ty::Tensor => write!(f, "Tensor"),
            Ty::Frame(None) => write!(f, "Frame"),
            Ty::Frame(Some(cols)) => {
                write!(f, "Frame{{")?;
                for (i, (n, t)) in cols.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, t)?;
                }
                write!(f, "}}")
            }
            Ty::Column(t) => write!(f, "Column({})", t),
            Ty::Json => write!(f, "Json"),
            Ty::Record(n) => write!(f, "Record({})", n),
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
            Ty::Handle(k) => write!(f, "Handle({})", k),
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
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub name: String,
    pub ty: Ty,
    pub default: Expr,
    /// Written by a handler, so held per session.
    pub session: bool,
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
}

#[derive(Debug, Clone)]
pub struct Test {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub body: Block,
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
    /// `parallel` when the body writes no outer variable and reaches no
    /// effect; decided here, not by the runtime.
    For { var: String, iter: Expr, body: Block, parallel: bool },
    Return(Option<Expr>),
    Break,
    Continue,
    /// An expression for its effect; an effect here is held to commit.
    Do(Expr),
    /// The spec's auto-propagation, made visible: if any named value is an
    /// error, a function returns it, a handler stops, the top level halts.
    Propagate { values: Vec<String> },
    /// A handler meets an error at a fact or an effect: commits nothing and
    /// reports (core-design.md §2; emitted once the session layer lands).
    Stop { value: String },
    /// A source line, for traces and the checker; not executed.
    Line(usize),
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
    /// A runtime function by its table name: `core.push`, `io.fs_read`.
    Runtime(String),
    /// A user function, by its (specialised) name.
    User(String),
    /// A module function: `math.sqrt`, `frame.load`, `db.connect`.
    Module(String, String),
}

#[allow(dead_code)] // the conversions the backends emit
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Convert {
    IntToFloat,
    /// The payload of a result, after a Propagate covered the error.
    Unwrap,
    /// Value to text, as `str[]` does: the text `log` prints.
    ToStr,
    /// A list of numbers to a tensor, or back.
    ToTensor,
    ToList,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Meta {
    Ok,
    Reason,
    Kind,
    Trace,
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
    Effect { name: String, args: Vec<Expr>, ty: Ty, answered: bool, propagates: bool },
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
    ForExpr { var: String, iter: Box<Expr>, body: Block, ty: Ty },
    /// A call held for someone else to make: a control's handler with its
    /// arguments (`click = set_speed[10.0]`), or the harness's
    /// `click[a, set_speed[10.0]]`. The arguments are evaluated where the
    /// call is written; the call happens when the event does.
    Deferred { name: String, args: Vec<Expr> },
}

impl Expr {
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
            },
            Expr::Interp(_) => Ty::Str,
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
    ("fmt", "display.fmt"),
    ("str", "display.str"),
    ("log", "display.log"),
    ("now", "moments.now"),
    ("moment", "moments.moment"),
    ("calendar", "moments.calendar"),
    ("parse_time", "moments.parse_time"),
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
    "rm", "mv", "cp", "fetch", "poll", "shell", "mail", "later", "notice", "now", "time",
    "input", "env", "sleep", "exit", "log", "csv", "lines", "load", "save", "send", "recv",
    "serve", "page", "route", "listen", "args",
];

pub fn is_effect(name: &str) -> bool {
    EFFECTS.contains(&name)
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
            let _ = writeln!(out, "  test {}({})", t.name.clone().unwrap_or_default(), t.params.join(", "));
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
            Stmt::Propagate { values } => {
                let _ = writeln!(out, "{}propagate {}", pad, values.join(", "));
            }
            Stmt::Stop { value } => {
                let _ = writeln!(out, "{}stop {}", pad, value);
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
                Target::Runtime(n) => n.clone(),
                Target::User(n) => format!("call {}", n),
                Target::Module(m, n) => format!("{}.{}", m, n),
            };
            format!("{}({})", t, args.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        Expr::Effect { name, args, answered, .. } => {
            format!(
                "{}!{}({})",
                name,
                if *answered { "now" } else { "held" },
                args.iter().map(expr).collect::<Vec<_>>().join(", ")
            )
        }
        Expr::Convert { how, expr: inner, .. } => match how {
            Convert::IntToFloat => format!("float({})", expr(inner)),
            Convert::Unwrap => format!("unwrap {}", expr(inner)),
            Convert::ToStr => format!("str({})", expr(inner)),
            Convert::ToTensor => format!("tensor({})", expr(inner)),
            Convert::ToList => format!("list({})", expr(inner)),
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
        Expr::ForExpr { var, iter, body, .. } => {
            let mut inner = String::new();
            block(&mut inner, body, 0);
            let lines: Vec<&str> = inner.lines().collect();
            format!("for {} in {} yield {{ {} }}", var, expr(iter), lines.join("; "))
        }
        Expr::Deferred { name, args } => {
            format!("defer {}({})", name, args.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
    }
}
