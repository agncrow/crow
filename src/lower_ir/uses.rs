//! What a program's uses say of a frame it cannot see (spec, Frames: a
//! frame's type is its columns). A frame loaded from a file has the
//! columns the reads of its cells say: a cell handed to `upper` is text,
//! one met as a number — compared with one, added to one, halved — a
//! number, held as floats whichever the file wrote (`q gt 20` holds for
//! 20.5: the program compares two numbers, and which kind the file's are
//! is the file's business), one handed to what takes nothing but an int —
//! a position, `head`, `repeat` — an int, one a function reads as text —
//! by that function's own uses — text. The shape is applied where the
//! frame is bound and held at run time there (`__expect_frame`), and the
//! frame is typed from then on in both modes. A column read as two kinds
//! is refused before the program runs, naming both reads. Nothing is
//! written in the program, and nothing is guessed: a column no read says
//! anything of stays as it comes.

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
    /// the read that decided each column, named in the stop at the load
    pub reasons: BTreeMap<FrameKey, BTreeMap<String, String>>,
}

/// One read of a column, and the kind it says. A read is strict when
/// what it hands the cell to takes nothing else (a builtin's row, a
/// position, a function's body handing it on); a number met in an
/// operator says a number, int or float as the file has it.
#[derive(Clone, Debug)]
struct Read {
    kind: Ty,
    line: usize,
    how: String,
    strict: bool,
    /// the whole column met (`df.qty gt 0`, a mask), not a cell of it: a
    /// mask or an aggregate works on ints or floats as they are, so such a
    /// read says what the column holds only where another read types a
    /// cell
    whole: bool,
}

/// The reads of every loaded frame's columns, across the program.
struct Uses<'c> {
    ctx: &'c TypeContext,
    /// (frame, column) → the reads of its cells
    reads: BTreeMap<(FrameKey, String), Vec<Read>>,
    /// the loaded frames of each body, by the name they go by there
    loaded: BTreeMap<String, Vec<String>>,
    /// a function's parameter read as a cell: what its body says (memoised)
    param_says: HashMap<(String, usize), Vec<(Ty, bool)>>,
    walking: HashSet<(String, usize)>,
    /// a function's body being walked with a frame as its parameter
    walking_frames: HashSet<(String, usize, FrameKey)>,
    line: usize,
}

/// The verbs whose answer has the columns of the frame they are given,
/// each cell as it was: a read of the answer's column is a read of that
/// frame's. (`with`, `rename`, `join`, `group`, `pivot`, `unpivot` and
/// `window` change the columns, and say nothing of the frame they take.)
const KEEPS_COLUMNS: &[&str] = &["filter", "sort", "head", "tail", "sample", "unique", "fill", "complete", "select"];

/// The frame a value is, when it is one the body holds: the name itself,
/// or a verb's answer over it.
fn frame_of(e: &A, env: &Env) -> Option<FrameKey> {
    match e {
        A::Ident(f) => env.frames.get(f).cloned(),
        A::Call { callee, args } => {
            let verb = match callee.as_ref() {
                A::Ident(n) => n.as_str(),
                A::FieldAccess { object, field } if matches!(object.as_ref(), A::Ident(m) if m == "frame") => field.as_str(),
                _ => return None,
            };
            if !KEEPS_COLUMNS.contains(&verb) {
                return None;
            }
            match args.first() {
                Some(A::Ident(f)) => env.frames.get(f).cloned(),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether a value is a frame the program cannot see: one loaded from a
/// file, or a page's data source (`fetch[url]`, `poll[url, seconds]`),
/// which the session loads.
fn is_load(e: &A) -> bool {
    match e {
        A::Call { callee, args } => match callee.as_ref() {
            A::FieldAccess { object, field } => matches!(object.as_ref(), A::Ident(m) if m == "frame") && field == "load",
            A::Ident(n) => n == "load" || (n == "fetch" && args.len() == 1) || (n == "poll" && args.len() == 2),
            _ => false,
        },
        _ => false,
    }
}

/// The definite kind a value says of what it meets: a literal's, a
/// moment's, a top-level variable's, a local's the body gave a literal
/// (`total = 0`, then `total + q`).
fn kind_of(e: &A, ctx: &TypeContext, env: &Env) -> Option<Ty> {
    match e {
        A::Int(_) => Some(Ty::Int),
        A::Float(_) => Some(Ty::Float),
        A::Str(_) => Some(Ty::Str),
        A::Bool(_) => Some(Ty::Bool),
        A::Call { callee, .. } if matches!(callee.as_ref(), A::Ident(n) if n == "moment" || n == "now") => Some(Ty::Moment),
        A::Ident(v) => match env.kinds.get(v).or_else(|| ctx.vars.get(v)) {
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
pub(crate) fn row_says(name: &str, module: Option<&str>, at: usize, argc: usize) -> Option<Ty> {
    if matches!(name, "str" | "int" | "float" | "moment" | "type" | "log" | "fmt" | "encode" | "len" | "assert" | "error") {
        return None;
    }
    let mut said: BTreeSet<String> = BTreeSet::new();
    let mut kind: Option<Ty> = None;
    for row in crate::table::ROWS.iter().filter(|r| r.name == name && module.is_none_or(|m| r.module == m)) {
        let Some(kinds) = crate::table::kinds_of(row.params, argc) else { continue };
        let Some(k) = kinds.get(at) else { continue };
        // a row that takes a parsed value there takes it as it is (`key in
        // j`, `has[j, key]`): nothing is read for it
        if matches!(k, crate::table::Kind::Json) {
            return None;
        }
        // (a row over a column, a list or any value says nothing of a cell)
        let t = k.ty();
        if !definite(&t) {
            continue;
        }
        said.insert(format!("{:?}", t));
        kind = Some(t);
    }
    match said.len() {
        1 => kind,
        // rows that take an int or a float there take a number: a float
        n if n > 0 && said.iter().all(|s| s == "Int" || s == "Float") => Some(Ty::Float),
        _ => None,
    }
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
                } else if let Some(key) = frame_of(value, env) {
                    // the frame itself, or a verb's answer with its columns:
                    // its reads are the frame's
                    env.frames.insert(name.clone(), key);
                    env.cells.remove(name);
                } else if let Some(cell) = self.cell_of(value, env) {
                    env.cells.insert(name.clone(), cell);
                    env.frames.remove(name);
                } else {
                    env.cells.remove(name);
                    env.frames.remove(name);
                }
                // a local given a literal says its kind where it meets a
                // cell later (a local given a cell is the cell)
                match kind_of(value, self.ctx, env) {
                    Some(k) if !env.cells.contains_key(name) => {
                        env.kinds.insert(name.clone(), k);
                    }
                    _ => {
                        env.kinds.remove(name);
                    }
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
                    self.note(&cell, Ty::Bool, "decided on", true);
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

    fn note(&mut self, cell: &(FrameKey, String), kind: Ty, how: &str, strict: bool) {
        self.reads.entry(cell.clone()).or_default().push(Read { kind, line: self.line, how: how.to_string(), strict, whole: false });
    }

    /// A whole column met beside a value of a kind.
    fn note_whole(&mut self, column: &(FrameKey, String), kind: Ty, how: &str) {
        self.reads.entry(column.clone()).or_default().push(Read { kind, line: self.line, how: how.to_string(), strict: false, whole: true });
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
                        for (kind, strict) in self.handed_says(module.as_deref(), &name, i, positional.len()) {
                            self.note(&cell, kind, &format!("handed to {}", name), strict);
                        }
                    }
                    // a frame handed to a function: the function's body reads
                    // the caller's frame through its parameter
                    let key = match module.as_deref() {
                        Some(m) => format!("{}_{}", m, name),
                        None => name.clone(),
                    };
                    if self.ctx.functions.contains_key(&key) {
                        for (i, a) in positional.iter().enumerate() {
                            if let Some(frame) = frame_of(a, env) {
                                self.frame_handed(&key, i, frame);
                            }
                        }
                    }
                    // a position into a list, a tensor or text takes nothing
                    // but an int, a key into a map text
                    if module.is_none() && !self.ctx.functions.contains_key(&name) {
                        let takes = match self.ctx.vars.get(&name).map(|t| t.payload().clone()) {
                            Some(Ty::List(_) | Ty::Tensor(..) | Ty::Str) => Some((Ty::Int, "a position in")),
                            Some(Ty::Map(..)) => Some((Ty::Str, "a key of")),
                            _ => None,
                        };
                        let first = positional.first().and_then(|a| self.cell_of(a, env));
                        if let (Some((kind, what)), Some(cell)) = (takes, first) {
                            self.note(&cell, kind, &format!("{} {}", what, name), true);
                        }
                    }
                }
                if let A::FieldAccess { .. } = callee.as_ref() {
                    // (a column indexed is a cell read, not a call — and its
                    // position an int)
                    match self.column_of(callee, env) {
                        Some((_, column)) => {
                            if let Some(cell) = positional.first().and_then(|a| self.cell_of(a, env)) {
                                self.note(&cell, Ty::Int, &format!("a position in column {}", column), true);
                            }
                        }
                        None => self.walk_expr(callee, env),
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
                            self.note(&c, Ty::Bool, "joined with and/or", true);
                        }
                    }
                    _ => {
                        let arithmetic = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow);
                        // a cell met beside a literal says the kind; a whole
                        // column (`df.qty gt 0`, a mask) says it too, but of
                        // numbers only where a cell is read as one
                        let how = format!("met in {:?}", op).to_lowercase();
                        let fits = |k: &Ty| !arithmetic || matches!(k, Ty::Int | Ty::Float | Ty::Moment);
                        if let Some(k) = kind_of(right, self.ctx, env).filter(fits) {
                            if let Some(c) = &lc {
                                self.note(c, k, &how, false);
                            } else if let Some(c) = self.column_of(left, env) {
                                self.note_whole(&c, k, &how);
                            }
                        }
                        if let Some(k) = kind_of(left, self.ctx, env).filter(fits) {
                            if let Some(c) = &rc {
                                self.note(c, k, &how, false);
                            } else if let Some(c) = self.column_of(right, env) {
                                self.note_whole(&c, k, &how);
                            }
                        }
                    }
                }
            }
            A::UnaryOp { op, expr } => {
                self.walk_expr(expr, env);
                if matches!(op, UnaryOp::Not) {
                    if let Some(c) = self.cell_of(expr, env) {
                        self.note(&c, Ty::Bool, "negated", true);
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
    fn handed_says(&mut self, module: Option<&str>, name: &str, at: usize, argc: usize) -> Vec<(Ty, bool)> {
        let key = match module {
            Some(m) => format!("{}_{}", m, name),
            None => name.to_string(),
        };
        let ctx: &'c TypeContext = self.ctx;
        if let Some(sig) = ctx.functions.get(&key) {
            if let Some((_, t)) = sig.params.get(at) {
                if definite(t) {
                    // (an int the signature says is asked of the file only
                    // where the body hands it to what takes nothing else:
                    // a parameter once given 3 widens to a float, a
                    // position does not)
                    let strict = *t == Ty::Int && self.param_says(&key, at).iter().any(|(k, s)| *s && *k == Ty::Int);
                    return vec![(t.clone(), strict)];
                }
            }
            return self.param_says(&key, at);
        }
        row_says(name, module, at, argc).into_iter().map(|t| (t, true)).collect()
    }

    /// A function handed a frame: its body walked with the parameter as
    /// that frame, so its reads are the frame's (once per frame handed;
    /// a body that hands it on again to itself stops there).
    fn frame_handed(&mut self, f: &str, at: usize, frame: FrameKey) {
        let key = (f.to_string(), at, frame.clone());
        if !self.walking_frames.insert(key.clone()) {
            return;
        }
        let ctx: &'c TypeContext = self.ctx;
        if let Some(sig) = ctx.functions.get(f) {
            if let Some((param, _)) = sig.params.get(at).cloned() {
                let top = self.loaded.get(TOP).cloned().unwrap_or_default();
                let mut env = Env::new(top.iter().map(|n| (n.clone(), (TOP.to_string(), n.clone()))).collect());
                env.body = f.to_string();
                env.frames.insert(param, frame);
                let line = self.line;
                self.walk_block(&sig.body, &mut env);
                self.line = line;
            }
        }
        self.walking_frames.remove(&key);
    }

    /// What a function's body says of a parameter read as a cell.
    fn param_says(&mut self, f: &str, at: usize) -> Vec<(Ty, bool)> {
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
        let said: Vec<(Ty, bool)> = self.reads.remove(&(probe, "\u{1}".to_string())).unwrap_or_default().into_iter().map(|r| (r.kind, r.strict)).collect();
        let mut distinct: Vec<(Ty, bool)> = Vec::new();
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
    /// the locals the body gave a literal, and the kind each says
    kinds: HashMap<String, Ty>,
}

impl Env {
    fn new(frames: HashMap<String, FrameKey>) -> Self {
        Env { body: String::new(), frames, cells: HashMap::new(), kinds: HashMap::new() }
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
    let mut uses = Uses { ctx, reads: BTreeMap::new(), loaded: BTreeMap::new(), param_says: HashMap::new(), walking: HashSet::new(), walking_frames: HashSet::new(), line: 0 };
    uses.walk_program(stmts);
    let mut shapes = Shapes::default();
    for ((frame, column), reads) in &uses.reads {
        if frame.0.starts_with('\u{1}') {
            continue;
        }
        // a number is a number — floats, whichever the file wrote — unless a
        // read takes nothing but an int; anything else read two ways is
        // refused
        let mut kinds: Vec<&Read> = Vec::new();
        for r in reads {
            if !kinds.iter().any(|k| k.kind == r.kind) {
                kinds.push(r);
            }
        }
        let numbers = kinds.iter().all(|k| matches!(k.kind, Ty::Int | Ty::Float));
        // a column met only whole as numbers — a mask, an aggregate — holds
        // ints or floats as they are: nothing of it is held
        if numbers && reads.iter().all(|r| r.whole) {
            continue;
        }
        let asks_int = reads.iter().find(|r| r.strict && r.kind == Ty::Int);
        let (kind, decided) = if numbers {
            match asks_int {
                Some(r) => (Ty::Int, r),
                None => (Ty::Float, reads.iter().find(|r| !r.whole).unwrap_or(kinds[0])),
            }
        } else if kinds.len() == 1 {
            (kinds[0].kind.clone(), kinds[0])
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
        shapes.reasons.entry(frame.clone()).or_default().insert(column.clone(), format!("{} at line {}", decided.how, decided.line));
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
    /// compared with an int a number — floats, whichever the file wrote —
    /// one used as a position, or handed to `head`, an int, one a function
    /// reads as text — by its own body — text; a column only shown says
    /// nothing. Each column names the read that decided it.
    /// A row that takes a parsed value at a position takes it as it is:
    /// `key in j` reads nothing, as `has[j, key]` did.
    #[test]
    fn a_row_that_takes_a_parsed_value_reads_nothing_for_it() {
        assert_eq!(row_says("contains", None, 0, 2), None);
        assert_eq!(row_says("has", None, 0, 2), None);
        assert_eq!(row_says("upper", None, 0, 1), Some(Ty::Str));
    }

    #[test]
    fn the_reads_say_a_loaded_frames_columns() {
        let s = shapes("use frame\nfn label[t]\n    return upper[t]\nfn half[x]\n    return x / 2.0\nname = args[][0]\nnames = ['a', 'b']\ndf = frame.load[name]\nfor t in df.sym\n    log[label[t]]\nfor p in df.px\n    log[half[p]]\nfor q in df.qty\n    if q gt 3\n        log[q]\nfor i in df.idx\n    log[names[i]]\nfor i in df.idx\n    log[i * 1.5]\nfor n in df.n\n    log[head[df, n]]\nlog[df.note]\n");
        let key = (TOP.to_string(), "df".to_string());
        let df = s.frames.get(&key).expect("df has a shape");
        assert_eq!(df.get("sym"), Some(&Ty::Str), "{:?}", df);
        assert_eq!(df.get("px"), Some(&Ty::Float), "{:?}", df);
        assert_eq!(df.get("qty"), Some(&Ty::Float), "{:?}", df);
        assert_eq!(df.get("idx"), Some(&Ty::Int), "{:?}", df);
        assert_eq!(df.get("n"), Some(&Ty::Int), "{:?}", df);
        assert_eq!(df.get("note"), None, "{:?}", df);
        assert!(s.conflicts.is_empty(), "{:?}", s.conflicts);
        let why = s.reasons.get(&key).expect("reasons");
        assert_eq!(why.get("qty").map(String::as_str), Some("met in gt at line 14"), "{:?}", why);
        assert_eq!(why.get("idx").map(String::as_str), Some("a position in names at line 17"), "{:?}", why);
        assert_eq!(why.get("n").map(String::as_str), Some("handed to head at line 21"), "{:?}", why);
        assert_eq!(why.get("sym").map(String::as_str), Some("handed to label at line 10"), "{:?}", why);
    }

    /// A function's parameter the signature says is an int asks the file
    /// for ints only where the body hands it to what takes nothing else:
    /// `first[n]` indexing by it does, `twice[n]` once given 3 does not — a
    /// cell of 2.5 handed to it widens the parameter, as any float would.
    #[test]
    fn an_int_parameter_asks_the_file_only_where_its_body_does() {
        let s = shapes("use frame\nnames = ['a', 'b']\nfn first[n]\n    return names[n]\nfn twice[n]\n    return n * 2\nlog[twice[3]]\ndf = frame.load[args[][0]]\nfor i in df.idx\n    log[first[i]]\nfor q in df.qty\n    log[twice[q]]\n");
        let df = s.frames.get(&(TOP.to_string(), "df".to_string())).expect("df");
        assert_eq!(df.get("idx"), Some(&Ty::Int), "{:?}", df);
        assert_eq!(df.get("qty"), Some(&Ty::Float), "{:?}", df);
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
        assert!(ir.contains("core.__expect_frame(unwrap frame.load!now(\"x.csv\"), [\"qty\", \"sym\"], [\"float\", \"str\"], [\"met in gt at line 9\", \"handed to label at line 6\"])"), "{}", ir);
        assert!(ir.contains("let df: Result(Frame{qty: Float, sym: Str, …})"), "{}", ir);
        assert!(ir.contains("pure label(t: Str) -> Str"), "{}", ir);
    }

    /// A page's data source is a frame the session loads: its reads, in
    /// the views and handlers that read it, say its columns as a loaded
    /// file's do — a local given a literal inside the view counting too.
    #[test]
    fn a_pages_data_source_is_read_as_a_loaded_frame() {
        let s = shapes("use web, ui\n\nfn rows[req]\n    return json['[{\"sym\": \"AAPL\", \"qty\": 100}]']\n\ndata = fetch['/api/rows']\n\nfn shout[t]\n    return upper[t]\n\nview render[]\n    names = for t in data.sym\n        shout[t]\n    total = 0\n    for q in data.qty\n        total = total + q\n    return stack[p['`len[data]` rows'], p[join[names, ', ']], p['total `total`']]\n\nserve[3461]\nroute['/api/rows', rows]\npage['/', render]\n");
        let key = (TOP.to_string(), "data".to_string());
        let df = s.frames.get(&key).unwrap_or_else(|| panic!("data has a shape: {:?}", s));
        assert_eq!(df.get("sym"), Some(&Ty::Str), "{:?}", df);
        assert_eq!(df.get("qty"), Some(&Ty::Float), "{:?}", df);
        assert!(s.conflicts.is_empty(), "{:?}", s.conflicts);
    }

    /// The shape reaches a page program: the source's fact is typed by
    /// the reads and carries what it is held to (`Fact::held`) for the
    /// session, where its answer arrives; the call is wrapped in nothing.
    #[test]
    fn a_data_source_fact_carries_what_it_is_held_to() {
        let src = "use web, ui\n\nfn rows[req]\n    return json['[{\"sym\": \"AAPL\", \"qty\": 100}]']\n\ndata = fetch['/api/rows']\n\nfn shout[t]\n    return upper[t]\n\nview render[]\n    names = for t in data.sym\n        shout[t]\n    total = 0\n    for q in data.qty\n        total = total + q\n    return stack[p['`len[data]` rows'], p[join[names, ', ']], p['total `total`']]\n\nserve[3461]\nroute['/api/rows', rows]\npage['/', render]\n";
        let stmts = crate::build(src).expect("parses");
        let stmts = crate::lower::lower_program(&stmts).expect("lowers");
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        // (the shapes are read off the lowered program, as the lowering reads them)
        let found = shapes_from_use(&stmts, &ctx);
        assert!(found.frames.contains_key(&(TOP.to_string(), "data".to_string())), "{:?}\n{:#?}", found, stmts.iter().take(8).collect::<Vec<_>>());
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowers to the IR");
        let fact = program.parts.iter().flat_map(|p| p.facts.iter()).find(|f| f.name == "data").expect("the data fact");
        assert_eq!(fact.held, vec![("qty".to_string(), "float".to_string(), "met in add at line 16".to_string()), ("sym".to_string(), "str".to_string(), "handed to shout at line 13".to_string())], "{:?}", fact.held);
        assert!(matches!(&fact.ty, Ty::Frame(Some(cols)) if cols.cols.len() == 2), "{:?}", fact.ty);
        let ir = crate::ir::print(&program);
        assert!(!ir.contains("__expect_frame"), "{}", ir);
    }

    /// A frame handed to a function is read through the function's own
    /// body; a frame a verb derives with its columns, or another name
    /// holds, is the frame it came from; a whole column compared with a
    /// literal says what its cells are.
    #[test]
    fn a_frame_handed_on_or_derived_is_read_as_itself() {
        let s = shapes("use frame\nfn total[rows]\n    t = 0\n    for q in rows.qty\n        t = t + q\n    return t\nfn names[rows]\n    kept = filter[rows, rows.sym eq 'AAPL']\n    labels = for t in kept.sym\n        upper[t]\n    return labels\ndf = frame.load[args[][0]]\nsame = df\nlog[total[df]]\nlog[names[same]]\nfor f in df.flag\n    if f\n        log[f]\n");
        let key = (TOP.to_string(), "df".to_string());
        let df = s.frames.get(&key).expect("df");
        assert_eq!(df.get("qty"), Some(&Ty::Float), "{:?}", df);
        assert_eq!(df.get("sym"), Some(&Ty::Str), "{:?}", df);
        assert_eq!(df.get("flag"), Some(&Ty::Bool), "{:?}", df);
        assert!(s.conflicts.is_empty(), "{:?}", s.conflicts);
        let why = s.reasons.get(&key).expect("reasons");
        assert_eq!(why.get("qty").map(String::as_str), Some("met in add at line 5"), "{:?}", why);
        assert!(why.get("sym").is_some_and(|r| r.starts_with("met in eq at line 8") || r.starts_with("handed to upper at line 10")), "{:?}", why);
    }

    /// A column met only whole as numbers — a mask, an aggregate — holds
    /// ints or floats as they are, and nothing of it is held (the book's
    /// `filter[trades, trades.qty gte 40]` had widened `qty` to floats,
    /// and the printed frame said so); a cell read as a number holds it to
    /// numbers; a column met whole as text is text.
    #[test]
    fn a_column_met_only_whole_as_numbers_is_left_as_it_comes() {
        let s = shapes("use frame\ndf = frame.load[args[][0]]\nbig = filter[df, df.qty gte 40]\nlog[len[big]]\nkept = filter[df, df.sym eq 'AAPL']\nlog[len[kept]]\nfor p in df.px\n    log[p * 2.0]\nhigh = filter[df, df.px gt 100]\nlog[len[high]]\n");
        let df = s.frames.get(&(TOP.to_string(), "df".to_string())).expect("df");
        assert_eq!(df.get("qty"), None, "{:?}", df);
        assert_eq!(df.get("sym"), Some(&Ty::Str), "{:?}", df);
        assert_eq!(df.get("px"), Some(&Ty::Float), "{:?}", df);
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
