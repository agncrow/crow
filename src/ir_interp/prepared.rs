//! A function body prepared for the walker: its locals numbered once, so a
//! read is an index into the frame and not a search by name; the hot shapes
//! (an assignment, `xs[i] = v`, a branch, a `while`, a `for` over a range or
//! a list, a return, a local, a constant, an operator, `xs[i]`, `len[xs]`, a
//! call) mirrored over those slots so they run without the general dispatch
//! and without copying a local to read it; everything else kept as the IR
//! statement or expression it is and run by the walker, which finds the
//! names it needs through the slot table that was in scope at that point
//! (`Frame::env`). The main frame, a derivation and a traced run keep the
//! walker: the hot code lives in functions (roadmap, "A slot-resolved
//! interpreter", steps 1 and 2).
//!
//! The slots follow the walker's own scoping exactly: a `let` binds in the
//! current block (a name bound again there keeps its slot; one bound in an
//! outer block is shadowed until the block ends), a loop variable is a new
//! binding of the loop's own block, and a block's bindings are not read
//! after it (the lowering refuses that), so one slot per binding is the
//! flat stack the walker kept, decided before the function runs.

use super::*;

/// The names in scope at a point of a prepared body, each to its slot: what
/// the walker reads for an IR node it runs on the function's behalf.
pub(super) type Env = Arc<HashMap<String, u32, Names>>;

pub(super) struct Prepared {
    pub(super) n_slots: usize,
    /// Each parameter's slot, in the parameters' order.
    pub(super) params: Vec<u32>,
    pub(super) body: Vec<PStmt>,
}

pub(super) enum PStmt {
    Line(usize),
    /// A `let` or a `set` of a local: the value into its slot.
    Set(u32, PExpr),
    /// `xs[i] = v` of a local, set where it lies.
    SetIndex { var: u32, path: Vec<PathStep>, index: PExpr, value: PExpr },
    Store(String, PExpr),
    If { cond: PExpr, then: Vec<PStmt>, els: Option<Vec<PStmt>> },
    While { cond: PExpr, body: Vec<PStmt> },
    /// `for i in range[n]` / `range[a, b]`, counted when the bounds are
    /// ints; any other outcome of the bounds goes to the row call the list
    /// path makes (ir_interp.rs, the For arm).
    ForRange { var: u32, row: &'static crate::table::Row, propagates: bool, args: Vec<PExpr>, body: Vec<PStmt> },
    ForList { var: u32, iter: PExpr, iter_ty: Ty, body: Vec<PStmt> },
    Return(Option<PExpr>),
    Break,
    Continue,
    Do(PExpr),
    Propagate { slots: Vec<u32>, at: Decision },
    /// The walker's: a parallel loop, an in-place builtin's assignment
    /// (`xs = push[move xs, v]`), a path of more than one step.
    Ir(Stmt, Env),
}

pub(super) enum PExpr {
    Local(u32),
    Move(u32),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    Bin { op: BinOp, l: Box<PExpr>, r: Box<PExpr> },
    Unary { op: UnaryOp, expr: Box<PExpr> },
    /// `xs[i]` of a local by a pure index, read where it lies; the IR and
    /// its scope for the cases the walker decides (a failed local, a kind
    /// the in-place read does not take).
    Index { var: u32, index: Box<PExpr>, ty: Ty, column: bool, ir: Expr, env: Env },
    /// `len[xs]` of a local, measured where it lies.
    Len { var: u32, target: Target, ir: Expr, env: Env },
    Call { target: Target, args: Vec<PExpr>, propagates: bool },
    Ir(Expr, Env),
}

/// The pass over a function's body: slots for its bindings, the walker's
/// scoping (one scope per block), the hot shapes mirrored.
struct Prep {
    n: u32,
    scopes: Vec<Vec<(String, u32)>>,
}

pub(super) fn prepare(f: &Fn) -> Prepared {
    let mut p = Prep { n: 0, scopes: vec![Vec::new()] };
    let params: Vec<u32> = f.params.iter().map(|(name, _)| p.fresh(name)).collect();
    let body = p.stmts(&f.body);
    Prepared { n_slots: p.n as usize, params, body }
}

impl Prep {
    fn resolve(&self, name: &str) -> Option<u32> {
        for scope in self.scopes.iter().rev() {
            if let Some((_, s)) = scope.iter().rev().find(|(n, _)| n == name) {
                return Some(*s);
            }
        }
        None
    }

    /// A `let`: the name bound in the current block keeps its slot, else
    /// a new one (as the walker's `bind` pushes a new binding).
    fn declare(&mut self, name: &str) -> u32 {
        if let Some((_, s)) = self.scopes.last().and_then(|scope| scope.iter().rev().find(|(n, _)| n == name)) {
            return *s;
        }
        self.fresh(name)
    }

    fn fresh(&mut self, name: &str) -> u32 {
        let s = self.n;
        self.n += 1;
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), s));
        }
        s
    }

    fn env(&self) -> Env {
        let mut m = HashMap::with_capacity_and_hasher(8, Names::default());
        for scope in &self.scopes {
            for (n, s) in scope {
                m.insert(n.clone(), *s);
            }
        }
        Arc::new(m)
    }

    /// A block of its own (an `if`'s or a `while`'s body): the walker
    /// pushes a scope for it.
    fn block(&mut self, b: &[Stmt]) -> Vec<PStmt> {
        self.scopes.push(Vec::new());
        let out = self.stmts(b);
        self.scopes.pop();
        out
    }

    fn stmts(&mut self, b: &[Stmt]) -> Vec<PStmt> {
        b.iter().map(|s| self.stmt(s)).collect()
    }

    fn ir_stmt(&self, s: &Stmt) -> PStmt {
        PStmt::Ir(s.clone(), self.env())
    }

    fn ir_expr(&self, e: &Expr) -> PExpr {
        PExpr::Ir(e.clone(), self.env())
    }

    fn stmt(&mut self, s: &Stmt) -> PStmt {
        match s {
            Stmt::Line(n) => PStmt::Line(*n),
            Stmt::Let { var, expr, .. } => {
                let e = self.expr(expr);
                let slot = self.declare(var);
                PStmt::Set(slot, e)
            }
            Stmt::Set { var, expr } => {
                // an in-place builtin's assignment (`xs = push[move xs, v]`)
                // is the walker's: it grows the list where it lies
                if moves_first(var, expr) {
                    return self.ir_stmt(s);
                }
                match self.resolve(var) {
                    Some(slot) => {
                        let e = self.expr(expr);
                        PStmt::Set(slot, e)
                    }
                    None => self.ir_stmt(s),
                }
            }
            Stmt::SetPath { var, path, expr } => {
                if let [PathStep::Index(i)] = path.as_slice()
                    && let Some(slot) = self.resolve(var)
                {
                    let value = self.expr(expr);
                    let index = self.expr(i);
                    return PStmt::SetIndex { var: slot, path: path.clone(), index, value };
                }
                self.ir_stmt(s)
            }
            Stmt::Store { fact, expr } => PStmt::Store(fact.clone(), self.expr(expr)),
            Stmt::If { cond, then, els } => {
                let cond = self.expr(cond);
                let then = self.block(then);
                let els = els.as_ref().map(|b| self.block(b));
                PStmt::If { cond, then, els }
            }
            Stmt::While { cond, body } => {
                let cond = self.expr(cond);
                let body = self.block(body);
                PStmt::While { cond, body }
            }
            Stmt::For { var, iter, body, parallel } => {
                if *parallel {
                    return self.ir_stmt(s);
                }
                if let Expr::Call { target: Target::Row(row), args, propagates, ty, .. } = iter
                    && matches!(row.id, "core.range_to" | "core.range_from")
                    && !ty.is_result()
                {
                    let args = args.iter().map(|a| self.expr(a)).collect();
                    // the loop's own scope: the variable, and the body's
                    // bindings, as the walker binds them (one scope, the
                    // round's bindings dropped after it)
                    self.scopes.push(Vec::new());
                    let slot = self.fresh(var);
                    let body = self.stmts(body);
                    self.scopes.pop();
                    return PStmt::ForRange { var: slot, row, propagates: *propagates, args, body };
                }
                let iter_p = self.expr(iter);
                self.scopes.push(Vec::new());
                let slot = self.fresh(var);
                let body = self.stmts(body);
                self.scopes.pop();
                PStmt::ForList { var: slot, iter: iter_p, iter_ty: iter.ty(), body }
            }
            Stmt::Return(e) => PStmt::Return(e.as_ref().map(|e| self.expr(e))),
            Stmt::Break => PStmt::Break,
            Stmt::Continue => PStmt::Continue,
            Stmt::Do(e) => PStmt::Do(self.expr(e)),
            Stmt::Propagate { values, at } => {
                let mut slots = Vec::with_capacity(values.len());
                for v in values {
                    match self.resolve(v) {
                        Some(slot) => slots.push(slot),
                        None => return self.ir_stmt(s),
                    }
                }
                PStmt::Propagate { slots, at: at.clone() }
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> PExpr {
        match e {
            Expr::Const(c, _) => match c {
                Const::Int(n) => PExpr::Int(*n),
                Const::Float(f) => PExpr::Float(*f),
                Const::Bool(b) => PExpr::Bool(*b),
                Const::Str(s) => PExpr::Str(s.clone()),
                Const::Unit => PExpr::Unit,
            },
            Expr::Local(v, _) => match self.resolve(v) {
                Some(slot) => PExpr::Local(slot),
                None => self.ir_expr(e),
            },
            Expr::Move(v, _) => match self.resolve(v) {
                Some(slot) => PExpr::Move(slot),
                None => self.ir_expr(e),
            },
            Expr::Binary { op, l, r, .. } => match binop_of(op) {
                Ok(op) => {
                    let l = Box::new(self.expr(l));
                    let r = Box::new(self.expr(r));
                    PExpr::Bin { op, l, r }
                }
                Err(_) => self.ir_expr(e),
            },
            Expr::Unary { op, expr, .. } => {
                let op = match op.as_str() {
                    "neg" => UnaryOp::Neg,
                    "not" => UnaryOp::Not,
                    _ => return self.ir_expr(e),
                };
                let expr = Box::new(self.expr(expr));
                PExpr::Unary { op, expr }
            }
            Expr::Index { base, indices, ty } => {
                if let [index] = indices.as_slice()
                    && let Expr::Local(v, _) = unwrapped(base)
                    && let Some(slot) = self.resolve(v)
                    && pure_index(index)
                {
                    let column = matches!(base.ty_ref(), Some(Ty::Column(_)));
                    let env = self.env();
                    let index = Box::new(self.expr(index));
                    return PExpr::Index { var: slot, index, ty: ty.clone(), column, ir: e.clone(), env };
                }
                self.ir_expr(e)
            }
            Expr::Call { target, args, propagates, .. } => {
                if is_len(target)
                    && args.len() == 1
                    && let Expr::Local(v, _) = unwrapped(&args[0])
                    && let Some(slot) = self.resolve(v)
                {
                    return PExpr::Len { var: slot, target: target.clone(), ir: e.clone(), env: self.env() };
                }
                // the shapes the walker's own arms take: an assertion's
                // message, a bound input in a view, a length read in place
                let own = match target {
                    Target::Runtime(name) => matches!(
                        name.rsplit('.').next().unwrap_or(name),
                        "assert" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs" | "select" | "len"
                    ),
                    _ => is_len(target),
                };
                if own {
                    return self.ir_expr(e);
                }
                let args = args.iter().map(|a| self.expr(a)).collect();
                PExpr::Call { target: target.clone(), args, propagates: *propagates }
            }
            _ => self.ir_expr(e),
        }
    }
}

/// An assignment whose value is a call taking the variable moved as its
/// first argument: the walker's in-place path decides it.
fn moves_first(var: &str, expr: &Expr) -> bool {
    let Expr::Call { args, .. } = expr else { return false };
    matches!(args.first().map(unwrapped), Some(Expr::Move(v, _)) if v == var)
}

/// An index the in-place read may evaluate before borrowing the list, and
/// the walker evaluate again when it decides the case: a local, a constant,
/// or an operator over those (nothing moved, nothing called).
fn pure_index(e: &Expr) -> bool {
    match e {
        Expr::Local(..) | Expr::Const(..) => true,
        Expr::Binary { l, r, .. } => pure_index(l) && pure_index(r),
        Expr::Unary { expr, .. } => pure_index(expr),
        _ => false,
    }
}

/// A value read from a slot or a list: a scalar copied in place, anything
/// else cloned (the generic clone is a call and a match over every kind;
/// a third of a tight loop's samples had been in it, copying ints).
#[inline]
fn copied(v: &CrowValue) -> CrowValue {
    if v.is_ok() {
        match &v.value {
            Value::Int(n) => return CrowValue::ok(Value::Int(*n)),
            Value::Float(f) => return CrowValue::ok(Value::Float(*f)),
            Value::Bool(b) => return CrowValue::ok(Value::Bool(*b)),
            _ => {}
        }
    }
    v.clone()
}

/// Two ints under an operator, as the compiled program computes them
/// (crow_runtime::stops, a stop on overflow); `None` for an operator the
/// general dispatch decides (a division answers a float).
#[inline]
fn int_fast(op: &BinOp, a: i64, b: i64) -> Result<Option<Value>, String> {
    Ok(Some(match op {
        BinOp::Add => Value::Int(crow_runtime::stops::add(a, b)?),
        BinOp::Sub => Value::Int(crow_runtime::stops::sub(a, b)?),
        BinOp::Mul => Value::Int(crow_runtime::stops::mul(a, b)?),
        BinOp::Mod => Value::Int(crow_runtime::stops::rem(a, b)?),
        BinOp::Eq => Value::Bool(a == b),
        BinOp::Ne => Value::Bool(a != b),
        BinOp::Gt => Value::Bool(a > b),
        BinOp::Lt => Value::Bool(a < b),
        BinOp::Gte => Value::Bool(a >= b),
        BinOp::Lte => Value::Bool(a <= b),
        _ => return Ok(None),
    }))
}

impl IrInterp {
    /// The function's prepared body, made on its first call and kept.
    pub(super) fn prepared_for(&mut self, f: &Arc<Fn>) -> Arc<Prepared> {
        if let Some(p) = self.prepared.get(&f.name) {
            return p.clone();
        }
        let p = Arc::new(prepare(f));
        self.prepared.insert(f.name.clone(), p.clone());
        p
    }

    #[inline]
    fn slots(&self) -> &[CrowValue] {
        match self.frames.last() {
            Some(f) => &f.slots,
            None => &[],
        }
    }

    #[inline]
    fn slot_mut(&mut self, s: u32) -> &mut CrowValue {
        let frame = self.frames.last_mut().expect("a prepared body runs in a frame");
        &mut frame.slots[s as usize]
    }

    /// The walker run on an IR node of a prepared body: the names in scope
    /// there resolve through the frame's slot table while it runs.
    fn with_env<T>(&mut self, env: &Env, run: impl FnOnce(&mut Self) -> T) -> T {
        let saved = match self.frames.last_mut() {
            Some(frame) => frame.env.replace(env.clone()),
            None => None,
        };
        let r = run(self);
        if let Some(frame) = self.frames.last_mut() {
            frame.env = saved;
        }
        r
    }

    pub(super) fn pexec(&mut self, b: &[PStmt]) -> Result<Flow, String> {
        for s in b {
            match self.pexec_stmt(s)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn pexec_stmt(&mut self, s: &PStmt) -> Result<Flow, String> {
        match s {
            PStmt::Line(n) => {
                self.line = *n;
                Ok(Flow::Next)
            }
            PStmt::Set(slot, e) => {
                let v = self.peval(e)?;
                *self.slot_mut(*slot) = v;
                Ok(Flow::Next)
            }
            PStmt::SetIndex { var, path, index, value } => {
                let new = self.peval(value)?;
                let at = match self.int_operand(index) {
                    Some(n) => CrowValue::ok(Value::Int(n)),
                    None => self.peval(index)?,
                };
                let root = self.slot_mut(*var);
                if root.is_error() {
                    return Err(format!("cannot set an item of an error value — {}", root.reason()));
                }
                if let (Value::Array(items), Value::Int(n)) = (&mut root.value, &at.value) {
                    let k = crow_runtime::stops::item(*n, items.len())?;
                    items[k] = new;
                    return Ok(Flow::Next);
                }
                set_path(&mut root.value, path, &[at], new)?;
                Ok(Flow::Next)
            }
            PStmt::Store(fact, e) => {
                let v = self.peval(e)?;
                if v.is_error() && self.held.is_some() {
                    self.refuse(v.clone(), &Decision::Branch)?;
                }
                self.set_fact(fact, v);
                Ok(Flow::Next)
            }
            PStmt::If { cond, then, els } => {
                let c = self.peval(cond)?;
                if c.is_error() {
                    return self.refuse(c, &Decision::Branch);
                }
                if crate::ops::is_truthy(&c) {
                    self.pexec(then)
                } else if let Some(e) = els {
                    self.pexec(e)
                } else {
                    Ok(Flow::Next)
                }
            }
            PStmt::While { cond, body } => {
                loop {
                    self.check_cancel()?;
                    let c = self.peval(cond)?;
                    if c.is_error() {
                        return self.refuse(c, &Decision::Branch);
                    }
                    if !crate::ops::is_truthy(&c) {
                        break;
                    }
                    match self.pexec(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            PStmt::ForRange { var, row, propagates, args, body } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.peval(a)?);
                }
                let bound = |v: &CrowValue| if v.is_ok() && let Value::Int(n) = &v.value { Some(*n) } else { None };
                let bounds = match vals.as_slice() {
                    [n] => bound(n).map(|hi| (0, hi)),
                    [a, b] => bound(a).zip(bound(b)),
                    _ => None,
                };
                if let Some((lo, hi)) = bounds {
                    let mut i = lo;
                    while i < hi {
                        self.check_cancel()?;
                        *self.slot_mut(*var) = CrowValue::ok(Value::Int(i));
                        match self.pexec(body)? {
                            Flow::Break => break,
                            Flow::Return(v) => return Ok(Flow::Return(v)),
                            Flow::Next | Flow::Continue => {}
                        }
                        i += 1;
                    }
                    return Ok(Flow::Next);
                }
                // what the call would have answered from these values
                let it = match (*propagates, self.first_error(&vals)) {
                    (true, Some(err)) => err,
                    _ => self.row_call(row, vals)?,
                };
                self.pfor_items(*var, it, &Ty::List(Box::new(Ty::Int)), body)
            }
            PStmt::ForList { var, iter, iter_ty, body } => {
                let it = self.peval(iter)?;
                self.pfor_items(*var, it, iter_ty, body)
            }
            PStmt::Return(None) => Ok(Flow::Return(CrowValue::done())),
            PStmt::Return(Some(e)) => {
                let v = self.peval(e)?;
                Ok(Flow::Return(v))
            }
            PStmt::Break => Ok(Flow::Break),
            PStmt::Continue => Ok(Flow::Continue),
            PStmt::Do(e) => {
                let v = self.peval(e)?;
                self.last_value = Some(v);
                Ok(Flow::Next)
            }
            PStmt::Propagate { slots, at } => {
                for s in slots {
                    let v = &self.slots()[*s as usize];
                    if v.is_error() {
                        let v = v.clone();
                        return self.refuse(v, at);
                    }
                }
                Ok(Flow::Next)
            }
            PStmt::Ir(stmt, env) => self.with_env(env, |it| it.exec_stmt(stmt)),
        }
    }

    /// The rounds of a loop over a list (or what a range row answered),
    /// the variable's slot rebound each round.
    fn pfor_items(&mut self, var: u32, it: CrowValue, iter_ty: &Ty, body: &[PStmt]) -> Result<Flow, String> {
        // a list that failed is refused like a condition (the lowering
        // refuses a typed one first; this is one it could not see)
        if it.is_error() {
            return self.refuse(it, &Decision::Loop);
        }
        let items = Self::walked_items_of(&it, iter_ty)?;
        for item in items {
            self.check_cancel()?;
            *self.slot_mut(var) = item;
            match self.pexec(body)? {
                Flow::Break => break,
                Flow::Return(v) => return Ok(Flow::Return(v)),
                Flow::Next | Flow::Continue => {}
            }
        }
        Ok(Flow::Next)
    }

    /// An operand read without a copy: an int constant, or a local holding
    /// an int.
    #[inline]
    fn int_operand(&self, e: &PExpr) -> Option<i64> {
        match e {
            PExpr::Int(n) => Some(*n),
            PExpr::Local(s) => {
                let v = &self.slots()[*s as usize];
                if v.is_ok() && let Value::Int(n) = &v.value { Some(*n) } else { None }
            }
            _ => None,
        }
    }

    pub(super) fn peval(&mut self, e: &PExpr) -> Result<CrowValue, String> {
        Ok(match e {
            PExpr::Local(s) => copied(&self.slots()[*s as usize]),
            PExpr::Move(s) => std::mem::replace(self.slot_mut(*s), CrowValue::done()),
            PExpr::Int(n) => CrowValue::ok(Value::Int(*n)),
            PExpr::Float(f) => CrowValue::ok(Value::Float(*f)),
            PExpr::Bool(b) => CrowValue::ok(Value::Bool(*b)),
            PExpr::Str(s) => CrowValue::ok(Value::Str(s.clone())),
            PExpr::Unit => CrowValue::done(),
            PExpr::Bin { op, l, r } => {
                // two ints read where they lie: no copy of either side
                if let (Some(a), Some(b)) = (self.int_operand(l), self.int_operand(r))
                    && let Some(v) = int_fast(op, a, b)?
                {
                    return Ok(CrowValue::ok(v));
                }
                // (each side read once, in order, as the walker reads them)
                let lv = self.peval(l)?;
                let rv = self.peval(r)?;
                if lv.is_ok()
                    && rv.is_ok()
                    && let (Value::Int(a), Value::Int(b)) = (&lv.value, &rv.value)
                    && let Some(v) = int_fast(op, *a, *b)?
                {
                    return Ok(CrowValue::ok(v));
                }
                crate::ops::binop(&lv, op, &rv)?
            }
            PExpr::Unary { op, expr } => {
                let v = self.peval(expr)?;
                crate::ops::unaryop(op, &v)?
            }
            PExpr::Index { var, index, ty, column, ir, env } => {
                let at = match self.int_operand(index) {
                    Some(n) => CrowValue::ok(Value::Int(n)),
                    None => self.peval(index)?,
                };
                let b = &self.slots()[*var as usize];
                if b.is_ok() {
                    if let (Value::Array(items), Value::Int(n)) = (&b.value, &at.value) {
                        let k = crow_runtime::stops::item(*n, items.len())?;
                        let cell = copied(&items[k]);
                        return Ok(if *column { placeholder(cell, ty) } else { cell });
                    }
                    if matches!(b.value, Value::NumArray(_) | Value::Map(_) | Value::IntMap(_) | Value::Str(_) | Value::Frame(_)) {
                        let cell = Self::index_value(b, vec![at], ty)?;
                        return Ok(if *column { placeholder(cell, ty) } else { cell });
                    }
                }
                // a failed local, or a kind the in-place read does not take:
                // the walker decides, as it always has
                self.with_env(env, |it| it.eval(ir))?
            }
            PExpr::Len { var, target, ir, env } => {
                if self.registry.has_function("len") {
                    return self.with_env(env, |it| it.eval(ir));
                }
                let trace = self.trace_entry();
                let b = &self.slots()[*var as usize];
                if b.is_error() {
                    return Ok(b.clone());
                }
                Self::len_of(target, b, &trace)?
            }
            PExpr::Call { target, args, propagates } => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.peval(a)?);
                }
                if *propagates && let Some(err) = self.first_error(&vals) {
                    return Ok(err);
                }
                match target {
                    Target::Row(row) => self.row_call(row, vals)?,
                    Target::Runtime(path) => self.runtime_call(path, vals)?,
                    Target::User(name) => self.call_user(name, vals)?,
                    Target::Module(m, f) => self.module_call(m, f, vals)?,
                }
            }
            PExpr::Ir(expr, env) => self.with_env(env, |it| it.eval(expr))?,
        })
    }
}
