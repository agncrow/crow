//! Lowering: the parser's tree to the IR (docs/ir-design.md §4).
//!
//! Two front-end passes have already run: named arguments are positional
//! (`lower.rs`) and the program's signatures are inferred
//! (`type_infer.rs`). This pass types each node from its children, refuses
//! what is not a program (`checks.rs`), and writes the decisions into the
//! IR, adding the ones the IR makes explicit: which calls are effects and
//! whether their answer is used, where a result is propagated, where an
//! int becomes a float, which read is a variable's last, which loops are
//! parallel, which functions are handlers and what each function can do.

use crate::ast::{BinOp, Expr as A, Stmt as S};
use crate::ir::*;
use crate::type_infer::{CrowType, TypeContext};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod checks;
mod shapes;
pub(crate) use checks::{BUILTINS, CORE_RESERVED, UI_RESERVED, WEB_EXTRA_FUNCTIONS, WEB_RESERVED};

/// Lower a whole (expanded, lowered, inferred) program.
pub fn lower(stmts: &[S], ctx: &mut TypeContext, part_name: &str) -> Result<Program, String> {
    lower_with_known(stmts, ctx, part_name, &HashSet::new())
}

/// Lowering with names already bound by an earlier fragment (the REPL's
/// session): a read of one is a local, an assignment to one is a `set`.
pub fn lower_with_known(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>) -> Result<Program, String> {
    lower_checked(stmts, ctx, part_name, known).map_err(|r| r.message())
}

/// Why a program was not lowered.
pub enum Refusal {
    /// What the program got wrong, each error with the top-level
    /// declaration it was found in: after module expansion a part's
    /// declarations carry its prefix, so a caller can say which file an
    /// error belongs to.
    Invalid(Vec<(String, Option<String>)>),
    /// A form the lowering has no IR for.
    Unlowered(String),
}

impl Refusal {
    /// The refusal as printed: every error, one per line, each variable
    /// named as the program wrote it.
    pub fn message(&self) -> String {
        match self {
            Refusal::Invalid(errors) => crate::lower::shown(&errors.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>().join("\n")),
            Refusal::Unlowered(e) => e.clone(),
        }
    }
}

/// A program lowered, refused with each error and the declaration it was
/// found in when it is not one: the lowering a pipeline that runs or builds
/// the program shares with its check (F5 S10).
pub fn lower_refusing(stmts: &[S], ctx: &mut TypeContext, part_name: &str) -> Result<Program, Refusal> {
    lower_checked(stmts, ctx, part_name, &HashSet::new())
}

/// Whether a program is one: lowered in a copy of its context and thrown
/// away, for the callers that only ask (`crow check`, the harness, the
/// language server).
pub fn check(stmts: &[S], ctx: &TypeContext) -> Result<(), Refusal> {
    let mut ctx = ctx.clone();
    lower_checked(stmts, &mut ctx, "check", &HashSet::new()).map(|_| ())
}

fn lower_checked(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>) -> Result<Program, Refusal> {
    let in_ui = stmts.iter().any(|s| matches!(s, S::Use { path, .. } if path == "ui" || path == "web.ui"));
    let mut l = Lowerer::new(ctx, in_ui);
    l.scopes[0].extend(known.iter().cloned());
    l.prescan(stmts, known);
    let part = l.lower_part(stmts, part_name);
    // what the program got wrong comes first: a form the lowering cannot
    // take is usually one of those errors seen from the other side
    if !l.v.errors.is_empty() {
        return Err(Refusal::Invalid(std::mem::take(&mut l.v.errors)));
    }
    let mut program = Program { parts: vec![part.map_err(Refusal::Unlowered)?] };
    decide_loops(&mut program);
    decide_answers(&mut program);
    // the parts agree before a backend reads them (ir_verify): a lowering
    // that disagrees with itself is refused as the bug in Crow it is, in a
    // debug build and in every harness; a release build checks when asked
    match crate::ir_verify::mode() {
        crate::ir_verify::Mode::Off => {}
        crate::ir_verify::Mode::Report => {
            for f in crate::ir_verify::verify(&program) {
                eprintln!("ir-verify: {}", f);
            }
        }
        crate::ir_verify::Mode::Strict => {
            let found = crate::ir_verify::verify(&program);
            if let Some(first) = found.first() {
                let more = if found.len() > 1 { format!(" (and {} more)", found.len() - 1) } else { String::new() };
                return Err(Refusal::Unlowered(format!(
                    "internal error: the lowered program disagrees with itself — a bug in Crow, not in this program: {}{}",
                    first, more
                )));
            }
        }
    }
    Ok(program)
}

// ── effects: answered, or held to the commit ────────────────────────

/// Decides the effects functions return, now that every caller is known.
/// A function's last line is its value (the parser writes it as a
/// return), but a handler's value goes to nobody: the page, `later` and
/// the harness name a handler only to run it. Nor does the value of a
/// function every caller calls as a statement. An effect such a function
/// returns is one whose answer nobody uses, so it waits for the commit as
/// a statement's does (spec, "A handler is a transaction").
fn decide_answers(program: &mut Program) {
    for part in &mut program.parts {
        let used = values_used(part);
        for f in &mut part.fns {
            if !used.contains(&f.name) {
                hold_returned(&mut f.body);
            }
        }
    }
}

/// The functions whose value something reads: a call whose value is used,
/// a function handed on as a value anywhere but to run it, and a function
/// whose call a used function returns.
fn values_used(part: &Part) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    // (caller, callee): the callee's value is the caller's
    let mut passes: Vec<(String, String)> = Vec::new();
    let mut refs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in &part.fns {
        used_in_block(&f.body, Some(&f.name), &mut used, &mut passes, &mut refs);
    }
    for d in &part.derives {
        used_in_block(&d.body, None, &mut used, &mut passes, &mut refs);
    }
    for t in &part.tests {
        used_in_block(&t.body, None, &mut used, &mut passes, &mut refs);
    }
    for fact in &part.facts {
        used_in_expr(&fact.default, &mut used, &mut refs);
    }
    used_in_block(&part.main, None, &mut used, &mut passes, &mut refs);
    // a function named somewhere other than to be run is a value a callee
    // may call and read
    used.extend(refs.into_iter().filter(|(_, (all, run))| all > run).map(|(n, _)| n));
    loop {
        let before = used.len();
        for (caller, callee) in &passes {
            if used.contains(caller) {
                used.insert(callee.clone());
            }
        }
        if used.len() == before {
            return used;
        }
    }
}

fn used_in_block(b: &Block, owner: Option<&str>, used: &mut BTreeSet<String>, passes: &mut Vec<(String, String)>, refs: &mut BTreeMap<String, (usize, usize)>) {
    for s in b {
        match s {
            // a call for what it does: its value goes nowhere
            Stmt::Do(Expr::Call { target: Target::User(_), args, .. }) => args.iter().for_each(|a| used_in_expr(a, used, refs)),
            // a call returned: its value is read when the function's is
            Stmt::Return(Some(Expr::Call { target: Target::User(n), args, .. })) if owner.is_some() => {
                passes.push((owner.unwrap_or_default().to_string(), n.clone()));
                args.iter().for_each(|a| used_in_expr(a, used, refs));
            }
            Stmt::If { cond, then, els } => {
                used_in_expr(cond, used, refs);
                used_in_block(then, owner, used, passes, refs);
                if let Some(e) = els {
                    used_in_block(e, owner, used, passes, refs);
                }
            }
            Stmt::While { cond: e, body } | Stmt::For { iter: e, body, .. } => {
                used_in_expr(e, used, refs);
                used_in_block(body, owner, used, passes, refs);
            }
            other => visit_stmt(other, &mut |e| note_used(e, used, refs)),
        }
    }
}

fn used_in_expr(e: &Expr, used: &mut BTreeSet<String>, refs: &mut BTreeMap<String, (usize, usize)>) {
    visit_expr(e, &mut |x| note_used(x, used, refs));
}

/// One node: a call's value is read where it is written (a collecting
/// loop's last line is what it yields), and each naming of a function is
/// counted, with the namings that only run it: a control's handler
/// (`click = save`, a widget's `on = map[pick = choose]`), `later[f]` and
/// the harness's `click[a, save]`.
fn note_used(e: &Expr, used: &mut BTreeSet<String>, refs: &mut BTreeMap<String, (usize, usize)>) {
    let run = |x: &Expr, refs: &mut BTreeMap<String, (usize, usize)>| {
        if let Expr::FnRef(n, _) = x {
            refs.entry(n.clone()).or_default().1 += 1;
        }
    };
    match e {
        Expr::Call { target: Target::User(n), .. } => {
            used.insert(n.clone());
        }
        Expr::FnRef(n, _) => refs.entry(n.clone()).or_default().0 += 1,
        Expr::Tree { attrs, .. } => {
            for (_, a) in attrs {
                match a {
                    Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| run(v, refs)),
                    other => run(other, refs),
                }
            }
        }
        Expr::Effect { args, .. } | Expr::Call { args, .. } if matches!(call_name(e).as_deref(), Some("later" | "click")) => {
            args.iter().for_each(|a| run(a, refs));
        }
        _ => {}
    }
}

/// Every effect a function returns, held: its answer goes to nobody.
fn hold_returned(b: &mut Block) {
    for s in b {
        match s {
            Stmt::Return(Some(Expr::Effect { answered, .. })) => *answered = false,
            Stmt::If { then, els, .. } => {
                hold_returned(then);
                if let Some(e) = els {
                    hold_returned(e);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => hold_returned(body),
            _ => {}
        }
    }
}

// ── loops: in parallel or in order ──────────────────────────────────

/// Decides every loop of the program at once, now that each function's
/// kind and reach are known: a loop runs its iterations in parallel when
/// nothing its body does can show the order they ran in (spec §14, For
/// Loops). Both modes read the flag; neither decides for itself.
fn decide_loops(program: &mut Program) {
    for part in &mut program.parts {
        let order = LoopOrder::of(part);
        let decide = |var: &str, body: &Block| !order.keeps_order(var, body);
        for f in &mut part.fns {
            set_loops(&mut f.body, &decide);
        }
        for d in &mut part.derives {
            set_loops(&mut d.body, &decide);
        }
        for t in &mut part.tests {
            set_loops(&mut t.body, &decide);
        }
        for fact in &mut part.facts {
            set_loops_expr(&mut fact.default, &decide);
        }
        set_loops(&mut part.main, &decide);
    }
}

/// What keeps a loop in order, program-wide: the functions a call to
/// which does, and whether a call through a function value might.
struct LoopOrder {
    known: BTreeSet<String>,
    ordered: BTreeSet<String>,
    through_values: bool,
}

impl LoopOrder {
    fn of(part: &Part) -> LoopOrder {
        let known: BTreeSet<String> = part.fns.iter().map(|f| f.name.clone()).collect();
        let mut ordered: BTreeSet<String> = BTreeSet::new();
        // each function's callees (a function handed to a call counts: the
        // callee may call it) and whether it calls through a value
        let mut calls: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();
        for f in &part.fns {
            let mut here = false;
            let mut callees: BTreeSet<String> = BTreeSet::new();
            let mut through = false;
            visit_block(&f.body, &mut |e| {
                here |= shows_order(e);
                match e {
                    Expr::Call { target: Target::User(n), .. } => {
                        callees.insert(n.clone());
                    }
                    Expr::Index { base, .. } if matches!(base.ty(), Ty::Fn(..)) => through = true,
                    _ => {}
                }
                callees.extend(handed(e));
            });
            visit_stmts(&f.body, &mut |s| here |= matches!(s, Stmt::Store { .. }));
            if here || !f.effects.is_empty() || matches!(f.kind, FnKind::Handler | FnKind::Helper | FnKind::View) {
                ordered.insert(f.name.clone());
            }
            calls.insert(f.name.clone(), (callees, through));
        }
        // every function the program holds as a value: a call through one
        // may reach any of them
        let mut values: BTreeSet<String> = BTreeSet::new();
        let mut note = |e: &Expr| {
            if let Expr::FnRef(n, _) = e {
                values.insert(n.clone());
            }
        };
        for f in &part.fns {
            visit_block(&f.body, &mut note);
        }
        for d in &part.derives {
            visit_block(&d.body, &mut note);
        }
        for t in &part.tests {
            visit_block(&t.body, &mut note);
        }
        for fact in &part.facts {
            visit_expr(&fact.default, &mut note);
        }
        visit_block(&part.main, &mut note);
        loop {
            let through_values = values.iter().any(|v| ordered.contains(v) || !known.contains(v));
            let mut grew = false;
            for (name, (callees, through)) in &calls {
                if ordered.contains(name) {
                    continue;
                }
                if callees.iter().any(|c| ordered.contains(c) || !known.contains(c)) || (*through && through_values) {
                    ordered.insert(name.clone());
                    grew = true;
                }
            }
            if !grew {
                return LoopOrder { known, ordered, through_values };
            }
        }
    }

    /// Whether a loop binding `var` over `body` must run in order: the body
    /// writes a variable from outside it or a fact, leaves the function (a
    /// return, a propagated error, a handler's stop: the first in input
    /// order decides), or reaches something whose order shows.
    fn keeps_order(&self, var: &str, body: &Block) -> bool {
        // the iteration's own names: the loop's variable, what the body
        // binds, and the variables of loops inside it
        let mut own: BTreeSet<String> = BTreeSet::new();
        own.insert(var.to_string());
        visit_stmts(body, &mut |s| match s {
            Stmt::Let { var, .. } | Stmt::For { var, .. } => {
                own.insert(var.clone());
            }
            _ => {}
        });
        visit_block(body, &mut |e| {
            if let Expr::ForExpr { var, .. } = e {
                own.insert(var.clone());
            }
        });
        let mut order = false;
        visit_stmts(body, &mut |s| {
            order |= match s {
                Stmt::Set { var, .. } | Stmt::SetPath { var, .. } => !own.contains(var),
                Stmt::Store { .. } | Stmt::Return(_) | Stmt::Propagate { .. } | Stmt::Stop { .. } => true,
                _ => false,
            }
        });
        visit_block(body, &mut |e| {
            order |= shows_order(e)
                || match e {
                    Expr::Call { target: Target::User(n), .. } => self.ordered.contains(n) || !self.known.contains(n),
                    Expr::Index { base, .. } => matches!(base.ty(), Ty::Fn(..)) && self.through_values,
                    // a collecting loop over a list that can fail refuses it
                    // where it starts: it can leave the function
                    Expr::ForExpr { iter, .. } => iter.ty().is_result(),
                    _ => false,
                }
                || handed(e).iter().any(|n| self.ordered.contains(n) || !self.known.contains(n));
        });
        order
    }
}

/// A node whose order shows by itself: an effect, a draw from the seeded
/// generator, an assertion, a look at background work, or a paged table
/// (the render numbers them in order).
fn shows_order(e: &Expr) -> bool {
    match e {
        Expr::Effect { .. } => true,
        Expr::Call { target: Target::Runtime(_) | Target::Module(..), .. } => {
            let n = call_name(e).unwrap_or_default();
            is_effect(&n) || DRAWS.contains(&n.as_str()) || matches!(n.as_str(), "assert" | "running")
        }
        Expr::Tree { element, attrs, .. } => element == "table" && attrs.iter().any(|(k, _)| k == "page"),
        _ => false,
    }
}

/// The functions a call hands over as values (`any[xs, is_bad]`): the
/// callee may call them.
fn handed(e: &Expr) -> Vec<String> {
    match e {
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args
            .iter()
            .filter_map(|a| match a {
                Expr::FnRef(n, _) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A lowered value's type as inference's rules read a child's
/// (`infer_expr`): markup is text there.
fn read(x: &Expr) -> CrowType {
    full(x).text()
}

/// A lowered value's type with markup told apart (`infer_expr_full`): a
/// tree is markup.
fn full(x: &Expr) -> CrowType {
    match x.ty() {
        Ty::Node => CrowType::Markup,
        t => crow_type(&t),
    }
}

pub fn ty_of(t: &CrowType) -> Ty {
    match t {
        CrowType::Int => Ty::Int,
        CrowType::Float => Ty::Float,
        CrowType::Str => Ty::Str,
        CrowType::Markup => Ty::Markup,
        CrowType::Bool => Ty::Bool,
        CrowType::Array(inner) => Ty::List(Box::new(ty_of(inner))),
        CrowType::NumArray(inner) => Ty::Tensor(Box::new(ty_of(inner))),
        CrowType::Map(inner) => Ty::Map(Box::new(ty_of(inner))),
        CrowType::Struct(n) => Ty::Record(n.clone()),
        CrowType::OptBoxStruct(n) => Ty::OptRecord(n.clone()),
        CrowType::Frame => Ty::Frame(None),
        CrowType::FrameCol => Ty::Column(Box::new(Ty::Unknown)),
        CrowType::Json => Ty::Json,
        CrowType::Result(inner) => Ty::Result(Box::new(ty_of(inner))),
        CrowType::Fn(ps, r) => Ty::Fn(ps.iter().map(ty_of).collect(), Box::new(ty_of(r))),
        CrowType::DbHandle => Ty::Handle("db".into()),
        CrowType::DbTx => Ty::Handle("tx".into()),
        CrowType::WebApp => Ty::Handle("web.app".into()),
        CrowType::WebResponse => Ty::Handle("web.response".into()),
        CrowType::WebRequest => Ty::Handle("web.request".into()),
        CrowType::WebDb => Ty::Handle("web.db".into()),
        CrowType::Sparse => Ty::Handle("sparse".into()),
        CrowType::Mailer => Ty::Handle("mailer".into()),
        CrowType::Void => Ty::Unit,
        CrowType::Unknown => Ty::Unknown,
    }
}

/// The inference type an IR type came from: the Rust backend reads the
/// type tables (Rust names, defaults) through it.
pub fn crow_type(t: &Ty) -> CrowType {
    match t {
        Ty::Int => CrowType::Int,
        Ty::Float => CrowType::Float,
        Ty::Bool => CrowType::Bool,
        Ty::Str => CrowType::Str,
        Ty::Markup => CrowType::Markup,
        Ty::Unit => CrowType::Void,
        Ty::List(inner) => CrowType::Array(Box::new(crow_type(inner))),
        Ty::Map(inner) => CrowType::Map(Box::new(crow_type(inner))),
        Ty::Tensor(inner) => CrowType::NumArray(Box::new(crow_type(inner))),
        Ty::Frame(_) => CrowType::Frame,
        Ty::Column(_) => CrowType::FrameCol,
        Ty::Json => CrowType::Json,
        Ty::Record(n) => CrowType::Struct(n.clone()),
        Ty::OptRecord(n) => CrowType::OptBoxStruct(n.clone()),
        Ty::Result(inner) => CrowType::Result(Box::new(crow_type(inner))),
        Ty::Fn(ps, r) => CrowType::Fn(ps.iter().map(crow_type).collect(), Box::new(crow_type(r))),
        Ty::Handle(k) => match k.as_str() {
            "db" => CrowType::DbHandle,
            "tx" => CrowType::DbTx,
            "sparse" => CrowType::Sparse,
            "web.app" => CrowType::WebApp,
            "web.response" => CrowType::WebResponse,
            "web.request" => CrowType::WebRequest,
            "web.db" => CrowType::WebDb,
            "mailer" => CrowType::Mailer,
            _ => CrowType::Unknown,
        },
        // a view's tree has no inference type of its own: the backends
        // hold it as a node
        Ty::Node => CrowType::Unknown,
        Ty::Unknown => CrowType::Unknown,
    }
}

/// What a provisional lowering met (F5, signatures found by lowering
/// bodies): what inference reads from a body once the lowering types it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Seen {
    /// Each call of a function or record the program declares, by the name
    /// written, with its arguments' types as lowered: markup told apart, a
    /// failure not yet unwrapped.
    pub calls: Vec<(String, Vec<CrowType>)>,
    /// The type of each value the body returns, where it is returned (a
    /// collecting loop's body returns nothing of the function's).
    pub returns: Vec<CrowType>,
    /// Each place an error leaves the body — a decision on a value that
    /// can fail, or an assignment refusing one — and whether it is inside
    /// a collecting loop.
    pub refusals: Vec<(Decision, bool)>,
}

/// How a body is lowered (F5).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pass {
    /// For the program: every check made, every decision kept.
    Final,
    /// To learn its locals' types: each variable settles across its
    /// assignments as the body runs, as inference's walk settles it.
    Settle,
    /// To learn what its calls say of their callees: each call notes its
    /// arguments' types, the environment kept as inference's call scan
    /// keeps it.
    Scan,
}

#[derive(Clone, Copy, PartialEq)]
enum Where {
    Top,
    In(FnKind),
    Derive,
    Test,
}

struct Lowerer<'a> {
    ctx: &'a mut TypeContext,
    /// Locals declared so far, innermost scope last.
    scopes: Vec<HashSet<String>>,
    /// Statements hoisted in front of the one being lowered (propagation).
    pre: Vec<Stmt>,
    temp: usize,
    at: Where,
    in_ui: bool,
    /// Names every function in the program reaches, for `Fn.effects`.
    calls: BTreeMap<String, BTreeSet<String>>,
    direct_effects: BTreeMap<String, BTreeSet<String>>,
    current_fn: String,
    /// What the checks keep (`checks.rs`).
    v: checks::Checks,
    /// How the body under way is lowered (F5, `provisional`).
    pass: Pass,
    /// While a body is lowered to learn its types, not to keep it: what
    /// the lowering met.
    seen: Option<Seen>,
    /// How many collecting loops the lowering is inside.
    collecting: usize,
    /// Above zero while the call scan lowers what inference's scan never
    /// reads (a callee lowered as a value).
    unscanned: usize,
    /// The collecting loops over a list of named functions the call scan
    /// is inside (`for f in [a, b]`): the loop's variable and the functions.
    fn_lists: Vec<(String, Vec<String>)>,
    /// A provisional lowering's locals, each with its first value's type:
    /// with the type it settles on, what it is declared as (`declared_as`).
    first_types: HashMap<String, CrowType>,
    /// A provisional lowering's assignments of a value that can fail to a
    /// variable declared before: the variable, the value's payload, and
    /// whether inside a collecting loop. Whether each refuses the failed
    /// value is known once the body's locals have settled.
    held: Vec<(String, CrowType, bool)>,
    /// While a test is lowered for the handlers it clicks: each handler
    /// call a click holds, with its arguments' types (`type_clicks`).
    clicks: Option<Vec<(String, Vec<CrowType>)>>,
    /// The shapes of the body's tensor locals the program states
    /// (`shapes.rs`).
    shapes: HashMap<String, shapes::Shape>,
}

impl<'a> Lowerer<'a> {
    fn new(ctx: &'a mut TypeContext, in_ui: bool) -> Self {
        Lowerer {
            ctx,
            scopes: vec![HashSet::new()],
            pre: Vec::new(),
            temp: 0,
            at: Where::Top,
            in_ui,
            calls: BTreeMap::new(),
            direct_effects: BTreeMap::new(),
            current_fn: String::new(),
            v: checks::Checks::default(),
            pass: Pass::Final,
            seen: None,
            collecting: 0,
            unscanned: 0,
            fn_lists: Vec::new(),
            first_types: HashMap::new(),
            held: Vec::new(),
            clicks: None,
            shapes: HashMap::new(),
        }
    }

    /// A lowering that only infers (F5): what the checks read before the
    /// walk is read, and nothing it finds is kept but types.
    fn for_inference(ctx: &'a mut TypeContext, in_ui: bool, stmts: &[S]) -> Self {
        let mut l = Lowerer::new(ctx, in_ui);
        l.prescan(stmts, &HashSet::new());
        l
    }

    /// A body lowered to learn its types, keeping nothing of it (F5): the
    /// checks say nothing, what the lowering keeps for the program is left
    /// as it was, and each variable's type settles across its assignments
    /// as the body runs, the way inference's walk settles it, so the body's
    /// environment is left in the context. `own` is the body's first scope
    /// (its parameters). An error is the final lowering's to report.
    fn provisional(&mut self, pass: Pass, own: Option<HashSet<String>>, body: &[S]) -> Seen {
        let outer = self.seen.replace(Seen::default());
        let outer_pass = std::mem::replace(&mut self.pass, pass);
        let temp = self.temp;
        let pre = std::mem::take(&mut self.pre);
        let depth = self.scopes.len();
        let (mute, line, collecting, unscanned, fn_lists) = (self.v.mute, self.v.line, self.collecting, self.unscanned, self.fn_lists.len());
        let local_types = self.v.local_types.clone();
        let stored = self.v.stored.clone();
        let outer_first = std::mem::take(&mut self.first_types);
        let outer_held = std::mem::take(&mut self.held);
        let outer_shapes = self.shapes.clone();
        self.v.mute += 1;
        if let Some(own) = own {
            self.scopes.push(own);
        }
        let _ = self.lower_block(body);
        self.scopes.truncate(depth);
        // a variable holds no error when it is declared with a type that
        // cannot fail: an assignment of a failed value to it refuses the
        // value and hands the error out of the body (spec, Reassignment),
        // decided with the type the final lowering declares it with
        for (name, payload, collecting) in std::mem::take(&mut self.held) {
            let settled = self.ctx.vars.get(&name);
            let held = match self.first_types.get(&name) {
                Some(first) => crow_type(&declared_as(&ty_of(first), settled)),
                None => settled.cloned().unwrap_or(CrowType::Unknown),
            };
            if crate::type_infer::settle::refuses_failed(&held, &payload) {
                if let Some(seen) = &mut self.seen {
                    seen.refusals.push((Decision::Assign(name), collecting));
                }
            }
        }
        self.first_types = outer_first;
        self.held = outer_held;
        self.shapes = outer_shapes;
        self.v.mute = mute;
        self.v.line = line;
        self.v.local_types = local_types;
        self.v.stored = stored;
        self.collecting = collecting;
        self.unscanned = unscanned;
        self.fn_lists.truncate(fn_lists);
        self.pre = pre;
        self.temp = temp;
        self.pass = outer_pass;
        std::mem::replace(&mut self.seen, outer).unwrap_or_default()
    }

    /// Whose the top-level statement being lowered is: the part its
    /// declaration's prefix names, or the entry's (a statement that
    /// declares nothing, `page[…]`, is the entry's). A part's top level
    /// reaches what it reaches under this name (`Part.main_effects`).
    fn main_of_part(&self) -> String {
        let prefix = self.v.decl.as_deref().and_then(crate::part_prefix_of_declaration).unwrap_or_default();
        format!("<main>{}", prefix)
    }

    /// The kind of function a declaration is, as the lowering writes it.
    fn kind_of(&self, name: &str, body: &[S], is_view: bool) -> FnKind {
        if is_view || self.ctx.views.contains(name) {
            FnKind::View
        } else if self.ctx.view_handlers.contains(name) {
            FnKind::Handler
        } else if self.in_ui && assigns_any(body, &own_facts(name, &self.ctx.facts)) {
            // (a script has no facts: its function's first assignment of a
            // name is a local, whatever the top level calls)
            FnKind::Helper
        } else {
            FnKind::Pure
        }
    }

    // ── inference by lowering (F5) ──────────────────────────────────

    /// The call scan: every body lowered in order, each call noting its
    /// arguments' types for its callee's signature — a function's body
    /// with its parameters bound, a derivation's like a parameterless
    /// function's, the top level's statements in the top level's
    /// environment. A test says nothing of the program's signatures.
    fn scan_program(&mut self, stmts: &[S]) {
        for s in stmts {
            match s {
                S::FnDecl { name, params, body, is_view, .. } => {
                    let scan = self.ctx.enter_fn_scan(name);
                    self.at = Where::In(self.kind_of(name, body, *is_view));
                    self.current_fn = name.clone();
                    self.provisional(Pass::Scan, Some(params.iter().cloned().collect()), body);
                    self.ctx.leave_fn_scan(name, scan);
                }
                S::Derive { name, body, .. } => {
                    let scan = self.ctx.enter_derive_scan(name);
                    self.at = Where::Derive;
                    self.current_fn = name.clone();
                    self.provisional(Pass::Scan, Some(HashSet::new()), body);
                    self.ctx.leave_derive_scan(name, body, scan);
                }
                S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } | S::Resource { .. } => {}
                other => {
                    self.at = Where::Top;
                    self.current_fn = "<main>".to_string();
                    self.provisional(Pass::Scan, None, std::slice::from_ref(other));
                }
            }
        }
    }

    /// A function's body typed (F5): its locals, its return type and
    /// whether an error leaves it, all as a provisional lowering met them
    /// (`finish_body`).
    fn infer_body(&mut self, name: &str, body: &[S]) {
        let params: HashSet<String> = self.ctx.functions.get(name).map(|sig| sig.params.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
        let scope = self.ctx.enter_body(name);
        self.at = if self.ctx.derives.contains(name) { Where::Derive } else { Where::In(self.kind_of(name, body, false)) };
        self.current_fn = name.to_string();
        let seen = self.provisional(Pass::Settle, Some(params), body);
        self.ctx.finish_body(name, scope, &seen.returns, !seen.refusals.is_empty());
    }

    /// What only a test's click types (F5): a handler a test clicks with
    /// arguments (`click[a, h[4]]`) takes their types for the parameters
    /// nothing in the program types — the click may be the only call it
    /// has, and a compiled program dispatches it by them. Each test is
    /// lowered as it runs, its own variables in view where its clicks'
    /// arguments are typed. Whether any parameter moved.
    fn type_clicks(&mut self, stmts: &[S]) -> bool {
        let mut clicks: Vec<(String, Vec<CrowType>)> = Vec::new();
        for s in stmts {
            if let S::TestBlock { name, params, body } = s {
                let saved = self.ctx.vars.clone();
                for p in params {
                    self.ctx.vars.insert(p.clone(), CrowType::Unknown);
                }
                self.at = Where::Test;
                self.current_fn = name.clone().unwrap_or_default();
                self.clicks = Some(Vec::new());
                self.provisional(Pass::Settle, Some(params.iter().cloned().collect()), body);
                clicks.extend(self.clicks.take().unwrap_or_default());
                self.ctx.vars = saved;
            }
        }
        self.ctx.type_clicked(&clicks)
    }

    /// The F2 monomorphization pass (F5): each clone's calls read with its
    /// parameters bound, then its body typed; every clone's body typed
    /// again after, so return types settle whatever the order clones were
    /// made in.
    fn specialize(&mut self) {
        for _round in 0..5 {
            let new_clones = self.ctx.new_clones();
            if new_clones.is_empty() {
                break;
            }
            for mangled in &new_clones {
                let body = self.ctx.functions.get(mangled).map(|sig| sig.body.clone()).unwrap_or_default();
                let params: HashSet<String> = self.ctx.functions.get(mangled).map(|sig| sig.params.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
                let saved = self.ctx.bind_params(mangled);
                self.at = Where::In(self.kind_of(mangled, &body, false));
                self.current_fn = mangled.clone();
                self.provisional(Pass::Scan, Some(params), &body);
                self.ctx.vars = saved;
                self.infer_body(mangled, &body);
            }
        }
        for _ in 0..2 {
            for mangled in self.ctx.clone_names() {
                if let Some(body) = self.ctx.reset_clone(&mangled) {
                    self.infer_body(&mangled, &body);
                }
            }
        }
    }

    /// The top level's variables (F5): its statements lowered as they run,
    /// each variable settled across its assignments.
    fn settle_top(&mut self, stmts: &[S]) {
        self.at = Where::Top;
        self.current_fn = "<main>".to_string();
        for s in stmts {
            match s {
                S::FnDecl { .. } | S::Derive { .. } | S::TestBlock { .. } | S::StructDecl { .. } | S::Use { .. } | S::Resource { .. } => {}
                other => {
                    self.provisional(Pass::Settle, None, std::slice::from_ref(other));
                }
            }
        }
    }

    /// A call's arguments' types noted for its callee's signature, in the
    /// call scan (F5).
    fn scan_call(&mut self, callee: &A, args: &[A], types: &[CrowType]) {
        if self.pass != Pass::Scan || self.unscanned > 0 {
            return;
        }
        if self.ctx.call_says_of_callee(callee) {
            self.ctx.note_call(callee, args, types);
        }
        // a call through a loop's variable over a list of named functions
        // types every one of them (an uncalled function would keep its
        // parameters unknown and not be emitted)
        if let A::Ident(n) = callee {
            if let Some((_, fns)) = self.fn_lists.iter().rev().find(|(v, _)| v == n) {
                let text: Vec<CrowType> = types.iter().map(|t| t.text()).collect();
                for f in fns.clone() {
                    self.ctx.note_through_list(&f, &text);
                }
            }
        }
    }

    /// A loop's body in the call scan (F5): read again when a variable from
    /// before the loop took a type in it, so its calls see that type.
    fn scan_loop_body(&mut self, body: &[S]) -> Result<Block, String> {
        let outer = self.ctx.vars.clone();
        let scope = self.scopes.last().cloned().unwrap_or_default();
        let b = self.lower_block(body)?;
        let moved = outer.iter().any(|(n, t)| self.ctx.vars.get(n).map_or(false, |now| now != t));
        if moved {
            if let Some(last) = self.scopes.last_mut() {
                *last = scope;
            }
            return self.lower_block(body);
        }
        Ok(b)
    }

    // ── declarations ────────────────────────────────────────────────

    fn lower_part(&mut self, stmts: &[S], name: &str) -> Result<Part, String> {
        let mut part = Part {
            name: name.to_string(),
            uses: vec![],
            cpu: None,
            gpu: None,
            nodes: None,
            structs: vec![],
            facts: vec![],
            derives: vec![],
            fns: vec![],
            tests: vec![],
            main: vec![],
            main_effects: BTreeMap::new(),
        };
        let fact_names: BTreeSet<String> = self.ctx.facts.clone();
        let written: BTreeSet<String> = facts_written_by_fns(stmts, &fact_names);
        for s in stmts {
            // an error is owned by the declaration it is found in; a line
            // marker belongs to the statement after it, and an attached test
            // (`test[p]` after a fn) to the fn it follows
            match s {
                S::Line(_) | S::TestBlock { name: None, .. } => {}
                _ => self.v.decl = checks::declared_name(s),
            }
            match s {
                S::Use { path, alias } => {
                    part.uses.push(match alias {
                        Some(a) => format!("{} as {}", path, a),
                        None => path.clone(),
                    });
                }
                S::Resource { kind, count } => {
                    let n = if let A::Int(n) = count { *n } else { 0 };
                    match kind.as_str() {
                        "gpu" => part.gpu = Some(n),
                        "nodes" => part.nodes = Some(n),
                        _ => part.cpu = Some(n),
                    }
                }
                S::StructDecl { name, fields } => {
                    self.check_struct(name, fields);
                    let sig = self.ctx.structs.get(name).cloned();
                    let fields = match sig {
                        Some(sig) => sig.fields.iter().map(|(n, t)| (n.clone(), ty_of(t))).collect(),
                        None => fields.iter().map(|f| (f.clone(), Ty::Unknown)).collect(),
                    };
                    part.structs.push(StructDef { name: name.clone(), fields });
                }
                S::FnDecl { name, params, body, is_view, .. } => {
                    self.check_reserved(name, if *is_view { "view" } else { "function" });
                    for p in params {
                        self.check_reserved(p, "parameter");
                    }
                    let f = self.lower_fn(name, params, body, *is_view)?;
                    part.fns.push(f);
                    // a function called at several signatures is one function
                    // per signature, each typed by its own (F2): the Rust
                    // backend compiles each. Its body was checked as the
                    // function's.
                    let clones: Vec<String> = self.ctx.specializations.get(name).map(|v| v.iter().map(|(_, m)| m.clone()).collect()).unwrap_or_default();
                    for clone in clones {
                        let clone_body = self.ctx.functions.get(&clone).map(|sig| sig.body.clone()).unwrap_or_else(|| body.clone());
                        let f = self.muted(|l| l.lower_fn(&clone, params, &clone_body, *is_view))?;
                        part.fns.push(f);
                    }
                }
                S::Derive { name, fresh, body } => {
                    self.check_derive(name, body);
                    let d = self.lower_derive(name, *fresh, body)?;
                    part.derives.push(d);
                }
                S::TestBlock { name, params, body } => {
                    self.v.attached_test = name.is_none();
                    let t = self.lower_test(name.clone(), params, body)?;
                    self.v.attached_test = false;
                    part.tests.push(t);
                }
                S::Assign { name, value } if self.in_ui && fact_names.contains(name) && !self.declared(name) => {
                    self.at = Where::Top;
                    self.current_fn = self.main_of_part();
                    if self.ctx.derives.contains(name) {
                        self.refuse(format!("'{}' is derived — it follows what it reads; change those facts instead of assigning it", name));
                    }
                    let default = self.lower_expr(value)?;
                    self.check_assign(name, &self.checked(&default));
                    self.flush_pre_into(&mut part.main);
                    let ty = self.ctx.vars.get(name).map(ty_of).unwrap_or_else(|| default.ty());
                    // an int default of a fact that holds floats is a float
                    // (spec §1): the interpreter held the int until a handler
                    // stored a float, where the compiled program converted it
                    let default = if ty == Ty::Float && default.ty() == Ty::Int {
                        Expr::Convert { how: Convert::IntToFloat, expr: Box::new(default), ty: Ty::Float }
                    } else {
                        default
                    };
                    self.scopes[0].insert(name.clone());
                    self.v.fact_lines.insert(name.clone(), self.v.line);
                    part.facts.push(Fact { name: name.clone(), ty, default, session: written.contains(name) });
                }
                other => {
                    self.at = Where::Top;
                    self.current_fn = self.main_of_part();
                    let lowered = self.lower_stmt(other)?;
                    self.flush_pre_into(&mut part.main);
                    part.main.extend(lowered);
                }
            }
        }
        place_moves_in(&mut part.main, &HashSet::new(), None);
        // what a derivation and each part's top level reach, transitively,
        // as a function's does below
        let reach = |start: &str, l: &Self| -> Vec<String> {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![start.to_string()];
            let mut effects: BTreeSet<String> = BTreeSet::new();
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(e) = l.direct_effects.get(&n) {
                    effects.extend(e.iter().cloned());
                }
                if let Some(c) = l.calls.get(&n) {
                    todo.extend(c.iter().cloned());
                }
            }
            effects.into_iter().collect()
        };
        for d in &mut part.derives {
            d.effects = reach(&d.name, self);
        }
        let mains: Vec<String> = self.direct_effects.keys().chain(self.calls.keys()).filter(|k| k.starts_with("<main>")).cloned().collect();
        for m in mains {
            let effects = reach(&m, self);
            let prefix = m.trim_start_matches("<main>").to_string();
            part.main_effects.entry(prefix).or_default().extend(effects);
        }
        for effects in part.main_effects.values_mut() {
            effects.sort();
            effects.dedup();
        }
        // what each function reaches, transitively
        let names: Vec<String> = part.fns.iter().map(|f| f.name.clone()).collect();
        for f in &mut part.fns {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![f.name.clone()];
            let mut effects: BTreeSet<String> = BTreeSet::new();
            let mut recursive = false;
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(e) = self.direct_effects.get(&n) {
                    effects.extend(e.iter().cloned());
                }
                if let Some(c) = self.calls.get(&n) {
                    for callee in c {
                        // reaching itself again: its calls can nest without end
                        recursive |= *callee == f.name;
                        if names.contains(callee) {
                            todo.push(callee.clone());
                        }
                    }
                }
            }
            f.effects = effects.into_iter().collect();
            f.recursive = recursive;
        }
        // a view or a derivation that reaches mail through a function it
        // calls would send it at every render (a call of its own is refused
        // where it is written)
        let renders: Vec<String> = part.fns.iter().filter(|f| f.kind == FnKind::View).map(|f| f.name.clone()).chain(part.derives.iter().map(|d| d.name.clone())).collect();
        for r in renders {
            let own = self.direct_effects.get(&r).map_or(false, |e| e.contains("mail"));
            let through = self.calls.get(&r).into_iter().flatten().find(|c| part.fns.iter().any(|f| &f.name == *c && f.effects.iter().any(|e| e == "mail")));
            if let (false, Some(via)) = (own, through) {
                let msg = format!("in {}: calls {}, which sends mail — {}", r, via, checks::MAIL_IN_A_RENDER);
                self.v.errors.push((msg, Some(r.clone())));
            }
        }
        // what the live layer memoizes each derivation by, and the tables
        // each function writes, recorded here so the Rust backend reads the
        // same decisions the interpreter does
        {
            let facts = crate::derive::fact_names(stmts);
            let derive_names = self.ctx.derives.clone();
            let bodies: BTreeMap<String, Vec<S>> = self
                .ctx
                .functions
                .iter()
                .filter(|(n, _)| !derive_names.contains(*n))
                .map(|(n, sig)| (n.clone(), sig.body.clone()))
                .collect();
            let params: BTreeMap<String, Vec<String>> = self.ctx.functions.iter().map(|(n, sig)| (n.clone(), sig.params.iter().map(|(p, _)| p.clone()).collect())).collect();
            let owned = crate::derive::owned_tables(stmts, &bodies);
            let mut specs: BTreeMap<String, crate::derive::ReadSet> = BTreeMap::new();
            for st in stmts {
                if let S::Derive { name, body, .. } = st {
                    specs.insert(name.clone(), crate::derive::read_set(body, &facts, &derive_names, &bodies, &params, &owned));
                }
            }
            let written = crate::derive::written_facts(&bodies, &facts);
            crate::derive::mark_shared(&mut specs, &written);
            for d in &mut part.derives {
                if let Some(rs) = specs.get(&d.name) {
                    d.spec = rs.clone();
                }
            }
            for f in &mut part.fns {
                if let Some(body) = bodies.get(&f.name) {
                    let reach = crate::derive::called_fns(body, &bodies);
                    let reach_bodies: BTreeMap<String, Vec<S>> = reach.iter().filter_map(|n| bodies.get(n).map(|b| (n.clone(), b.clone()))).collect();
                    f.tables_written = crate::derive::owned_tables(body, &reach_bodies).into_iter().collect();
                }
            }
        }
        // a derivation reads the clock when anything it calls does
        for d in &mut part.derives {
            if d.reads.clock {
                continue;
            }
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![d.name.clone()];
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if self.direct_effects.get(&n).map_or(false, |e| e.contains("now")) {
                    d.reads.clock = true;
                    break;
                }
                if let Some(c) = self.calls.get(&n) {
                    todo.extend(c.iter().cloned());
                }
            }
        }
        Ok(part)
    }

    fn lower_fn(&mut self, name: &str, params: &[String], body: &[S], is_view: bool) -> Result<Fn, String> {
        let kind = self.kind_of(name, body, is_view);
        // the body typed as inference types it: parameters from the
        // signature, first assignments new bindings
        let scope = self.ctx.enter_body(name);
        let sig = self.ctx.functions.get(name).cloned();
        let mut ps: Vec<(String, Ty)> = Vec::new();
        for p in params {
            let t = sig
                .as_ref()
                .and_then(|s| s.params.iter().find(|(n, _)| n == p).map(|(_, t)| t.clone()))
                .unwrap_or(CrowType::Unknown);
            ps.push((p.clone(), ty_of(&t)));
        }
        let ret = sig.as_ref().map(|s| ty_of(&s.return_type)).unwrap_or(Ty::Unknown);
        let ret = if kind == FnKind::View { Ty::Node } else { ret };
        self.at = Where::In(kind);
        self.current_fn = name.to_string();
        // a view or handler of a ui program sees its reactive state, the
        // sanctioned exception to function-scope isolation
        self.v.reactive = self.v.ui_mode && matches!(kind, FnKind::View | FnKind::Handler);
        self.v.local_types.clear();
        self.v.stored.clear();
        // lowered once to learn its types: each local settled across its
        // assignments, which a first value of no known type is declared by
        self.provisional(Pass::Settle, Some(params.iter().cloned().collect()), body);
        self.record_env(name, params, body);
        self.scopes.push(params.iter().cloned().collect());
        let outer_shapes = std::mem::take(&mut self.shapes);
        let block = self.lower_block(body);
        self.shapes = outer_shapes;
        self.scopes.pop();
        self.v.reactive = false;
        let mut block = block?;
        self.ctx.leave_body(scope);
        let param_set: HashSet<String> = params.iter().cloned().collect();
        place_moves_in(&mut block, &HashSet::new(), Some(&param_set));
        Ok(Fn { name: name.to_string(), params: ps, ret, body: block, kind, effects: vec![], tables_written: vec![], recursive: false })
    }

    /// A body's parameters and locals as its lowering begins with them, kept
    /// when `crow build --types` asks (`type_infer::dump_types`).
    fn record_env(&mut self, key: &str, params: &[String], body: &[S]) {
        if !self.ctx.record_envs {
            return;
        }
        let mut own: HashSet<String> = params.iter().cloned().collect();
        own.extend(crate::lower::assigned(body));
        loop_vars(body, &mut own);
        let env: BTreeMap<String, CrowType> = own.into_iter().filter_map(|n| self.ctx.vars.get(&n).map(|t| (n, t.clone()))).collect();
        self.ctx.body_envs.insert(key.to_string(), env);
    }

    fn lower_derive(&mut self, name: &str, fresh: f64, body: &[S]) -> Result<Derive, String> {
        // a derivation's locals are its own, as a function's are: typed in a
        // body of their own, first assignments new bindings (an empty list's
        // type is its own pushes', not another body's local of that name)
        let scope = self.ctx.enter_body(name);
        self.at = Where::Derive;
        self.current_fn = name.to_string();
        self.v.local_types.clear();
        self.v.stored.clear();
        self.provisional(Pass::Settle, Some(HashSet::new()), body);
        self.record_env(name, &[], body);
        self.scopes.push(HashSet::new());
        let outer_shapes = std::mem::take(&mut self.shapes);
        let block = self.lower_block(body);
        self.shapes = outer_shapes;
        let mut block = block?;
        self.scopes.pop();
        self.ctx.leave_body(scope);
        place_moves_in(&mut block, &HashSet::new(), Some(&HashSet::new()));
        let ty = self.ctx.vars.get(name).map(ty_of).unwrap_or(Ty::Unknown);
        let mut reads = ReadSet::default();
        let mut names = BTreeSet::new();
        let mut clock = false;
        reads_of_stmts(body, &mut names, &mut clock);
        for n in names {
            if self.ctx.derives.contains(&n) && n != name {
                reads.derives.push(n);
            } else if self.ctx.facts.contains(&n) {
                reads.facts.push(n);
            }
        }
        reads.clock = clock;
        Ok(Derive { name: name.to_string(), ty, body: block, reads, fresh, spec: Default::default(), effects: vec![] })
    }

    fn lower_test(&mut self, name: Option<String>, params: &[String], body: &[S]) -> Result<Test, String> {
        let saved = self.ctx.vars.clone();
        for p in params {
            self.ctx.vars.insert(p.clone(), CrowType::Unknown);
        }
        self.at = Where::Test;
        self.current_fn = name.clone().unwrap_or_default();
        self.provisional(Pass::Settle, Some(params.iter().cloned().collect()), body);
        self.record_env(&format!("test {}", name.clone().unwrap_or_default()), params, body);
        self.scopes.push(params.iter().cloned().collect());
        let outer_shapes = std::mem::take(&mut self.shapes);
        let block = self.lower_block(body);
        self.shapes = outer_shapes;
        let mut block = block?;
        self.scopes.pop();
        self.ctx.vars = saved;
        place_moves_in(&mut block, &HashSet::new(), Some(&params.iter().cloned().collect()));
        Ok(Test { name, params: params.to_vec(), body: block })
    }

    // ── statements ──────────────────────────────────────────────────

    fn lower_block(&mut self, stmts: &[S]) -> Result<Block, String> {
        let mut out = Vec::new();
        for s in stmts {
            let lowered = self.lower_stmt(s)?;
            self.flush_pre_into(&mut out);
            out.extend(lowered);
        }
        Ok(out)
    }

    fn flush_pre_into(&mut self, out: &mut Block) {
        if !self.pre.is_empty() {
            out.extend(std::mem::take(&mut self.pre));
        }
    }

    fn declared(&self, name: &str) -> bool {
        // a function sees its parameters and its own locals: the program's
        // top-level names are invisible inside it (spec, No Ambient
        // Mutability), so an assignment there binds a new local
        let own_only = matches!(self.at, Where::In(_) | Where::Derive);
        self.scopes.iter().enumerate().any(|(i, s)| !(own_only && i == 0) && s.contains(name))
    }

    fn is_fact(&self, name: &str) -> bool {
        self.in_ui && self.ctx.facts.contains(name) && !self.scopes.iter().skip(1).any(|s| s.contains(name))
    }

    fn sees_facts(&self) -> bool {
        matches!(self.at, Where::Top | Where::Derive | Where::Test | Where::In(FnKind::View) | Where::In(FnKind::Handler) | Where::In(FnKind::Helper))
    }

    fn lower_stmt(&mut self, s: &S) -> Result<Vec<Stmt>, String> {
        Ok(match s {
            S::Line(n) => {
                self.v.line = *n;
                vec![Stmt::Line(*n)]
            }
            S::Assign { name, value } => {
                if self.ctx.derives.contains(name) {
                    self.refuse(format!("'{}' is derived — it follows what it reads; change those facts instead of assigning it", name));
                }
                if !self.defined(name) {
                    self.check_reserved(name, "variable");
                }
                self.check_top_write(name);
                let expr = self.lower_expr(value)?;
                if self.pass == Pass::Final {
                    self.check_assign(name, &self.checked(&expr));
                }
                if self.pass == Pass::Scan {
                    // the call scan settles the variable once its value's
                    // calls are read, with the value's type
                    self.ctx.note_scanned_assign(name, full(&expr));
                    self.ctx.note_scanned_value(name, value);
                } else if let Some(schema) = self.ctx.frame_schema_of(value) {
                    // the columns a local reads are those of the value
                    // assigned here, in this function — not the last
                    // function's local of the same name (inference keeps
                    // one map by name)
                    self.ctx.frame_schemas.insert(name.clone(), schema);
                }
                let stmt = self.assign(name, expr);
                if matches!(stmt, Stmt::Store { .. }) {
                    self.v.stored.insert(name.clone());
                }
                vec![stmt]
            }
            S::MultiAssign { names, values } if self.pass == Pass::Settle => self.settled_multi(names, values)?,
            S::MultiAssign { names, values } if self.pass == Pass::Scan => self.scanned_multi(names, values)?,
            S::MultiAssign { names, values } => {
                // each new name checked against the names taken, after the
                // values it is given
                let new: Vec<String> = names.iter().filter(|n| !self.defined(n)).cloned().collect();
                for n in names {
                    self.check_top_write(n);
                }
                let out = self.multi_assign(names, values)?;
                for st in &out {
                    if let Stmt::Store { fact, .. } = st {
                        self.v.stored.insert(fact.clone());
                    }
                }
                for n in &new {
                    self.check_reserved(n, "variable");
                }
                out
            }
            S::FieldAssign { object, field, value } => {
                // the target is checked before its value, then lowered after
                self.check_target(object);
                if let Some(root) = target_root(object) {
                    self.check_top_write(&root);
                }
                let expr = self.lower_expr(value)?;
                let (var, mut path) = self.muted_target(object)?;
                path.push(PathStep::Field(field.clone()));
                self.shapes.remove(&var);
                vec![Stmt::SetPath { var, path, expr }]
            }
            S::IndexAssign { object, index, value } => {
                self.check_target(object);
                if let Some(root) = target_root(object) {
                    self.check_top_write(&root);
                }
                let expr = self.lower_expr(value)?;
                if self.pass == Pass::Settle {
                    self.settled_entry(object, &expr);
                }
                let idx = self.lower_expr(index)?;
                let (var, mut path) = self.muted_target(object)?;
                path.push(PathStep::Index(idx));
                self.shapes.remove(&var);
                vec![Stmt::SetPath { var, path, expr }]
            }
            S::If { condition, then_body, else_body } => {
                let cond = self.lower_cond(condition)?;
                let mut prelude = std::mem::take(&mut self.pre);
                self.scopes.push(HashSet::new());
                let then = self.lower_block(then_body)?;
                self.scopes.pop();
                let els = match else_body {
                    Some(b) => {
                        self.scopes.push(HashSet::new());
                        let e = self.lower_block(b)?;
                        self.scopes.pop();
                        Some(e)
                    }
                    None => None,
                };
                prelude.push(Stmt::If { cond, then, els });
                prelude
            }
            S::While { condition, body } => {
                let cond = self.lower_cond(condition)?;
                let prelude = std::mem::take(&mut self.pre);
                self.scopes.push(HashSet::new());
                let b = if self.pass == Pass::Scan { self.scan_loop_body(body)? } else { self.lower_block(body)? };
                self.scopes.pop();
                if prelude.is_empty() {
                    vec![Stmt::While { cond, body: b }]
                } else {
                    // a condition that had to be propagated first is
                    // evaluated again before each test, as it is written
                    let mut each = prelude;
                    each.push(Stmt::If {
                        cond: Expr::Unary { op: "not".to_string(), expr: Box::new(cond), ty: Ty::Bool },
                        then: vec![Stmt::Break],
                        els: None,
                    });
                    each.extend(b);
                    vec![Stmt::While { cond: Expr::Const(Const::Bool(true), Ty::Bool), body: each }]
                }
            }
            S::For { var, iter, body } => {
                self.check_reserved(var, "loop variable");
                // a list that can fail is refused like a condition
                let it = self.lower_expr(iter)?;
                let walks = read(&it);
                let it = self.decided(it, Decision::Loop);
                let mut prelude = std::mem::take(&mut self.pre);
                self.scopes.push([var.clone()].into_iter().collect());
                let b = if self.pass != Pass::Final {
                    // inference binds the variable to an item of what the
                    // loop walks, read as text, and leaves it bound
                    self.ctx.scan_loop_var(var, walks);
                    if self.pass == Pass::Scan {
                        self.scan_loop_body(body)?
                    } else {
                        self.lower_block(body)?
                    }
                } else {
                    let saved = self.bind_loop_var(var, &it.ty());
                    let b = self.lower_block(body)?;
                    self.unbind_loop_var(var, saved);
                    b
                };
                self.scopes.pop();
                // in parallel or in order: decided once every function is
                // known (decide_loops)
                prelude.push(Stmt::For { var: var.clone(), iter: it, body: b, parallel: false });
                prelude
            }
            S::Return(None) => {
                if let (Some(seen), 0) = (&mut self.seen, self.collecting) {
                    seen.returns.push(CrowType::Void);
                }
                vec![Stmt::Return(None)]
            }
            S::Return(Some(e)) => {
                let x = self.lower_expr(e)?;
                if let (Some(seen), 0) = (&mut self.seen, self.collecting) {
                    seen.returns.push(full(&x));
                }
                if self.pass == Pass::Scan {
                    self.ctx.note_return_frame(e);
                }
                // an int returned by a function whose value is a float is a
                // float (spec §1), as one assigned to a float variable is: the
                // interpreter handed the caller the int
                let floats = self.pass == Pass::Final
                    && self.ctx.functions.get(&self.current_fn).is_some_and(|sig| match &sig.return_type {
                        CrowType::Result(inner) => matches!(inner.as_ref(), CrowType::Float),
                        t => matches!(t, CrowType::Float),
                    });
                let mut x = if floats && x.ty() == Ty::Int { Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float } } else { x };
                // a handler's value goes to nobody: a helper it returns is
                // called for what it writes, as a statement's is
                if self.at == Where::In(FnKind::Handler) {
                    self.refuse_before_helper(&mut x);
                }
                vec![Stmt::Return(Some(x))]
            }
            S::Break => vec![Stmt::Break],
            S::Continue => vec![Stmt::Continue],
            S::ExprStmt(e) => {
                let mut x = self.lower_expr(e)?;
                if self.pass == Pass::Settle {
                    self.settled_set(e, &x);
                }
                if let Expr::Effect { answered, .. } = &mut x {
                    *answered = false;
                }
                self.refuse_before_helper(&mut x);
                vec![Stmt::Do(x)]
            }
            S::FnDecl { name, .. } => return Err(format!("lowering: a function ('{}') declared inside a body", name)),
            S::StructDecl { name, .. } => return Err(format!("lowering: a struct ('{}') declared inside a body", name)),
            S::Derive { name, .. } => return Err(format!("lowering: a derivation ('{}') inside a body", name)),
            S::TestBlock { .. } => return Err("lowering: a test block inside a body".into()),
            S::Use { path, .. } => return Err(format!("lowering: use '{}' inside a body", path)),
            S::Resource { .. } => vec![],
        })
    }

    /// Several names assigned at once: one value unpacked into them, or a
    /// value each, all read before any is written when they read each other.
    fn multi_assign(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        if values.len() == 1 && names.len() > 1 {
            // one value unpacked into several names
            let e = self.lower_expr(&values[0])?;
            let t = self.fresh();
            let ty = e.ty();
            out.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: e });
            self.scopes.last_mut().unwrap().insert(t.clone());
            for (i, n) in names.iter().enumerate() {
                let item = Expr::Index {
                    base: Box::new(Expr::Local(t.clone(), ty.clone())),
                    indices: vec![Expr::Const(Const::Int(i as i64), Ty::Int)],
                    ty: match &ty {
                        Ty::List(inner) => (**inner).clone(),
                        _ => Ty::Unknown,
                    },
                };
                let stmt = self.assign(n, item);
                self.flush_pre_into(&mut out);
                out.push(stmt);
            }
        } else {
            let targets: BTreeSet<String> = names.iter().cloned().collect();
            let mut reads = BTreeSet::new();
            let mut clock = false;
            for v in values {
                let mut r = BTreeSet::new();
                reads_of_expr(v, &mut r, &mut clock);
                reads.extend(r);
            }
            if reads.is_disjoint(&targets) {
                for (n, v) in names.iter().zip(values) {
                    let e = self.lower_expr(v)?;
                    let stmt = self.assign(n, e);
                    self.flush_pre_into(&mut out);
                    out.push(stmt);
                }
            } else {
                let mut temps = Vec::new();
                for v in values {
                    let e = self.lower_expr(v)?;
                    let t = self.fresh();
                    let ty = e.ty();
                    self.flush_pre_into(&mut out);
                    out.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: e });
                    self.scopes.last_mut().unwrap().insert(t.clone());
                    temps.push((t, ty));
                }
                for (n, (t, ty)) in names.iter().zip(temps) {
                    let stmt = self.assign(n, Expr::Local(t, ty));
                    self.flush_pre_into(&mut out);
                    out.push(stmt);
                }
            }
        }
        Ok(out)
    }

    /// A condition: a fallible one is propagated first (spec §10, Errors
    /// in Conditions), then decided on its payload.
    fn lower_cond(&mut self, c: &A) -> Result<Expr, String> {
        let e = self.lower_expr(c)?;
        Ok(self.decided(e, Decision::Branch))
    }

    /// What a decision reads: a value that can fail is refused first when it
    /// failed (a function hands it back, a handler stops, the top level
    /// halts), and the decision reads its payload.
    fn decided(&mut self, e: Expr, at: Decision) -> Expr {
        if e.ty().is_result() {
            let ty = e.ty();
            let t = match &e {
                Expr::Local(v, _) => v.clone(),
                _ => {
                    let t = self.fresh();
                    self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: e });
                    self.scopes.last_mut().unwrap().insert(t.clone());
                    t
                }
            };
            self.saw_refusal(&at);
            self.pre.push(Stmt::Propagate { values: vec![t.clone()], at });
            return Expr::Convert {
                how: Convert::Unwrap,
                expr: Box::new(Expr::Local(t, ty.clone())),
                ty: ty.payload().clone(),
            };
        }
        e
    }

    fn assign(&mut self, name: &str, expr: Expr) -> Stmt {
        match self.pass {
            Pass::Settle => return self.settled(name, expr),
            Pass::Scan => return self.scanned(name, expr),
            Pass::Final => {}
        }
        let expr = if self.declared(name) && !(self.is_fact(name) && self.sees_facts()) { self.held_by(name, expr) } else { expr };
        // an int assigned to a float variable is a float (spec §1): the
        // conversion is explicit in the form
        let expr = if matches!(self.ctx.vars.get(name), Some(CrowType::Float)) && expr.ty() == Ty::Int {
            Expr::Convert { how: Convert::IntToFloat, expr: Box::new(expr), ty: Ty::Float }
        } else {
            expr
        };
        if self.is_fact(name) && self.sees_facts() {
            return Stmt::Store { fact: name.to_string(), expr };
        }
        if self.declared(name) {
            self.note_shape(name, &expr, false);
            return Stmt::Set { var: name.to_string(), expr };
        }
        self.note_shape(name, &expr, true);
        self.scopes.last_mut().unwrap().insert(name.to_string());
        // what reads the local from here on is typed by the local as
        // declared: its first value's type, or the type it settles on
        let ty = declared_as(&expr.ty(), self.ctx.vars.get(name));
        if !matches!(ty, Ty::Unknown) {
            self.ctx.vars.insert(name.to_string(), crow_type(&ty));
        }
        Stmt::Let { var: name.to_string(), ty, expr }
    }

    /// A value that can fail, assigned to a variable whose first value
    /// could not: the variable holds no error (spec, Reassignment), so a
    /// failed value is refused where it is assigned, as a decision refuses
    /// one, and its payload lands.
    fn held_by(&mut self, name: &str, expr: Expr) -> Expr {
        let payload = match expr.ty() {
            Ty::Result(p) => *p,
            _ => return expr,
        };
        let held = match self.ctx.vars.get(name) {
            Some(t) => t.clone(),
            None => return expr,
        };
        if !crate::type_infer::settle::refuses_failed(&held, &crow_type(&payload)) {
            return expr;
        }
        let at = Decision::Assign(name.to_string());
        // `xs = push[xs, float[s]]`: the list holds no error, so only an item
        // can fail; the item is refused before the call, and the list still
        // grows where it lies
        if let Expr::Call { target: Target::Runtime(path), args, ty: _, propagates } = &expr {
            let own = matches!(args.first(), Some(Expr::Local(v, _)) if v == name);
            if own && in_place_builtin(path) {
                let mut out = vec![args[0].clone()];
                let mut failing = Vec::new();
                for a in &args[1..] {
                    match a {
                        Expr::Convert { how: Convert::Unwrap, expr: inner, ty } if inner.ty().is_result() => {
                            let t = self.fresh();
                            self.pre.push(Stmt::Let { var: t.clone(), ty: inner.ty(), expr: (**inner).clone() });
                            self.scopes.last_mut().unwrap().insert(t.clone());
                            // known good once refused: nothing propagates from it
                            out.push(Expr::Convert { how: Convert::Known, expr: Box::new(Expr::Local(t.clone(), inner.ty())), ty: ty.clone() });
                            failing.push(t);
                        }
                        other => out.push(other.clone()),
                    }
                }
                if !failing.is_empty() {
                    self.saw_refusal(&at);
                    self.pre.push(Stmt::Propagate { values: failing, at });
                    return Expr::Call { target: Target::Runtime(path.clone()), args: out, ty: payload, propagates: *propagates };
                }
            }
        }
        self.decided(expr, at)
    }

    /// An error leaving the body, for a provisional lowering's observer.
    /// A helper called for what it writes — its value dropped — handed a
    /// value that can fail: the value is refused before the call, so a
    /// handler stops with its reason, as it does for an effect given one
    /// (docs/bug-classes.md, decision 1). By auto-propagation the error was
    /// the call's value, which nobody read: the click went on without the
    /// helper's writes and said nothing (and the modes told the click's
    /// own result apart).
    fn refuse_before_helper(&mut self, x: &mut Expr) {
        if !matches!(self.at, Where::In(FnKind::Handler | FnKind::Helper)) {
            return;
        }
        let Expr::Call { target: Target::User(callee), args, propagates: true, .. } = x else {
            return;
        };
        let Some(body) = self.ctx.functions.get(callee.as_str()).map(|sig| sig.body.clone()) else {
            return;
        };
        if !matches!(self.kind_of(callee, &body, false), FnKind::Handler | FnKind::Helper) {
            return;
        }
        let mut failing = Vec::new();
        for a in args.iter_mut() {
            if let Expr::Convert { how: Convert::Unwrap, expr: inner, ty } = a {
                if inner.ty().is_result() {
                    let t = self.fresh();
                    self.pre.push(Stmt::Let { var: t.clone(), ty: inner.ty(), expr: (**inner).clone() });
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(t.clone());
                    }
                    // known good once refused: nothing propagates from it
                    *a = Expr::Convert { how: Convert::Known, expr: Box::new(Expr::Local(t.clone(), inner.ty())), ty: ty.clone() };
                    failing.push(t);
                }
            }
        }
        if !failing.is_empty() {
            self.saw_refusal(&Decision::Branch);
            self.pre.push(Stmt::Propagate { values: failing, at: Decision::Branch });
        }
    }

    fn saw_refusal(&mut self, at: &Decision) {
        let collecting = self.collecting > 0;
        if let Some(seen) = &mut self.seen {
            seen.refusals.push((at.clone(), collecting));
        }
    }

    /// A provisional lowering's assignment (F5): the variable's type
    /// settles with its value's, as inference's walk settles it — a
    /// function's first assignment of a name binds it with the value's type
    /// (spec §7), a later one joins it — and the scopes are kept as the
    /// final lowering keeps them. A failed value's refusal is observed
    /// (`held_by`), the value's own type settling.
    fn settled(&mut self, name: &str, expr: Expr) -> Stmt {
        let ty = full(&expr);
        let store = self.is_fact(name) && self.sees_facts();
        self.note_held(name, &ty, store);
        let settled = if self.ctx.fresh_binding(name) {
            ty
        } else {
            let existing = self.ctx.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
            crate::type_infer::settle::settle(&existing, ty, self.ctx.body_assigned.is_some())
        };
        self.ctx.vars.insert(name.to_string(), settled);
        if !store && !self.declared(name) {
            self.scopes.last_mut().unwrap().insert(name.to_string());
        }
        Stmt::Set { var: name.to_string(), expr }
    }

    /// What a provisional lowering's assignment says of the variable's
    /// declaration: a new local's first value's type is kept, and a value
    /// that can fail, given to a variable declared before, is held to be
    /// judged once the body has settled (`provisional`).
    fn note_held(&mut self, name: &str, ty: &CrowType, store: bool) {
        if store {
            return;
        }
        if self.declared(name) {
            if let CrowType::Result(payload) = ty {
                self.held.push((name.to_string(), payload.as_ref().clone(), self.collecting > 0));
            }
        } else {
            self.first_types.entry(name.to_string()).or_insert_with(|| ty.clone());
        }
    }

    /// A provisional lowering's `a, b = x, y` (F5): each name settles with
    /// its own value's type read as text, as inference's walk pairs them
    /// (one value unpacked settles only the first name, with the whole),
    /// and no name is a new binding there.
    fn settled_multi(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut types = Vec::with_capacity(values.len());
        let mut fulls = Vec::with_capacity(values.len());
        for v in values {
            let x = self.lower_expr(v)?;
            types.push(read(&x));
            fulls.push(full(&x));
        }
        if names.len() == values.len() {
            for (n, t) in names.iter().zip(&fulls) {
                let store = self.is_fact(n) && self.sees_facts();
                self.note_held(n, t, store);
            }
        }
        let in_body = self.ctx.body_assigned.is_some();
        for (n, t) in names.iter().zip(types) {
            let existing = self.ctx.vars.get(n).cloned().unwrap_or(CrowType::Unknown);
            self.ctx.vars.insert(n.clone(), crate::type_infer::settle::settle(&existing, t, in_body));
        }
        for n in names {
            let store = self.is_fact(n) && self.sees_facts();
            if !store && !self.declared(n) {
                self.scopes.last_mut().unwrap().insert(n.clone());
            }
        }
        Ok(vec![])
    }

    /// The call scan's assignment (F5): the variable settled before its
    /// value was lowered (`note_scanned_assign`); the scopes are kept as the
    /// final lowering keeps them.
    fn scanned(&mut self, name: &str, expr: Expr) -> Stmt {
        let store = self.is_fact(name) && self.sees_facts();
        if !store && !self.declared(name) {
            self.scopes.last_mut().unwrap().insert(name.to_string());
        }
        Stmt::Set { var: name.to_string(), expr }
    }

    /// The call scan's `a, b = x, y` (F5): once the values' calls are
    /// read, each name settles with its own value's type read as text, as
    /// the scan pairs them.
    fn scanned_multi(&mut self, names: &[String], values: &[A]) -> Result<Vec<Stmt>, String> {
        let mut types = Vec::with_capacity(values.len());
        for v in values {
            let x = self.lower_expr(v)?;
            types.push(read(&x));
        }
        let in_body = self.ctx.body_assigned.is_some();
        for (n, t) in names.iter().zip(types) {
            let existing = self.ctx.vars.get(n).cloned().unwrap_or(CrowType::Unknown);
            self.ctx.vars.insert(n.clone(), crate::type_infer::settle::settle(&existing, t, in_body));
        }
        for n in names {
            let store = self.is_fact(n) && self.sees_facts();
            if !store && !self.declared(n) {
                self.scopes.last_mut().unwrap().insert(n.clone());
            }
        }
        Ok(vec![])
    }

    /// A provisional lowering's `m[k] = v` (F5): a map's values take the
    /// value's type, as inference's walk has it.
    fn settled_entry(&mut self, object: &A, value: &Expr) {
        if let A::Ident(var) = object {
            if let CrowType::Map(inner) = self.ctx.ident_type(var).text() {
                let joined = inner.unify(&read(value));
                self.ctx.vars.insert(var.clone(), CrowType::Map(Box::new(joined)));
            }
        }
    }

    /// A provisional lowering's `set[m, k, v]` as a statement (F5): the
    /// map's values take v's type, as inference's walk has it.
    fn settled_set(&mut self, e: &A, x: &Expr) {
        let A::Call { callee, args } = e else { return };
        let (true, [A::Ident(var), _, _]) = (matches!(callee.as_ref(), A::Ident(n) if n == "set"), args.as_slice()) else { return };
        let value = match x {
            Expr::Call { args, .. } | Expr::Effect { args, .. } => match args.get(2) {
                // the value as written, before its failure was unwrapped
                Some(Expr::Convert { how: Convert::Unwrap, expr, .. }) => read(expr),
                Some(v) => read(v),
                None => return,
            },
            _ => return,
        };
        let existing = self.ctx.vars.get(var).cloned().unwrap_or(CrowType::Unknown);
        self.ctx.vars.insert(var.clone(), existing.unify(&CrowType::Map(Box::new(value))));
    }

    /// While a loop's body is lowered, its variable has the element type
    /// of what it iterates (inference typed the body in a copy of its
    /// context, so the lowering's context has not seen the variable).
    fn bind_loop_var(&mut self, var: &str, iter_ty: &Ty) -> Option<CrowType> {
        let saved = self.ctx.vars.get(var).cloned();
        let elem = match iter_ty.payload() {
            Ty::List(t) | Ty::Tensor(t) => crow_type(t),
            _ => CrowType::Unknown,
        };
        if !matches!(elem, CrowType::Unknown) {
            self.ctx.vars.insert(var.to_string(), elem);
        }
        saved
    }

    fn unbind_loop_var(&mut self, var: &str, saved: Option<CrowType>) {
        match saved {
            Some(t) => {
                self.ctx.vars.insert(var.to_string(), t);
            }
            None => {
                self.ctx.vars.remove(var);
            }
        }
    }

    fn fresh(&mut self) -> String {
        self.temp += 1;
        format!("t{}", self.temp)
    }

    /// An assignment target's path, its variable and fields already
    /// checked (`check_target`); an index's expressions are checked as they
    /// are lowered.
    fn muted_target(&mut self, object: &A) -> Result<(String, Vec<PathStep>), String> {
        let (v, p, _) = match object {
            A::Ident(_) | A::FieldAccess { .. } => self.muted(|l| l.path_of(object))?,
            _ => self.path_of(object)?,
        };
        Ok((v, p))
    }

    /// An assignment target's variable, its path, and the type of what the
    /// path names, by inference's rules along it (read as text).
    fn path_of(&mut self, object: &A) -> Result<(String, Vec<PathStep>, CrowType), String> {
        match object {
            A::Ident(n) => {
                self.check_ident(n);
                Ok((n.clone(), vec![], self.ctx.ident_type(n).text()))
            }
            A::FieldAccess { object, field } => {
                let (v, mut p, object_type) = self.path_of(object)?;
                self.check_field(&object_type, field);
                let ty = self.ctx.field_type(object, object_type, field).text();
                p.push(PathStep::Field(field.clone()));
                Ok((v, p, ty))
            }
            A::Call { callee, args } => {
                // a name indexed is checked as the call it is written as
                let (v, mut p, callee_type) = match callee.as_ref() {
                    A::Ident(_) => self.muted(|l| l.path_of(callee))?,
                    _ => self.path_of(callee)?,
                };
                let mut arg_types = Vec::with_capacity(args.len());
                let mut types = Vec::with_capacity(args.len());
                let mut checks = Vec::with_capacity(args.len());
                for a in args {
                    let i = self.lower_expr(a)?;
                    arg_types.push(read(&i));
                    types.push(full(&i));
                    checks.push(self.checked(&i));
                    p.push(PathStep::Index(i));
                }
                self.check_callee(callee, args, &checks);
                self.scan_call(callee, args, &types);
                let ty = crow_type(&self.call_type(callee, args, &arg_types, &types, Some(callee_type))).text();
                Ok((v, p, ty))
            }
            _ => Err("lowering: an assignment target must be a variable, a field or an index".into()),
        }
    }

    /// What an assignment's target names, before its value is lowered:
    /// inference's rules along the path, an index typed by a lowering of it
    /// that keeps nothing (its own checks are made when it is lowered).
    fn target_type(&mut self, e: &A) -> CrowType {
        match e {
            A::Ident(n) => self.ctx.ident_type(n).text(),
            A::FieldAccess { object, field } => {
                let object_type = self.target_type(object);
                self.ctx.field_type(object, object_type, field).text()
            }
            _ => {
                let (pre, temp) = (std::mem::take(&mut self.pre), self.temp);
                self.unscanned += 1;
                let t = self.muted(|l| l.lower_expr(e)).map(|x| read(&x)).unwrap_or(CrowType::Unknown);
                self.unscanned -= 1;
                self.pre = pre;
                self.temp = temp;
                t
            }
        }
    }

    // ── expressions ─────────────────────────────────────────────────

    /// An expression lowered and typed: each node is typed from its
    /// lowered children by inference's rules, markup told apart from text (a
    /// view places the one as markup and the other as text, as the
    /// interpreter's values do).
    fn lower_expr(&mut self, e: &A) -> Result<Expr, String> {
        Ok(match e {
            A::Int(n) => Expr::Const(Const::Int(*n), Ty::Int),
            A::Float(f) => Expr::Const(Const::Float(*f), Ty::Float),
            A::Bool(b) => Expr::Const(Const::Bool(*b), Ty::Bool),
            A::Str(s) => {
                if s.contains('`') {
                    Expr::Interp(self.interpolation(s)?)
                } else {
                    Expr::Const(Const::Str(s.clone()), Ty::Str)
                }
            }
            A::Ident(name) => {
                self.check_ident(name);
                let ty = ty_of(&self.ctx.ident_type(name));
                self.ident(name, ty)
            }
            A::BinOp { left, op, right } => {
                let mut l = self.lower_expr(left)?;
                let mut r = self.lower_expr(right)?;
                self.check_binop(op, self.checked(&l), self.checked(&r));
                // a frame's column in arithmetic is its numbers, as it is
                // handed to a function that takes them: converted the same
                // way in both modes, a missing cell the reason it has none
                // (the interpreter refused `df.a + df.b`, the compiled
                // program read a missing cell as 0); a comparison is a
                // mask, whose missing cells are false (spec, Boolean Masks)
                if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow) {
                    let numbers = |x: Expr| match x.ty() {
                        Ty::Column(_) => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: Ty::Tensor(Box::new(Ty::Float)) },
                        _ => x,
                    };
                    l = numbers(l);
                    r = numbers(r);
                }
                self.check_shapes(op, &l, &r);
                let ty = ty_of(&self.ctx.binop_type(op, read(&l), read(&r)));
                let arithmetic = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte);
                if arithmetic {
                    match (l.ty().payload(), r.ty().payload()) {
                        (Ty::Int, Ty::Float) => l = Expr::Convert { how: Convert::IntToFloat, expr: Box::new(l), ty: Ty::Float },
                        (Ty::Float, Ty::Int) => r = Expr::Convert { how: Convert::IntToFloat, expr: Box::new(r), ty: Ty::Float },
                        _ => {}
                    }
                }
                Expr::Binary { op: format!("{:?}", op).to_lowercase(), l: Box::new(l), r: Box::new(r), ty }
            }
            A::UnaryOp { op, expr } => {
                let x = self.lower_expr(expr)?;
                let ty = ty_of(&self.ctx.unary_type(op, read(&x)));
                Expr::Unary { op: format!("{:?}", op).to_lowercase(), expr: Box::new(x), ty }
            }
            A::Call { callee, args } => self.call(callee, args)?,
            A::FieldAccess { object, field } => {
                // `m.x` where `m` is no variable names what a part declares:
                // `m` is not read as one
                let member = matches!(object.as_ref(), A::Ident(m) if self.names_module_member(m, field));
                match field.as_str() {
                    "ok" | "reason" | "kind" | "trace" => {
                        let base = if member { self.muted(|l| l.lower_expr(object))? } else { self.lower_expr(object)? };
                        let which = match field.as_str() {
                            // a frame column's `.ok` is its rows' validity;
                            // any other value's is its own status
                            "ok" if matches!(base.ty().payload(), Ty::Column(_)) => Meta::Valid,
                            "ok" => Meta::Ok,
                            "reason" => Meta::Reason,
                            "kind" => Meta::Kind,
                            _ => Meta::Trace,
                        };
                        Expr::Meta { base: Box::new(base), which }
                    }
                    _ => {
                        if let A::Ident(m) = object.as_ref() {
                            if self.is_module(m) && !self.declared(m) {
                                let ty = ty_of(&self.qualified_type(object, field));
                                return Ok(Expr::Constant(format!("{}.{}", m, field), ty));
                            }
                        }
                        let base = if member { self.muted(|l| l.lower_expr(object))? } else { self.lower_expr(object)? };
                        if !member {
                            self.check_field(&read(&base), field);
                        }
                        let ty = ty_of(&self.ctx.field_type(object, read(&base), field));
                        Expr::Field { base: Box::new(base), name: field.clone(), ty }
                    }
                }
            }
            A::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.lower_expr(i)?);
                }
                let ty = ty_of(&TypeContext::array_type(out.iter().map(full)));
                Expr::ListLit(out, ty)
            }
            A::Map(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    if let A::Ident(n) = k {
                        self.check_ident(n);
                    }
                    let key = match k {
                        A::Ident(n) | A::Str(n) => n.clone(),
                        other => format!("{:?}", other),
                    };
                    out.push((key, self.lower_expr(v)?));
                }
                let ty = ty_of(&TypeContext::map_type(out.iter().map(|(_, v)| read(v))));
                Expr::MapLit(out, ty)
            }
            A::Named { name, .. } => return Err(format!("lowering: a named argument ('{}') survived the lowering of named arguments", name)),
            A::ForExpr { var, iter, body } => {
                // a list that can fail is refused where the loop starts, like
                // a condition: an expression has no statement before it to
                // hold the refusal (a fact's default has none), so both
                // backends refuse a list typed as able to fail right there.
                // What the iterable hoisted waits while the body is lowered,
                // or the body's first statement would take it.
                let it = self.lower_expr(iter)?;
                // a list that can fail is refused where the loop starts (by
                // the backends, not a propagation): an error leaves there
                if it.ty().is_result() {
                    self.saw_refusal(&Decision::Loop);
                }
                let before = std::mem::take(&mut self.pre);
                self.scopes.push([var.clone()].into_iter().collect());
                // inference's walk reads a collecting loop's yield, never its
                // body: a provisional lowering's body leaves nothing behind
                let outer = (self.pass == Pass::Settle).then(|| (self.ctx.vars.clone(), self.ctx.body_assigned.clone(), self.ctx.frame_schemas.clone()));
                // the call scan reads the body with the variable bound as
                // an item, what it assigns left behind
                let fn_list = match iter.as_ref() {
                    A::Array(elems) if !elems.is_empty() && elems.iter().all(|e| matches!(e, A::Ident(n) if self.ctx.functions.contains_key(n))) => {
                        Some(elems.iter().filter_map(|e| if let A::Ident(n) = e { Some(n.clone()) } else { None }).collect::<Vec<String>>())
                    }
                    _ => None,
                };
                let saved = if self.pass == Pass::Scan {
                    self.ctx.scan_loop_var(var, read(&it));
                    None
                } else {
                    Some(self.bind_loop_var(var, &it.ty()))
                };
                let listed = self.pass == Pass::Scan && fn_list.is_some();
                if let (true, Some(fns)) = (listed, fn_list) {
                    self.fn_lists.push((var.clone(), fns));
                }
                self.collecting += 1;
                let mut b = self.lower_block(body)?;
                self.collecting -= 1;
                if listed {
                    self.fn_lists.pop();
                }
                // a list of what the body yields, as the lowering typed it
                let ty = ty_of(&TypeContext::for_type(yield_of(&b)));
                if let Some(saved) = saved {
                    self.unbind_loop_var(var, saved);
                }
                if let Some((vars, assigned, frames)) = outer {
                    self.ctx.vars = vars;
                    self.ctx.body_assigned = assigned;
                    self.ctx.frame_schemas = frames;
                }
                self.scopes.pop();
                let left = std::mem::replace(&mut self.pre, before);
                self.pre.extend(left);
                place_moves_in(&mut b, &HashSet::new(), Some(&[var.clone()].into_iter().collect()));
                // decided with the statement form's rule (decide_loops)
                Expr::ForExpr { var: var.clone(), iter: Box::new(it), body: b, ty, parallel: false }
            }
        })
    }

    fn is_module(&self, name: &str) -> bool {
        self.ctx.bare_modules.contains(name) || self.ctx.module_aliases.contains_key(name) || self.ctx.registry.is_loaded(name)
    }

    fn ident(&mut self, name: &str, ty: Ty) -> Expr {
        if self.declared(name) && !(self.is_fact(name) && self.sees_facts() && !self.scopes.iter().skip(1).any(|s| s.contains(name))) {
            return Expr::Local(name.to_string(), ty);
        }
        if self.is_fact(name) && self.sees_facts() {
            return Expr::Load(name.to_string(), ty);
        }
        if self.ctx.derives.contains(name) {
            return Expr::Derived(name.to_string(), ty);
        }
        if self.ctx.functions.contains_key(name) {
            return Expr::FnRef(name.to_string(), ty);
        }
        if self.ctx.registry.has_constant(name) && !self.declared(name) {
            return Expr::Constant(name.to_string(), ty);
        }
        Expr::Local(name.to_string(), ty)
    }

    fn call(&mut self, callee: &A, args: &[A]) -> Result<Expr, String> {
        // a variable indexed: xs[0], m['k'], r.symbol[0] — except that
        // inside a view an element name is the element (the vocabulary
        // boundary, spec §12): a session named `a` outside does not turn
        // the view's anchors into lookups
        let element_in_view = |n: &str| self.in_ui && self.at == Where::In(FnKind::View) && crate::lower::UI_ELEMENTS.contains(&n);
        let indexes_value = match callee {
            A::Ident(n) => (self.declared(n) || self.is_fact(n) || self.ctx.derives.contains(n)) && !self.ctx.functions.contains_key(n) && !element_in_view(n),
            A::FieldAccess { object, .. } => match object.as_ref() {
                A::Ident(m) => !self.is_module(m) || self.declared(m),
                _ => true,
            },
            _ => true,
        };
        if indexes_value {
            // a name or `m.x` indexed is checked as the call it is written
            // as, after its arguments
            self.unscanned += 1;
            let base = match callee {
                A::Ident(_) => self.muted(|l| l.lower_expr(callee))?,
                A::FieldAccess { object, .. } if matches!(object.as_ref(), A::Ident(_)) => self.muted(|l| l.lower_expr(callee))?,
                _ => self.lower_expr(callee)?,
            };
            self.unscanned -= 1;
            let mut idx = Vec::new();
            for a in args {
                idx.push(self.lower_expr(a)?);
            }
            let arg_types: Vec<CrowType> = idx.iter().map(read).collect();
            let full_types: Vec<CrowType> = idx.iter().map(full).collect();
            self.scan_call(callee, args, &full_types);
            let checks: Vec<CrowType> = idx.iter().map(|x| self.checked(x)).collect();
            self.check_callee(callee, args, &checks);
            let ty = self.call_type(callee, args, &arg_types, &full_types, Some(read(&base)));
            return Ok(Expr::Index { base: Box::new(base), indices: idx, ty });
        }
        let (module, name) = match callee {
            A::Ident(n) => (None, n.clone()),
            A::FieldAccess { object, field } => match object.as_ref() {
                A::Ident(m) => (Some(m.clone()), field.clone()),
                _ => return Err("lowering: a call through an expression".into()),
            },
            _ => return Err("lowering: a call through an expression".into()),
        };
        // a read a guard narrowed (lower.rs): the value, never its failure
        if module.is_none() && name == crate::lower::KNOWN && args.len() == 1 {
            let x = self.lower_expr(&args[0])?;
            self.scan_call(callee, args, &[full(&x)]);
            self.check_call(&name, args, &[self.checked(&x)]);
            if x.ty().is_result() {
                let payload = x.ty().payload().clone();
                return Ok(Expr::Convert { how: Convert::Known, expr: Box::new(x), ty: payload });
            }
            return Ok(x);
        }
        // a struct constructor
        if module.is_none() && self.ctx.structs.contains_key(&name) {
            let declared: Vec<Ty> = self.ctx.structs.get(&name).map(|s| s.fields.iter().map(|(_, t)| ty_of(t)).collect()).unwrap_or_default();
            let mut fields = Vec::new();
            let mut types = Vec::new();
            let mut checks = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let value = self.lower_expr(a)?;
                types.push(full(&value));
                checks.push(self.checked(&value));
                // a list of numbers is held as a list or as a tensor: a
                // field holds the form its record was settled on, so a value
                // in the other is converted (`[h]` beside a list pushed to);
                // an int given a float field is a float, as one handed to a
                // float parameter is (compiled, `P[1, 2]` beside `P[1, 2.5]`
                // did not build, and the interpreter kept the int)
                let how = match (declared.get(i).map(|t| t.payload()), value.ty()) {
                    (Some(Ty::List(_)), Ty::Tensor(_)) => Some(Convert::ToList),
                    (Some(Ty::Tensor(_)), Ty::List(_)) => Some(Convert::ToTensor),
                    (Some(Ty::Float), Ty::Int) if !matches!(value, Expr::Convert { how: Convert::Unwrap, .. }) => Some(Convert::IntToFloat),
                    _ => None,
                };
                fields.push(match how {
                    Some(how) => {
                        let ty = declared[i].payload().clone();
                        Expr::Convert { how, expr: Box::new(value), ty }
                    }
                    None => value,
                });
            }
            self.check_call(&name, args, &checks);
            self.scan_call(callee, args, &types);
            if let Some(seen) = &mut self.seen {
                seen.calls.push((name.clone(), types));
            }
            return Ok(Expr::RecordLit { name, fields });
        }
        // a ui element with its attributes first (lower.rs put them there)
        if module.is_none() && self.in_ui && crate::lower::UI_ELEMENTS.contains(&name.as_str()) {
            let mut attrs = Vec::new();
            let mut children = Vec::new();
            // what the call scan reads of the arguments: the attributes as
            // one map, each held handler call as the call it holds
            let mut types: Vec<CrowType> = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                if i == 0 {
                    if let A::Map(pairs) = a {
                        for (k, v) in pairs {
                            if let A::Ident(n) = k {
                                self.check_ident(n);
                            }
                            let key = match k {
                                A::Ident(n) | A::Str(n) => n.clone(),
                                other => format!("{:?}", other),
                            };
                            // an event attribute holds a handler, or a
                            // handler with its arguments, to call later
                            let is_event = matches!(key.as_str(), "click" | "input" | "submit" | "change" | "seek");
                            if is_event {
                                if let Some(d) = self.deferred(v)? {
                                    attrs.push((key, d));
                                    continue;
                                }
                            }
                            attrs.push((key, self.lower_expr(v)?));
                        }
                        let values: Vec<CrowType> = pairs.iter().zip(&attrs).map(|((_, v), (_, x))| if matches!(x, Expr::Deferred { .. }) { self.held_type(v, x).text() } else { read(x) }).collect();
                        types.push(TypeContext::map_type(values.into_iter()));
                        continue;
                    }
                }
                let child = self.lower_expr(a)?;
                // a response is what a route returns, not something a page
                // shows (the interpreter showed `text[s]` as its text; the
                // compiled page did not build)
                if matches!(child.ty().payload(), Ty::Handle(h) if h == "web.response") {
                    let what = match a {
                        A::Call { callee, .. } => match callee.as_ref() {
                            A::Ident(f) => format!("{}[…]", f),
                            A::FieldAccess { field, .. } => format!("{}[…]", field),
                            _ => "this".to_string(),
                        },
                        _ => "this".to_string(),
                    };
                    self.refuse(format!("{}: {} is a web response, which a route returns — a page shows elements and text: place the text itself, or p[…] or span[…]", name, what));
                }
                types.push(full(&child));
                children.push(child);
            }
            let checks: Vec<CrowType> = types.iter().map(|t| t.text()).collect();
            self.check_call(&name, args, &checks);
            self.scan_call(callee, args, &types);
            return Ok(Expr::Tree { element: name, attrs, children });
        }
        // arguments: a fallible one reaches the callee as its payload, and
        // an error among them is the call's value (`propagates`); a view
        // is the exception, like the elements it is made of: it renders
        // what it is given (spec §10), so its arguments arrive as they are
        let propagates = !(module.is_none() && (matches!(name.as_str(), "log" | "type" | "error" | "assert") || self.ctx.views.contains(&name)));
        let mut lowered = Vec::with_capacity(args.len());
        let mut arg_types = Vec::with_capacity(args.len());
        let mut full_types = Vec::with_capacity(args.len());
        let mut checks = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            // the harness's click[a, h[x]] holds the handler call for the click
            if module.is_none() && name == "click" && i == 1 {
                if let Some(d) = self.deferred(a)? {
                    if let (Some(clicks), Expr::Deferred { name: h, args: held_args }) = (&mut self.clicks, &d) {
                        clicks.push((h.clone(), held_args.iter().map(read).collect()));
                    }
                    // typed as the call it holds
                    let held = self.held_type(a, &d);
                    arg_types.push(held.text());
                    checks.push(held.text());
                    full_types.push(held);
                    lowered.push(d);
                    continue;
                }
            }
            let x = self.lower_expr(a)?;
            arg_types.push(read(&x));
            full_types.push(full(&x));
            checks.push(self.checked(&x));
            if propagates && x.ty().is_result() {
                let rty = x.ty();
                lowered.push(Expr::Convert { how: Convert::Unwrap, expr: Box::new(x), ty: rty.payload().clone() });
            } else {
                lowered.push(x);
            }
        }
        self.check_callee(callee, args, &checks);
        self.scan_call(callee, args, &full_types);
        // a list handed to a `?` is sent as (?, ?, …), which only `IN` reads:
        // finch's `n.symbol = ANY(?)` handed its symbols failed at the
        // database every time (Postgres wants an array there, not a row)
        if matches!(name.as_str(), "query" | "exec") && matches!(module.as_deref(), None | Some("db")) {
            let items = match lowered.get(2) {
                Some(Expr::ListLit(items, _)) => Some(items),
                Some(Expr::Convert { how: Convert::Unwrap, expr, .. }) => match expr.as_ref() {
                    Expr::ListLit(items, _) => Some(items),
                    _ => None,
                },
                _ => None,
            };
            if let (Some(A::Str(sql)), Some(A::Array(_)), Some(items)) = (args.get(1), args.get(2), items) {
                let before = words_before_placeholders(sql);
                for (i, item) in items.iter().enumerate() {
                    let t = read(item);
                    let t = if let CrowType::Result(inner) = t { *inner } else { t };
                    let is_list = matches!(t, CrowType::Array(_) | CrowType::NumArray(_));
                    if is_list && before.get(i).is_some_and(|w| !w.eq_ignore_ascii_case("IN")) {
                        self.refuse(format!("{}: parameter {} is a list, which is sent as (?, ?, …) — only `IN ?` reads one: write `x IN ?`", name, i + 1));
                    }
                }
            }
        }
        // a mask decides each row, and an error decides nothing: a list of
        // values that can fail (a cell of a column that can be missing) is
        // not a mask, in either mode (compiled, it did not build; the
        // interpreter stopped only when a cell was missing)
        if name == "filter" && matches!(module.as_deref(), None | Some("frame")) {
            if let Some(Ty::List(item)) = lowered.get(1).map(|m| m.ty()) {
                if item.is_result() {
                    let what = match args.get(1) {
                        Some(A::Ident(v)) => format!("'{}'", crate::lower::shown(v)),
                        _ => "the mask".to_string(),
                    };
                    self.refuse(format!("filter: {} holds values that can fail — a cell of a column that can be missing — and a mask decides every row: fill the column first (fill[frame, 'column', value]) or guard each value with .ok", what));
                }
            }
        }
        let ty = self.call_type(callee, args, &arg_types, &full_types, None);
        let current = self.current_fn.clone();
        if let Some(m) = module {
            let m = self.ctx.module_aliases.get(&m).cloned().unwrap_or(m);
            if is_effect(&name) {
                // a qualified effect keeps its module: frame.load, db.query
                if self.pass == Pass::Final {
                    self.direct_effects.entry(current).or_default().insert(name.clone());
                }
                return Ok(Expr::Effect { name: format!("{}.{}", m, name), args: lowered, ty, answered: true, propagates });
            }
            // a count no row of the name takes is refused before either
            // mode runs (the interpreter stopped, the compiled program did
            // not build)
            if let Some(e) = crate::table::miscounted(&m, &name, lowered.len()) {
                self.refuse(e);
            }
            let lowered = self.row_arguments(&m, &name, lowered);
            return Ok(Expr::Call { target: Target::Module(m, name), args: lowered, ty, propagates });
        }
        if self.ctx.functions.contains_key(&name) || self.ctx.specializations.contains_key(&name) {
            if let Some(seen) = &mut self.seen {
                seen.calls.push((name.clone(), full_types.clone()));
            }
            // a render reads state and never moves it (spec, Reactive State):
            // the interpreter wrote the fact mid-render, and the compiled
            // program did not build
            if self.pass == Pass::Final && matches!(self.at, Where::In(FnKind::View) | Where::Derive) {
                if let Some(body) = self.ctx.functions.get(&name).map(|sig| sig.body.clone()) {
                    if matches!(self.kind_of(&name, &body, false), FnKind::Handler | FnKind::Helper) {
                        let from = if self.at == Where::Derive { "a derivation" } else { "a view" };
                        self.refuse(format!(
                            "{} calls '{}', which assigns reactive state — {} reads state and never changes it: move the assignment into a handler, or give the local a name that is not a fact's",
                            from, name, from
                        ));
                    }
                }
            }
            let resolved = self.specialised(&name, &full_types);
            // an int handed to a float parameter is a float (spec §1), as one
            // assigned to a float variable is: the conversion is explicit
            // — and numbers held as a tensor handed to a parameter that holds
            // them as a list, or the other way, are converted where they meet
            // (a literal handed where a call elsewhere made the parameter a
            // list did not build)
            if let Some(sig) = self.ctx.functions.get(&resolved) {
                let params: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                lowered = lowered
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| match (params.get(i), x.ty()) {
                        (Some(CrowType::Float), Ty::Int) => Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float },
                        (Some(p @ CrowType::Array(_)), Ty::Tensor(_)) => Expr::Convert { how: Convert::ToList, expr: Box::new(x), ty: ty_of(p) },
                        // (an empty literal, whose items are not known, is
                        // any list the backends need: parity 71)
                        (Some(p @ CrowType::NumArray(_)), Ty::List(e)) if *e != Ty::Unknown => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: ty_of(p) },
                        _ => x,
                    })
                    .collect();
            }
            if self.pass == Pass::Final {
                self.calls.entry(current).or_default().insert(resolved.clone());
            }
            return Ok(Expr::Call { target: Target::User(resolved), args: lowered, ty, propagates });
        }
        if is_effect(&name) {
            if self.pass == Pass::Final {
                self.direct_effects.entry(current).or_default().insert(name.clone());
            }
            return Ok(Expr::Effect { name, args: lowered, ty, answered: true, propagates });
        }
        if RUNTIME_TABLE.iter().any(|(b, _)| *b == name) {
            // a list of numbers is held as a list or as a tensor: `concat`
            // joins them in the first's form, so the second is converted
            if name == "concat" && lowered.len() >= 2 {
                let how = match (lowered[0].ty().payload(), lowered[1].ty().payload(), ty.payload()) {
                    (Ty::List(_), Ty::Tensor(_), Ty::List(_)) => Some(Convert::ToList),
                    (Ty::Tensor(_), Ty::List(_), Ty::Tensor(_)) => Some(Convert::ToTensor),
                    _ => None,
                };
                if let Some(how) = how {
                    let second = lowered.remove(1);
                    lowered.insert(1, Expr::Convert { how, expr: Box::new(second), ty: ty.payload().clone() });
                }
            }
            let lowered = self.core_row_arguments(&name, lowered);
            return Ok(Expr::Call { target: Target::Runtime(runtime_name(&name)), args: lowered, ty, propagates });
        }
        // a bare name a loaded module provides resolves at run time, by the
        // registry's own rule, as the interpreter resolves it
        let lowered = match self.ctx.registry.module_name_for(&name).map(str::to_string) {
            Some(m) => {
                // a name only a module answers — not the core's, not an
                // element's — reaches it, so a count none of its rows take
                // is refused as a qualified call's is
                if !CORE_RESERVED.contains(&name.as_str()) && !crate::lower::UI_ELEMENTS.contains(&name.as_str()) {
                    if let Some(e) = crate::table::miscounted(&m, &name, lowered.len()) {
                        self.refuse(e);
                    }
                }
                self.row_arguments(&m, &name, lowered)
            }
            None => self.core_row_arguments(&name, lowered),
        };
        Ok(Expr::Call { target: Target::Runtime(name), args: lowered, ty, propagates })
    }

    /// A builtin row's arguments as its parameters take them (docs/
    /// bug-classes.md §4): numbers held as a list, or a frame's column,
    /// handed to a parameter that takes a tensor are converted where they
    /// meet, the same way in both modes — compiled, a list did not build
    /// and a column's missing cells were read as 0, where the interpreter
    /// read the list and stopped at the missing cell. A name the core also
    /// answers (`max`, `abs`) picks its builtin by the values at run time —
    /// a list goes to the core's, a tensor to the module's — so a list
    /// handed to it is left as it is, and a column, numbers either way, is
    /// converted. An argument that can fail is handed on as its error.
    /// An empty list literal handed to a row's list parameter is a list of
    /// what the parameter takes: its items' type is not known from the
    /// literal, and the compiled program must name it (`join[[], ',']` did
    /// not build).
    fn typed_empties(kinds: &[crate::table::Kind], lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::Kind;
        lowered
            .into_iter()
            .zip(kinds.iter().copied().map(Some).chain(std::iter::repeat(None)))
            .map(|(x, k)| {
                let item = match k {
                    Some(Kind::Words | Kind::Strs | Kind::Texts) => Ty::Str,
                    Some(Kind::Ints) => Ty::Int,
                    Some(Kind::Floats) => Ty::Float,
                    Some(Kind::Bools) => Ty::Bool,
                    Some(Kind::Tensors) => Ty::Tensor(Box::new(Ty::Float)),
                    Some(Kind::Tensor) => Ty::Float,
                    _ => return x,
                };
                match x {
                    Expr::ListLit(items, Ty::List(e)) if items.is_empty() && *e == Ty::Unknown => {
                        let list = Expr::ListLit(items, Ty::List(Box::new(item)));
                        if k == Some(Kind::Tensor) {
                            Expr::Convert { how: Convert::ToTensor, expr: Box::new(list), ty: Ty::Tensor(Box::new(Ty::Float)) }
                        } else {
                            list
                        }
                    }
                    other => other,
                }
            })
            .collect()
    }

    /// A core builtin's row's arguments, as a module row's are given theirs:
    /// an empty list literal is typed by the parameter it is handed to, and
    /// numbers held as a list handed to a tensor's parameter are converted
    /// (`push[m, range[3]]`). An int kept in or compared with a collection
    /// of floats is a float, as one assigned to a float variable is
    /// (compiled, `push[xs, 2]` onto floats did not build).
    fn core_row_arguments(&self, name: &str, lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::Kind;
        let args: Vec<crate::table::Arg> = lowered.iter().map(|x| crate::table::Arg::of_type(&read(x))).collect();
        let Some(kinds) = crate::table::core_row(name, &args).and_then(|row| crate::table::kinds_of(row.params, lowered.len())) else {
            return lowered;
        };
        let lowered = Self::typed_empties(&kinds, lowered);
        let floats = kinds.iter().zip(&lowered).any(|(k, x)| match (k, x.ty().payload()) {
            (Kind::List | Kind::Seq, Ty::List(e)) | (Kind::MapOf | Kind::MapRef, Ty::Map(e)) => **e == Ty::Float,
            _ => false,
        });
        lowered
            .into_iter()
            .zip(kinds)
            .map(|(x, k)| match (k, x.ty()) {
                (Kind::Tensor, Ty::List(e)) if matches!(*e, Ty::Int | Ty::Float) => Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: Ty::Tensor(Box::new(Ty::Float)) },
                // (a plain int: one that can fail is handed on as its error,
                // through the mark the backends read)
                (Kind::Item | Kind::ItemRef, Ty::Int) if floats && !matches!(x, Expr::Convert { how: Convert::Unwrap, .. }) => {
                    Expr::Convert { how: Convert::IntToFloat, expr: Box::new(x), ty: Ty::Float }
                }
                _ => x,
            })
            .collect()
    }

    fn row_arguments(&self, module: &str, name: &str, lowered: Vec<Expr>) -> Vec<Expr> {
        use crate::table::{self, Arg, Kind};
        let core = CORE_RESERVED.contains(&name);
        let args: Vec<Arg> = lowered.iter().map(|x| Arg::of_type(&read(x))).collect();
        let Some(kinds) = table::module(module, name, &args).and_then(|row| table::kinds_of(row.params, lowered.len())) else {
            return lowered;
        };
        let lowered = Self::typed_empties(&kinds, lowered);
        lowered
            .into_iter()
            .zip(kinds)
            .map(|(x, k)| {
                let numbers = match x.ty() {
                    _ if matches!(x, Expr::Convert { how: Convert::Unwrap, .. }) => false,
                    Ty::List(e) => !core && *e != Ty::Unknown && !e.is_result(),
                    Ty::Column(_) => true,
                    _ => false,
                };
                if k == Kind::Tensor && numbers {
                    Expr::Convert { how: Convert::ToTensor, expr: Box::new(x), ty: Ty::Tensor(Box::new(Ty::Float)) }
                } else {
                    x
                }
            })
            .collect()
    }

    /// A call's type by inference's rules, from its arguments' types (each
    /// as lowered, before a failure among them is unwrapped) and, when the
    /// callee was lowered as a value, the callee's. The rule is picked by the
    /// callee's form, as the walk picks it.
    fn call_type(&self, callee: &A, args: &[A], arg_types: &[CrowType], full_types: &[CrowType], callee_type: Option<CrowType>) -> Ty {
        // a cell of a column whose type the database told us: frame.col[i]
        if let A::FieldAccess { object, field } = callee {
            if let (Some(t), [index]) = (self.ctx.column_type(object, field), arg_types) {
                if !matches!(index, CrowType::NumArray(_) | CrowType::Array(_)) {
                    return ty_of(&t);
                }
            }
        }
        let t = match callee {
            A::Ident(name) => self.ctx.named_call_type(name, args, callee, arg_types, full_types),
            _ => {
                let module = match callee {
                    A::FieldAccess { object, field } => match object.as_ref() {
                        A::Ident(m) => self.ctx.module_call_type(m, field, arg_types),
                        _ => None,
                    },
                    _ => None,
                };
                match module {
                    Some(t) => t,
                    None => {
                        // indexing, or a function value applied: the callee
                        // as lowered, or a module's value it names (m.f[x])
                        let callee_type = match (callee_type, callee) {
                            (Some(t), _) => t,
                            (None, A::FieldAccess { object, field }) => self.qualified_type(object, field),
                            (None, _) => CrowType::Unknown,
                        };
                        self.ctx.index_type(callee_type, arg_types.first().cloned())
                    }
                }
            }
        };
        ty_of(&t)
    }

    /// A call's own checks, made after its arguments': a name called is
    /// checked as a call by name, `m.f[…]` as a module's.
    fn check_callee(&mut self, callee: &A, args: &[A], types: &[CrowType]) {
        match callee {
            A::Ident(n) => self.check_call(n, args, types),
            A::FieldAccess { object, field } => {
                if let A::Ident(m) = object.as_ref() {
                    self.check_module_call(m, field);
                }
            }
            _ => {}
        }
    }

    /// The type of a value a module names, `m.x`.
    fn qualified_type(&self, object: &A, field: &str) -> CrowType {
        let object_type = match object {
            A::Ident(m) => self.ctx.ident_type(m).text(),
            _ => CrowType::Unknown,
        };
        self.ctx.field_type(object, object_type, field)
    }

    /// A user function called with arguments, as a call held for later,
    /// when the expression is one.
    fn deferred(&mut self, e: &A) -> Result<Option<Expr>, String> {
        if let A::Call { callee, args } = e {
            if let A::Ident(f) = callee.as_ref() {
                if self.ctx.functions.contains_key(f) && !self.declared(f) {
                    let mut lowered = Vec::with_capacity(args.len());
                    for a in args {
                        lowered.push(self.lower_expr(a)?);
                    }
                    let checks: Vec<CrowType> = lowered.iter().map(|x| self.checked(x)).collect();
                    self.check_call(f, args, &checks);
                    let types: Vec<CrowType> = lowered.iter().map(full).collect();
                    self.scan_call(callee, args, &types);
                    return Ok(Some(Expr::Deferred { name: f.clone(), args: lowered }));
                }
            }
        }
        Ok(None)
    }

    /// The type of the call a deferred call holds (`h[x]` in `click[a,
    /// h[x]]`), from its lowered arguments, markup told apart.
    fn held_type(&self, a: &A, d: &Expr) -> CrowType {
        match (a, d) {
            (A::Call { callee, args }, Expr::Deferred { args: lowered, .. }) => {
                let arg_types: Vec<CrowType> = lowered.iter().map(read).collect();
                let full_types: Vec<CrowType> = lowered.iter().map(full).collect();
                match self.call_type(callee, args, &arg_types, &full_types, None) {
                    Ty::Node => CrowType::Markup,
                    t => crow_type(&t),
                }
            }
            _ => CrowType::Unknown,
        }
    }

    /// The specialised name of a user function called at these argument
    /// types, when it has one.
    fn specialised(&self, name: &str, full_types: &[CrowType]) -> String {
        self.ctx.specialization_for(name, full_types).unwrap_or_else(|| name.to_string())
    }

    /// An interpolated string's pieces, each expression parsed as the
    /// interpreter parses it.
    fn interpolation(&mut self, s: &str) -> Result<Vec<Piece>, String> {
        let chars: Vec<char> = s.chars().collect();
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                text.push('`');
                i += 2;
                continue;
            }
            if chars[i] == '`' {
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() {
                            i += 2;
                            continue;
                        }
                        if c == '\'' {
                            in_str = false;
                        }
                    } else if c == '\'' {
                        in_str = true;
                    } else if c == '`' {
                        break;
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated interpolation — close it with ` or write \\` for a literal backtick".to_string());
                }
                let src: String = chars[start..i].iter().collect();
                i += 1;
                if src.trim().is_empty() {
                    return Err("empty interpolation — backticks must contain an expression".to_string());
                }
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                let mut lexer = crate::lexer::Lexer::new(&src);
                let tokens = lexer.tokenize()?;
                let mut parser = crate::parser::Parser::new(tokens);
                let stmts = parser.parse()?;
                let stmts = crate::lower::lower_fragment(&stmts).unwrap_or(stmts);
                let expr = stmts.iter().find_map(|st| if let S::ExprStmt(e) = st { Some(e.clone()) } else { None });
                match expr {
                    Some(e) => pieces.push(Piece::Expr(self.lower_expr(&e)?)),
                    None => return Err(format!("interpolation is not an expression: `{}`", src)),
                }
            } else {
                text.push(chars[i]);
                i += 1;
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Ok(pieces)
    }
}

// ── inference by lowering (F5) ───────────────────────────────────────

/// Signatures found by lowering bodies (F5, core-rewrite-notes §10):
/// inference's rounds, where each round's call scan lowers every body in
/// order, each call noting its arguments' types for its callee, and each
/// function's locals are what a provisional lowering of its body leaves.
/// A body's return type and whether an error leaves it are still decided
/// from that environment (`finish_body`).
pub(crate) fn infer_by_lowering(ctx: &mut TypeContext, stmts: &[S]) {
    let in_ui = stmts.iter().any(|s| matches!(s, S::Use { path, .. } if path == "ui" || path == "web.ui"));
    ctx.begin_inference(stmts);
    // five rounds at least, as ever; then more while a signature still
    // moves — a chain of derivations learns one link a round
    for round in 0..16 {
        let before = ctx.signatures();
        ctx.sync_derive_types();
        ctx.begin_scan();
        Lowerer::for_inference(ctx, in_ui, stmts).scan_program(stmts);
        let frames_moved = ctx.end_scan();
        // types from indirect calls back to the functions named
        ctx.propagate_indirect_fn_types(stmts);
        infer_bodies(ctx, in_ui, stmts);
        if round >= 4 && before == ctx.signatures() && !frames_moved {
            break;
        }
    }
    // what the last round learned, where the top level reads it
    ctx.sync_derive_types();
    // what only a test's click types, last, and the bodies again
    if Lowerer::for_inference(ctx, in_ui, stmts).type_clicks(stmts) {
        infer_bodies(ctx, in_ui, stmts);
    }
    // F2: a function called at several signatures is one per signature
    Lowerer::for_inference(ctx, in_ui, stmts).specialize();
    ctx.sync_derive_types();
    Lowerer::for_inference(ctx, in_ui, stmts).settle_top(stmts);
}

/// Every function's body typed, in name order (`Lowerer::infer_body`).
fn infer_bodies(ctx: &mut TypeContext, in_ui: bool, stmts: &[S]) {
    let fns: Vec<(String, Vec<S>)> = ctx.functions.iter().map(|(n, sig)| (n.clone(), sig.body.clone())).collect();
    let mut l = Lowerer::for_inference(ctx, in_ui, stmts);
    for (name, body) in &fns {
        l.infer_body(name, body);
    }
}

// ── analyses over the parser's tree ─────────────────────────────────

/// The word before each `?` of a statement, outside quotes, an opening
/// parenthesis passed over (`ANY(?)` is `ANY`): where a list parameter
/// may go (only after `IN`).
pub(crate) fn words_before_placeholders(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '?' => {
                let mut j = i;
                while j > 0 && (chars[j - 1].is_whitespace() || chars[j - 1] == '(') {
                    j -= 1;
                }
                let end = j;
                while j > 0 && (chars[j - 1].is_ascii_alphanumeric() || chars[j - 1] == '_') {
                    j -= 1;
                }
                out.push(chars[j..end].iter().collect());
            }
            None => {}
        }
    }
    out
}

/// The type a local is declared with, from its first value's type and the
/// type inference settled on across its assignments: the first value's (a
/// node's value is its markup, an int given to a float variable is a
/// float, spec §1), unless it is not known yet, when the settled type is
/// taken — its payload when the first value cannot fail, which makes a
/// variable that holds no error (spec, Reassignment), as the Rust backend
/// declares it (`xs = []`, then a push of a value that can fail, refused
/// where it is assigned).
fn declared_as(first: &Ty, settled: Option<&CrowType>) -> Ty {
    if matches!(first, Ty::Node) {
        return Ty::Markup;
    }
    if *first == Ty::Int && matches!(settled, Some(CrowType::Float)) {
        return Ty::Float;
    }
    if !format!("{:?}", first).contains("Unknown") {
        return first.clone();
    }
    match settled {
        Some(t) if !matches!(t, CrowType::Unknown) => {
            let settled = ty_of(t);
            if first.is_result() {
                settled
            } else {
                settled.payload().clone()
            }
        }
        _ => first.clone(),
    }
}

/// What a collecting loop's lowered body yields: its last expression, or
/// what the branches of a trailing `if` yield (the filter pattern);
/// unknown when the body ends otherwise.
fn yield_of(body: &Block) -> CrowType {
    match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
        Some(Stmt::Do(x)) => full(x),
        Some(Stmt::If { then, els, .. }) => {
            let t = yield_of(then);
            match els {
                Some(e) => t.unify(&yield_of(e)),
                None => t,
            }
        }
        _ => CrowType::Unknown,
    }
}

/// The facts a function can write: a part's function any part's, whose
/// names carry that part's prefix — an unprefixed name there is a local.
fn own_facts(f: &str, facts: &BTreeSet<String>) -> BTreeSet<String> {
    match crate::part_prefix_of_declaration(f) {
        // any part's (a part writes one it uses), never the entry's
        Some(_) => facts.iter().filter(|m| crate::part_prefix_of_declaration(m).is_some()).cloned().collect(),
        None => facts.clone(),
    }
}

/// The loop variables of a body, at any depth.
fn loop_vars(body: &[S], out: &mut HashSet<String>) {
    for s in body {
        match s {
            S::For { var, body, .. } => {
                out.insert(var.clone());
                loop_vars(body, out);
            }
            S::While { body, .. } => loop_vars(body, out),
            S::If { then_body, else_body, .. } => {
                loop_vars(then_body, out);
                if let Some(b) = else_body {
                    loop_vars(b, out);
                }
            }
            _ => {}
        }
    }
}

fn assigns_any(body: &[S], names: &BTreeSet<String>) -> bool {
    body.iter().any(|s| match s {
        S::Assign { name, .. } => names.contains(name),
        S::MultiAssign { names: ns, .. } => ns.iter().any(|n| names.contains(n)),
        S::FieldAssign { object, .. } | S::IndexAssign { object, .. } => root_ident(object).map_or(false, |n| names.contains(n)),
        S::If { then_body, else_body, .. } => assigns_any(then_body, names) || else_body.as_ref().map_or(false, |b| assigns_any(b, names)),
        S::For { body, .. } | S::While { body, .. } => assigns_any(body, names),
        _ => false,
    })
}


fn root_ident(e: &A) -> Option<&str> {
    match e {
        A::Ident(n) => Some(n),
        A::FieldAccess { object, .. } => root_ident(object),
        A::Call { callee, .. } => root_ident(callee),
        _ => None,
    }
}

/// The facts any function's body writes: those are per session.
/// The variable a field or index assignment writes into: `a` of `a.b[0]`.
fn target_root(object: &A) -> Option<String> {
    match object {
        A::Ident(n) => Some(n.clone()),
        A::FieldAccess { object, .. } => target_root(object),
        A::Call { callee, .. } => target_root(callee),
        _ => None,
    }
}

fn facts_written_by_fns(stmts: &[S], facts: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(body: &[S], facts: &BTreeSet<String>, out: &mut BTreeSet<String>) {
        for s in body {
            match s {
                S::Assign { name, .. } if facts.contains(name) => {
                    out.insert(name.clone());
                }
                S::MultiAssign { names, .. } => {
                    for n in names {
                        if facts.contains(n) {
                            out.insert(n.clone());
                        }
                    }
                }
                S::FieldAssign { object, .. } | S::IndexAssign { object, .. } => {
                    if let Some(n) = root_ident(object) {
                        if facts.contains(n) {
                            out.insert(n.to_string());
                        }
                    }
                }
                S::If { then_body, else_body, .. } => {
                    walk(then_body, facts, out);
                    if let Some(b) = else_body {
                        walk(b, facts, out);
                    }
                }
                S::For { body, .. } | S::While { body, .. } => walk(body, facts, out),
                _ => {}
            }
        }
    }
    for s in stmts {
        if let S::FnDecl { body, .. } = s {
            walk(body, facts, &mut out);
        }
    }
    out
}

fn reads_of_expr(e: &A, names: &mut BTreeSet<String>, clock: &mut bool) {
    reads_of_stmts(&[S::ExprStmt(e.clone())], names, clock);
}

fn reads_of_stmts(body: &[S], names: &mut BTreeSet<String>, clock: &mut bool) {
    fn in_expr(e: &A, names: &mut BTreeSet<String>, clock: &mut bool) {
        match e {
            A::Ident(n) => {
                names.insert(n.clone());
            }
            A::Call { callee, args } => {
                if let A::Ident(n) = callee.as_ref() {
                    if n == "now" {
                        *clock = true;
                    }
                    names.insert(n.clone());
                } else {
                    in_expr(callee, names, clock);
                }
                for a in args {
                    in_expr(a, names, clock);
                }
            }
            A::BinOp { left, right, .. } => {
                in_expr(left, names, clock);
                in_expr(right, names, clock);
            }
            A::UnaryOp { expr, .. } => in_expr(expr, names, clock),
            A::FieldAccess { object, .. } => in_expr(object, names, clock),
            A::Array(items) => items.iter().for_each(|i| in_expr(i, names, clock)),
            A::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                in_expr(k, names, clock);
                in_expr(v, names, clock);
            }),
            A::Named { value, .. } => in_expr(value, names, clock),
            A::ForExpr { iter, body, .. } => {
                in_expr(iter, names, clock);
                reads_of_stmts(body, names, clock);
            }
            A::Str(s) => {
                for n in crate::parallel::str_interpolation_reads(s) {
                    names.insert(n);
                }
                if s.contains("now[") {
                    *clock = true;
                }
            }
            _ => {}
        }
    }
    for s in body {
        match s {
            S::Assign { value, .. } => in_expr(value, names, clock),
            S::MultiAssign { values, .. } => values.iter().for_each(|v| in_expr(v, names, clock)),
            S::FieldAssign { value, .. } => in_expr(value, names, clock),
            S::IndexAssign { index, value, .. } => {
                in_expr(index, names, clock);
                in_expr(value, names, clock);
            }
            S::ExprStmt(e) | S::Return(Some(e)) => in_expr(e, names, clock),
            S::If { condition, then_body, else_body } => {
                in_expr(condition, names, clock);
                reads_of_stmts(then_body, names, clock);
                if let Some(b) = else_body {
                    reads_of_stmts(b, names, clock);
                }
            }
            S::For { iter, body, .. } => {
                in_expr(iter, names, clock);
                reads_of_stmts(body, names, clock);
            }
            S::While { condition, body } => {
                in_expr(condition, names, clock);
                reads_of_stmts(body, names, clock);
            }
            _ => {}
        }
    }
}

// ── moves: a local's last read ──────────────────────────────────────

/// Turn the last read of each local into a move. `live_after` are the
/// locals read after this block; `allowed` restricts moves to these
/// locals (inside a loop body, the ones the body itself defines, since
/// the body repeats), `None` allows every local the block defines.
fn place_moves_in(block: &mut Block, live_after: &HashSet<String>, allowed: Option<&HashSet<String>>) {
    let defined: HashSet<String> = defined_in(block);
    let mut allowed_set: HashSet<String> = defined.clone();
    if let Some(a) = allowed {
        allowed_set.extend(a.iter().cloned());
    }
    let mut live: HashSet<String> = live_after.clone();
    for s in block.iter_mut().rev() {
        let mut reads = HashSet::new();
        stmt_reads(s, &mut reads);
        // nested blocks first, with what is live after them
        match s {
            Stmt::If { then, els, .. } => {
                let after = live.clone();
                place_moves_in(then, &after, Some(&allowed_set));
                if let Some(e) = els {
                    place_moves_in(e, &after, Some(&allowed_set));
                }
            }
            Stmt::While { body, .. } => {
                // the body repeats: nothing defined outside it moves inside
                let mut body_reads = HashSet::new();
                for b in body.iter() {
                    stmt_reads(b, &mut body_reads);
                }
                let mut after = live.clone();
                after.extend(body_reads);
                let inner_allowed = defined_in(body);
                place_moves_in(body, &after, Some(&inner_allowed));
            }
            Stmt::For { var, body, .. } => {
                let mut body_reads = HashSet::new();
                for b in body.iter() {
                    stmt_reads(b, &mut body_reads);
                }
                let mut after = live.clone();
                after.extend(body_reads);
                let mut inner_allowed = defined_in(body);
                inner_allowed.insert(var.clone());
                place_moves_in(body, &after, Some(&inner_allowed));
            }
            _ => {}
        }
        // this statement's own expressions, last read first; `x = f[x]`
        // redefines x before anyone reads it again, so the old x moves
        // even when the statement sits in a loop; the head of an if, a
        // while or a for is read before its blocks, so what they read is
        // still to come
        let mut seen: HashSet<String> = HashSet::new();
        let (live_here, allowed_here) = match s {
            Stmt::Set { var, .. } => {
                let mut l = live.clone();
                l.remove(var);
                let mut a = allowed_set.clone();
                a.insert(var.clone());
                (l, a)
            }
            Stmt::If { then, els, .. } => {
                let mut l = live.clone();
                for b in then.iter() {
                    stmt_reads(b, &mut l);
                }
                if let Some(e) = els {
                    for b in e.iter() {
                        stmt_reads(b, &mut l);
                    }
                }
                (l, allowed_set.clone())
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                let mut l = live.clone();
                for b in body.iter() {
                    stmt_reads(b, &mut l);
                }
                (l, allowed_set.clone())
            }
            _ => (live.clone(), allowed_set.clone()),
        };
        for e in stmt_exprs_mut(s).into_iter().rev() {
            move_last_reads(e, &live_here, &allowed_here, &mut seen);
        }
        match s {
            Stmt::Let { var, .. } | Stmt::Set { var, .. } => {
                live.remove(var);
            }
            _ => {}
        }
        live.extend(reads);
    }
}

fn defined_in(block: &Block) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in block {
        if let Stmt::Let { var, .. } = s {
            out.insert(var.clone());
        }
    }
    out
}

/// The top-level expressions of a statement, in evaluation order.
fn stmt_exprs_mut(s: &mut Stmt) -> Vec<&mut Expr> {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => vec![expr],
        Stmt::SetPath { path, expr, .. } => {
            let mut v: Vec<&mut Expr> = Vec::new();
            for p in path.iter_mut() {
                if let PathStep::Index(i) = p {
                    v.push(i);
                }
            }
            v.push(expr);
            v
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => vec![cond],
        Stmt::For { iter, .. } => vec![iter],
        Stmt::Return(Some(e)) => vec![e],
        _ => vec![],
    }
}

/// Walk an expression in reverse evaluation order; the first `Local`
/// met for a variable is its last read.
fn move_last_reads(e: &mut Expr, live: &HashSet<String>, allowed: &HashSet<String>, seen: &mut HashSet<String>) {
    match e {
        Expr::Local(v, ty) => {
            if movable(ty) && !live.contains(v) && allowed.contains(v) && !seen.contains(v) {
                let name = v.clone();
                let t = ty.clone();
                *e = Expr::Move(name, t);
            }
            if let Expr::Move(v, _) | Expr::Local(v, _) = e {
                seen.insert(v.clone());
            }
        }
        Expr::Call { args, .. } | Expr::Effect { args, .. } => {
            for a in args.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => move_last_reads(expr, live, allowed, seen),
        Expr::Binary { l, r, .. } => {
            move_last_reads(r, live, allowed, seen);
            move_last_reads(l, live, allowed, seen);
        }
        Expr::Index { base, indices, .. } => {
            for i in indices.iter_mut().rev() {
                move_last_reads(i, live, allowed, seen);
            }
            // an indexed base is borrowed, never moved
            note_reads(base, seen);
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => note_reads(base, seen),
        Expr::Interp(pieces) => {
            for p in pieces.iter_mut().rev() {
                if let Piece::Expr(x) = p {
                    move_last_reads(x, live, allowed, seen);
                }
            }
        }
        Expr::ListLit(items, _) => {
            for i in items.iter_mut().rev() {
                move_last_reads(i, live, allowed, seen);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (_, v) in pairs.iter_mut().rev() {
                move_last_reads(v, live, allowed, seen);
            }
        }
        Expr::RecordLit { fields, .. } => {
            for f in fields.iter_mut().rev() {
                move_last_reads(f, live, allowed, seen);
            }
        }
        Expr::Tree { attrs, children, .. } => {
            for c in children.iter_mut().rev() {
                move_last_reads(c, live, allowed, seen);
            }
            for (_, a) in attrs.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            let mut body_reads = HashSet::new();
            for b in body.iter() {
                stmt_reads(b, &mut body_reads);
            }
            seen.extend(body_reads);
            move_last_reads(iter, live, allowed, seen);
        }
        Expr::Deferred { args, .. } => {
            for a in args.iter_mut().rev() {
                move_last_reads(a, live, allowed, seen);
            }
        }
        _ => {}
    }
}

/// A value worth moving: anything that owns memory. A scalar copies.
fn movable(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::Float | Ty::Bool | Ty::Unit | Ty::Fn(_, _) | Ty::Handle(_) => false,
        Ty::Result(inner) => movable(inner),
        _ => true,
    }
}

fn note_reads(e: &Expr, seen: &mut HashSet<String>) {
    let mut r = HashSet::new();
    expr_reads(e, &mut r);
    seen.extend(r);
}

fn stmt_reads(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => expr_reads(expr, out),
        Stmt::SetPath { var, path, expr } => {
            out.insert(var.clone());
            for p in path {
                if let PathStep::Index(i) = p {
                    expr_reads(i, out);
                }
            }
            expr_reads(expr, out);
        }
        Stmt::If { cond, then, els } => {
            expr_reads(cond, out);
            then.iter().for_each(|x| stmt_reads(x, out));
            if let Some(e) = els {
                e.iter().for_each(|x| stmt_reads(x, out));
            }
        }
        Stmt::While { cond, body } => {
            expr_reads(cond, out);
            body.iter().for_each(|x| stmt_reads(x, out));
        }
        Stmt::For { iter, body, .. } => {
            expr_reads(iter, out);
            body.iter().for_each(|x| stmt_reads(x, out));
        }
        Stmt::Return(Some(e)) => expr_reads(e, out),
        Stmt::Propagate { values, .. } => out.extend(values.iter().cloned()),
        Stmt::Stop { value } => {
            out.insert(value.clone());
        }
        _ => {}
    }
}

fn expr_reads(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Local(v, _) | Expr::Move(v, _) => {
            out.insert(v.clone());
        }
        Expr::Call { args, .. } | Expr::Effect { args, .. } => args.iter().for_each(|a| expr_reads(a, out)),
        Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => expr_reads(expr, out),
        Expr::Binary { l, r, .. } => {
            expr_reads(l, out);
            expr_reads(r, out);
        }
        Expr::Index { base, indices, .. } => {
            expr_reads(base, out);
            indices.iter().for_each(|i| expr_reads(i, out));
        }
        Expr::Field { base, .. } | Expr::Meta { base, .. } => expr_reads(base, out),
        Expr::Interp(pieces) => pieces.iter().for_each(|p| {
            if let Piece::Expr(x) = p {
                expr_reads(x, out);
            }
        }),
        Expr::ListLit(items, _) => items.iter().for_each(|i| expr_reads(i, out)),
        Expr::MapLit(pairs, _) => pairs.iter().for_each(|(_, v)| expr_reads(v, out)),
        Expr::RecordLit { fields, .. } => fields.iter().for_each(|f| expr_reads(f, out)),
        Expr::Tree { attrs, children, .. } => {
            attrs.iter().for_each(|(_, a)| expr_reads(a, out));
            children.iter().for_each(|c| expr_reads(c, out));
        }
        Expr::ForExpr { iter, body, .. } => {
            expr_reads(iter, out);
            body.iter().for_each(|s| stmt_reads(s, out));
        }
        Expr::Deferred { args, .. } => args.iter().for_each(|a| expr_reads(a, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ir_of(src: &str) -> String {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let program = lower(&stmts, &mut ctx, "t").unwrap();
        print(&program)
    }

    /// A function's last line is its value, but a handler's value goes to
    /// nobody: an effect it returns waits for the commit, as does one a
    /// helper returns to a caller that calls it as a statement; an effect
    /// whose answer is read runs when asked.
    #[test]
    fn an_effect_whose_answer_nobody_reads_waits_for_the_commit() {
        let ir = ir_of("use web, ui\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'noreply@example.com', 'secret']\nn = 0\nfn notify[m, to]\n    mail[m, to, 'Hello', 'From a helper']\nfn welcome[]\n    n = n + 1\n    mail[mailer, 'ada@example.com', 'Hello', 'Welcome, number `n`']\nfn broken[]\n    mail[mailer, 'bob@example.com', 'Lost', 'never sent']\n    x = float['no']\n    if x gt 1.0\n        n = 1\nfn asked[]\n    r = mail[mailer, 'cy@example.com', 'Asked', 'sent at once']\n    if r.ok\n        n = 10\nfn helped[]\n    notify[mailer, 'di@example.com']\nview v[]\n    return stack[p['`n`'], button['Welcome', click = welcome], button['Broken', click = broken], button['Asked', click = asked], button['Helped', click = helped]]\n");
        assert!(ir.contains("return mail!held(load mailer, \"ada@example.com\""), "{}", ir);
        assert!(ir.contains("return mail!held(m, move to"), "{}", ir);
        assert!(ir.contains("let r: Result(Bool) = mail!now(load mailer"), "{}", ir);
        // read by its caller, the helper's effect answers
        let ir = ir_of("use web, ui\nuse web.mail\nmailer = smtp['h', 587, 'u', 'p']\nsent = false\nfn notify[m, to]\n    mail[m, to, 'Hello', 'x']\nfn go[]\n    r = notify[mailer, 'a@b.c']\n    sent = r.ok\nview v[]\n    return button['Go', click = go]\n");
        assert!(ir.contains("return mail!now(m, move to"), "{}", ir);
    }

    /// A function's body lowered provisionally (F5): what it met, the
    /// environment it leaves, and the lowering's state after it, which
    /// must be as before.
    fn provisional_of(src: &str, f: &str) -> (Seen, BTreeMap<String, CrowType>) {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let body = ctx.functions[f].body.clone();
        let params: HashSet<String> = ctx.functions[f].params.iter().map(|(p, _)| p.clone()).collect();
        let mut l = Lowerer::new(&mut ctx, false);
        l.prescan(&stmts, &HashSet::new());
        let scope = l.ctx.enter_body(f);
        l.at = Where::In(FnKind::Pure);
        l.current_fn = f.to_string();
        let seen = l.provisional(Pass::Settle, Some(params), &body);
        let lowered = l.ctx.vars.clone();
        assert_eq!((l.temp, l.pre.len(), l.scopes.len(), l.v.mute, l.pass), (0, 0, 1, 0, Pass::Final));
        assert!(l.calls.is_empty() && l.direct_effects.is_empty() && l.v.errors.is_empty() && l.seen.is_none());
        l.ctx.leave_body(scope);
        (seen, lowered)
    }

    /// Lowered to learn its types, a body leaves the environment inference
    /// reads — a variable settled across its assignments, a loop's variable
    /// left bound, a collecting loop's locals left out, a map's values taken
    /// from `set[]` and `m[k] = v` — and says what it met: the calls it
    /// makes, the values it returns, the errors it hands back.
    #[test]
    fn a_body_lowered_to_learn_its_types_settles_as_inference_walks() {
        let src = "fn half[x]\n    return x / 2.0\nfn f[n, s]\n    xs = []\n    for i in range[n]\n        xs = push[xs, i * 1.5]\n    ys = for x in xs\n        y = half[x]\n        y\n    m = map[a = 1]\n    set[m, 'b', 2]\n    m['c'] = 3\n    a, b = 1, 'x'\n    k = int[s]\n    if k gt 5\n        return half[2]\n    return len[ys] * 1.0\nlog[f[3, '7']]\n";
        let (seen, lowered) = provisional_of(src, "f");
        assert_eq!(lowered.get("xs"), Some(&CrowType::Array(Box::new(CrowType::Float))));
        assert_eq!(lowered.get("i"), Some(&CrowType::Int));
        assert!(!lowered.contains_key("y") && !lowered.contains_key("x"), "{:?}", lowered);
        assert_eq!(lowered.get("m"), Some(&CrowType::Map(Box::new(CrowType::Int))));
        assert_eq!((lowered.get("a"), lowered.get("b")), (Some(&CrowType::Int), Some(&CrowType::Str)));
        assert_eq!(seen.calls, vec![("half".to_string(), vec![CrowType::Float]), ("half".to_string(), vec![CrowType::Int])]);
        assert_eq!(seen.returns, vec![CrowType::Float, CrowType::Float]);
        assert_eq!(seen.refusals, vec![(Decision::Branch, false)]);
    }

    #[test]
    fn a_collecting_loop_moves_its_list_into_push() {
        let ir = ir_of("xs = [1]\ni = 0\nwhile i lt 3\n    xs = push[xs, i]\n    i = i + 1\nlog[xs]\n");
        assert!(ir.contains("xs = core.push(move xs, i)"), "{}", ir);
        assert!(ir.contains("log!held(move xs)"), "{}", ir);
        assert!(ir.contains("i = (i add 1)"), "{}", ir);
    }

    #[test]
    fn a_fact_and_a_wired_handler_lower_to_store_and_handler() {
        let ir = ir_of("use web, ui\ncount = 0\nfn inc[]\n    count = count + 1\nview render[]\n    return button['+', click = inc]\n");
        assert!(ir.contains("fact count: Int = 0            session"), "{}", ir);
        assert!(ir.contains("handler inc() -> Unit"), "{}", ir);
        assert!(ir.contains("store count ← (load count add 1)"), "{}", ir);
        assert!(ir.contains("view render() -> Node"), "{}", ir);
        assert!(ir.contains("tree button(click: fn inc)[\"+\"]"), "{}", ir);
    }

    #[test]
    fn a_fallible_argument_reaches_the_callee_as_its_payload() {
        // the call propagates an error among its arguments (spec §10); the
        // IR says so on the call and hoists nothing
        let ir = ir_of("r = read['x.txt']\nn = len[r]\nlog[n]\n");
        assert!(ir.contains("let r: Result(Str) = read!now(\"x.txt\")"), "{}", ir);
        assert!(!ir.contains("propagate r"), "{}", ir);
        // len[] of an error is the error, as every call's is: the value can fail
        assert!(ir.contains("let n: Result(Int) = len(unwrap move r)"), "{}", ir);
    }

    #[test]
    fn a_fallible_condition_is_propagated_first() {
        let ir = ir_of("fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nlog[check['7']]\n");
        assert!(ir.contains("propagate"), "{}", ir);
        assert!(ir.contains("propagate t1\n    if unwrap t1"), "{}", ir);
    }

    #[test]
    fn ints_meet_floats_through_an_explicit_conversion() {
        let ir = ir_of("x = 2\ny = x * 1.5\nlog[y]\n");
        assert!(ir.contains("(float(x) mul 1.5)"), "{}", ir);
    }

    #[test]
    fn a_derivation_lists_what_it_reads() {
        let ir = ir_of("use web, ui\nbase = 0.0\nspeed = 1.0\nderive head\n    return base + (now[] - 0.0) * speed\nview v[]\n    return p['`head`']\n");
        assert!(ir.contains("derive head: Float  reads {facts: base speed, clock}"), "{}", ir);
    }

    #[test]
    fn a_parallel_loop_is_marked_and_a_writing_one_is_not() {
        let ir = ir_of("total = 0\nfor x in [1, 2, 3]\n    total = total + x\nfor y in [1, 2]\n    z = y * 2\nlog[total]\n");
        assert!(ir.contains("for x in [1, 2, 3]\n"), "{}", ir);
        assert!(ir.contains("for y in [1, 2]  parallel"), "{}", ir);
    }

    /// The printed line of the loop whose body mentions `needle`.
    fn loop_line<'a>(ir: &'a str, needle: &str) -> &'a str {
        ir.lines().find(|l| l.contains(" for ") && l.contains(needle)).unwrap_or_else(|| panic!("no loop over {} in\n{}", needle, ir))
    }

    /// One rule for both modes (spec §14, For Loops): a loop over a
    /// function runs in parallel when the function has no side effects,
    /// however deep they are, and in order when it has one
    #[test]
    fn a_loop_over_a_function_is_parallel_only_without_side_effects() {
        let ir = ir_of("fn sq[x]\n    return x * x\nfn say[x]\n    log[x]\n    return x\nfn relay[x]\n    return say[x]\nxs = for i in [1, 2, 3]\n    sq[i]\nys = for i in [1, 2, 3]\n    relay[i]\nlog[xs]\nlog[ys]\n");
        assert!(loop_line(&ir, "call sq(").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "call relay(").ends_with("parallel"), "{}", ir);
    }

    /// A seeded program's draws repeat only in the order written: a loop
    /// that draws runs in order (it ran in parallel, and three runs of a
    /// seeded script gave three answers)
    #[test]
    fn a_loop_that_draws_runs_in_order() {
        let ir = ir_of("use math\nseed[1]\nxs = for i in [1, 2, 3]\n    rand[]\nys = for i in [1, 2]\n    randn[2]\nlog[xs]\nlog[ys]\n");
        assert!(!loop_line(&ir, "rand()").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "randn").ends_with("parallel"), "{}", ir);
    }

    /// A view renders in order (the page's stale marks and paged tables
    /// are kept in render order); plain elements are values
    #[test]
    fn a_loop_that_renders_a_view_runs_in_order() {
        let ir = ir_of("use web, ui\n\nitems = ['a', 'b']\n\nview item[x]\n    return p[x]\n\nview render[]\n    plain = for x in items\n        span[x]\n    viewed = for x in items\n        item[x]\n    return stack[plain, viewed]\n\nserve[3996]\npage['/', render]\n");
        assert!(loop_line(&ir, "span").ends_with("parallel"), "{}", ir);
        assert!(!loop_line(&ir, "call item(").ends_with("parallel"), "{}", ir);
    }

    /// A call through a function value reaches any function the program
    /// holds as a value; handing one to a builtin is a call of it too
    #[test]
    fn a_function_value_counts_by_what_it_could_be() {
        let pure = ir_of("fn keep[items, f]\n    out = for x in items\n        if f[x]\n            x\n    return out\nfn even[x]\n    return x % 2 eq 0\nlog[keep[[1, 2, 3], even]]\nrows = for r in [[1, 2], [3]]\n    any[r, even]\nlog[rows]\n");
        assert!(loop_line(&pure, "f[").ends_with("parallel"), "{}", pure);
        assert!(loop_line(&pure, "any(").ends_with("parallel"), "{}", pure);
        let loud = ir_of("fn keep[items, f]\n    out = for x in items\n        if f[x]\n            x\n    return out\nfn loud[x]\n    log[x]\n    return true\nlog[keep[[1, 2, 3], loud]]\nrows = for r in [[1, 2], [3]]\n    any[r, loud]\nlog[rows]\n");
        assert!(!loop_line(&loud, "f[").ends_with("parallel"), "{}", loud);
        assert!(!loop_line(&loud, "any(").ends_with("parallel"), "{}", loud);
    }

    /// A loop's list that can fail is refused first, like a condition: the
    /// statement form's refusal comes before it, and the loop walks the
    /// value; a collecting loop, an expression, keeps the list and is
    /// refused where it starts, and a loop around it runs in order
    #[test]
    fn a_loop_over_a_list_that_can_fail_is_refused_first() {
        let ir = ir_of("fn f[s]\n    for i in range[int[s]]\n        log[i]\n    out = for j in range[int[s]]\n        j * 2\n    return out\nlog[f['2']]\nys = for k in [1, 2]\n    zs = for m in range[int['3']]\n        m\n    len[zs]\nlog[ys]\n");
        assert_eq!(ir.matches("(a loop's list)").count(), 1, "{}", ir);
        assert!(ir.contains("for i in unwrap"), "{}", ir);
        let collecting = loop_line(&ir, "for j in");
        assert!(!collecting.contains("propagate") && !collecting.contains("unwrap t"), "refused where it starts: {}", collecting);
        assert!(!loop_line(&ir, "for k in").ends_with("parallel"), "a loop around one runs in order: {}", ir);
    }

    /// A body that can leave the function (an error handed back from a
    /// condition) runs in order: the first failure in input order decides
    #[test]
    fn a_loop_that_can_return_an_error_runs_in_order() {
        let ir = ir_of("fn f[xs]\n    out = for x in xs\n        if int[x] gt 0\n            x\n    return len[out]\nlog[f[['1', '2']]]\n");
        assert!(!loop_line(&ir, "for x in").ends_with("parallel"), "{}", ir);
    }
}
