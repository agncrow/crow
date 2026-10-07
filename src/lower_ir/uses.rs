//! What a program's uses say of a frame it cannot see (spec, Frames: a
//! frame's type is its columns). A frame loaded from a file has the
//! columns the reads of its cells say: a cell handed to `upper` is text,
//! one multiplied by a float a float, one compared with an int an int, one
//! a function reads as text — by that function's own uses — text. The
//! shape is applied where the frame is bound and held at run time there
//! (`__expect_frame`), and the frame is typed from then on in both modes.
//! A column read as two kinds is refused before the program runs, naming
//! both reads. Nothing is written in the program, and nothing is guessed:
//! a column no read says anything of stays as it comes.

use crate::ast::{BinOp, Expr as A, Stmt as S, UnaryOp};
use crate::type_infer::{Ty, TypeContext};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The top level, as the shapes name it.
pub(crate) const TOP: &str = "<main>";

/// A loaded frame: the body that binds it and its name.
pub(crate) type FrameKey = (String, String);

/// What the uses said: each loaded frame's columns, and the reads that
/// disagree.
#[derive(Default, Debug)]
pub(crate) struct Shapes {
    pub frames: BTreeMap<FrameKey, BTreeMap<String, Ty>>,
    /// a column read two ways: the first read's line, and the refusal
    pub conflicts: Vec<(usize, String)>,
}

/// One read of a column, and the kind it says.
#[derive(Clone, Debug)]
struct Read {
    kind: Ty,
    line: usize,
    how: String,
}

/// The reads of every loaded frame's columns, across the program.
struct Uses<'c> {
    ctx: &'c TypeContext,
    /// (frame, column) → the reads of its cells
    reads: BTreeMap<(FrameKey, String), Vec<Read>>,
    /// the loaded frames of each body, by the name they go by there
    loaded: BTreeMap<String, Vec<String>>,
    /// a function's parameter read as a cell: what its body says (memoised)
    param_says: HashMap<(String, usize), Vec<Ty>>,
    walking: HashSet<(String, usize)>,
    line: usize,
}

/// Whether a value is a frame loaded from a file.
fn is_load(e: &A) -> bool {
    match e {
        A::Call { callee, .. } => match callee.as_ref() {
            A::FieldAccess { object, field } => matches!(object.as_ref(), A::Ident(m) if m == "frame") && field == "load",
            A::Ident(n) => n == "load",
            _ => false,
        },
        _ => false,
    }
}

/// The definite kind a value says of what it meets: a literal's, a
/// moment's, a top-level variable's.
fn kind_of(e: &A, ctx: &TypeContext) -> Option<Ty> {
    match e {
        A::Int(_) => Some(Ty::Int),
        A::Float(_) => Some(Ty::Float),
        A::Str(_) => Some(Ty::Str),
        A::Bool(_) => Some(Ty::Bool),
        A::Call { callee, .. } if matches!(callee.as_ref(), A::Ident(n) if n == "moment" || n == "now") => Some(Ty::Moment),
        A::Ident(v) => match ctx.vars.get(v) {
            Some(t @ (Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment)) => Some(t.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A frame the compiler cannot see the columns of: none known, or none
/// known and open (a file loaded, a query no database described).
pub(crate) fn dynamic_frame(t: &Ty) -> bool {
    match t.payload() {
        Ty::Frame(None) => true,
        Ty::Frame(Some(cols)) => cols.open && cols.cols.is_empty(),
        _ => false,
    }
}

fn definite(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment)
}

/// What a builtin's rows say a value handed at a position is: the kind
/// every row of the name agrees on, when all do (`upper` takes text;
/// `abs` takes an int or a float, which says nothing here). A read
/// (`str`, `int`, `float`, `moment`) or a show says nothing.
fn row_says(name: &str, module: Option<&str>, at: usize, argc: usize) -> Option<Ty> {
    if matches!(name, "str" | "int" | "float" | "moment" | "type" | "log" | "fmt" | "encode" | "len" | "assert" | "error") {
        return None;
    }
    let mut said: BTreeSet<String> = BTreeSet::new();
    let mut kind: Option<Ty> = None;
    for row in crate::table::ROWS.iter().filter(|r| r.name == name && module.is_none_or(|m| r.module == m)) {
        let Some(kinds) = crate::table::kinds_of(row.params, argc) else { continue };
        let Some(k) = kinds.get(at) else { continue };
        // (a row over a column, a list or any value says nothing of a cell)
        let t = k.ty();
        if !definite(&t) {
            continue;
        }
        said.insert(format!("{:?}", t));
        kind = Some(t);
    }
    if said.len() == 1 { kind } else { None }
}

impl<'c> Uses<'c> {
    /// The loaded frames and their reads, over every body.
    fn walk_program(&mut self, stmts: &[S]) {
        // the top level's loads first: a page's views and handlers read them
        let mut top: Vec<String> = Vec::new();
        for s in stmts {
            if let S::Assign { name, value } = s {
                if is_load(value) {
                    top.push(name.clone());
                }
            }
        }
        self.loaded.insert(TOP.to_string(), top.clone());
        for s in stmts {
            match s {
                S::FnDecl { name, params, body, .. } => {
                    let mut env = Env::new(top.iter().map(|n| (n.clone(), (TOP.to_string(), n.clone()))).collect());
                    env.body = name.clone();
                    let _ = params;
                    self.walk_block(body, &mut env);
                }
                S::Derive { name, body, .. } => {
                    let mut env = Env::new(top.iter().map(|n| (n.clone(), (TOP.to_string(), n.clone()))).collect());
                    env.body = name.clone();
                    self.walk_block(body, &mut env);
                }
                S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } | S::Resource { .. } => {}
                _ => {}
            }
        }
        let mut env = Env::new(top.iter().map(|n| (n.clone(), (TOP.to_string(), n.clone()))).collect());
        env.body = TOP.to_string();
        let main: Vec<S> = stmts.iter().filter(|s| !matches!(s, S::FnDecl { .. } | S::Derive { .. } | S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } | S::Resource { .. })).cloned().collect();
        self.walk_block(&main, &mut env);
    }

    fn walk_block(&mut self, block: &[S], env: &mut Env) {
        for s in block {
            self.walk_stmt(s, env);
        }
    }

    fn walk_stmt(&mut self, s: &S, env: &mut Env) {
        match s {
            S::Line(n) => self.line = *n,
            S::Assign { name, value } => {
                self.walk_expr(value, env);
                if is_load(value) {
                    env.frames.insert(name.clone(), (env.body.clone(), name.clone()));
                    self.loaded.entry(env.body.clone()).or_default().push(name.clone());
                    env.cells.remove(name);
                } else if let Some(cell) = self.cell_of(value, env) {
                    env.cells.insert(name.clone(), cell);
                    env.frames.remove(name);
                } else {
                    env.cells.remove(name);
                    env.frames.remove(name);
                }
            }
            S::MultiAssign { names, values } => {
                for v in values {
                    self.walk_expr(v, env);
                }
                for n in names {
                    env.cells.remove(n);
                    env.frames.remove(n);
                }
            }
            S::ExprStmt(e) | S::Return(Some(e)) => self.walk_expr(e, env),
            S::If { condition, then_body, else_body } => {
                if let Some(cell) = self.cell_of(condition, env) {
                    self.note(&cell, Ty::Bool, "decided on");
                }
                self.walk_expr(condition, env);
                self.walk_block(then_body, env);
                if let Some(e) = else_body {
                    self.walk_block(e, env);
                }
            }
            S::For { var, iter, body } => {
                self.walk_expr(iter, env);
                let walked = self.column_of(iter, env);
                let saved = env.cells.get(var).cloned();
                match walked {
                    Some(cell) => {
                        env.cells.insert(var.clone(), cell);
                    }
                    None => {
                        env.cells.remove(var);
                    }
                }
                self.walk_block(body, env);
                match saved {
                    Some(c) => {
                        env.cells.insert(var.clone(), c);
                    }
                    None => {
                        env.cells.remove(var);
                    }
                }
            }
            S::While { condition, body } => {
                self.walk_expr(condition, env);
                self.walk_block(body, env);
            }
            S::FieldAssign { object, value, .. } | S::IndexAssign { object, value, .. } => {
                self.walk_expr(object, env);
                self.walk_expr(value, env);
            }
            S::FnDecl { .. } | S::Derive { .. } | S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } | S::Resource { .. } | S::Return(None) | S::Break | S::Continue => {}
        }
    }

    /// The frame and column a value walks (`df.col`), when the frame is loaded.
    fn column_of(&self, e: &A, env: &Env) -> Option<(FrameKey, String)> {
        if let A::FieldAccess { object, field } = e {
            if let A::Ident(df) = object.as_ref() {
                if let Some(key) = env.frames.get(df) {
                    return Some((key.clone(), field.clone()));
                }
            }
        }
        None
    }

    /// The cell a value is: a column indexed (`df.col[i]`), or a local
    /// holding one.
    fn cell_of(&self, e: &A, env: &Env) -> Option<(FrameKey, String)> {
        match e {
            A::Ident(v) => env.cells.get(v).cloned(),
            A::Call { callee, args } if args.len() == 1 && !matches!(args[0], A::Named { .. }) => self.column_of(callee, env),
            _ => None,
        }
    }

    fn note(&mut self, cell: &(FrameKey, String), kind: Ty, how: &str) {
        self.reads.entry(cell.clone()).or_default().push(Read { kind, line: self.line, how: how.to_string() });
    }

    fn walk_expr(&mut self, e: &A, env: &mut Env) {
        match e {
            A::Call { callee, args } => {
                // the arguments' own reads, and what the call says of a cell among them
                for a in args {
                    self.walk_expr(a, env);
                }
                let positional: Vec<&A> = args.iter().filter(|a| !matches!(a, A::Named { .. })).collect();
                let (module, name): (Option<String>, Option<String>) = match callee.as_ref() {
                    A::Ident(n) => (None, Some(n.clone())),
                    A::FieldAccess { object, field } => match object.as_ref() {
                        A::Ident(m) if env.frames.get(m).is_none() && env.cells.get(m).is_none() => (Some(m.clone()), Some(field.clone())),
                        _ => (None, None),
                    },
                    _ => (None, None),
                };
                if let Some(name) = name {
                    for (i, a) in positional.iter().enumerate() {
                        let Some(cell) = self.cell_of(a, env) else { continue };
                        for kind in self.handed_says(module.as_deref(), &name, i, positional.len()) {
                            self.note(&cell, kind, &format!("handed to {}", name));
                        }
                    }
                }
                if let A::FieldAccess { .. } = callee.as_ref() {
                    // (a column indexed is a cell read, not a call)
                    if self.column_of(callee, env).is_none() {
                        self.walk_expr(callee, env);
                    }
                }
            }
            A::BinOp { left, op, right } => {
                self.walk_expr(left, env);
                self.walk_expr(right, env);
                let (lc, rc) = (self.cell_of(left, env), self.cell_of(right, env));
                match op {
                    BinOp::And | BinOp::Or => {
                        for c in [lc, rc].into_iter().flatten() {
                            self.note(&c, Ty::Bool, "joined with and/or");
                        }
                    }
                    _ => {
                        let arithmetic = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow);
                        if let Some(c) = &lc {
                            if let Some(k) = kind_of(right, self.ctx).filter(|k| !arithmetic || matches!(k, Ty::Int | Ty::Float | Ty::Moment)) {
                                self.note(c, k, &format!("met in {:?}", op).to_lowercase());
                            }
                        }
                        if let Some(c) = &rc {
                            if let Some(k) = kind_of(left, self.ctx).filter(|k| !arithmetic || matches!(k, Ty::Int | Ty::Float | Ty::Moment)) {
                                self.note(c, k, &format!("met in {:?}", op).to_lowercase());
                            }
                        }
                    }
                }
            }
            A::UnaryOp { op, expr } => {
                self.walk_expr(expr, env);
                if matches!(op, UnaryOp::Not) {
                    if let Some(c) = self.cell_of(expr, env) {
                        self.note(&c, Ty::Bool, "negated");
                    }
                }
            }
            A::FieldAccess { object, .. } => self.walk_expr(object, env),
            A::Array(items) => {
                for i in items {
                    self.walk_expr(i, env);
                }
            }
            A::Map(pairs) => {
                for (_, v) in pairs {
                    self.walk_expr(v, env);
                }
            }
            A::Named { value, .. } => self.walk_expr(value, env),
            A::ForExpr { var, iter, body } => {
                self.walk_expr(iter, env);
                let walked = self.column_of(iter, env);
                let saved = env.cells.get(var).cloned();
                match walked {
                    Some(cell) => {
                        env.cells.insert(var.clone(), cell);
                    }
                    None => {
                        env.cells.remove(var);
                    }
                }
                self.walk_block(body, env);
                match saved {
                    Some(c) => {
                        env.cells.insert(var.clone(), c);
                    }
                    None => {
                        env.cells.remove(var);
                    }
                }
            }
            A::Int(_) | A::Float(_) | A::Str(_) | A::Bool(_) | A::Ident(_) => {}
        }
    }

    /// What handing a cell to a call at a position says of it: a
    /// function's parameter type where known, else what the function's
    /// own body reads the parameter as; a builtin's row where every row
    /// agrees.
    fn handed_says(&mut self, module: Option<&str>, name: &str, at: usize, argc: usize) -> Vec<Ty> {
        let key = match module {
            Some(m) => format!("{}_{}", m, name),
            None => name.to_string(),
        };
        if let Some(sig) = self.ctx.functions.get(&key) {
            if let Some((_, t)) = sig.params.get(at) {
                if definite(t) {
                    return vec![t.clone()];
                }
            }
            return self.param_says(&key, at);
        }
        row_says(name, module, at, argc).into_iter().collect()
    }

    /// What a function's body says of a parameter read as a cell.
    fn param_says(&mut self, f: &str, at: usize) -> Vec<Ty> {
        let key = (f.to_string(), at);
        if let Some(said) = self.param_says.get(&key) {
            return said.clone();
        }
        if !self.walking.insert(key.clone()) {
            return Vec::new();
        }
        let Some(sig) = self.ctx.functions.get(f) else { return Vec::new() };
        let Some((param, _)) = sig.params.get(at).cloned() else { return Vec::new() };
        let body = sig.body.clone();
        // the parameter walked as a cell of a frame of its own
        let probe: FrameKey = (format!("\u{1}{}", f), param.clone());
        let mut env = Env::new(HashMap::new());
        env.body = f.to_string();
        env.cells.insert(param.clone(), (probe.clone(), "\u{1}".to_string()));
        let line = self.line;
        self.walk_block(&body, &mut env);
        self.line = line;
        let said: Vec<Ty> = self.reads.remove(&(probe, "\u{1}".to_string())).unwrap_or_default().into_iter().map(|r| r.kind).collect();
        let mut distinct: Vec<Ty> = Vec::new();
        for t in said {
            if !distinct.contains(&t) {
                distinct.push(t);
            }
        }
        self.walking.remove(&key);
        self.param_says.insert(key, distinct.clone());
        distinct
    }
}

/// The names in a body's scope: which hold a loaded frame, which a cell
/// of one.
struct Env {
    body: String,
    frames: HashMap<String, FrameKey>,
    cells: HashMap<String, (FrameKey, String)>,
}

impl Env {
    fn new(frames: HashMap<String, FrameKey>) -> Self {
        Env { body: String::new(), frames, cells: HashMap::new() }
    }
}

/// The name of a kind, in the program's words.
fn word(t: &Ty) -> &'static str {
    match t {
        Ty::Int => "an int",
        Ty::Float => "a float",
        Ty::Str => "text",
        Ty::Bool => "a bool",
        Ty::Moment => "a moment",
        _ => "a value",
    }
}

/// What the program's reads say of each loaded frame's columns.
pub(crate) fn shapes_from_use(stmts: &[S], ctx: &TypeContext) -> Shapes {
    let mut uses = Uses { ctx, reads: BTreeMap::new(), loaded: BTreeMap::new(), param_says: HashMap::new(), walking: HashSet::new(), line: 0 };
    uses.walk_program(stmts);
    let mut shapes = Shapes::default();
    for ((frame, column), reads) in &uses.reads {
        if frame.0.starts_with('\u{1}') {
            continue;
        }
        // ints read beside floats are floats; anything else read two ways is refused
        let mut kinds: Vec<&Read> = Vec::new();
        for r in reads {
            if !kinds.iter().any(|k| k.kind == r.kind) {
                kinds.push(r);
            }
        }
        let numbers = kinds.iter().all(|k| matches!(k.kind, Ty::Int | Ty::Float));
        let kind = if kinds.len() == 1 {
            kinds[0].kind.clone()
        } else if numbers {
            Ty::Float
        } else {
            let a = kinds[0];
            let b = kinds[1];
            shapes.conflicts.push((
                a.line,
                format!(
                    "`{}.{}` is read as {} ({}) and as {} (line {}, {}): a column holds one kind — read it as one first where it is the other",
                    frame.1, column, word(&a.kind), a.how, word(&b.kind), b.line, b.how
                ),
            ));
            continue;
        };
        shapes.frames.entry(frame.clone()).or_default().insert(column.clone(), kind);
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(src: &str) -> Shapes {
        let stmts = crate::build(src).expect("parses");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        shapes_from_use(&stmts, &ctx)
    }

    /// A loaded frame's columns are what the program reads them as: a cell
    /// handed to `upper` is text, one multiplied by a float a float, one
    /// compared with an int a number, one a function reads as text — by its
    /// own body — text; a column only shown says nothing.
    #[test]
    fn the_reads_say_a_loaded_frames_columns() {
        let s = shapes("use frame\nfn label[t]\n    return upper[t]\nfn half[x]\n    return x / 2.0\nname = args[][0]\ndf = frame.load[name]\nfor t in df.sym\n    log[label[t]]\nfor p in df.px\n    log[half[p]]\nfor q in df.qty\n    if q gt 3\n        log[q]\nlog[df.note]\n");
        let df = s.frames.get(&(TOP.to_string(), "df".to_string())).expect("df has a shape");
        assert_eq!(df.get("sym"), Some(&Ty::Str), "{:?}", df);
        assert_eq!(df.get("px"), Some(&Ty::Float), "{:?}", df);
        assert_eq!(df.get("qty"), Some(&Ty::Int), "{:?}", df);
        assert_eq!(df.get("note"), None, "{:?}", df);
        assert!(s.conflicts.is_empty(), "{:?}", s.conflicts);
    }

    /// The shape reaches the program: the loaded frame is bound held to
    /// it (`__expect_frame`), its columns typed, and a function handed a
    /// cell takes the cell's kind — not a parsed value — so the compiled
    /// program holds the function.
    #[test]
    fn a_loaded_frame_is_held_to_what_the_program_reads() {
        let src = "use frame\nfn label[t]\n    return upper[t]\ndf = frame.load['x.csv']\nlabels = for t in df.sym\n    label[t]\nlog[labels]\nfor q in df.qty\n    if q gt 20\n        log[q]\n";
        let stmts = crate::build(src).expect("parses");
        let stmts = crate::lower::lower_program(&stmts).expect("lowers");
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions.get("label").map(|s| s.params.clone()), Some(vec![("t".to_string(), Ty::Str)]));
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowers to the IR");
        let ir = crate::ir::print(&program);
        assert!(ir.contains("core.__expect_frame(unwrap frame.load!now(\"x.csv\"), [\"qty\", \"sym\"], [\"int\", \"str\"])"), "{}", ir);
        assert!(ir.contains("let df: Result(Frame{qty: Int, sym: Str, …})"), "{}", ir);
        assert!(ir.contains("pure label(t: Str) -> Str"), "{}", ir);
    }

    /// A column read as text and as a number is refused, naming both reads;
    /// ints beside floats are floats.
    #[test]
    fn a_column_read_two_ways_is_refused() {
        let s = shapes("use frame\ndf = frame.load[args[][0]]\nfor t in df.a\n    log[upper[t]]\nfor t in df.a\n    log[t * 2.0]\nfor q in df.b\n    log[q + 1]\nfor q in df.b\n    log[q * 1.5]\n");
        assert_eq!(s.conflicts.len(), 1, "{:?}", s.conflicts);
        assert_eq!(s.conflicts[0].0, 4, "{:?}", s.conflicts[0]);
        assert!(s.conflicts[0].1.contains("`df.a` is read as text") && s.conflicts[0].1.contains("and as a float"), "{}", s.conflicts[0].1);
        let df = s.frames.get(&(TOP.to_string(), "df".to_string())).expect("df");
        assert_eq!(df.get("b"), Some(&Ty::Float));
    }
}
