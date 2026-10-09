//! A function's body prepared for the interpreter (roadmap, "A slot-resolved
//! interpreter"): compiled on its first call into a flat sequence of
//! instructions over numbered registers, and kept. Its locals are registers
//! decided once, by the walker's own scoping; its control flow is jumps; an
//! operator, a comparison that decides a branch, and a read of a list's
//! item or a store into one, whose operand types the IR fixes (two ints, two
//! floats, two truths), are typed instructions that compute in place — each
//! with the general dispatch behind it for a value the IR could not see (a
//! dual number a function `grad` differentiates computes with, a failed
//! value), so a typed instruction is never wrong, only fast. Everything else
//! stays the IR node it is, run by the walker with the registers lent to the
//! frame (`Frame::slots`, the names in scope there in `Frame::env`). Each
//! instruction's source line is a table, read where a stop, a call or a
//! failed value's trace needs it. The main frame, a derivation and a traced
//! run keep the walker.
//!
//! The registers follow the walker's scoping exactly: a `let` binds in its
//! block (a name bound again there keeps its register; one bound in an outer
//! block is shadowed until the block ends), a loop variable is a new binding
//! of the loop's own block — so a nested `for i` inside `for i` restores the
//! outer `i` — and a block's bindings are not read after it (the lowering
//! refuses that). One register per binding is the walker's flat stack,
//! decided before the function runs. A binding that holds more than a number
//! is dropped where the walker drops it, at its block's end.

use super::*;

/// The names in scope at a point of a prepared body, each to its register:
/// what the walker reads for an IR node it runs on the function's behalf.
pub(super) type Env = Arc<HashMap<String, u32, Names>>;

type R = u32;

/// An int operand: a register, or a constant the program wrote.
#[derive(Clone, Copy, Debug)]
enum Opnd {
    R(R),
    K(i64),
}

/// An operator whose two operands the IR types as ints.
#[derive(Clone, Copy, Debug)]
enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Lte,
    Gte,
}

/// An operator whose two operands the IR types as floats.
#[derive(Clone, Copy, Debug)]
enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Lte,
    Gte,
}

impl IntOp {
    fn of(op: &BinOp) -> Option<IntOp> {
        Some(match op {
            BinOp::Add => IntOp::Add,
            BinOp::Sub => IntOp::Sub,
            BinOp::Mul => IntOp::Mul,
            BinOp::Div => IntOp::Div,
            BinOp::Mod => IntOp::Mod,
            BinOp::Pow => IntOp::Pow,
            BinOp::Eq => IntOp::Eq,
            BinOp::Ne => IntOp::Ne,
            BinOp::Lt => IntOp::Lt,
            BinOp::Gt => IntOp::Gt,
            BinOp::Lte => IntOp::Lte,
            BinOp::Gte => IntOp::Gte,
            _ => return None,
        })
    }

    fn general(self) -> BinOp {
        match self {
            IntOp::Add => BinOp::Add,
            IntOp::Sub => BinOp::Sub,
            IntOp::Mul => BinOp::Mul,
            IntOp::Div => BinOp::Div,
            IntOp::Mod => BinOp::Mod,
            IntOp::Pow => BinOp::Pow,
            IntOp::Eq => BinOp::Eq,
            IntOp::Ne => BinOp::Ne,
            IntOp::Lt => BinOp::Lt,
            IntOp::Gt => BinOp::Gt,
            IntOp::Lte => BinOp::Lte,
            IntOp::Gte => BinOp::Gte,
        }
    }

    fn compares(self) -> bool {
        matches!(self, IntOp::Eq | IntOp::Ne | IntOp::Lt | IntOp::Gt | IntOp::Lte | IntOp::Gte)
    }

    /// Two ints under an arithmetic operator, as the compiled program
    /// computes them: the runtime's checked arithmetic (a stop past 64 bits,
    /// on a zero divisor).
    #[inline(always)]
    fn arith(self, a: i64, b: i64) -> Result<i64, String> {
        match self {
            IntOp::Add => crow_runtime::stops::add(a, b),
            IntOp::Sub => crow_runtime::stops::sub(a, b),
            IntOp::Mul => crow_runtime::stops::mul(a, b),
            IntOp::Div => crow_runtime::stops::div(a, b),
            IntOp::Mod => crow_runtime::stops::rem(a, b),
            IntOp::Pow => crow_runtime::stops::pow(a, b),
            _ => Ok(0),
        }
    }

    #[inline(always)]
    fn holds(self, a: i64, b: i64) -> bool {
        match self {
            IntOp::Eq => a == b,
            IntOp::Ne => a != b,
            IntOp::Lt => a < b,
            IntOp::Gt => a > b,
            IntOp::Lte => a <= b,
            IntOp::Gte => a >= b,
            _ => false,
        }
    }
}

impl FloatOp {
    fn of(op: &BinOp) -> Option<FloatOp> {
        Some(match op {
            BinOp::Add => FloatOp::Add,
            BinOp::Sub => FloatOp::Sub,
            BinOp::Mul => FloatOp::Mul,
            BinOp::Div => FloatOp::Div,
            BinOp::Pow => FloatOp::Pow,
            BinOp::Eq => FloatOp::Eq,
            BinOp::Ne => FloatOp::Ne,
            BinOp::Lt => FloatOp::Lt,
            BinOp::Gt => FloatOp::Gt,
            BinOp::Lte => FloatOp::Lte,
            BinOp::Gte => FloatOp::Gte,
            _ => return None,
        })
    }

    fn general(self) -> BinOp {
        match self {
            FloatOp::Add => BinOp::Add,
            FloatOp::Sub => BinOp::Sub,
            FloatOp::Mul => BinOp::Mul,
            FloatOp::Div => BinOp::Div,
            FloatOp::Pow => BinOp::Pow,
            FloatOp::Eq => BinOp::Eq,
            FloatOp::Ne => BinOp::Ne,
            FloatOp::Lt => BinOp::Lt,
            FloatOp::Gt => BinOp::Gt,
            FloatOp::Lte => BinOp::Lte,
            FloatOp::Gte => BinOp::Gte,
        }
    }

    fn compares(self) -> bool {
        matches!(self, FloatOp::Eq | FloatOp::Ne | FloatOp::Lt | FloatOp::Gt | FloatOp::Lte | FloatOp::Gte)
    }

    #[inline(always)]
    fn farith(self, a: f64, b: f64) -> f64 {
        match self {
            FloatOp::Add => a + b,
            FloatOp::Sub => a - b,
            FloatOp::Mul => a * b,
            FloatOp::Div => a / b,
            FloatOp::Pow => a.powf(b),
            _ => 0.0,
        }
    }

    #[inline(always)]
    fn fholds(self, a: f64, b: f64) -> bool {
        match self {
            FloatOp::Eq => a == b,
            FloatOp::Ne => a != b,
            FloatOp::Lt => a < b,
            FloatOp::Gt => a > b,
            FloatOp::Lte => a <= b,
            FloatOp::Gte => a >= b,
            _ => false,
        }
    }

    /// Two floats, as IEEE says and the compiled program computes them (a
    /// comparison with NaN false, `ne` true — the runtime's cells agree).
    #[inline(always)]
    fn apply(self, a: f64, b: f64) -> Value {
        match self {
            FloatOp::Add => Value::Float(a + b),
            FloatOp::Sub => Value::Float(a - b),
            FloatOp::Mul => Value::Float(a * b),
            FloatOp::Div => Value::Float(a / b),
            FloatOp::Pow => Value::Float(a.powf(b)),
            FloatOp::Eq => Value::Bool(a == b),
            FloatOp::Ne => Value::Bool(a != b),
            FloatOp::Lt => Value::Bool(a < b),
            FloatOp::Gt => Value::Bool(a > b),
            FloatOp::Lte => Value::Bool(a <= b),
            FloatOp::Gte => Value::Bool(a >= b),
        }
    }
}

/// What a `for` loop walks.
#[derive(Debug)]
enum ForSrc {
    /// `range[n]` or `range[a, b]`: counted when both bounds are ints; any
    /// other outcome goes to the row call the list path makes.
    Range { row: &'static crate::table::Row, propagates: bool, args: Box<[R]> },
    /// Anything else: the iterable's register (taken: it was computed for
    /// the loop) and its type, which says how a tensor is walked.
    Items { it: R, take: bool, ty: Ty },
}

#[derive(Debug)]
enum Ins {
    /// A constant into a register (text cloned each time, as the walker does).
    Const { d: R, v: CrowValue },
    /// A register's value into another: a number copied, anything else cloned.
    Copy { d: R, s: R },
    /// A register's value taken out (`move x`), done left behind.
    Take { d: R, s: R },
    /// A binding dropped where the walker drops it: its block's end.
    Clear { d: R },
    Int { op: IntOp, d: R, a: Opnd, b: Opnd },
    Float { op: FloatOp, d: R, a: R, b: R },
    /// `and`/`or` of two truths, both sides evaluated, as in both modes.
    Logic { and: bool, d: R, a: R, b: R },
    Bin { op: BinOp, d: R, a: R, b: R },
    Not { d: R, s: R },
    Neg { d: R, s: R },
    /// An int read as a float; any other value as it is (the walker's rule).
    ToFloat { d: R, s: R },
    Jump { to: u32 },
    /// A `while` loop's first round begins: the cancellation check.
    Cancel,
    /// A `while` loop's back edge: the next round's cancellation check, then
    /// the jump to its condition.
    Loop { to: u32 },
    /// A decision: a failed value refused, a false one jumps.
    JumpUnless { c: R, to: u32 },
    /// A decision on two ints compared, fused: jumps when it is false.
    JumpUnlessInt { op: IntOp, a: Opnd, b: Opnd, to: u32 },
    /// A `for` loop begins: the bounds or the items into its hidden
    /// registers, the first round's value bound — or, with nothing to walk,
    /// a jump past the loop.
    ForStart { var: R, ctr: R, hi: R, items: R, src: Box<ForSrc>, end: u32 },
    /// A `for` loop's next round, or a fall past it.
    ForNext { var: R, ctr: R, hi: R, items: R, body: u32 },
    /// `xs[i]` of a local, read where it lies; the walker decides any other
    /// case (a failed local, a kind the in-place read does not take).
    Index { d: R, base: R, idx: Opnd, how: Box<IndexHow> },
    /// `xs = push[move xs, v]` and the other builtins that make their
    /// answer of the collection they take (`ast::IN_PLACE_BUILTINS`): the
    /// local taken into the row and its answer put back — the walker's
    /// in-place path, without resolving the row again at each call.
    InPlace { d: R, row: &'static crate::table::Row, rest: Box<[(R, bool)]>, propagates: bool },
    /// `xs = push[move xs, v]` of a local list: the item appended where the
    /// list lies (`core.push` is `Vec::push`); anything the in-place path
    /// would not take goes to `InPlace`'s general call.
    Push { d: R, v: R, take: bool, row: &'static crate::table::Row, propagates: bool },
    /// `xs[i] = ys[j]` of two locals: the item read (the walker decides a
    /// value the in-place read does not take), then set where it lies.
    CopyItem { dst: R, di: Opnd, src: R, si: Opnd, how: Box<CopyHow> },
    /// `xs[i] = v` of a local, set where it lies; `v` taken when it is a
    /// temporary.
    SetIndex { base: R, idx: Opnd, v: R, take: bool, path: Box<[PathStep]> },
    /// `len[xs]` of a local, measured where it lies.
    Len { d: R, base: R, how: Box<(Target, Expr, Env)> },
    /// A call: each argument cloned from its register, or taken from a
    /// temporary.
    Call { d: R, target: Box<Target>, args: Box<[(R, bool)]>, propagates: bool },
    Store { fact: Box<str>, s: R, take: bool },
    Do { s: R },
    Propagate { regs: Box<[R]>, at: Box<Decision> },
    Return { s: Option<(R, bool)> },
    /// The walker's: an expression it evaluates over the lent registers.
    Eval { d: R, ir: Box<(Expr, Env)> },
    /// The walker's: a statement it runs over the lent registers; a break or
    /// a continue it answers jumps where the loop's own would.
    Exec { ir: Box<(Stmt, Env)>, brk: u32, cont: u32 },
}

#[derive(Debug)]
struct CopyHow {
    /// The value's expression (`ys[j]`), for the walker to read when the
    /// in-place read does not take it.
    ir: Expr,
    env: Env,
    path: Box<[PathStep]>,
}

#[derive(Debug)]
struct IndexHow {
    ty: Ty,
    column: bool,
    ir: Expr,
    env: Env,
}

pub(super) struct Prepared {
    pub(super) n_regs: usize,
    /// Each register's kind (`RKind`), fixed for the function: a local's by
    /// its type, a temporary's by the pool it was drawn from.
    pub(super) kinds: Vec<RKind>,
    /// Each parameter's register, in the parameters' order.
    pub(super) params: Vec<R>,
    code: Vec<Ins>,
    /// Each instruction's source line (0: the line already current).
    lines: Vec<usize>,
    /// The same body with its int and float registers kept bare (step 4):
    /// none when it has no such register.
    typed: Option<Typed>,
}

/// The position a step answers when the body is done: its outcome is in
/// the step's `out`.
const DONE: usize = usize::MAX;

/// What a register holds for the whole of a function: a number its kind
/// says, or any value. A typed run keeps an int or a float register as the
/// bare machine value; the boxed program reads every register as a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RKind {
    Boxed = 0,
    Int = 1,
    Float = 2,
    /// A local list of ints a typed run keeps as `Vec<i64>` — where the
    /// function uses it only through list instructions (`typed`'s rule).
    IntList = 3,
}

impl RKind {
    /// A temporary's kind by its value's type.
    fn of(t: &Ty) -> RKind {
        match t {
            Ty::Int => RKind::Int,
            Ty::Float => RKind::Float,
            _ => RKind::Boxed,
        }
    }

    /// A local's: a list of ints may stay bare as well.
    fn of_local(t: &Ty) -> RKind {
        match t {
            Ty::List(e) if **e == Ty::Int => RKind::IntList,
            other => RKind::of(other),
        }
    }

    /// The pool a copy of a register of this kind is drawn from.
    fn pool(self) -> RKind {
        match self {
            RKind::IntList => RKind::Boxed,
            k => k,
        }
    }
}

/// Where an expression's value is.
#[derive(Clone, Copy)]
enum Place {
    /// A local's register: read, never taken.
    Slot(R),
    /// A temporary register holding the value: taken by whoever uses it.
    Temp(R),
    /// An int the program wrote.
    IntK(i64),
}

/// A loop the compiler is inside: where a `break` and a `continue` go, and
/// the scope depth they leave to.
struct LoopCtx {
    breaks: Vec<usize>,
    /// The jumps a `continue` makes, patched to the loop's back edge (which
    /// checks for cancellation, as every round of the walker's does).
    conts: Vec<usize>,
    depth: usize,
}

struct Comp {
    code: Vec<Ins>,
    lines: Vec<usize>,
    line: usize,
    /// The scopes, innermost last: each binding's name, register, and
    /// whether it holds only a number (nothing to drop at the block's end).
    scopes: Vec<Vec<(String, R, bool)>>,
    n_slots: u32,
    /// The temporaries' pools, one per kind (boxed, int, float), so a
    /// register's kind never changes within a function: each pool's first
    /// register, how many are in use, and the most ever in use.
    temp_base: [u32; 3],
    temps: [u32; 3],
    max_temps: [u32; 3],
    /// Each local's (and loop register's) kind, by register.
    kinds: Vec<RKind>,
    loops: Vec<LoopCtx>,
}

pub(super) fn prepare(f: &Fn) -> Prepared {
    // twice: the first pass counts the locals, so the temporaries of the
    // second sit after them (the numbering is the same both times)
    let mut first = Comp::new([1 << 28, 2 << 28, 3 << 28]);
    first.function(f);
    let n = first.n_slots;
    let [mb, mi, mf] = first.max_temps;
    let mut c = Comp::new([n, n + mb, n + mb + mi]);
    let params = c.function(f);
    let mut kinds = c.kinds.clone();
    kinds.extend(std::iter::repeat_n(RKind::Boxed, mb as usize));
    kinds.extend(std::iter::repeat_n(RKind::Int, mi as usize));
    kinds.extend(std::iter::repeat_n(RKind::Float, mf as usize));
    let mut p = Prepared { n_regs: kinds.len(), kinds, params, code: c.code, lines: c.lines, typed: None };
    p.typed = typed(&p);
    p
}

fn scalar(t: &Ty) -> bool {
    matches!(t, Ty::Int | Ty::Float | Ty::Bool)
}

impl Comp {
    fn new(temp_base: [u32; 3]) -> Comp {
        Comp { code: Vec::new(), lines: Vec::new(), line: 0, scopes: vec![Vec::new()], n_slots: 0, temp_base, temps: [0; 3], max_temps: [0; 3], kinds: Vec::new(), loops: Vec::new() }
    }

    fn function(&mut self, f: &Fn) -> Vec<R> {
        let params = f.params.iter().map(|(name, ty)| self.fresh(name, scalar(ty), RKind::of_local(ty))).collect();
        self.block_stmts(&f.body);
        params
    }

    fn emit(&mut self, ins: Ins) -> usize {
        self.code.push(ins);
        self.lines.push(self.line);
        self.code.len() - 1
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch(&mut self, at: usize, to: u32) {
        match &mut self.code[at] {
            Ins::Jump { to: t } | Ins::Loop { to: t } | Ins::JumpUnless { to: t, .. } | Ins::JumpUnlessInt { to: t, .. } => *t = to,
            Ins::ForStart { end, .. } => *end = to,
            _ => {}
        }
    }

    /// A temporary of a kind, from that kind's pool.
    fn temp(&mut self, k: RKind) -> R {
        let i = k as usize;
        let r = self.temp_base[i] + self.temps[i];
        self.temps[i] += 1;
        self.max_temps[i] = self.max_temps[i].max(self.temps[i]);
        r
    }

    /// A register's kind: a local's as it was bound, a temporary's by its pool.
    fn kind(&self, r: R) -> RKind {
        if let Some(k) = self.kinds.get(r as usize) {
            return *k;
        }
        if r >= self.temp_base[2] {
            RKind::Float
        } else if r >= self.temp_base[1] {
            RKind::Int
        } else {
            RKind::Boxed
        }
    }

    fn resolve(&self, name: &str) -> Option<R> {
        for scope in self.scopes.iter().rev() {
            if let Some((_, r, _)) = scope.iter().rev().find(|(n, _, _)| n == name) {
                return Some(*r);
            }
        }
        None
    }

    /// A `let`: a name bound in the current block keeps its register, else
    /// a new one (as the walker's `bind` pushes a new binding).
    fn declare(&mut self, name: &str, plain: bool, k: RKind) -> R {
        if let Some((_, r, p)) = self.scopes.last_mut().and_then(|scope| scope.iter_mut().rev().find(|(n, _, _)| n == name)) {
            *p &= plain;
            return *r;
        }
        self.fresh(name, plain, k)
    }

    /// The register `declare` will give a name: its own in the current
    /// block, or the next new one.
    fn peek(&self, name: &str) -> R {
        match self.scopes.last().and_then(|scope| scope.iter().rev().find(|(n, _, _)| n == name)) {
            Some((_, r, _)) => *r,
            None => self.n_slots,
        }
    }

    fn fresh(&mut self, name: &str, plain: bool, k: RKind) -> R {
        let r = self.n_slots;
        self.n_slots += 1;
        self.kinds.push(k);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_string(), r, plain));
        }
        r
    }

    /// A register of the loop's own, no name: its counter, bound, items.
    fn hidden(&mut self, k: RKind) -> R {
        let r = self.n_slots;
        self.n_slots += 1;
        self.kinds.push(k);
        r
    }

    fn env(&self) -> Env {
        let mut m = HashMap::with_capacity_and_hasher(8, Names::default());
        for scope in &self.scopes {
            for (n, r, _) in scope {
                m.insert(n.clone(), *r);
            }
        }
        Arc::new(m)
    }

    /// The drops at the end of the scopes from the innermost down to
    /// `depth` (kept): what the walker's block ends drop.
    fn clears_to(&mut self, depth: usize) {
        let regs: Vec<R> = self.scopes[depth..].iter().rev().flat_map(|s| s.iter().rev().filter(|(_, _, p)| !*p).map(|(_, r, _)| *r)).collect();
        for d in regs {
            self.emit(Ins::Clear { d });
        }
    }

    /// A block of its own (an `if`'s, a `while`'s body): the walker pushes a
    /// scope for it, and drops what it bound at its end.
    fn block(&mut self, b: &[Stmt]) {
        self.scopes.push(Vec::new());
        self.block_stmts(b);
        let depth = self.scopes.len() - 1;
        self.clears_to(depth);
        self.scopes.pop();
    }

    fn block_stmts(&mut self, b: &[Stmt]) {
        for s in b {
            self.temps = [0; 3];
            self.stmt(s);
        }
    }

    fn exec(&mut self, s: &Stmt) {
        let at = self.emit(Ins::Exec { ir: Box::new((s.clone(), self.env())), brk: u32::MAX, cont: u32::MAX });
        if let Some(l) = self.loops.last_mut() {
            l.breaks.push(at);
            l.conts.push(at);
        }
    }

    fn eval(&mut self, e: &Expr, d: R) {
        self.emit(Ins::Eval { d, ir: Box::new((e.clone(), self.env())) });
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Line(n) => self.line = *n,
            Stmt::Let { var, ty, expr } => {
                // the value first, then the binding: the name is not in
                // scope for its own value's expression (an outer `x` it
                // reads stays the outer one) — computed straight into the
                // register the binding will have
                let d = self.peek(var);
                let p = self.expr_place(expr, d);
                let bound = self.declare(var, scalar(ty), RKind::of_local(ty));
                debug_assert_eq!(bound, d);
                self.put(p, d);
            }
            Stmt::Set { var, expr } => {
                // an in-place builtin's assignment (`xs = push[move xs, v]`):
                // the row the IR names takes the local's value, by the
                // walker's own rule (a row whose first parameter it owns)
                if moves_first(var, expr)
                    && let Expr::Call { target: Target::Row(row), args, propagates, .. } = expr
                    && row.module.is_empty()
                    && crate::ast::IN_PLACE_BUILTINS.contains(&row.name)
                    && row.params.first().is_some_and(|k| k.owned())
                    && let Some(d) = self.resolve(var)
                {
                    let mut rest = Vec::with_capacity(args.len().saturating_sub(1));
                    for (i, a) in args.iter().enumerate().skip(1) {
                        let p = self.expr(a);
                        let p = self.kept_place(p, &args[i + 1..]);
                        rest.push(self.owned(p));
                    }
                    if row.id == "core.push" && rest.len() == 1 {
                        let (v, take) = rest[0];
                        self.emit(Ins::Push { d, v, take, row, propagates: *propagates });
                    } else {
                        self.emit(Ins::InPlace { d, row, rest: rest.into_boxed_slice(), propagates: *propagates });
                    }
                    return;
                }
                if moves_first(var, expr) {
                    return self.exec(s);
                }
                match self.resolve(var) {
                    Some(d) => {
                        let p = self.expr_place(expr, d);
                        self.put(p, d);
                    }
                    None => self.exec(s),
                }
            }
            Stmt::SetPath { var, path, expr } => {
                // `xs[i] = ys[j]`: the item read and written by one
                // instruction, the value's index first as the walker reads it
                // (the destination's index a local or a constant, so nothing
                // between the two can fail out of order)
                if let [PathStep::Index(i)] = path.as_slice()
                    && let Some(dst) = self.resolve(var)
                    && matches!(i, Expr::Local(..) | Expr::Const(..))
                    && let Expr::Index { base: sb, indices, .. } = expr
                    && let [si] = indices.as_slice()
                    && let Expr::Local(sv, _) = unwrapped(sb)
                    && let Some(src) = self.resolve(sv)
                    && pure_index(si)
                    && !matches!(sb.ty_ref(), Some(Ty::Column(_)))
                {
                    let env = self.env();
                    let si = self.opnd(si);
                    let di = self.opnd(i);
                    self.emit(Ins::CopyItem { dst, di, src, si, how: Box::new(CopyHow { ir: expr.clone(), env, path: path.clone().into_boxed_slice() }) });
                    return;
                }
                if let [PathStep::Index(i)] = path.as_slice()
                    && let Some(base) = self.resolve(var)
                    && pure_index(i)
                {
                    // the value first, then the index, as the walker reads them
                    let vp = self.expr(expr);
                    let (v, take) = self.owned(vp);
                    let idx = self.opnd(i);
                    self.emit(Ins::SetIndex { base, idx, v, take, path: path.clone().into_boxed_slice() });
                    return;
                }
                self.exec(s)
            }
            Stmt::Store { fact, expr } => {
                let p = self.expr(expr);
                let (s, take) = self.owned(p);
                self.emit(Ins::Store { fact: fact.as_str().into(), s, take });
            }
            Stmt::If { cond, then, els } => {
                let jump = self.decide(cond);
                self.block(then);
                match els {
                    Some(e) => {
                        let over = self.emit(Ins::Jump { to: u32::MAX });
                        let here = self.here();
                        if let Some(j) = jump {
                            self.patch(j, here);
                        }
                        self.block(e);
                        let end = self.here();
                        self.patch(over, end);
                    }
                    None => {
                        let end = self.here();
                        if let Some(j) = jump {
                            self.patch(j, end);
                        }
                    }
                }
            }
            Stmt::While { cond, body } => {
                // each round checks for cancellation before its condition, as
                // the walker's does: the first round here, the others in the
                // back edge
                self.emit(Ins::Cancel);
                let head = self.here();
                let exit = self.decide(cond);
                self.temps = [0; 3];
                self.loops.push(LoopCtx { breaks: exit.into_iter().collect(), conts: Vec::new(), depth: self.scopes.len() });
                self.block(body);
                let back = self.emit(Ins::Loop { to: head }) as u32;
                let end = self.here();
                let ctx = self.loops.pop().expect("the loop pushed");
                for at in ctx.breaks {
                    match &mut self.code[at] {
                        Ins::Exec { brk, .. } => *brk = end,
                        _ => self.patch(at, end),
                    }
                }
                for at in ctx.conts {
                    match &mut self.code[at] {
                        Ins::Exec { cont, .. } => *cont = back,
                        _ => self.patch(at, back),
                    }
                }
            }
            Stmt::For { var, iter, body, parallel } => {
                if *parallel {
                    return self.exec(s);
                }
                let item_ty = match iter.ty() {
                    Ty::List(t) => *t,
                    _ => Ty::Unknown,
                };
                let src = if let Expr::Call { target: Target::Row(row), args, propagates, ty, .. } = iter
                    && matches!(row.id, "core.range_to" | "core.range_from")
                    && !ty.is_result()
                {
                    let mut regs = Vec::with_capacity(args.len());
                    for a in args {
                        let p = self.expr(a);
                        regs.push(self.reg(p));
                    }
                    ForSrc::Range { row, propagates: *propagates, args: regs.into_boxed_slice() }
                } else {
                    let p = self.expr(iter);
                    let (it, take) = self.owned(p);
                    ForSrc::Items { it, take, ty: iter.ty() }
                };
                // a range whose bounds are int registers counts in int
                // registers (a typed run keeps them bare)
                let counted = match &src {
                    ForSrc::Range { args, .. } => args.iter().all(|r| self.kind(*r) == RKind::Int),
                    ForSrc::Items { .. } => false,
                };
                let ck = if counted { RKind::Int } else { RKind::Boxed };
                let ctr = self.hidden(ck);
                let hi = self.hidden(ck);
                let items = self.hidden(RKind::Boxed);
                let var_kind = match &src {
                    ForSrc::Range { .. } => ck,
                    ForSrc::Items { .. } => RKind::of(&item_ty),
                };
                // the loop's own scope: the variable and the body's bindings,
                // as the walker binds them (one scope; a round's bindings
                // dropped after it)
                self.scopes.push(Vec::new());
                let plain = matches!(src, ForSrc::Range { .. }) || scalar(&item_ty);
                let v = self.fresh(var, plain, var_kind);
                let start = self.emit(Ins::ForStart { var: v, ctr, hi, items, src: Box::new(src), end: u32::MAX });
                let body_at = self.here();
                self.temps = [0; 3];
                // (break and continue are a while loop's: a `for` refuses them)
                self.loops.push(LoopCtx { breaks: Vec::new(), conts: Vec::new(), depth: self.scopes.len() });
                self.scopes.push(Vec::new());
                self.block_stmts(body);
                let depth = self.scopes.len() - 1;
                self.clears_to(depth);
                self.scopes.pop();
                self.loops.pop();
                self.emit(Ins::ForNext { var: v, ctr, hi, items, body: body_at });
                let end = self.here();
                self.patch(start, end);
                self.emit(Ins::Clear { d: items });
                let depth = self.scopes.len() - 1;
                self.clears_to(depth);
                self.scopes.pop();
            }
            Stmt::Return(e) => {
                let s = e.as_ref().map(|e| {
                    let p = self.expr(e);
                    self.owned(p)
                });
                self.emit(Ins::Return { s });
            }
            Stmt::Break | Stmt::Continue => {
                let Some(depth) = self.loops.last().map(|l| l.depth) else {
                    return self.exec(s);
                };
                self.clears_to(depth);
                if matches!(s, Stmt::Break) {
                    let at = self.emit(Ins::Jump { to: u32::MAX });
                    if let Some(l) = self.loops.last_mut() {
                        l.breaks.push(at);
                    }
                } else {
                    let at = self.emit(Ins::Jump { to: u32::MAX });
                    if let Some(l) = self.loops.last_mut() {
                        l.conts.push(at);
                    }
                }
            }
            Stmt::Do(e) => {
                let p = self.expr(e);
                let s = self.reg(p);
                self.emit(Ins::Do { s });
            }
            Stmt::Propagate { values, at } => {
                let mut regs = Vec::with_capacity(values.len());
                for v in values {
                    match self.resolve(v) {
                        Some(r) => regs.push(r),
                        None => return self.exec(s),
                    }
                }
                self.emit(Ins::Propagate { regs: regs.into_boxed_slice(), at: Box::new(at.clone()) });
            }
        }
    }

    /// A decision on a condition: the jump taken when it is false (patched
    /// by the caller), none for a condition written `true`. Two ints
    /// compared decide without a truth in between.
    fn decide(&mut self, cond: &Expr) -> Option<usize> {
        match cond {
            Expr::Const(Const::Bool(true), _) => return None,
            Expr::Const(Const::Bool(false), _) => return Some(self.emit(Ins::Jump { to: u32::MAX })),
            _ => {}
        }
        if let Expr::Binary { op, l, r, .. } = cond
            && l.ty() == Ty::Int
            && r.ty() == Ty::Int
            && let Ok(bop) = binop_of(op)
            && let Some(op) = IntOp::of(&bop)
            && op.compares()
        {
            let (a, b) = self.two_opnds(l, r);
            return Some(self.emit(Ins::JumpUnlessInt { op, a, b, to: u32::MAX }));
        }
        let p = self.expr(cond);
        let c = self.reg(p);
        Some(self.emit(Ins::JumpUnless { c, to: u32::MAX }))
    }

    /// A value placed into register `d` (a local's, or the destination).
    fn put(&mut self, p: Place, d: R) {
        match p {
            Place::Slot(s) if s == d => {}
            Place::Slot(s) => {
                self.emit(Ins::Copy { d, s });
            }
            Place::Temp(t) if t == d => {}
            Place::Temp(t) => {
                self.emit(Ins::Take { d, s: t });
            }
            Place::IntK(n) => {
                self.emit(Ins::Const { d, v: CrowValue::ok(Value::Int(n)) });
            }
        }
    }

    /// A register holding the value (an int constant written into a
    /// temporary).
    fn reg(&mut self, p: Place) -> R {
        match p {
            Place::Slot(r) | Place::Temp(r) => r,
            Place::IntK(n) => {
                let t = self.temp(RKind::Int);
                self.emit(Ins::Const { d: t, v: CrowValue::ok(Value::Int(n)) });
                t
            }
        }
    }

    /// A register and whether its value may be taken (a temporary's is).
    fn owned(&mut self, p: Place) -> (R, bool) {
        match p {
            Place::Slot(r) => (r, false),
            Place::Temp(r) => (r, true),
            Place::IntK(_) => (self.reg(p), true),
        }
    }

    fn opnd(&mut self, e: &Expr) -> Opnd {
        match self.expr(e) {
            Place::IntK(n) => Opnd::K(n),
            Place::Slot(r) | Place::Temp(r) => Opnd::R(r),
        }
    }

    /// Two operands, the left read before the right is computed: a local
    /// the right moves (or a statement inside it could assign) is copied
    /// first, as the walker reads the left's value before the right runs.
    fn two_opnds(&mut self, l: &Expr, r: &Expr) -> (Opnd, Opnd) {
        let a = self.opnd(l);
        let a = self.kept(a, std::slice::from_ref(r));
        let b = self.opnd(r);
        (a, b)
    }

    fn two_regs(&mut self, l: &Expr, r: &Expr) -> (R, R) {
        let pa = self.expr(l);
        let pa = self.kept_place(pa, std::slice::from_ref(r));
        let a = self.reg(pa);
        let pb = self.expr(r);
        let b = self.reg(pb);
        (a, b)
    }

    fn kept(&mut self, o: Opnd, later: &[Expr]) -> Opnd {
        match o {
            Opnd::R(r) if self.is_slot(r) && later.iter().any(may_write) => {
                let t = self.temp(self.kind(r).pool());
                self.emit(Ins::Copy { d: t, s: r });
                Opnd::R(t)
            }
            o => o,
        }
    }

    fn kept_place(&mut self, p: Place, later: &[Expr]) -> Place {
        match p {
            Place::Slot(r) if later.iter().any(may_write) => {
                let t = self.temp(self.kind(r).pool());
                self.emit(Ins::Copy { d: t, s: r });
                Place::Temp(t)
            }
            p => p,
        }
    }

    fn is_slot(&self, r: R) -> bool {
        r < self.temp_base[0]
    }

    /// Where an expression's value is: a local's register as it is, an int
    /// constant as written, anything else computed into a temporary.
    fn expr(&mut self, e: &Expr) -> Place {
        match e {
            Expr::Const(Const::Int(n), _) => Place::IntK(*n),
            Expr::Local(v, _) => match self.resolve(v) {
                Some(r) => Place::Slot(r),
                None => {
                    let t = self.temp(RKind::of(&e.ty()));
                    self.eval(e, t);
                    Place::Temp(t)
                }
            },
            // a payload the lowering's check made safe: the value itself
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => self.expr(expr),
            _ => {
                let t = self.temp(RKind::of(&e.ty()));
                self.compute(e, t);
                Place::Temp(t)
            }
        }
    }

    /// An expression's value for register `d`: computed straight into it
    /// when the expression computes, else where it already is.
    fn expr_place(&mut self, e: &Expr, d: R) -> Place {
        match e {
            Expr::Const(Const::Int(n), _) => Place::IntK(*n),
            Expr::Local(..) => self.expr(e),
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => self.expr_place(expr, d),
            _ => {
                self.compute(e, d);
                Place::Temp(d)
            }
        }
    }

    /// An expression computed into register `d`.
    fn compute(&mut self, e: &Expr, d: R) {
        match e {
            Expr::Const(c, _) => {
                let v = match c {
                    Const::Int(n) => CrowValue::ok(Value::Int(*n)),
                    Const::Float(f) => CrowValue::ok(Value::Float(*f)),
                    Const::Bool(b) => CrowValue::ok(Value::Bool(*b)),
                    Const::Str(s) => CrowValue::ok(Value::Str(s.clone())),
                    Const::Unit => CrowValue::done(),
                };
                self.emit(Ins::Const { d, v });
            }
            Expr::Local(v, _) => match self.resolve(v) {
                Some(s) => {
                    self.emit(Ins::Copy { d, s });
                }
                None => self.eval(e, d),
            },
            Expr::Move(v, _) => match self.resolve(v) {
                Some(s) => {
                    self.emit(Ins::Take { d, s });
                }
                None => self.eval(e, d),
            },
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, .. } => {
                let p = self.expr_place(expr, d);
                self.put(p, d);
            }
            Expr::Convert { how: Convert::IntToFloat, expr, .. } => {
                let p = self.expr(expr);
                let s = self.reg(p);
                self.emit(Ins::ToFloat { d, s });
            }
            Expr::Binary { op, l, r, .. } => {
                let Ok(bop) = binop_of(op) else {
                    return self.eval(e, d);
                };
                let (lt, rt) = (l.ty(), r.ty());
                if lt == Ty::Int && rt == Ty::Int && let Some(op) = IntOp::of(&bop) {
                    let (a, b) = self.two_opnds(l, r);
                    self.emit(Ins::Int { op, d, a, b });
                } else if lt == Ty::Float && rt == Ty::Float && let Some(op) = FloatOp::of(&bop) {
                    let (a, b) = self.two_regs(l, r);
                    self.emit(Ins::Float { op, d, a, b });
                } else if lt == Ty::Bool && rt == Ty::Bool && matches!(bop, BinOp::And | BinOp::Or) {
                    let (a, b) = self.two_regs(l, r);
                    self.emit(Ins::Logic { and: matches!(bop, BinOp::And), d, a, b });
                } else {
                    let (a, b) = self.two_regs(l, r);
                    self.emit(Ins::Bin { op: bop, d, a, b });
                }
            }
            Expr::Unary { op, expr, .. } => {
                let neg = match op.as_str() {
                    "neg" => true,
                    "not" => false,
                    _ => return self.eval(e, d),
                };
                let p = self.expr(expr);
                let s = self.reg(p);
                self.emit(if neg { Ins::Neg { d, s } } else { Ins::Not { d, s } });
            }
            Expr::Index { base, indices, ty } => {
                if let [index] = indices.as_slice()
                    && let Expr::Local(v, _) = unwrapped(base)
                    && let Some(b) = self.resolve(v)
                    && pure_index(index)
                {
                    let column = matches!(base.ty_ref(), Some(Ty::Column(_)));
                    let env = self.env();
                    let idx = self.opnd(index);
                    self.emit(Ins::Index { d, base: b, idx, how: Box::new(IndexHow { ty: ty.clone(), column, ir: e.clone(), env }) });
                    return;
                }
                self.eval(e, d)
            }
            Expr::Call { target, args, propagates, .. } => {
                if is_len(target)
                    && args.len() == 1
                    && let Expr::Local(v, _) = unwrapped(&args[0])
                    && let Some(b) = self.resolve(v)
                {
                    self.emit(Ins::Len { d, base: b, how: Box::new((target.clone(), e.clone(), self.env())) });
                    return;
                }
                // the shapes the walker's own arms give a meaning of their
                // own: an assertion's message, a bound input in a view, a
                // length read in place
                let own = match target {
                    Target::Runtime(name) => matches!(
                        name.rsplit('.').next().unwrap_or(name),
                        "assert" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs" | "select" | "len"
                    ),
                    _ => is_len(target),
                };
                if own {
                    return self.eval(e, d);
                }
                let mut regs = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    let p = self.expr(a);
                    let p = self.kept_place(p, &args[i + 1..]);
                    regs.push(self.owned(p));
                }
                self.emit(Ins::Call { d, target: Box::new(target.clone()), args: regs.into_boxed_slice(), propagates: *propagates });
            }
            _ => self.eval(e, d),
        }
    }
}

/// An assignment whose value is a call taking the variable moved as its
/// first argument: the walker's in-place path decides it.
fn moves_first(var: &str, expr: &Expr) -> bool {
    let Expr::Call { args, .. } = expr else { return false };
    matches!(args.first().map(unwrapped), Some(Expr::Move(v, _)) if v == var)
}

/// An index the in-place read may evaluate before reading the list, and
/// the walker evaluate again when it decides the case: a local, a constant,
/// or an operator over those (nothing moved, nothing called).
fn pure_index(e: &Expr) -> bool {
    match e {
        Expr::Local(..) | Expr::Const(..) => true,
        Expr::Binary { l, r, .. } => pure_index(l) && pure_index(r),
        Expr::Unary { expr, .. } => pure_index(expr),
        Expr::Convert { how: Convert::Unwrap | Convert::Known | Convert::IntToFloat, expr, .. } => pure_index(expr),
        _ => false,
    }
}

/// Whether evaluating an expression could change a local: it moves one, or
/// holds a loop the walker runs (whose body assigns).
fn may_write(e: &Expr) -> bool {
    let mut w = false;
    crate::ir::visit_expr(e, &mut |x| w |= matches!(x, Expr::Move(..) | Expr::ForExpr { .. }));
    w
}

/// A value read from a register or a list: a number copied in place,
/// anything else cloned (the generic clone is a call and a match over every
/// kind; a third of a tight loop's samples had been in it, copying ints).
#[inline(always)]
fn copied(v: &CrowValue) -> CrowValue {
    match &v.value {
        Value::Int(n) if v.is_ok() => CrowValue::ok(Value::Int(*n)),
        Value::Float(f) if v.is_ok() => CrowValue::ok(Value::Float(*f)),
        Value::Bool(b) if v.is_ok() => CrowValue::ok(Value::Bool(*b)),
        _ => v.clone(),
    }
}

#[inline(always)]
fn int_at(regs: &[CrowValue], o: Opnd) -> Option<i64> {
    match o {
        Opnd::K(n) => Some(n),
        Opnd::R(r) => regs[r as usize].plain_int(),
    }
}

#[inline(always)]
fn value_at(regs: &[CrowValue], o: Opnd) -> CrowValue {
    match o {
        Opnd::K(n) => CrowValue::ok(Value::Int(n)),
        Opnd::R(r) => copied(&regs[r as usize]),
    }
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

    /// The walker run on an IR node of a prepared body: the registers lent
    /// to the frame, the names in scope there resolving to them.
    fn lend<T>(&mut self, regs: &mut Vec<CrowValue>, env: &Env, run: impl FnOnce(&mut Self) -> T) -> T {
        if let Some(frame) = self.frames.last_mut() {
            std::mem::swap(&mut frame.slots, regs);
            frame.env = Some(env.clone());
        }
        let r = run(self);
        if let Some(frame) = self.frames.last_mut() {
            frame.env = None;
            std::mem::swap(&mut frame.slots, regs);
        }
        r
    }

    #[inline(always)]
    fn line_of(&mut self, p: &Prepared, pc: usize) {
        if let Some(&n) = p.lines.get(pc)
            && n != 0
        {
            self.line = n;
        }
    }

    /// `xs[i] = new` where `xs` lies: a list's item by an int set in place
    /// (a plain number into a plain number's payload), anything else by the
    /// walker's own path; a value that failed has no items to set.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn store_item(&mut self, p: &Prepared, at_pc: usize, regs: &mut [CrowValue], base: R, idx: Opnd, at: Option<i64>, new: CrowValue, path: &[PathStep]) -> Result<(), String> {
        let root = &mut regs[base as usize];
        if root.is_error() {
            let why = format!("cannot set an item of an error value — {}", root.reason());
            self.line_of(p, at_pc);
            return Err(why);
        }
        if let (Value::Array(items), Some(n)) = (&mut root.value, at) {
            let k = crow_runtime::stops::item(n, items.len())?;
            items[k].assign(new);
            return Ok(());
        }
        let at = match idx {
            Opnd::K(n) => CrowValue::ok(Value::Int(n)),
            Opnd::R(r) => copied(&regs[r as usize]),
        };
        set_path(&mut regs[base as usize].value, path, &[at], new)
    }

    /// A prepared body run: its registers, its parameters' values in theirs.
    pub(super) fn run_prepared(&mut self, p: &Prepared, args: Vec<CrowValue>) -> Result<Flow, String> {
        let mut regs = vec![CrowValue::done(); p.n_regs];
        for (v, r) in args.into_iter().zip(p.params.iter()) {
            regs[*r as usize] = v;
        }
        if let Some(tc) = &p.typed {
            let mut bare = Bare { ints: vec![0i64; p.n_regs], floats: vec![0f64; p.n_regs], lists: vec![Vec::new(); p.n_regs] };
            // each typed parameter a plain number of its kind (a list of
            // plain ints) — or the boxed program runs (a dual number a
            // function `grad` differentiates computes with, a value the IR
            // could not see)
            for r in p.params.iter() {
                if !unbox_reg(&tc.kinds, &regs, &mut bare, *r) {
                    return self.run_code(p, &mut regs);
                }
            }
            return self.run_typed(p, tc, &mut regs, &mut bare);
        }
        self.run_code(p, &mut regs)
    }

    fn run_code(&mut self, p: &Prepared, regs: &mut Vec<CrowValue>) -> Result<Flow, String> {
        self.run_code_from(p, regs, 0)
    }

    /// The boxed program run from `start`: its first instruction, or where
    /// a typed run handed over.
    pub(super) fn run_code_from(&mut self, p: &Prepared, regs: &mut Vec<CrowValue>, start: usize) -> Result<Flow, String> {
        let code = &p.code[..];
        let mut pc = start;
        let mut out = None;
        loop {
            // (`DONE` is past every program's end: the one bounds check ends
            // the body with its outcome, or at its end with nothing)
            let Some(ins) = code.get(pc) else {
                return out.take().unwrap_or(Ok(Flow::Next));
            };
            pc = self.step(p, pc, ins, regs, &mut out);
        }
    }

    /// One instruction of the boxed program run: the position after it, or
    /// `DONE` with the body's outcome in `out` (a return, a refusal, a
    /// stop) — a position in a register, the outcome only when there is one.
    #[inline(always)]
    fn step(&mut self, p: &Prepared, pc: usize, ins: &Ins, regs: &mut Vec<CrowValue>, out: &mut Option<Result<Flow, String>>) -> usize {
        // a stop names the line of the instruction it happened at
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.line_of(p, pc);
                        *out = Some(Err(e));
                        return DONE;
                    }
                }
            };
        }
            match ins {
                Ins::Const { d, v } => regs[*d as usize].assign(copied(v)),
                Ins::Copy { d, s } => {
                    let v = copied(&regs[*s as usize]);
                    regs[*d as usize].assign(v);
                }
                Ins::Take { d, s } => {
                    let v = std::mem::replace(&mut regs[*s as usize], CrowValue::done());
                    regs[*d as usize].assign(v);
                }
                Ins::Clear { d } => regs[*d as usize] = CrowValue::done(),
                Ins::Int { op, d, a, b } => {
                    if let (Some(x), Some(y)) = (int_at(regs, *a), int_at(regs, *b)) {
                        // the int written into the register's payload, no
                        // value built between
                        if op.compares() {
                            regs[*d as usize].set_bool(op.holds(x, y));
                        } else {
                            let n = t!(op.arith(x, y));
                            regs[*d as usize].set_int(n);
                        }
                    } else {
                        self.line_of(p, pc);
                        let (x, y) = (value_at(regs, *a), value_at(regs, *b));
                        regs[*d as usize].assign(t!(crate::ops::binop(&x, &op.general(), &y)));
                    }
                }
                Ins::Float { op, d, a, b } => {
                    let (x, y) = (&regs[*a as usize], &regs[*b as usize]);
                    if let (Some(x), Some(y)) = (x.plain_float(), y.plain_float()) {
                        regs[*d as usize].set_plain(op.apply(x, y));
                    } else {
                        self.line_of(p, pc);
                        let v = t!(crate::ops::binop(x, &op.general(), y));
                        regs[*d as usize].assign(v);
                    }
                }
                Ins::Logic { and, d, a, b } => {
                    let (x, y) = (&regs[*a as usize], &regs[*b as usize]);
                    if let (Some(x), Some(y)) = (x.plain_bool(), y.plain_bool()) {
                        regs[*d as usize].set_plain(Value::Bool(if *and { x && y } else { x || y }));
                    } else {
                        self.line_of(p, pc);
                        let op = if *and { BinOp::And } else { BinOp::Or };
                        let v = t!(crate::ops::binop(x, &op, y));
                        regs[*d as usize].assign(v);
                    }
                }
                Ins::Bin { op, d, a, b } => {
                    self.line_of(p, pc);
                    let v = t!(crate::ops::binop(&regs[*a as usize], op, &regs[*b as usize]));
                    regs[*d as usize].assign(v);
                }
                Ins::Not { d, s } => {
                    if let Some(b) = regs[*s as usize].plain_bool() {
                        regs[*d as usize].set_plain(Value::Bool(!b));
                    } else {
                        let v = t!(crate::ops::unaryop(&UnaryOp::Not, &regs[*s as usize]));
                        regs[*d as usize].assign(v);
                    }
                }
                Ins::Neg { d, s } => {
                    let x = &regs[*s as usize];
                    let v = if let Some(n) = x.plain_int() {
                        CrowValue::ok(Value::Int(t!(crow_runtime::stops::neg(n))))
                    } else if let Some(f) = x.plain_float() {
                        CrowValue::ok(Value::Float(-f))
                    } else {
                        t!(crate::ops::unaryop(&UnaryOp::Neg, x))
                    };
                    regs[*d as usize].assign(v);
                }
                Ins::ToFloat { d, s } => {
                    let x = &regs[*s as usize];
                    let v = match x.plain_int() {
                        Some(n) => CrowValue::ok(Value::Float(n as f64)),
                        None => x.clone(),
                    };
                    regs[*d as usize].assign(v);
                }
                Ins::Jump { to } => return *to as usize,
                Ins::Cancel => t!(self.check_cancel()),
                Ins::Loop { to } => {
                    t!(self.check_cancel());
                    return *to as usize;
                }
                Ins::JumpUnless { c, to } => {
                    let v = &regs[*c as usize];
                    if let Some(b) = v.plain_bool() {
                        if !b {
                            return *to as usize;
                        }
                        return pc + 1;
                    }
                    if v.is_error() {
                        let v = v.clone();
                        self.line_of(p, pc);
                        { *out = Some(self.refuse(v, &Decision::Branch)); return DONE; }
                    }
                    if !crate::ops::is_truthy(v) {
                        return *to as usize;
                    }
                }
                Ins::JumpUnlessInt { op, a, b, to } => {
                    if let (Some(x), Some(y)) = (int_at(regs, *a), int_at(regs, *b)) {
                        if !op.holds(x, y) {
                            return *to as usize;
                        }
                    } else {
                        self.line_of(p, pc);
                        let (x, y) = (value_at(regs, *a), value_at(regs, *b));
                        let c = t!(crate::ops::binop(&x, &op.general(), &y));
                        if c.is_error() {
                            { *out = Some(self.refuse(c, &Decision::Branch)); return DONE; }
                        }
                        if !crate::ops::is_truthy(&c) {
                            return *to as usize;
                        }
                    }
                }
                Ins::ForStart { var, ctr, hi, items, src, end } => {
                    t!(self.check_cancel());
                    let list = match &**src {
                        ForSrc::Range { row, propagates, args } => {
                            let bound = |r: &R| regs[*r as usize].plain_int();
                            let bounds = match &args[..] {
                                [n] => bound(n).map(|h| (0, h)),
                                [a, b] => bound(a).zip(bound(b)),
                                _ => None,
                            };
                            if let Some((lo, h)) = bounds {
                                regs[*ctr as usize] = CrowValue::ok(Value::Int(lo));
                                regs[*hi as usize] = CrowValue::ok(Value::Int(h));
                                regs[*items as usize] = CrowValue::done();
                                if lo >= h {
                                    return *end as usize;
                                } else {
                                    regs[*var as usize].set_int(lo);
                                }
                                return pc + 1;
                            }
                            // what the call would have answered from these values
                            self.line_of(p, pc);
                            let vals: Vec<CrowValue> = args.iter().map(|r| copied(&regs[*r as usize])).collect();
                            let it = match (*propagates, self.first_error(&vals)) {
                                (true, Some(err)) => err,
                                _ => t!(self.row_call(row, vals)),
                            };
                            // a list that failed is refused like a condition
                            if it.is_error() {
                                { *out = Some(self.refuse(it, &Decision::Loop)); return DONE; }
                            }
                            t!(Self::walked_items_of(&it, &Ty::List(Box::new(Ty::Int))))
                        }
                        ForSrc::Items { it, take, ty } => {
                            // walked where it lies (the walker had copied the
                            // list, then its items)
                            let v = &regs[*it as usize];
                            if v.is_error() {
                                let v = v.clone();
                                self.line_of(p, pc);
                                { *out = Some(self.refuse(v, &Decision::Loop)); return DONE; }
                            }
                            let list = t!(Self::walked_items_of(v, ty));
                            if *take {
                                regs[*it as usize] = CrowValue::done();
                            }
                            list
                        }
                    };
                    let mut list = list;
                    let n = list.len();
                    if n == 0 {
                        regs[*items as usize] = CrowValue::done();
                        return *end as usize;
                    }
                    let first = std::mem::replace(&mut list[0], CrowValue::done());
                    regs[*ctr as usize] = CrowValue::ok(Value::Int(0));
                    regs[*hi as usize] = CrowValue::ok(Value::Int(n as i64));
                    regs[*items as usize] = CrowValue::ok(Value::Array(list));
                    regs[*var as usize].assign(first);
                }
                Ins::ForNext { var, ctr, hi, items, body } => {
                    let c = regs[*ctr as usize].plain_int().unwrap_or(i64::MAX - 1) + 1;
                    let h = regs[*hi as usize].plain_int().unwrap_or(0);
                    if c < h {
                        t!(self.check_cancel());
                        regs[*ctr as usize].set_int(c);
                        let v = match &mut regs[*items as usize].value {
                            Value::Array(list) => std::mem::replace(&mut list[c as usize], CrowValue::done()),
                            _ => CrowValue::ok(Value::Int(c)),
                        };
                        regs[*var as usize].assign(v);
                        return *body as usize;
                    }
                }
                Ins::Index { d, base, idx, how } => {
                    let at = int_at(regs, *idx);
                    let b = &regs[*base as usize];
                    if b.is_ok() {
                        if let (Value::Array(items), Some(n)) = (&b.value, at) {
                            let k = t!(crow_runtime::stops::item(n, items.len()));
                            let cell = copied(&items[k]);
                            regs[*d as usize].assign(if how.column { placeholder(cell, &how.ty) } else { cell });
                            return pc + 1;
                        }
                        if matches!(b.value, Value::NumArray(_) | Value::Map(_) | Value::IntMap(_) | Value::Str(_) | Value::Frame(_)) {
                            let at = value_at(regs, *idx);
                            let cell = t!(Self::index_value(&regs[*base as usize], vec![at], &how.ty));
                            regs[*d as usize].assign(if how.column { placeholder(cell, &how.ty) } else { cell });
                            return pc + 1;
                        }
                    }
                    // a failed local, or a kind the in-place read does not
                    // take: the walker decides, as it always has
                    self.line_of(p, pc);
                    let v = t!(self.lend(regs, &how.env, |it| it.eval(&how.ir)));
                    regs[*d as usize].assign(v);
                }
                Ins::SetIndex { base, idx, v, take, path } => {
                    let new = if *take { std::mem::replace(&mut regs[*v as usize], CrowValue::done()) } else { copied(&regs[*v as usize]) };
                    let at = int_at(regs, *idx);
                    t!(self.store_item(p, pc, regs, *base, *idx, at, new, path));
                }
                Ins::InPlace { d, row, rest, propagates } => {
                    let mut vals = Vec::with_capacity(1 + rest.len());
                    vals.push(CrowValue::done());
                    for (r, take) in rest.iter() {
                        vals.push(if *take { std::mem::replace(&mut regs[*r as usize], CrowValue::done()) } else { copied(&regs[*r as usize]) });
                    }
                    // the walker grows the collection in place only when
                    // nothing failed; otherwise its general call decides
                    let clean = !regs[*d as usize].is_error() && !vals[1..].iter().any(|v| v.is_error());
                    vals[0] = std::mem::replace(&mut regs[*d as usize], CrowValue::done());
                    self.line_of(p, pc);
                    let out = if clean {
                        let trace = if matches!(row.fails, crate::table::Fails::Value(_)) { self.trace_entry() } else { String::new() };
                        t!((row.take)(vals, &trace))
                    } else if *propagates && let Some(err) = self.first_error(&vals) {
                        err
                    } else {
                        t!(self.row_call(row, vals))
                    };
                    regs[*d as usize].assign(out);
                }
                Ins::Push { d, v, take, row, propagates } => {
                    let item = if *take { std::mem::replace(&mut regs[*v as usize], CrowValue::done()) } else { copied(&regs[*v as usize]) };
                    let list = &mut regs[*d as usize];
                    if !item.is_error()
                        && list.is_ok()
                        && let Value::Array(items) = &mut list.value
                    {
                        items.push(item);
                        return pc + 1;
                    }
                    // a failed item or list: the general call decides, as the
                    // walker's does
                    let vals = vec![std::mem::replace(&mut regs[*d as usize], CrowValue::done()), item];
                    self.line_of(p, pc);
                    let out = if *propagates && let Some(err) = self.first_error(&vals) { err } else { t!(self.row_call(row, vals)) };
                    regs[*d as usize].assign(out);
                }
                Ins::CopyItem { dst, di, src, si, how } => {
                    // the value first, as the walker reads it
                    let read = {
                        let b = &regs[*src as usize];
                        match (&b.value, int_at(regs, *si)) {
                            (Value::Array(items), Some(n)) if b.is_ok() => Some(crow_runtime::stops::item(n, items.len()).map(|k| copied(&items[k]))),
                            _ => None,
                        }
                    };
                    let new = match read {
                        Some(r) => t!(r),
                        None => {
                            self.line_of(p, pc);
                            t!(self.lend(regs, &how.env, |it| it.eval(&how.ir)))
                        }
                    };
                    let at = int_at(regs, *di);
                    t!(self.store_item(p, pc, regs, *dst, *di, at, new, &how.path));
                }
                Ins::Len { d, base, how } => {
                    let (target, ir, env) = &**how;
                    if self.registry.has_function("len") {
                        self.line_of(p, pc);
                        let v = t!(self.lend(regs, env, |it| it.eval(ir)));
                        regs[*d as usize].assign(v);
                        return pc + 1;
                    }
                    self.line_of(p, pc);
                    let trace = match target {
                        Target::Row(row) if !matches!(row.fails, crate::table::Fails::Value(_)) => String::new(),
                        _ => self.trace_entry(),
                    };
                    let b = &regs[*base as usize];
                    let v = if b.is_error() { b.clone() } else { t!(Self::len_of(target, b, &trace)) };
                    regs[*d as usize].assign(v);
                }
                Ins::Call { d, target, args, propagates } => {
                    let mut vals = Vec::with_capacity(args.len());
                    for (r, take) in args.iter() {
                        vals.push(if *take { std::mem::replace(&mut regs[*r as usize], CrowValue::done()) } else { copied(&regs[*r as usize]) });
                    }
                    self.line_of(p, pc);
                    if *propagates && let Some(err) = self.first_error(&vals) {
                        regs[*d as usize].assign(err);
                        return pc + 1;
                    }
                    let v = match &**target {
                        Target::Row(row) => t!(self.row_call(row, vals)),
                        Target::Runtime(path) => t!(self.runtime_call(path, vals)),
                        Target::User(name) => t!(self.call_user(name, vals)),
                        Target::Module(m, f) => t!(self.module_call(m, f, vals)),
                    };
                    regs[*d as usize].assign(v);
                }
                Ins::Store { fact, s, take } => {
                    let v = if *take { std::mem::replace(&mut regs[*s as usize], CrowValue::done()) } else { regs[*s as usize].clone() };
                    // a fact never holds an error: the handler stops instead
                    if v.is_error() && self.held.is_some() {
                        self.line_of(p, pc);
                        t!(self.refuse(v.clone(), &Decision::Branch));
                    }
                    self.set_fact(fact, v);
                }
                Ins::Do { s } => {
                    let v = std::mem::replace(&mut regs[*s as usize], CrowValue::done());
                    self.last_value = Some(v);
                }
                Ins::Propagate { regs: rs, at } => {
                    for r in rs.iter() {
                        if regs[*r as usize].is_error() {
                            let v = regs[*r as usize].clone();
                            self.line_of(p, pc);
                            { *out = Some(self.refuse(v, at)); return DONE; }
                        }
                    }
                }
                Ins::Return { s } => {
                    let v = match s {
                        Some((r, true)) => std::mem::replace(&mut regs[*r as usize], CrowValue::done()),
                        Some((r, false)) => regs[*r as usize].clone(),
                        None => CrowValue::done(),
                    };
                    { *out = Some(Ok(Flow::Return(v))); return DONE; }
                }
                Ins::Eval { d, ir } => {
                    self.line_of(p, pc);
                    let (e, env) = &**ir;
                    let v = t!(self.lend(regs, env, |it| it.eval(e)));
                    regs[*d as usize].assign(v);
                }
                Ins::Exec { ir, brk, cont } => {
                    self.line_of(p, pc);
                    let (s, env) = &**ir;
                    match t!(self.lend(regs, env, |it| it.exec_stmt(s))) {
                        Flow::Next => {}
                        Flow::Return(v) => { *out = Some(Ok(Flow::Return(v))); return DONE; }
                        Flow::Break if *brk != u32::MAX => return *brk as usize,
                        Flow::Continue if *cont != u32::MAX => return *cont as usize,
                        // (no loop of this body's own to leave: the walker's
                        // flow goes up as it would have)
                        other => { *out = Some(Ok(other)); return DONE; }
                    }
                }
            }
        pc + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lowered_fn(src: &str, name: &str) -> Fn {
        let stmts = crate::build(src).expect("parses");
        let program = crate::ir_interp::prepare(&stmts, "t").expect("lowers").0;
        program.parts[0].fns.iter().find(|f| f.name == name).expect("lowered").clone()
    }

    /// fannkuch's flip loop compiles to typed instructions only — a fused
    /// int decision, int arithmetic, a list's items read and set where they
    /// lie — with no walker fallback in it: a silent fallback is a slow
    /// interpreter that passes every other test.
    #[test]
    fn a_tight_loop_compiles_to_typed_instructions() {
        let f = lowered_fn(
            "fn flip[perm, k]\n    i = 0\n    j = k\n    while i lt j\n        t = perm[i]\n        perm[i] = perm[j]\n        perm[j] = t\n        i = i + 1\n        j = j - 1\n    return perm\nlog[flip[range[5], 4]]\n",
            "flip",
        );
        let shown = format!("{:?}", prepare(&f).code);
        assert!(shown.contains("JumpUnlessInt"), "{}", shown);
        assert!(shown.contains("Int { op: Add") && shown.contains("Int { op: Sub"), "{}", shown);
        assert!(shown.contains("Index {") && shown.contains("SetIndex {"), "{}", shown);
        assert!(!shown.contains("Eval {") && !shown.contains("Exec {"), "{}", shown);
    }

    /// A list of ints the function uses only through list instructions is
    /// kept bare in the typed run (`LIndex`, `LCopy`, `LSet`); one it
    /// answers stays a list of values, converted nowhere in its loop.
    #[test]
    fn a_list_of_ints_is_bare_only_where_list_instructions_use_it() {
        let f = lowered_fn(
            "fn flip[n]\n    perm = range[n]\n    i = 0\n    j = n - 1\n    while i lt j\n        t = perm[i]\n        perm[i] = perm[j]\n        perm[j] = t\n        i = i + 1\n        j = j - 1\n    return perm[0]\nlog[flip[5]]\n",
            "flip",
        );
        let p = prepare(&f);
        let shown = format!("{:?}", p.typed.as_ref().map(|tc| &tc.code));
        assert!(shown.contains("LIndex") && shown.contains("LCopy") && shown.contains("LSet"), "{}", shown);
        let g = lowered_fn("fn keep[n]\n    xs = range[n]\n    xs[0] = 9\n    return xs\nlog[keep[3]]\n", "keep");
        let q = prepare(&g);
        let shown = format!("{:?}", q.typed.as_ref().map(|tc| &tc.code));
        assert!(!shown.contains("LSet"), "{}", shown);
    }

    /// A counted loop over a range is a counted loop, and a nested one
    /// reusing the name binds a register of its own.
    #[test]
    fn a_loop_over_a_range_counts_in_registers_of_its_own() {
        let f = lowered_fn(
            "fn tri[n]\n    s = 0\n    for i in range[n]\n        for i in range[3]\n            s = s + i\n        s = s + i\n    return s\nlog[tri[4]]\n",
            "tri",
        );
        let code = prepare(&f).code;
        let vars: Vec<R> = code.iter().filter_map(|i| match i { Ins::ForStart { var, .. } => Some(*var), _ => None }).collect();
        assert_eq!(vars.len(), 2, "{:?}", code);
        assert_ne!(vars[0], vars[1], "{:?}", code);
        assert!(!format!("{:?}", code).contains("Eval {"), "{:?}", code);
    }
}

// ── The typed program (step 4) ─────────────────────────────────────────
//
// The boxed program's instructions, one for one, with the int and float
// registers kept as bare machine values: an operator, a decision, a counted
// loop and a list's item read or set by an int run without a tag, a status
// or a value built. An instruction with no typed form is the boxed one, its
// typed registers boxed before it and unboxed after it, each checked; a
// value a typed register cannot hold — an item that is no plain int, a call
// answering a dual number — hands the run over to the boxed program at the
// same position (or, after a boxed instruction has run, at the next), with
// every typed register boxed. The positions agree, so the hand-over is a
// jump, and the boxed program finishes the call.

/// An operand of a typed instruction: an int register, kept bare, or a constant.
#[derive(Clone, Copy, Debug)]
enum TOp {
    R(R),
    K(i64),
}

#[inline(always)]
fn iv(ints: &[i64], o: TOp) -> i64 {
    match o {
        TOp::R(r) => ints[r as usize],
        TOp::K(n) => n,
    }
}

#[derive(Debug)]
enum TIns {
    /// The boxed instruction at this position: `pre` boxed before it,
    /// `post` unboxed after it.
    Boxed { pre: Box<[R]>, post: Box<[R]> },
    Nop,
    Jump { to: u32 },
    Loop { to: u32 },
    Cancel,
    IConst { d: R, n: i64 },
    FConst { d: R, x: f64 },
    IMov { d: R, s: R },
    FMov { d: R, s: R },
    IArith { op: IntOp, d: R, a: TOp, b: TOp },
    /// `x + k`, `x - k`, `x + y`, `x - y`: the commonest arithmetic, with
    /// no operand to decode and no operator to choose.
    IAddK { d: R, a: R, k: i64 },
    ISubK { d: R, a: R, k: i64 },
    IAddR { d: R, a: R, b: R },
    ISubR { d: R, a: R, b: R },
    /// Two ints compared into a truth (a boxed register).
    ICmp { op: IntOp, d: R, a: TOp, b: TOp },
    /// Two ints compared deciding a branch: jumps when it is false.
    IJump { op: IntOp, a: TOp, b: TOp, to: u32 },
    /// The same, of two registers and of a register and a constant.
    IJumpR { op: IntOp, a: R, b: R, to: u32 },
    IJumpK { op: IntOp, a: R, k: i64, to: u32 },
    INeg { d: R, s: R },
    FArith { op: FloatOp, d: R, a: R, b: R },
    FCmp { op: FloatOp, d: R, a: R, b: R },
    FNeg { d: R, s: R },
    I2F { d: R, s: R },
    /// A list's (or a tensor's) item by an int, into an int register.
    IIndex { d: R, base: R, idx: TOp },
    FIndex { d: R, base: R, idx: TOp },
    /// An item by an int, into a boxed register (the boxed instruction's
    /// own reads: a list, a tensor, a map, text, a frame).
    XIndex { d: R, base: R, idx: TOp },
    ISet { base: R, idx: TOp, v: R },
    FSet { base: R, idx: TOp, v: R },
    ICopy { dst: R, di: TOp, src: R, si: TOp },
    XLen { d: R, base: R },
    /// An int list's item, set, copied, measured, appended to, dropped.
    LIndex { d: R, base: R, idx: TOp },
    LSet { base: R, idx: TOp, v: R },
    LCopy { dst: R, di: TOp, src: R, si: TOp },
    LLen { d: R, base: R },
    LPush { d: R, v: R },
    LClear { d: R },
    ForCount { var: R, ctr: R, hi: R, lo: Option<TOp>, top: TOp, end: u32 },
    ForNextCount { var: R, ctr: R, hi: R, body: u32 },
}

pub(super) struct Typed {
    code: Vec<TIns>,
    /// Each register's kind in the typed run: the compiler's, with a list
    /// of ints the function uses otherwise than through list instructions
    /// boxed after all.
    kinds: Vec<RKind>,
    /// The int, float and int-list registers: what a hand-over boxes.
    ints: Box<[R]>,
    floats: Box<[R]>,
    lists: Box<[R]>,
}

/// The typed run's bare registers.
pub(super) struct Bare {
    ints: Vec<i64>,
    floats: Vec<f64>,
    lists: Vec<Vec<i64>>,
}

/// A typed register's value into its boxed register.
#[inline(always)]
fn box_reg(kinds: &[RKind], regs: &mut [CrowValue], bare: &Bare, r: R) {
    let i = r as usize;
    match kinds[i] {
        RKind::Int => regs[i].set_int(bare.ints[i]),
        RKind::Float => regs[i].set_float(bare.floats[i]),
        RKind::IntList => regs[i] = CrowValue::ok(Value::Array(bare.lists[i].iter().map(|n| CrowValue::ok(Value::Int(*n))).collect())),
        RKind::Boxed => {}
    }
}

/// A boxed register's value into its typed register, when it is a plain
/// number of that kind (false: it is not, and the run hands over).
#[inline(always)]
fn unbox_reg(kinds: &[RKind], regs: &[CrowValue], bare: &mut Bare, r: R) -> bool {
    let i = r as usize;
    match kinds[i] {
        RKind::Int => match regs[i].plain_int() {
            Some(n) => {
                bare.ints[i] = n;
                true
            }
            None => false,
        },
        RKind::Float => match regs[i].plain_float() {
            Some(x) => {
                bare.floats[i] = x;
                true
            }
            None => false,
        },
        // a list of plain ints, each item read; any other value hands over
        RKind::IntList => match &regs[i].value {
            Value::Array(items) if regs[i].is_ok() => {
                let mut ns = Vec::with_capacity(items.len());
                for it in items {
                    match it.plain_int() {
                        Some(n) => ns.push(n),
                        None => return false,
                    }
                }
                bare.lists[i] = ns;
                true
            }
            _ => false,
        },
        RKind::Boxed => true,
    }
}

/// The registers an instruction reads and writes (an instruction the walker
/// runs reads and writes the locals its IR names).
fn reads_writes(ins: &Ins) -> (Vec<R>, Vec<R>) {
    let rs = |os: &[Opnd]| os.iter().filter_map(|o| if let Opnd::R(r) = o { Some(*r) } else { None }).collect::<Vec<R>>();
    match ins {
        Ins::Const { d, .. } => (vec![], vec![*d]),
        Ins::Copy { d, s } | Ins::Take { d, s } | Ins::Not { d, s } | Ins::Neg { d, s } | Ins::ToFloat { d, s } => (vec![*s], vec![*d]),
        Ins::Clear { d } => (vec![], vec![*d]),
        Ins::Int { d, a, b, .. } => (rs(&[*a, *b]), vec![*d]),
        Ins::Float { d, a, b, .. } | Ins::Logic { d, a, b, .. } | Ins::Bin { d, a, b, .. } => (vec![*a, *b], vec![*d]),
        Ins::Jump { .. } | Ins::Loop { .. } | Ins::Cancel => (vec![], vec![]),
        Ins::JumpUnless { c, .. } => (vec![*c], vec![]),
        Ins::JumpUnlessInt { a, b, .. } => (rs(&[*a, *b]), vec![]),
        Ins::ForStart { var, ctr, hi, items, src, .. } => {
            let reads = match &**src {
                ForSrc::Range { args, .. } => args.to_vec(),
                ForSrc::Items { it, .. } => vec![*it],
            };
            (reads, vec![*var, *ctr, *hi, *items])
        }
        Ins::ForNext { var, ctr, hi, items, .. } => (vec![*ctr, *hi, *items], vec![*var, *ctr, *items]),
        Ins::Index { d, base, idx, .. } => {
            let mut r = rs(&[*idx]);
            r.push(*base);
            (r, vec![*d])
        }
        Ins::CopyItem { dst, di, src, si, .. } => {
            let mut r = rs(&[*di, *si]);
            r.push(*dst);
            r.push(*src);
            (r, vec![*dst])
        }
        Ins::SetIndex { base, idx, v, .. } => {
            let mut r = rs(&[*idx]);
            r.push(*base);
            r.push(*v);
            (r, vec![*base])
        }
        Ins::InPlace { d, rest, .. } => {
            let mut r: Vec<R> = rest.iter().map(|(x, _)| *x).collect();
            r.push(*d);
            (r, vec![*d])
        }
        Ins::Push { d, v, .. } => (vec![*d, *v], vec![*d]),
        Ins::Len { d, base, .. } => (vec![*base], vec![*d]),
        Ins::Call { d, args, .. } => (args.iter().map(|(x, _)| *x).collect(), vec![*d]),
        Ins::Store { s, .. } | Ins::Do { s } => (vec![*s], vec![]),
        Ins::Propagate { regs, .. } => (regs.to_vec(), vec![]),
        Ins::Return { s } => (s.iter().map(|(x, _)| *x).collect(), vec![]),
        Ins::Eval { d, ir } => {
            let named = named_regs_expr(&ir.0, &ir.1);
            (named, vec![*d])
        }
        Ins::Exec { ir, .. } => {
            let named = named_regs_stmt(&ir.0, &ir.1);
            (named.clone(), named)
        }
    }
}

/// The registers of the locals an expression names.
fn named_regs_expr(e: &Expr, env: &Env) -> Vec<R> {
    let mut out = Vec::new();
    crate::ir::visit_expr(e, &mut |x| {
        if let Expr::Local(n, _) | Expr::Move(n, _) = x
            && let Some(r) = env.get(n)
        {
            out.push(*r);
        }
    });
    out
}

/// The registers of the locals a statement names, read or assigned.
fn named_regs_stmt(s: &Stmt, env: &Env) -> Vec<R> {
    let mut out = Vec::new();
    crate::ir::visit_stmt(s, &mut |x| {
        if let Expr::Local(n, _) | Expr::Move(n, _) = x
            && let Some(r) = env.get(n)
        {
            out.push(*r);
        }
    });
    fn assigned(s: &Stmt, env: &Env, out: &mut Vec<R>) {
        match s {
            Stmt::Let { var, .. } | Stmt::Set { var, .. } | Stmt::SetPath { var, .. } => {
                if let Some(r) = env.get(var) {
                    out.push(*r);
                }
            }
            Stmt::If { then, els, .. } => {
                then.iter().for_each(|x| assigned(x, env, out));
                if let Some(e) = els {
                    e.iter().for_each(|x| assigned(x, env, out));
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => body.iter().for_each(|x| assigned(x, env, out)),
            Stmt::Propagate { values, .. } => out.extend(values.iter().filter_map(|v| env.get(v).copied())),
            _ => {}
        }
    }
    assigned(s, env, &mut out);
    out
}

/// Whether an instruction uses an int-list register `r` only as a list
/// instruction does — read and set by an int, copied item to item between
/// two such lists, measured, appended an int, dropped — or only writes it
/// (its value converted once, where it is made). Anything else reads it as
/// a value, and the list stays boxed.
fn list_use_ok(ins: &Ins, r: R, k: &[RKind]) -> bool {
    let int_op = |o: &Opnd| match o {
        Opnd::K(_) => true,
        Opnd::R(x) => k[*x as usize] == RKind::Int,
    };
    let ki = |x: &R| k[*x as usize];
    match ins {
        Ins::Index { d, base, idx, how } if *base == r => !how.column && ki(d) == RKind::Int && int_op(idx) && *d != r,
        Ins::CopyItem { dst, di, src, si, .. } if *dst == r || *src == r => ki(dst) == RKind::IntList && ki(src) == RKind::IntList && int_op(di) && int_op(si),
        Ins::SetIndex { base, idx, v, .. } if *base == r => int_op(idx) && ki(v) == RKind::Int,
        Ins::Len { d, base, .. } if *base == r => ki(d) == RKind::Int,
        Ins::Push { d, v, .. } if *d == r => ki(v) == RKind::Int,
        Ins::Clear { d } if *d == r => true,
        other => {
            let (reads, _) = reads_writes(other);
            !reads.contains(&r)
        }
    }
}

/// The typed program of a prepared body, when it has a typed register.
fn typed(p: &Prepared) -> Option<Typed> {
    // a list of ints stays bare only where every instruction that meets it
    // is a list instruction or makes it (a list a call takes, a loop walks,
    // a fallback names or the body answers is boxed: converting it there
    // would cost a pass over it each time)
    let mut kinds = p.kinds.clone();
    loop {
        let mut demoted = false;
        for r in 0..kinds.len() as R {
            if kinds[r as usize] == RKind::IntList && !p.code.iter().all(|ins| list_use_ok(ins, r, &kinds)) {
                kinds[r as usize] = RKind::Boxed;
                demoted = true;
            }
        }
        if !demoted {
            break;
        }
    }
    let k = &kinds;
    let ints: Vec<R> = (0..k.len() as R).filter(|r| k[*r as usize] == RKind::Int).collect();
    let floats: Vec<R> = (0..k.len() as R).filter(|r| k[*r as usize] == RKind::Float).collect();
    let lists: Vec<R> = (0..k.len() as R).filter(|r| k[*r as usize] == RKind::IntList).collect();
    if ints.is_empty() && floats.is_empty() && lists.is_empty() {
        return None;
    }
    // (the walker reads a lent list's boxed register; no int list is named
    // by a fallback, by the rule above)
    let all: Box<[R]> = ints.iter().chain(floats.iter()).copied().collect();
    let lk = |r: R| k[r as usize] == RKind::IntList;
    let ik = |r: R| k[r as usize] == RKind::Int;
    let fk = |r: R| k[r as usize] == RKind::Float;
    let tk = |r: R| k[r as usize] != RKind::Boxed;
    let top = |o: Opnd| match o {
        Opnd::K(n) => Some(TOp::K(n)),
        Opnd::R(r) if ik(r) => Some(TOp::R(r)),
        Opnd::R(_) => None,
    };
    let regs_of = |os: &[Opnd]| os.iter().filter_map(|o| if let Opnd::R(r) = o { Some(*r) } else { None }).collect::<Vec<R>>();
    let wrap = |reads: &[R], writes: &[R]| TIns::Boxed {
        pre: reads.iter().copied().filter(|r| tk(*r)).collect(),
        post: writes.iter().copied().filter(|r| tk(*r)).collect(),
    };
    // (an instruction that may lend the registers to the walker, which
    // reads and writes locals by name: every int and float register both
    // ways, and an int list it makes converted after it — no int list is
    // read by a fallback, by the rule above)
    let lend = |ins: &Ins| {
        let (_, writes) = reads_writes(ins);
        let mut post = all.to_vec();
        post.extend(writes.into_iter().filter(|r| k[*r as usize] == RKind::IntList));
        TIns::Boxed { pre: all.clone(), post: post.into() }
    };
    let code = p
        .code
        .iter()
        .map(|ins| match ins {
            Ins::Const { d, v } => {
                if ik(*d)
                    && let Some(n) = v.plain_int()
                {
                    TIns::IConst { d: *d, n }
                } else if fk(*d)
                    && let Some(x) = v.plain_float()
                {
                    TIns::FConst { d: *d, x }
                } else {
                    wrap(&[], &[*d])
                }
            }
            Ins::Copy { d, s } | Ins::Take { d, s } => {
                if ik(*d) && ik(*s) {
                    TIns::IMov { d: *d, s: *s }
                } else if fk(*d) && fk(*s) {
                    TIns::FMov { d: *d, s: *s }
                } else {
                    wrap(&[*s], &[*d])
                }
            }
            Ins::Clear { d } => {
                if lk(*d) {
                    TIns::LClear { d: *d }
                } else if tk(*d) {
                    TIns::Nop
                } else {
                    wrap(&[], &[])
                }
            }
            Ins::Int { op, d, a, b } => match (top(*a), top(*b)) {
                (Some(x), Some(y)) if op.compares() && !tk(*d) => TIns::ICmp { op: *op, d: *d, a: x, b: y },
                (Some(TOp::R(x)), Some(TOp::K(k))) if matches!(op, IntOp::Add) && ik(*d) => TIns::IAddK { d: *d, a: x, k },
                (Some(TOp::R(x)), Some(TOp::K(k))) if matches!(op, IntOp::Sub) && ik(*d) => TIns::ISubK { d: *d, a: x, k },
                (Some(TOp::R(x)), Some(TOp::R(y))) if matches!(op, IntOp::Add) && ik(*d) => TIns::IAddR { d: *d, a: x, b: y },
                (Some(TOp::R(x)), Some(TOp::R(y))) if matches!(op, IntOp::Sub) && ik(*d) => TIns::ISubR { d: *d, a: x, b: y },
                (Some(x), Some(y)) if !op.compares() && ik(*d) => TIns::IArith { op: *op, d: *d, a: x, b: y },
                _ => wrap(&regs_of(&[*a, *b]), &[*d]),
            },
            Ins::Float { op, d, a, b } => {
                if fk(*a) && fk(*b) && op.compares() && !tk(*d) {
                    TIns::FCmp { op: *op, d: *d, a: *a, b: *b }
                } else if fk(*a) && fk(*b) && !op.compares() && fk(*d) {
                    TIns::FArith { op: *op, d: *d, a: *a, b: *b }
                } else {
                    wrap(&[*a, *b], &[*d])
                }
            }
            Ins::Logic { d, a, b, .. } | Ins::Bin { d, a, b, .. } => wrap(&[*a, *b], &[*d]),
            Ins::Not { d, s } => wrap(&[*s], &[*d]),
            Ins::Neg { d, s } => {
                if ik(*d) && ik(*s) {
                    TIns::INeg { d: *d, s: *s }
                } else if fk(*d) && fk(*s) {
                    TIns::FNeg { d: *d, s: *s }
                } else {
                    wrap(&[*s], &[*d])
                }
            }
            Ins::ToFloat { d, s } => {
                if fk(*d) && ik(*s) {
                    TIns::I2F { d: *d, s: *s }
                } else {
                    wrap(&[*s], &[*d])
                }
            }
            Ins::Jump { to } => TIns::Jump { to: *to },
            Ins::Loop { to } => TIns::Loop { to: *to },
            Ins::Cancel => TIns::Cancel,
            Ins::JumpUnless { c, .. } => wrap(&[*c], &[]),
            Ins::JumpUnlessInt { op, a, b, to } => match (top(*a), top(*b)) {
                (Some(TOp::R(x)), Some(TOp::R(y))) => TIns::IJumpR { op: *op, a: x, b: y, to: *to },
                (Some(TOp::R(x)), Some(TOp::K(k))) => TIns::IJumpK { op: *op, a: x, k, to: *to },
                (Some(x), Some(y)) => TIns::IJump { op: *op, a: x, b: y, to: *to },
                _ => wrap(&regs_of(&[*a, *b]), &[]),
            },
            Ins::ForStart { var, ctr, hi, src, end, .. } => match &**src {
                ForSrc::Range { args, .. } if ik(*ctr) => match &args[..] {
                    [n] => TIns::ForCount { var: *var, ctr: *ctr, hi: *hi, lo: None, top: TOp::R(*n), end: *end },
                    [a, b] => TIns::ForCount { var: *var, ctr: *ctr, hi: *hi, lo: Some(TOp::R(*a)), top: TOp::R(*b), end: *end },
                    _ => wrap(args, &[*var, *ctr, *hi]),
                },
                ForSrc::Range { args, .. } => wrap(args, &[*var, *ctr, *hi]),
                ForSrc::Items { it, .. } => wrap(&[*it], &[*var]),
            },
            Ins::ForNext { var, ctr, hi, body, .. } => {
                if ik(*ctr) {
                    TIns::ForNextCount { var: *var, ctr: *ctr, hi: *hi, body: *body }
                } else {
                    wrap(&[], &[*var])
                }
            }
            Ins::Index { d, base, idx, how } => match top(*idx) {
                Some(i) if lk(*base) => TIns::LIndex { d: *d, base: *base, idx: i },
                Some(i) if ik(*d) && !how.column => TIns::IIndex { d: *d, base: *base, idx: i },
                Some(i) if fk(*d) && !how.column => TIns::FIndex { d: *d, base: *base, idx: i },
                Some(i) if !tk(*d) => TIns::XIndex { d: *d, base: *base, idx: i },
                _ => lend(ins),
            },
            Ins::CopyItem { dst, di, src, si, .. } => match (top(*di), top(*si)) {
                (Some(a), Some(b)) if lk(*dst) && lk(*src) => TIns::LCopy { dst: *dst, di: a, src: *src, si: b },
                (Some(a), Some(b)) => TIns::ICopy { dst: *dst, di: a, src: *src, si: b },
                _ => lend(ins),
            },
            Ins::SetIndex { base, idx, v, .. } => match top(*idx) {
                Some(i) if lk(*base) => TIns::LSet { base: *base, idx: i, v: *v },
                Some(i) if ik(*v) => TIns::ISet { base: *base, idx: i, v: *v },
                Some(i) if fk(*v) => TIns::FSet { base: *base, idx: i, v: *v },
                _ => {
                    let mut reads = regs_of(&[*idx]);
                    reads.push(*v);
                    wrap(&reads, &[])
                }
            },
            Ins::InPlace { d, rest, .. } => wrap(&rest.iter().map(|(r, _)| *r).collect::<Vec<R>>(), &[*d]),
            Ins::Push { d, v, .. } => {
                if lk(*d) {
                    TIns::LPush { d: *d, v: *v }
                } else {
                    wrap(&[*v], &[*d])
                }
            }
            Ins::Len { d, base, .. } => {
                if lk(*base) {
                    TIns::LLen { d: *d, base: *base }
                } else if ik(*d) {
                    TIns::XLen { d: *d, base: *base }
                } else {
                    lend(ins)
                }
            }
            Ins::Call { d, args, .. } => wrap(&args.iter().map(|(r, _)| *r).collect::<Vec<R>>(), &[*d]),
            Ins::Store { s, .. } | Ins::Do { s } => wrap(&[*s], &[]),
            Ins::Propagate { regs, .. } => wrap(regs, &[]),
            Ins::Return { s } => wrap(&s.iter().map(|(r, _)| *r).collect::<Vec<R>>(), &[]),
            Ins::Eval { .. } | Ins::Exec { .. } => lend(ins),
        })
        .collect();
    Some(Typed { code, kinds, ints: ints.into(), floats: floats.into(), lists: lists.into() })
}

impl IrInterp {
    /// Every typed register boxed, but those in `skip` (a boxed instruction
    /// has just written their boxed registers).
    fn box_typed(&self, tc: &Typed, regs: &mut [CrowValue], bare: &Bare, skip: &[R]) {
        for r in tc.ints.iter().chain(tc.floats.iter()).chain(tc.lists.iter()) {
            if !skip.contains(r) {
                box_reg(&tc.kinds, regs, bare, *r);
            }
        }
    }

    fn run_typed(&mut self, p: &Prepared, tc: &Typed, regs: &mut Vec<CrowValue>, bare: &mut Bare) -> Result<Flow, String> {
        let code = &tc.code[..];
        let mut pc = 0usize;
        let mut out = None;
        // a stop names the line of the instruction it happened at
        macro_rules! stop {
            ($e:expr) => {{
                self.line_of(p, pc);
                return Err($e);
            }};
        }
        // a value the typed form does not take here: the boxed program runs
        // this instruction, and the rest of the call
        macro_rules! hand_over {
            () => {{
                self.box_typed(tc, regs, bare, &[]);
                return self.run_code_from(p, regs, pc);
            }};
        }
        loop {
            let Some(ins) = code.get(pc) else {
                return Ok(Flow::Next);
            };
            match ins {
                TIns::Boxed { pre, post } => {
                    for r in pre.iter() {
                        box_reg(&tc.kinds, regs, bare, *r);
                    }
                    let next = self.step(p, pc, &p.code[pc], regs, &mut out);
                    if next == DONE {
                        return out.take().unwrap_or(Ok(Flow::Next));
                    }
                    for r in post.iter() {
                        if !unbox_reg(&tc.kinds, regs, bare, *r) {
                            // (the boxed registers this instruction wrote are
                            // already the values)
                            self.box_typed(tc, regs, bare, post);
                            return self.run_code_from(p, regs, next);
                        }
                    }
                    pc = next;
                    continue;
                }
                TIns::Nop => {}
                TIns::Jump { to } => {
                    pc = *to as usize;
                    continue;
                }
                TIns::Loop { to } => {
                    if let Err(e) = self.check_cancel() {
                        stop!(e)
                    }
                    pc = *to as usize;
                    continue;
                }
                TIns::Cancel => {
                    if let Err(e) = self.check_cancel() {
                        stop!(e)
                    }
                }
                TIns::IConst { d, n } => bare.ints[*d as usize] = *n,
                TIns::FConst { d, x } => bare.floats[*d as usize] = *x,
                TIns::IMov { d, s } => bare.ints[*d as usize] = bare.ints[*s as usize],
                TIns::FMov { d, s } => bare.floats[*d as usize] = bare.floats[*s as usize],
                TIns::IArith { op, d, a, b } => match op.arith(iv(&bare.ints, *a), iv(&bare.ints, *b)) {
                    Ok(n) => bare.ints[*d as usize] = n,
                    Err(e) => stop!(e),
                },
                TIns::IAddK { d, a, k } => match bare.ints[*a as usize].checked_add(*k) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => stop!(crow_runtime::stops::add(bare.ints[*a as usize], *k).err().unwrap_or_default()),
                },
                TIns::ISubK { d, a, k } => match bare.ints[*a as usize].checked_sub(*k) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => stop!(crow_runtime::stops::sub(bare.ints[*a as usize], *k).err().unwrap_or_default()),
                },
                TIns::IAddR { d, a, b } => match bare.ints[*a as usize].checked_add(bare.ints[*b as usize]) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => stop!(crow_runtime::stops::add(bare.ints[*a as usize], bare.ints[*b as usize]).err().unwrap_or_default()),
                },
                TIns::ISubR { d, a, b } => match bare.ints[*a as usize].checked_sub(bare.ints[*b as usize]) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => stop!(crow_runtime::stops::sub(bare.ints[*a as usize], bare.ints[*b as usize]).err().unwrap_or_default()),
                },
                TIns::IJumpR { op, a, b, to } => {
                    if !op.holds(bare.ints[*a as usize], bare.ints[*b as usize]) {
                        pc = *to as usize;
                        continue;
                    }
                }
                TIns::IJumpK { op, a, k, to } => {
                    if !op.holds(bare.ints[*a as usize], *k) {
                        pc = *to as usize;
                        continue;
                    }
                }
                TIns::ICmp { op, d, a, b } => regs[*d as usize].set_bool(op.holds(iv(&bare.ints, *a), iv(&bare.ints, *b))),
                TIns::IJump { op, a, b, to } => {
                    if !op.holds(iv(&bare.ints, *a), iv(&bare.ints, *b)) {
                        pc = *to as usize;
                        continue;
                    }
                }
                TIns::INeg { d, s } => match crow_runtime::stops::neg(bare.ints[*s as usize]) {
                    Ok(n) => bare.ints[*d as usize] = n,
                    Err(e) => stop!(e),
                },
                TIns::FArith { op, d, a, b } => bare.floats[*d as usize] = op.farith(bare.floats[*a as usize], bare.floats[*b as usize]),
                TIns::FCmp { op, d, a, b } => regs[*d as usize].set_bool(op.fholds(bare.floats[*a as usize], bare.floats[*b as usize])),
                TIns::FNeg { d, s } => bare.floats[*d as usize] = -bare.floats[*s as usize],
                TIns::I2F { d, s } => bare.floats[*d as usize] = bare.ints[*s as usize] as f64,
                TIns::IIndex { d, base, idx } => {
                    let i = iv(&bare.ints, *idx);
                    let b = &regs[*base as usize];
                    let got = match &b.value {
                        Value::Array(items) if b.is_ok() => match crow_runtime::stops::item(i, items.len()) {
                            Ok(k) => items[k].plain_int(),
                            Err(e) => stop!(e),
                        },
                        Value::NumArray(t) if b.is_ok() && t.ndim() == 1 => match crow_runtime::core::number_at(t, i) {
                            Ok(x) => Some(x as i64),
                            Err(e) => stop!(e),
                        },
                        _ => None,
                    };
                    match got {
                        Some(n) => bare.ints[*d as usize] = n,
                        None => hand_over!(),
                    }
                }
                TIns::FIndex { d, base, idx } => {
                    let i = iv(&bare.ints, *idx);
                    let b = &regs[*base as usize];
                    let got = match &b.value {
                        Value::Array(items) if b.is_ok() => match crow_runtime::stops::item(i, items.len()) {
                            Ok(k) => items[k].plain_float(),
                            Err(e) => stop!(e),
                        },
                        Value::NumArray(t) if b.is_ok() && t.ndim() == 1 => match crow_runtime::core::number_at(t, i) {
                            Ok(x) => Some(x),
                            Err(e) => stop!(e),
                        },
                        _ => None,
                    };
                    match got {
                        Some(x) => bare.floats[*d as usize] = x,
                        None => hand_over!(),
                    }
                }
                TIns::XIndex { d, base, idx } => {
                    let Ins::Index { how, .. } = &p.code[pc] else { hand_over!() };
                    let i = iv(&bare.ints, *idx);
                    let b = &regs[*base as usize];
                    let cell = if !b.is_ok() {
                        None
                    } else if let Value::Array(items) = &b.value {
                        match crow_runtime::stops::item(i, items.len()) {
                            Ok(k) => Some(copied(&items[k])),
                            Err(e) => stop!(e),
                        }
                    } else if matches!(b.value, Value::NumArray(_) | Value::Map(_) | Value::IntMap(_) | Value::Str(_) | Value::Frame(_)) {
                        match Self::index_value(b, vec![CrowValue::ok(Value::Int(i))], &how.ty) {
                            Ok(c) => Some(c),
                            Err(e) => stop!(e),
                        }
                    } else {
                        None
                    };
                    match cell {
                        Some(c) => regs[*d as usize].assign(if how.column { placeholder(c, &how.ty) } else { c }),
                        None => hand_over!(),
                    }
                }
                TIns::ISet { base, idx, v } => {
                    let i = iv(&bare.ints, *idx);
                    let n = bare.ints[*v as usize];
                    let root = &mut regs[*base as usize];
                    let ok = root.is_ok();
                    match &mut root.value {
                        Value::Array(items) if ok => match crow_runtime::stops::item(i, items.len()) {
                            Ok(k) => items[k].set_int(n),
                            Err(e) => stop!(e),
                        },
                        _ => hand_over!(),
                    }
                }
                TIns::FSet { base, idx, v } => {
                    let i = iv(&bare.ints, *idx);
                    let x = bare.floats[*v as usize];
                    let root = &mut regs[*base as usize];
                    let ok = root.is_ok();
                    match &mut root.value {
                        Value::Array(items) if ok => match crow_runtime::stops::item(i, items.len()) {
                            Ok(k) => items[k].set_float(x),
                            Err(e) => stop!(e),
                        },
                        _ => hand_over!(),
                    }
                }
                TIns::ICopy { dst, di, src, si } => {
                    // the value first, as the walker reads it
                    let s = iv(&bare.ints, *si);
                    let b = &regs[*src as usize];
                    let new = match &b.value {
                        Value::Array(items) if b.is_ok() => match crow_runtime::stops::item(s, items.len()) {
                            Ok(k) => copied(&items[k]),
                            Err(e) => stop!(e),
                        },
                        _ => hand_over!(),
                    };
                    let di = iv(&bare.ints, *di);
                    let root = &mut regs[*dst as usize];
                    let ok = root.is_ok();
                    match &mut root.value {
                        Value::Array(items) if ok => match crow_runtime::stops::item(di, items.len()) {
                            Ok(k) => items[k].assign(new),
                            Err(e) => stop!(e),
                        },
                        _ => hand_over!(),
                    }
                }
                TIns::LIndex { d, base, idx } => {
                    let i = iv(&bare.ints, *idx);
                    let l = &bare.lists[*base as usize];
                    match crow_runtime::stops::item(i, l.len()) {
                        Ok(k) => {
                            let n = l[k];
                            bare.ints[*d as usize] = n;
                        }
                        Err(e) => stop!(e),
                    }
                }
                TIns::LSet { base, idx, v } => {
                    let i = iv(&bare.ints, *idx);
                    let n = bare.ints[*v as usize];
                    let l = &mut bare.lists[*base as usize];
                    match crow_runtime::stops::item(i, l.len()) {
                        Ok(k) => l[k] = n,
                        Err(e) => stop!(e),
                    }
                }
                TIns::LCopy { dst, di, src, si } => {
                    // the value first, as the walker reads it
                    let s = iv(&bare.ints, *si);
                    let l = &bare.lists[*src as usize];
                    let n = match crow_runtime::stops::item(s, l.len()) {
                        Ok(k) => l[k],
                        Err(e) => stop!(e),
                    };
                    let d = iv(&bare.ints, *di);
                    let l = &mut bare.lists[*dst as usize];
                    match crow_runtime::stops::item(d, l.len()) {
                        Ok(k) => l[k] = n,
                        Err(e) => stop!(e),
                    }
                }
                TIns::LLen { d, base } => {
                    if self.registry.has_function("len") {
                        hand_over!()
                    }
                    bare.ints[*d as usize] = bare.lists[*base as usize].len() as i64;
                }
                TIns::LPush { d, v } => {
                    let n = bare.ints[*v as usize];
                    bare.lists[*d as usize].push(n);
                }
                TIns::LClear { d } => bare.lists[*d as usize] = Vec::new(),
                TIns::XLen { d, base } => {
                    let b = &regs[*base as usize];
                    let n = match &b.value {
                        Value::Array(items) if b.is_ok() && !self.registry.has_function("len") => items.len() as i64,
                        _ => hand_over!(),
                    };
                    bare.ints[*d as usize] = n;
                }
                TIns::ForCount { var, ctr, hi, lo, top, end } => {
                    if let Err(e) = self.check_cancel() {
                        stop!(e)
                    }
                    let l = lo.map_or(0, |o| iv(&bare.ints, o));
                    let h = iv(&bare.ints, *top);
                    bare.ints[*ctr as usize] = l;
                    bare.ints[*hi as usize] = h;
                    if l >= h {
                        pc = *end as usize;
                        continue;
                    }
                    bare.ints[*var as usize] = l;
                }
                TIns::ForNextCount { var, ctr, hi, body } => {
                    // (the counter is under the bound: one more cannot pass 64 bits)
                    let c = bare.ints[*ctr as usize] + 1;
                    if c < bare.ints[*hi as usize] {
                        if let Err(e) = self.check_cancel() {
                            stop!(e)
                        }
                        bare.ints[*ctr as usize] = c;
                        bare.ints[*var as usize] = c;
                        pc = *body as usize;
                        continue;
                    }
                }
            }
            pc += 1;
        }
    }
}
