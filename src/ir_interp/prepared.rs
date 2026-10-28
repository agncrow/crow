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
    /// A value handed where 64-bit ints are taken (`Convert::ToSmall`): an
    /// int past them is refused, naming what took it.
    Small { d: R, s: R, who: crate::ir::Who },
    Jump { to: u32 },
    /// A `while` loop's first round begins: the cancellation check.
    Cancel,
    /// A `while` loop's back edge: the next round's cancellation check, then
    /// the jump to its condition.
    Loop { to: u32 },
    /// A decision: a failed value refused, a false one jumps.
    JumpUnless { c: R, to: u32 },
    /// `and`/`or`'s left side decided — an error, false for `and`, true for
    /// `or`: it is the value, into `d`, and the right side is jumped over.
    JumpDecided { c: R, d: R, and: bool, to: u32 },
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
    /// `local.field` of a record: the field's value read where the record
    /// lies (a record's value is shared: the copy is a count); anything
    /// else — a map's key, a frame's column, a session's state, a failed
    /// local — the walker reads, as it always has (`how`: the expression
    /// and the locals it names). A record's field had been a walker
    /// fallback, the record copied whole to read it.
    Field { d: R, base: R, name: Box<str>, how: Box<(Expr, Env)> },
    /// `Tree[l, r, false]`: a record of its fields' values, computed into
    /// registers in their declaration's order, each put where the layout
    /// every value of the record shares holds it (resolved at the first
    /// run). The literal had been a walker fallback, the calls among its
    /// fields with it.
    Record { d: R, name: Box<str>, fields: Box<[(R, bool)]>, shape: Box<std::sync::OnceLock<super::Shape>> },
    /// `xs = push[move xs, v]` and the other builtins that make their
    /// answer of the collection they take (`Prop::InPlace`): the
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
    /// `known`: the slot of what is known of the start of a measured text
    /// (`text_slots`).
    Len { d: R, base: R, how: Box<(Target, Expr, Env)>, known: Option<u16> },
    /// A call: each argument cloned from its register, or taken from a
    /// temporary.
    /// `known`: the slot of what is known of the start of the text a slice
    /// or a length reads (`text_slots`), when the text does not change in
    /// the call.
    Call { d: R, target: Box<Target>, args: Box<[(R, bool)]>, propagates: bool, known: Option<u16> },
    /// An effect (`log!held`, `fetch!now`) of arguments in registers, run as
    /// the walker runs one (`IrInterp::effect`).
    Effect { d: R, how: Box<EffectHow>, args: Box<[(R, bool)]> },
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

/// An effect's own parts (`Ins::Effect`).
#[derive(Debug)]
struct EffectHow {
    name: String,
    row: Option<&'static crate::table::Row>,
    propagates: bool,
    answered: bool,
}

#[derive(Debug)]
struct IndexHow {
    ty: Ty,
    column: bool,
    ir: Expr,
    env: Env,
    /// The slot of what is known of the start of an indexed text
    /// (`text_slots`), when the text does not change in the call.
    known: Option<u16>,
}

pub(super) struct Prepared {
    /// The function's name as calls write it (a specialised name is its
    /// own): the name its frames carry.
    pub(super) name: Arc<str>,
    /// The scope its module's names resolve in (modules::scope_of_call).
    pub(super) scope: &'static str,
    body: Arc<Body>,
}

/// A prepared function's body, and what a call needs of the function —
/// shared by every handle on it.
pub(super) struct Body {
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
    /// Each parameter: whether it is a float — an int handed to it is a
    /// float where the call is made (`call_prepared`).
    pub(super) float_params: Vec<bool>,
    pub(super) view: bool,
    /// Whether it can call itself: its calls count toward the limit.
    pub(super) recursive: bool,
    /// Its body's first line: where a recursion past the limit stops.
    pub(super) first_line: usize,
    /// Whether it calls `cpu[…]`: a call puts the caller's count back as it
    /// returns (crow_runtime::cores).
    sets_cores: bool,
    /// How many texts it reads by position with what is known of their
    /// start (`text_slots`): its frame holds one `Known` each.
    pub(super) n_known: usize,
    /// Each parameter: whether the body only reads it (`ir::only_reads`),
    /// so a call may lend it the caller's value and take it back.
    read_params: Vec<bool>,
    /// Whether its typed program runs on bare registers alone: int and
    /// float parameters, and only instructions that read and write bare
    /// registers (`pure`). A typed call of it (`call_typed`) hands over
    /// its arguments bare and boxes nothing.
    pure: bool,
}

impl std::ops::Deref for Prepared {
    type Target = Body;
    fn deref(&self) -> &Body {
        &self.body
    }
}

impl Prepared {
    /// A handle of its own on the same body, named as a call writes it. A
    /// call clones the handle it runs and its frame the name: each thread
    /// of a parallel loop gets its own handle on each body it reaches, or
    /// every call of every thread writes the same two reference counts —
    /// a 16-thread loop had run no faster than one thread.
    pub(super) fn handle(&self, name: &str) -> Prepared {
        Prepared { name: Arc::from(name), scope: crate::modules::scope_of_call(name), body: self.body.clone() }
    }
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

/// A function prepared alone (its calls' lending decided without the rest
/// of the program): the tests' way in.
#[cfg(test)]
pub(super) fn prepare(f: &Fn) -> Prepared {
    let lend = crate::ir::lendable_params(&[f]);
    prepare_lent(f, lend.get(&f.name).map(|v| v.as_slice()).unwrap_or(&[]))
}

/// A function prepared with the parameters a call may lend it
/// (`ir::lendable_params`, over the whole program): their moves read
/// where they lie, so the caller's value is whole when it goes back.
pub(super) fn prepare_lent(f: &Fn, lend: &[bool]) -> Prepared {
    let lent: Vec<&str> = f.params.iter().zip(lend.iter()).filter(|(_, l)| **l).map(|((p, _), _)| p.as_str()).collect();
    let read_params: Vec<bool> = f.params.iter().enumerate().map(|(i, _)| lend.get(i).copied().unwrap_or(false)).collect();
    let rewritten;
    let f = if lent.is_empty() {
        f
    } else {
        rewritten = Fn { body: crate::ir::reading_lent(&f.body, &lent), ..f.clone() };
        &rewritten
    };
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
    let mut code = c.code;
    let n_known = text_slots(&mut code, &params);
    let mut b = Body {
        n_regs: kinds.len(),
        kinds,
        params,
        code,
        lines: c.lines,
        typed: None,
        float_params: f.params.iter().map(|(_, t)| matches!(t, Ty::Float)).collect(),
        view: f.kind == crate::ir::FnKind::View,
        recursive: f.recursive,
        first_line: crate::ir::first_line(&f.body),
        sets_cores: crate::ir::sets_cores(&f.body),
        n_known,
        read_params,
        pure: false,
    };
    b.typed = typed(&b);
    b.pure = !b.view && b.typed.as_ref().is_some_and(|t| pure(t, &b.params));
    Prepared { name: Arc::from(f.name.as_str()), scope: crate::modules::scope_of_call(&f.name), body: Arc::new(b) }
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
            Ins::Jump { to: t } | Ins::Loop { to: t } | Ins::JumpUnless { to: t, .. } | Ins::JumpUnlessInt { to: t, .. } | Ins::JumpDecided { to: t, .. } => *t = to,
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
                    && row.takes_in_place()
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
                // (a call the IR names a row or a function for takes the
                // variable as an ordinary call does — the walker had run
                // the statement, `out = push[move out, n]` once per limb;
                // any other target is the walker's in-place path's)
                let named = matches!(expr, Expr::Call { target: Target::Row(row), .. } if !row.is_crow()) || matches!(expr, Expr::Call { target: Target::User(_), .. });
                if moves_first(var, expr) && !named {
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
            // a local freed after its last read (the move pass's `move x`
            // alone): its register let go, as its block's end lets it go
            Stmt::Do(Expr::Move(v, _)) if self.resolve(v).is_some() => {
                if let Some(d) = self.resolve(v) {
                    self.emit(Ins::Clear { d });
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
            // the interpreter's ints grow as they run: an int held growing is
            // the value as it is
            // (proven arithmetic is the arithmetic: the typed registers' own
            // check never fires)
            Expr::Convert { how: Convert::ToBig | Convert::Proven, expr, .. } => {
                let p = self.expr_place(expr, d);
                self.put(p, d);
            }
            Expr::Convert { how: Convert::ToSmall(who), expr, .. } => {
                let p = self.expr(expr);
                let s = self.reg(p);
                self.emit(Ins::Small { d, s, who: *who });
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
                } else if matches!(bop, BinOp::And | BinOp::Or) && crate::ir::short_circuits(l, r) {
                    // the left first; where it decides it is the value, and
                    // the right side's instructions are jumped over (spec,
                    // Operators)
                    let and = matches!(bop, BinOp::And);
                    let pa = self.expr(l);
                    let pa = self.kept_place(pa, std::slice::from_ref(r));
                    let a = self.reg(pa);
                    let at = self.emit(Ins::JumpDecided { c: a, d, and, to: u32::MAX });
                    let pb = self.expr(r);
                    let b = self.reg(pb);
                    if lt == Ty::Bool && rt == Ty::Bool {
                        self.emit(Ins::Logic { and, d, a, b });
                    } else {
                        self.emit(Ins::Bin { op: bop, d, a, b });
                    }
                    let end = self.here();
                    self.patch(at, end);
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
            Expr::RecordLit { name, fields } => {
                let mut regs = Vec::with_capacity(fields.len());
                for (i, f) in fields.iter().enumerate() {
                    let p = self.expr(f);
                    let p = self.kept_place(p, &fields[i + 1..]);
                    regs.push(self.owned(p));
                }
                self.emit(Ins::Record { d, name: name.as_str().into(), fields: regs.into_boxed_slice(), shape: Box::default() });
            }
            Expr::Field { base, name, .. }
                if !matches!(name.as_str(), "ok" | "reason" | "kind" | "trace" | "len")
                    && matches!(unwrapped(base), Expr::Local(..) | Expr::Move(..)) =>
            {
                let (Expr::Local(v, _) | Expr::Move(v, _)) = unwrapped(base) else { return self.eval(e, d) };
                let Some(b) = self.resolve(v) else { return self.eval(e, d) };
                let env = self.env();
                self.emit(Ins::Field { d, base: b, name: name.as_str().into(), how: Box::new((e.clone(), env)) });
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
                    self.emit(Ins::Index { d, base: b, idx, how: Box::new(IndexHow { ty: ty.clone(), column, ir: e.clone(), env, known: None }) });
                    return;
                }
                self.eval(e, d)
            }
            // an effect's arguments computed where they are, as a call's
            // are (the walker had run the whole effect, copying each local
            // it was handed: `log[slice[r, i, j]]` copied `r` per line)
            Expr::Effect { name, row, args, propagates, answered, .. } => {
                let mut regs = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    let p = self.expr(a);
                    let p = self.kept_place(p, &args[i + 1..]);
                    regs.push(self.owned(p));
                }
                self.emit(Ins::Effect { d, how: Box::new(EffectHow { name: name.clone(), row: *row, propagates: *propagates, answered: *answered }), args: regs.into_boxed_slice() });
            }
            Expr::Call { target, args, propagates, .. } => {
                // (a local read for the last time is measured where it lies
                // too: nothing reads it after, so nothing is taken)
                if is_len(target)
                    && args.len() == 1
                    && let Expr::Local(v, _) | Expr::Move(v, _) = unwrapped(&args[0])
                    && let Some(b) = self.resolve(v)
                {
                    self.emit(Ins::Len { d, base: b, how: Box::new((target.clone(), e.clone(), self.env())), known: None });
                    return;
                }
                // the length of what a call makes (`len[slice[seq, i, j]]`):
                // the call into a register of its own, then its length, by the
                // row the IR names — the walker had run the whole expression,
                // copying each of the call's arguments
                if let Target::Row(_) = target
                    && is_len(target)
                    && args.len() == 1
                    && matches!(unwrapped(&args[0]), Expr::Call { .. })
                {
                    let p = self.expr(&args[0]);
                    let (b, _) = self.owned(p);
                    self.emit(Ins::Len { d, base: b, how: Box::new((target.clone(), e.clone(), self.env())), known: None });
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
                self.emit(Ins::Call { d, target: Box::new(target.clone()), args: regs.into_boxed_slice(), propagates: *propagates, known: None });
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
        // (a refusal past 64 bits comes the same the second time)
        Expr::Convert { how: Convert::Unwrap | Convert::Known | Convert::IntToFloat | Convert::ToBig | Convert::ToSmall(_) | Convert::Proven, expr, .. } => pure_index(expr),
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
    /// A call of a prepared body whose `n` arguments `arg` hands over one
    /// by one, each into its parameter's register: a call from a prepared
    /// body moves them from its own registers, with no list between.
    pub(super) fn call_prepared_with(&mut self, name: &str, p: &Prepared, n: usize, args: &mut dyn CallArgs) -> Result<CrowValue, String> {
        if p.float_params.len() != n {
            return Err(format!("{}[] expects {} arguments, got {}", name, p.float_params.len(), n));
        }
        let _cores = p.sets_cores.then(crow_runtime::cores::Restore::new);
        let (mut regs, mut bare) = self.reg_pool.take();
        regs.resize(p.n_regs, CrowValue::done());
        // (`arguments_checked`, as each arrives; a parameter the body only
        // reads may be lent the caller's value, to go back as the call ends)
        let mut failed = None;
        let mut lent: u64 = 0;
        for (i, (float, r)) in p.float_params.iter().zip(p.params.iter()).enumerate() {
            let (mut v, lends) = args.arg(i, i < 64 && p.read_params.get(i).copied().unwrap_or(false));
            if lends {
                lent |= 1 << i;
            }
            if *float && let Some(k) = v.plain_int() {
                v = CrowValue::ok(Value::Float(k as f64));
            }
            if p.view && failed.is_none() && v.is_error() {
                failed = Some(v.reason().to_string());
            }
            regs[*r as usize] = v;
        }
        let give_back = |regs: &mut Vec<CrowValue>, args: &mut dyn CallArgs| {
            for (i, r) in p.params.iter().enumerate().take(64) {
                if lent & (1 << i) != 0 {
                    args.back(i, std::mem::replace(&mut regs[*r as usize], CrowValue::done()));
                }
            }
        };
        if let Some(reason) = failed {
            give_back(&mut regs, args);
            self.reg_pool.give(regs, bare);
            return Ok(CrowValue::markup(crate::harness::html_escape_text(&crow_web::core::response::failed_in_view(&reason))));
        }
        if p.recursive && let Err(e) = self.deeper(p.first_line) {
            give_back(&mut regs, args);
            self.reg_pool.give(regs, bare);
            return Err(e);
        }
        let saved_line = self.line;
        if p.view {
            self.in_view += 1;
        }
        self.frames.push(Frame { scopes: Vec::new(), locals: Vec::new(), marks: Vec::new(), fn_name: p.name.clone(), slots: Vec::new(), env: None, texts: (0..p.n_known).map(|_| crow_runtime::text::Known::new()).collect() });
        let r = self.run_body(p, &mut regs, &mut bare);
        give_back(&mut regs, args);
        self.reg_pool.give(regs, bare);
        self.call_ended(r, p.view, p.recursive, saved_line)
    }

    /// A slice or a length of a text read with what is known of its start
    /// (the frame's slot `k`): the text and its positions read where they
    /// lie. None when they are not plain (a failure, a dual number): the
    /// row's own call decides.
    fn text_known(&self, row: &'static crate::table::Row, regs: &[CrowValue], args: &[(R, bool)], k: u16) -> Option<CrowValue> {
        let s = match &regs[args[0].0 as usize] {
            v if v.is_ok() => match &v.value {
                Value::Str(s) => s,
                _ => return None,
            },
            _ => return None,
        };
        let known = &self.frames.last()?.texts.get(k as usize)?;
        match (row.id, args.len()) {
            ("core.slice_str", 3) => {
                let a = regs[args[1].0 as usize].plain_int()?;
                let b = regs[args[2].0 as usize].plain_int()?;
                Some(CrowValue::ok(Value::Str(crow_runtime::text::slice_known(s, known, a, b))))
            }
            ("core.len_text", 1) => Some(CrowValue::ok(Value::Int(crow_runtime::text::len_known(s, known)))),
            _ => None,
        }
    }

    /// A builtin's row called with its arguments lent: each moved out of its
    /// register (a register named twice is copied for the second), the row
    /// read through its call that reads arguments where they lie, and each
    /// moved back — a taken one dropped. A row called with a long text and a
    /// position had copied the text at every call (k-nucleotide's windows,
    /// a slice of 250,000 characters each). The row's answer is the one the
    /// copying call gave: both are the same function read two ways.
    fn row_lent(&mut self, row: &'static crate::table::Row, regs: &mut [CrowValue], args: &[(R, bool)]) -> Result<CrowValue, String> {
        let mut lent: [CrowValue; LENT] = std::array::from_fn(|_| CrowValue::done());
        let mut moved = [false; LENT];
        for (i, (r, take)) in args.iter().enumerate() {
            // (a register named twice — `stack[f, f]` — copies what the first
            // naming lent: the register holds nothing while it is lent)
            lent[i] = match args[..i].iter().position(|(q, _)| q == r) {
                Some(j) => lent[j].clone(),
                None => {
                    moved[i] = !*take;
                    std::mem::replace(&mut regs[*r as usize], CrowValue::done())
                }
            };
        }
        let trace = if row.module.is_empty() && matches!(row.fails, crate::table::Fails::Value(_)) { self.trace_entry() } else { String::new() };
        let v = (row.call)(&lent[..args.len()], &trace).map(|v| Self::stamp_row(row, v));
        for (i, (r, _)) in args.iter().enumerate() {
            if moved[i] {
                regs[*r as usize] = std::mem::replace(&mut lent[i], CrowValue::done());
            }
        }
        v
    }

    /// A user function called from a typed body with int and float
    /// arguments (`TIns::CallT`). A pure callee (`Body::pure`) gets them
    /// bare, in a bare register file of its own, and runs with no boxed
    /// register at all; any other callee gets them boxed, as the boxed
    /// call hands them over.
    fn call_typed(&mut self, name: &str, kinds: &[RKind], bare: &Bare, args: &[R]) -> Result<CrowValue, String> {
        let boxed = |i: usize| {
            let a = args[i] as usize;
            match kinds[a] {
                RKind::Int => CrowValue::ok(Value::Int(bare.ints[a])),
                RKind::Float => CrowValue::ok(Value::Float(bare.floats[a])),
                _ => CrowValue::done(),
            }
        };
        if self.tracing {
            let vals = (0..args.len()).map(boxed).collect();
            return self.call_user(name, vals);
        }
        self.check_cancel()?;
        let p = match self.prepared.get(name) {
            Some(p) => p.clone(),
            None => self.prepare_called(name)?,
        };
        let scope = self.registry.enter_scope(p.scope);
        let r = match &p.typed {
            Some(ctc) if p.pure && p.params.len() == args.len() && args.iter().zip(p.params.iter()).all(|(a, r)| handed_bare(kinds[*a as usize], ctc.kinds[*r as usize])) => self.call_pure(&p, ctc, kinds, bare, args),
            _ => self.call_prepared_with(name, &p, args.len(), &mut Given((0..args.len()).map(boxed))),
        };
        self.registry.resume_call(scope);
        r
    }

    /// A pure body called (`call_typed`): each argument into its parameter's
    /// bare register — an int handed to a float parameter a float, as the
    /// boxed call converts it.
    fn call_pure(&mut self, p: &Prepared, tc: &Typed, kinds: &[RKind], bare: &Bare, args: &[R]) -> Result<CrowValue, String> {
        let (mut regs, mut callee) = self.reg_pool.take();
        callee.ints.resize(p.n_regs, 0);
        callee.floats.resize(p.n_regs, 0.0);
        for (a, r) in args.iter().zip(p.params.iter()) {
            let (a, r) = (*a as usize, *r as usize);
            match (kinds[a], tc.kinds[r]) {
                (RKind::Int, RKind::Int) => callee.ints[r] = bare.ints[a],
                (RKind::Int, _) => callee.floats[r] = bare.ints[a] as f64,
                _ => callee.floats[r] = bare.floats[a],
            }
        }
        if p.recursive && let Err(e) = self.deeper(p.first_line) {
            self.reg_pool.give(regs, callee);
            return Err(e);
        }
        let saved_line = self.line;
        self.frames.push(Frame { scopes: Vec::new(), locals: Vec::new(), marks: Vec::new(), fn_name: p.name.clone(), slots: Vec::new(), env: None, texts: Vec::new() });
        let r = self.run_typed(p, tc, &mut regs, &mut callee);
        self.reg_pool.give(regs, callee);
        self.call_ended(r, false, p.recursive, saved_line)
    }

    /// A user function called from a prepared body: its arguments moved
    /// (or copied) from the caller's registers into the callee's.
    fn call_user_from(&mut self, name: &str, regs: &mut [CrowValue], args: &[(R, bool)]) -> Result<CrowValue, String> {
        self.check_cancel()?;
        let p = match self.prepared.get(name) {
            Some(p) => p.clone(),
            None => self.prepare_called(name)?,
        };
        let scope = self.registry.enter_scope(p.scope);
        let r = self.call_prepared_with(name, &p, args.len(), &mut FromRegs { regs, args });
        self.registry.resume_call(scope);
        r
    }

    pub(super) fn prepared_for(&mut self, f: &Arc<Fn>) -> Arc<Prepared> {
        if let Some(p) = self.prepared.get(&f.name) {
            return p.clone();
        }
        let lend = self.lendable.get(&f.name).cloned().unwrap_or_default();
        let p = Arc::new(prepare_lent(f, &lend));
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
            std::sync::Arc::make_mut(items)[k].assign(new);
            return Ok(());
        }
        let at = match idx {
            Opnd::K(n) => CrowValue::ok(Value::Int(n)),
            Opnd::R(r) => copied(&regs[r as usize]),
        };
        set_path(&mut regs[base as usize].value, path, &[at], new)
    }

    /// A prepared body run: its registers, its parameters' values in theirs.
    /// A prepared body run over a register file its parameters are in (the
    /// file from the pool, `call_prepared_with`).
    pub(super) fn run_body(&mut self, p: &Prepared, regs: &mut Vec<CrowValue>, bare: &mut Bare) -> Result<Flow, String> {
        if let Some(tc) = &p.typed {
            bare.ints.resize(p.n_regs, 0);
            bare.floats.resize(p.n_regs, 0.0);
            bare.lists.resize_with(p.n_regs, Vec::new);
            // each typed parameter a plain number of its kind (a list of
            // plain ints) — or the boxed program runs (a dual number a
            // function `grad` differentiates computes with, a value the IR
            // could not see)
            for r in p.params.iter() {
                if !unbox_reg(&tc.kinds, regs, bare, *r) {
                    return self.run_code(p, regs);
                }
            }
            return self.run_typed(p, tc, regs, bare);
        }
        self.run_code(p, regs)
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
                            match op.arith(x, y) {
                                Ok(n) => regs[*d as usize].set_int(n),
                                // (past 64 bits the int grows; a zero divisor stops)
                                Err(_) => {
                                    self.line_of(p, pc);
                                    let (x, y) = (value_at(regs, *a), value_at(regs, *b));
                                    regs[*d as usize].assign(t!(crate::ops::binop(&x, &op.general(), &y)));
                                }
                            }
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
                        CrowValue::ok(Value::of_int(crow_runtime::int::Int::Small(n).neg()))
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
                        None => match &x.value {
                            Value::Big(b) => CrowValue::ok(Value::Float(crow_runtime::int::Int::Big(b.clone()).to_f64())),
                            _ => x.clone(),
                        },
                    };
                    regs[*d as usize].assign(v);
                }
                Ins::Small { d, s, who } => {
                    let x = &regs[*s as usize];
                    t!(crate::ops::small_check(&x.value, *who));
                    let v = x.clone();
                    regs[*d as usize].assign(v);
                }
                Ins::Jump { to } => return *to as usize,
                Ins::JumpDecided { c, d, and, to } => {
                    let v = &regs[*c as usize];
                    if v.is_error() || v.plain_bool() == Some(!*and) {
                        let v = copied(v);
                        regs[*d as usize].assign(v);
                        return *to as usize;
                    }
                }
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
                    regs[*items as usize] = CrowValue::ok(Value::Array(std::sync::Arc::new(list)));
                    regs[*var as usize].assign(first);
                }
                Ins::ForNext { var, ctr, hi, items, body } => {
                    let c = regs[*ctr as usize].plain_int().unwrap_or(i64::MAX - 1) + 1;
                    let h = regs[*hi as usize].plain_int().unwrap_or(0);
                    if c < h {
                        t!(self.check_cancel());
                        regs[*ctr as usize].set_int(c);
                        let v = match &mut regs[*items as usize].value {
                            Value::Array(list) => std::mem::replace(&mut std::sync::Arc::make_mut(list)[c as usize], CrowValue::done()),
                            _ => CrowValue::ok(Value::Int(c)),
                        };
                        regs[*var as usize].assign(v);
                        return *body as usize;
                    }
                }
                Ins::Record { d, name, fields, shape } => {
                    let shape: super::Shape = match shape.get() {
                        Some(s) => s,
                        None => match self.layouts.get(&**name).copied() {
                            Some(s) => {
                                let _ = shape.set(s);
                                s
                            }
                            None => t!(Err::<super::Shape, String>(format!("unknown struct '{}'", name))),
                        },
                    };
                    if shape.1.len() != fields.len() {
                        t!(Err::<(), String>(format!("{}[] expects {} fields, got {}", name, shape.1.len(), fields.len())))
                    }
                    let mut vals = vec![CrowValue::done(); fields.len()];
                    for (i, (r, take)) in fields.iter().enumerate() {
                        vals[shape.1[i]] = if *take { std::mem::replace(&mut regs[*r as usize], CrowValue::done()) } else { copied(&regs[*r as usize]) };
                    }
                    regs[*d as usize].assign(CrowValue::ok(Value::StructInstance(Arc::new(crate::value::StructInstance::with_layout(shape.0, vals)))));
                }
                Ins::Field { d, base, name, how } => {
                    let b = &regs[*base as usize];
                    if b.is_ok()
                        && let Value::StructInstance(si) = &b.value
                        && let Some(v) = si.get(name)
                    {
                        let v = copied(v);
                        regs[*d as usize].assign(v);
                        return pc + 1;
                    }
                    self.line_of(p, pc);
                    let v = t!(self.lend(regs, &how.1, |it| it.eval(&how.0)));
                    regs[*d as usize].assign(v);
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
                        if let (Value::Str(s), Some(k), Some(n)) = (&b.value, how.known, at) {
                            let c = t!(crow_runtime::text::char_at_known(s, &self.frames[self.frames.len() - 1].texts[k as usize], n));
                            regs[*d as usize].assign(CrowValue::ok(Value::Str(c)));
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
                        std::sync::Arc::make_mut(items).push(item);
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
                Ins::Len { d, base, how, known } => {
                    let (target, ir, env) = &**how;
                    // (a length the IR names a row for is that row's, as the
                    // walker reads it; a module's own `len` takes the rest)
                    if !matches!(target, Target::Row(_)) && self.registry.has_function("len") {
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
                    if let (Value::Str(s), Some(k), true) = (&b.value, known, b.is_ok()) {
                        let n = crow_runtime::text::len_known(s, &self.frames[self.frames.len() - 1].texts[*k as usize]);
                        regs[*d as usize].assign(CrowValue::ok(Value::Int(n)));
                        return pc + 1;
                    }
                    let v = if b.is_error() { b.clone() } else { t!(Self::len_of(target, b, &trace)) };
                    regs[*d as usize].assign(v);
                }
                Ins::Effect { d, how, args } => {
                    let mut vals = Vec::with_capacity(args.len());
                    for (r, take) in args.iter() {
                        vals.push(if *take { std::mem::replace(&mut regs[*r as usize], CrowValue::done()) } else { copied(&regs[*r as usize]) });
                    }
                    self.line_of(p, pc);
                    if how.propagates && let Some(err) = self.first_error(&vals) {
                        regs[*d as usize].assign(err);
                        return pc + 1;
                    }
                    let asked = std::mem::replace(&mut self.answering, how.answered);
                    let r = self.effect(&how.name, how.row, vals);
                    self.answering = asked;
                    let v = t!(r);
                    regs[*d as usize].assign(v);
                }
                Ins::Call { d, target, args, propagates, .. } if !self.tracing && matches!(&**target, Target::User(n) if n != PUT) => {
                    let Target::User(name) = &**target else { return pc + 1 };
                    self.line_of(p, pc);
                    if *propagates && let Some(err) = args.iter().find(|(r, _)| regs[*r as usize].is_error()) {
                        let mut err = regs[err.0 as usize].clone();
                        err.append_trace(&self.trace_entry());
                        regs[*d as usize].assign(err);
                        return pc + 1;
                    }
                    let v = t!(self.call_user_from(name, regs, args));
                    regs[*d as usize].assign(v);
                }
                // a builtin's row given a large value a later instruction
                // reads: lent where it lies, not copied (`rows_lent`)
                Ins::Call { d, target, args, propagates, known } if matches!(&**target, Target::Row(row) if !row.is_crow() && lends(regs, args, row)) => {
                    let Target::Row(row) = &**target else { return pc + 1 };
                    self.line_of(p, pc);
                    if *propagates && let Some((r, _)) = args.iter().find(|(r, _)| regs[*r as usize].is_error()) {
                        let mut err = regs[*r as usize].clone();
                        err.append_trace(&self.trace_entry());
                        regs[*d as usize].assign(err);
                        return pc + 1;
                    }
                    // a slice or a length of a text that keeps what is known
                    // of its start (`text_slots`)
                    if let Some(v) = known.and_then(|k| self.text_known(row, regs, args, k)) {
                        regs[*d as usize].assign(v);
                        return pc + 1;
                    }
                    let v = t!(self.row_lent(row, regs, args));
                    regs[*d as usize].assign(v);
                }
                Ins::Call { d, target, args, propagates, .. } => {
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

    /// A list a function is handed stays as it came — read where it lies
    /// (`IIndex`), never made bare at the call (`LIndex`): a binary search
    /// called 80,000 times had read the whole list at every call.
    #[test]
    fn a_list_handed_to_a_function_is_read_where_it_lies() {
        let f = lowered_fn(
            "fn below[a, x]\n    lo = 0\n    hi = len[a]\n    while lo lt hi\n        mid = (lo + hi) / 2\n        if a[mid] lt x\n            lo = mid + 1\n        else\n            hi = mid\n    return lo\nxs = for i in range[10]\n    i * 3\nlog[below[xs, 7]]\n",
            "below",
        );
        let p = prepare(&f);
        let shown = format!("{:?}", p.typed.as_ref().map(|tc| &tc.code));
        assert!(shown.contains("IIndex"), "{}", shown);
        assert!(!shown.contains("LIndex") && !shown.contains("LLen"), "{}", shown);
    }

    /// A text the body reads by position keeps what is known of its start
    /// where it does not change while the call runs — a parameter, a local
    /// bound once outside every loop — and a text given new values in a
    /// loop gets none (another text could lie where the first did).
    #[test]
    fn a_steady_text_read_by_position_has_a_slot() {
        let f = lowered_fn(
            "fn windows[seq, k]\n    n = 0\n    once = 'acgt'\n    t = 'ab'\n    for i in range[len[seq] - k + 1]\n        w = slice[seq, i, i + k]\n        a = slice[once, 0, 2]\n        b = slice[t, 0, 1]\n        n = n + len[w] + len[a] + len[b] + len[slice[seq, i, i + 1]]\n        t = 'xy'\n    return n\nlog[windows['acgtacgt', 3]]\n",
            "windows",
        );
        let p = prepare(&f);
        assert_eq!(p.n_known, 2, "seq and once: {:?}", p.code);
        let slots: Vec<Option<u16>> = p.code.iter().filter_map(|i| match i { Ins::Call { target, known, .. } if matches!(&**target, Target::Row(r) if r.id == "core.slice_str") => Some(*known), _ => None }).collect();
        assert_eq!(slots.iter().filter(|k| k.is_some()).count(), 3, "{:?}", slots);
        assert_eq!(slots.iter().filter(|k| k.is_none()).count(), 1, "t, given new values in the loop: {:?}", slots);
        assert!(p.code.iter().any(|i| matches!(i, Ins::Len { known: Some(_), .. })), "len[seq] reads with seq's slot: {:?}", p.code);
        assert!(!format!("{:?}", p.code).contains("Eval {"), "a length of a slice is no walker fallback: {:?}", p.code);
    }

    /// An effect's arguments are computed into registers, as a call's are:
    /// the walker had run the whole effect, copying each local it was
    /// handed (`log[slice[r, i, j]]` copied `r` for every line it logged).
    #[test]
    fn an_effects_arguments_are_registers() {
        let f = lowered_fn("fn lines_of[r]\n    i = 0\n    while i lt len[r]\n        log[slice[r, i, i + 2]]\n        i = i + 2\n    return i\nlog[lines_of['abcdef']]\n", "lines_of");
        let p = prepare(&f);
        assert!(p.code.iter().any(|i| matches!(i, Ins::Effect { .. })), "{:?}", p.code);
        assert!(!format!("{:?}", p.code).contains("Eval {"), "{:?}", p.code);
    }

    /// A parameter the body only reads — a move of it at its last read
    /// borrowing (a loop's list, an index, a builtin that reads it) — is
    /// lendable; one the body assigns, or hands to a list, is not.
    #[test]
    fn a_parameter_only_read_is_lendable() {
        let stmts = crate::build("fn reads[xs, m]\n    t = 0\n    for x in xs\n        t = t + len[x]\n    return t + len[m]\nfn keeps[xs]\n    ys = [xs]\n    return len[ys]\nfn changes[xs]\n    xs[0] = 'z'\n    return xs\nlog[reads[['a'], 'bc']]\nlog[keeps[['a']]]\nlog[changes[['a']]]\n").expect("parses");
        let program = crate::ir_interp::prepare(&stmts, "t").expect("lowers").0;
        let fns: Vec<&Fn> = program.parts[0].fns.iter().collect();
        let lend = crate::ir::lendable_params(&fns);
        assert_eq!(lend["reads"], vec![true, true], "{:?}", lend);
        assert_eq!(lend["keeps"], vec![false], "{:?}", lend);
        assert_eq!(lend["changes"], vec![false], "{:?}", lend);
    }

    /// A counted loop over a range is a counted loop, and a nested one
    /// reusing the name binds a register of its own.
    #[test]
    fn a_loop_over_a_range_counts_in_registers_of_its_own() {
        let f = lowered_fn(
            "fn tri[n]\n    s = 0\n    for i in range[n]\n        for i in range[3]\n            s = s + i\n        s = s + i\n    return s\nlog[tri[4]]\n",
            "tri",
        );
        let p = prepare(&f);
        let code = &p.code;
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
    /// An int or a float answered (the boxed return, of a bare register).
    IRet { s: R },
    FRet { s: R },
    /// A user function called with int and float arguments, answering an
    /// int or a float: the arguments handed over bare (`call_typed`).
    CallT { d: R, name: Box<str>, args: Box<[R]> },
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

/// Register files a returned call left, kept for the next call (each
/// empty, its capacity kept). A clone of the interpreter — a parallel
/// loop's — starts with none of its own.
#[derive(Default)]
pub(super) struct Pool(Vec<(Vec<CrowValue>, Bare)>);

impl Clone for Pool {
    fn clone(&self) -> Pool {
        Pool(Vec::new())
    }
}

impl Pool {
    fn take(&mut self) -> (Vec<CrowValue>, Bare) {
        self.0.pop().unwrap_or_else(|| (Vec::new(), Bare { ints: Vec::new(), floats: Vec::new(), lists: Vec::new() }))
    }

    /// A file back: its values dropped now, where a returning frame drops
    /// its locals.
    fn give(&mut self, mut regs: Vec<CrowValue>, mut bare: Bare) {
        regs.clear();
        bare.ints.clear();
        bare.floats.clear();
        bare.lists.clear();
        if self.0.len() < 64 {
            self.0.push((regs, bare));
        }
    }
}

/// Whether an argument of one kind is handed bare to a parameter of
/// another: an int to an int or a float, a float to a float (a float to an
/// int parameter is the boxed call's to refuse).
fn handed_bare(arg: RKind, param: RKind) -> bool {
    matches!((arg, param), (RKind::Int, RKind::Int) | (RKind::Int, RKind::Float) | (RKind::Float, RKind::Float))
}

/// The boxed registers sized for the body: a pure body runs without them
/// until a hand-over needs them.
fn boxed_sized(regs: &mut Vec<CrowValue>, p: &Prepared) {
    if regs.len() < p.n_regs {
        regs.resize_with(p.n_regs, CrowValue::done);
    }
}

/// Where a call's arguments come from, and where the ones lent go back.
pub(super) trait CallArgs {
    /// The `i`th argument, and whether it is lent — the caller's own value,
    /// handed over and taken back as the call ends — which it may be when
    /// the callee only reads the parameter (`read_only`).
    fn arg(&mut self, i: usize, read_only: bool) -> (CrowValue, bool);
    /// A lent argument, back where it came from.
    fn back(&mut self, i: usize, v: CrowValue);
}

/// Arguments already made (a list a walker built, numbers a typed call
/// boxed): none lent.
pub(super) struct Given<I: Iterator<Item = CrowValue>>(pub(super) I);

impl<I: Iterator<Item = CrowValue>> CallArgs for Given<I> {
    fn arg(&mut self, _: usize, _: bool) -> (CrowValue, bool) {
        (self.0.next().unwrap_or_else(CrowValue::done), false)
    }
    fn back(&mut self, _: usize, _: CrowValue) {}
}

/// Arguments in a prepared caller's registers: one the callee only reads
/// and the caller reads again — a large value (a list, a map, a text) named
/// once in the call — is lent, not copied, and goes back to its register as
/// the call ends; one taken is moved; any other copied. A long list handed
/// to a function called once per item had been copied at every call
/// (revcomp's lines; spectral norm's vector).
struct FromRegs<'a> {
    regs: &'a mut [CrowValue],
    args: &'a [(R, bool)],
}

impl CallArgs for FromRegs<'_> {
    fn arg(&mut self, i: usize, read_only: bool) -> (CrowValue, bool) {
        let (r, take) = self.args[i];
        let v = &self.regs[r as usize];
        if take {
            return (std::mem::replace(&mut self.regs[r as usize], CrowValue::done()), false);
        }
        let large = !matches!(v.value, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Moment(_) | Value::Done);
        let once = self.args.iter().filter(|(q, _)| *q == r).count() == 1;
        if read_only && large && once {
            return (std::mem::replace(&mut self.regs[r as usize], CrowValue::done()), true);
        }
        (copied(v), false)
    }
    fn back(&mut self, i: usize, v: CrowValue) {
        self.regs[self.args[i].0 as usize] = v;
    }
}

/// Whether a row's call lends its arguments rather than copying them: one
/// a later instruction reads holds a large value (a text, a list, a map, a
/// tensor, a frame), and none it takes holds one its row could make its
/// answer of (`push[xs, v]` with `xs` dead grows `xs` where it lies), and
/// none is a dual number or a traced tensor (their rows are the kernels').
fn lends(regs: &[CrowValue], args: &[(R, bool)], row: &crate::table::Row) -> bool {
    let large = |v: &CrowValue| !matches!(v.value, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Moment(_) | Value::Done);
    let mut kept_large = false;
    for (i, (r, take)) in args.iter().enumerate() {
        let v = &regs[*r as usize];
        if matches!(v.value, Value::Dual(..) | Value::Traced(_)) {
            return false;
        }
        if large(v) {
            // a large value read for the last time goes to the row's take
            // path only where the row makes its answer of it; elsewhere it
            // is lent like the rest and dropped after (a call taking one
            // had copied every other large argument it was handed)
            if *take && crate::table::keeps_storage(row, i, args.len()) {
                return false;
            }
            kept_large = true;
        }
    }
    kept_large && args.len() <= LENT
}

/// The most arguments a row's call lends (a call with more copies them).
const LENT: usize = 6;

/// A typed register's value into its boxed register.
#[inline(always)]
fn box_reg(kinds: &[RKind], regs: &mut [CrowValue], bare: &Bare, r: R) {
    let i = r as usize;
    match kinds[i] {
        RKind::Int => regs[i].set_int(bare.ints[i]),
        RKind::Float => regs[i].set_float(bare.floats[i]),
        RKind::IntList => regs[i] = CrowValue::ok(Value::Array(std::sync::Arc::new(bare.lists[i].iter().map(|n| CrowValue::ok(Value::Int(*n))).collect()))),
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
                for it in items.iter() {
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
        Ins::Copy { d, s } | Ins::Take { d, s } | Ins::Not { d, s } | Ins::Neg { d, s } | Ins::ToFloat { d, s } | Ins::Small { d, s, .. } => (vec![*s], vec![*d]),
        Ins::Clear { d } => (vec![], vec![*d]),
        Ins::Int { d, a, b, .. } => (rs(&[*a, *b]), vec![*d]),
        Ins::Float { d, a, b, .. } | Ins::Logic { d, a, b, .. } | Ins::Bin { d, a, b, .. } => (vec![*a, *b], vec![*d]),
        Ins::Jump { .. } | Ins::Loop { .. } | Ins::Cancel => (vec![], vec![]),
        Ins::JumpUnless { c, .. } => (vec![*c], vec![]),
        Ins::JumpDecided { c, d, .. } => (vec![*c], vec![*d]),
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
        Ins::Field { d, base, .. } => (vec![*base], vec![*d]),
        Ins::Record { d, fields, .. } => (fields.iter().map(|(r, _)| *r).collect(), vec![*d]),
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
        Ins::Call { d, args, .. } | Ins::Effect { d, args, .. } => (args.iter().map(|(x, _)| *x).collect(), vec![*d]),
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
/// The slots of what is known of the start of each text the body reads by
/// position — a slice's or a length's text, an indexed text — where the
/// text does not change while the call runs: a parameter no instruction
/// writes, or a local one instruction writes outside every loop (a block's
/// end dropping it is no write: nothing reads it after). The slots are
/// numbered into the instructions; the count is the frame's (`Frame::texts`).
/// A text written again could be another text at the same address, which
/// what is known of the first would misread — so it gets none.
fn text_slots(code: &mut [Ins], params: &[R]) -> usize {
    let mut writes: HashMap<R, Vec<usize>> = HashMap::new();
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (pc, ins) in code.iter().enumerate() {
        if !matches!(ins, Ins::Clear { .. }) {
            for w in reads_writes(ins).1 {
                writes.entry(w).or_default().push(pc);
            }
        }
        let back = match ins {
            Ins::Loop { to } | Ins::Jump { to } | Ins::JumpUnless { to, .. } | Ins::JumpUnlessInt { to, .. } => Some(*to as usize),
            Ins::ForNext { body, .. } => Some(*body as usize),
            _ => None,
        };
        if let Some(to) = back.filter(|to| *to <= pc) {
            loops.push((to, pc));
        }
    }
    let steady = |r: R| match writes.get(&r).map(|w| w.as_slice()) {
        None => params.contains(&r),
        Some([pc]) => !params.contains(&r) && !loops.iter().any(|(a, b)| (*a..=*b).contains(pc)),
        Some(_) => false,
    };
    let mut slots: HashMap<R, u16> = HashMap::new();
    let slot_of = |r: R, slots: &mut HashMap<R, u16>| -> Option<u16> {
        if !steady(r) || slots.len() >= u16::MAX as usize {
            return None;
        }
        let n = slots.len() as u16;
        Some(*slots.entry(r).or_insert(n))
    };
    for ins in code.iter_mut() {
        match ins {
            Ins::Call { target, args, known, .. } if matches!(&**target, Target::Row(row) if matches!(row.id, "core.slice_str" | "core.len_text")) && !args.is_empty() => {
                *known = slot_of(args[0].0, &mut slots);
            }
            Ins::Index { base, how, .. } if matches!(&how.ir, Expr::Index { base: b, .. } if matches!(b.ty().payload(), Ty::Str)) => {
                how.known = slot_of(*base, &mut slots);
            }
            Ins::Len { base, how, known, .. } if matches!(&how.0, Target::Row(row) if row.id == "core.len_text") => {
                *known = slot_of(*base, &mut slots);
            }
            _ => {}
        }
    }
    slots.len()
}

/// Whether a typed program runs on bare registers alone: each parameter
/// an int or a float, and every instruction one that reads and writes bare
/// registers only. Its one way back to the boxed program is a typed call
/// answering what its register cannot hold, which sizes the boxed
/// registers first.
fn pure(t: &Typed, params: &[R]) -> bool {
    params.iter().all(|r| matches!(t.kinds[*r as usize], RKind::Int | RKind::Float))
        && t.lists.is_empty()
        && t.code.iter().all(|ins| {
            matches!(
                ins,
                TIns::Nop
                    | TIns::Jump { .. }
                    | TIns::Loop { .. }
                    | TIns::Cancel
                    | TIns::IConst { .. }
                    | TIns::FConst { .. }
                    | TIns::IMov { .. }
                    | TIns::FMov { .. }
                    | TIns::IArith { .. }
                    | TIns::IAddK { .. }
                    | TIns::ISubK { .. }
                    | TIns::IAddR { .. }
                    | TIns::ISubR { .. }
                    | TIns::IJump { .. }
                    | TIns::IJumpR { .. }
                    | TIns::IJumpK { .. }
                    | TIns::INeg { .. }
                    | TIns::FArith { .. }
                    | TIns::FNeg { .. }
                    | TIns::I2F { .. }
                    | TIns::ForCount { .. }
                    | TIns::ForNextCount { .. }
                    | TIns::IRet { .. }
                    | TIns::FRet { .. }
                    | TIns::CallT { .. }
            )
        })
}

fn typed(p: &Body) -> Option<Typed> {
    // a list of ints stays bare only where every instruction that meets it
    // is a list instruction or makes it (a list a call takes, a loop walks,
    // a fallback names or the body answers is boxed: converting it there
    // would cost a pass over it each time)
    let mut kinds = p.kinds.clone();
    // (a parameter's list arrives boxed, and is read where it lies: made
    // bare, it was converted at every call — a binary search's seventeen
    // reads of a list of 80,000 had read all 80,000 first, every call)
    for r in p.params.iter() {
        if kinds[*r as usize] == RKind::IntList {
            kinds[*r as usize] = RKind::Boxed;
        }
    }
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
            Ins::Small { d, s, .. } => wrap(&[*s], &[*d]),
            Ins::Jump { to } => TIns::Jump { to: *to },
            Ins::Loop { to } => TIns::Loop { to: *to },
            Ins::Cancel => TIns::Cancel,
            Ins::JumpUnless { c, .. } => wrap(&[*c], &[]),
            Ins::JumpDecided { c, d, .. } => wrap(&[*c], &[*d]),
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
            Ins::Record { d, fields, .. } => {
                let reads: Vec<R> = fields.iter().map(|(r, _)| *r).collect();
                wrap(&reads, &[*d])
            }
            Ins::Field { d, base, how, .. } => {
                let mut reads = vec![*base];
                crate::ir::visit_expr(&how.0, &mut |e| {
                    if let Expr::Local(n, _) | Expr::Move(n, _) = e
                        && let Some(r) = how.1.get(n.as_str())
                    {
                        reads.push(*r);
                    }
                });
                if reads.iter().any(|r| lk(*r)) {
                    lend(ins)
                } else {
                    wrap(&reads, &[*d])
                }
            }
            Ins::Index { d, base, idx, how } => match top(*idx) {
                Some(i) if lk(*base) => TIns::LIndex { d: *d, base: *base, idx: i },
                Some(i) if ik(*d) && !how.column => TIns::IIndex { d: *d, base: *base, idx: i },
                Some(i) if fk(*d) && !how.column => TIns::FIndex { d: *d, base: *base, idx: i },
                Some(i) if !tk(*d) => TIns::XIndex { d: *d, base: *base, idx: i },
                // a key that is no int (`counts[w]`): the boxed read, with the
                // registers it reads boxed — the walker it may lend to reads
                // only what the expression names — and its answer unboxed
                // (every typed register boxed both ways had cost more than
                // the read)
                _ => {
                    let mut reads = vec![*base];
                    reads.extend(regs_of(&[*idx]));
                    crate::ir::visit_expr(&how.ir, &mut |e| {
                        if let Expr::Local(n, _) | Expr::Move(n, _) = e
                            && let Some(r) = how.env.get(n.as_str())
                        {
                            reads.push(*r);
                        }
                    });
                    if reads.iter().any(|r| lk(*r)) {
                        lend(ins)
                    } else {
                        wrap(&reads, &[*d])
                    }
                }
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
            // `float[n]` of an int: the conversion itself (the boxed program
            // keeps the row, for a value the IR could not see)
            Ins::Call { d, target, args, .. } if matches!(&**target, Target::Row(row) if row.id == "core.float_of_int") && matches!(&args[..], [(s, _)] if ik(*s)) && fk(*d) => TIns::I2F { d: *d, s: args[0].0 },
            // (an int or a float argument is never a failure: nothing to
            // propagate)
            Ins::Call { d, target, args, .. } if (ik(*d) || fk(*d)) && args.iter().all(|(r, _)| ik(*r) || fk(*r)) && matches!(&**target, Target::User(n) if n != PUT) => {
                let Target::User(name) = &**target else { return wrap(&[], &[*d]) };
                TIns::CallT { d: *d, name: name.as_str().into(), args: args.iter().map(|(r, _)| *r).collect() }
            }
            Ins::Call { d, args, .. } | Ins::Effect { d, args, .. } => wrap(&args.iter().map(|(r, _)| *r).collect::<Vec<R>>(), &[*d]),
            Ins::Store { s, .. } | Ins::Do { s } => wrap(&[*s], &[]),
            Ins::Propagate { regs, .. } => wrap(regs, &[]),
            Ins::Return { s: Some((r, _)) } if ik(*r) => TIns::IRet { s: *r },
            Ins::Return { s: Some((r, _)) } if fk(*r) => TIns::FRet { s: *r },
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
                boxed_sized(regs, p);
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
                // (an answer past 64 bits, a zero divisor, a negative power:
                // the boxed program runs the instruction — it grows the int,
                // or stops — and the rest of the call)
                TIns::IArith { op, d, a, b } => match op.arith(iv(&bare.ints, *a), iv(&bare.ints, *b)) {
                    Ok(n) => bare.ints[*d as usize] = n,
                    Err(_) => hand_over!(),
                },
                TIns::IAddK { d, a, k } => match bare.ints[*a as usize].checked_add(*k) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => hand_over!(),
                },
                TIns::ISubK { d, a, k } => match bare.ints[*a as usize].checked_sub(*k) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => hand_over!(),
                },
                TIns::IAddR { d, a, b } => match bare.ints[*a as usize].checked_add(bare.ints[*b as usize]) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => hand_over!(),
                },
                TIns::ISubR { d, a, b } => match bare.ints[*a as usize].checked_sub(bare.ints[*b as usize]) {
                    Some(n) => bare.ints[*d as usize] = n,
                    None => hand_over!(),
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
                    Err(_) => hand_over!(),
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
                    } else if let (Value::Str(s), Some(k)) = (&b.value, how.known) {
                        match crow_runtime::text::char_at_known(s, &self.frames[self.frames.len() - 1].texts[k as usize], i) {
                            Ok(c) => Some(CrowValue::ok(Value::Str(c))),
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
                            Ok(k) => std::sync::Arc::make_mut(items)[k].set_int(n),
                            Err(e) => stop!(e),
                        },
                        // a tensor's number, set where it lies (as `set_path`)
                        Value::NumArray(t) if ok => {
                            if let Err(e) = crow_runtime::core::set_flat_at(t, i, n as f64) {
                                stop!(e)
                            }
                        }
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
                            Ok(k) => std::sync::Arc::make_mut(items)[k].set_float(x),
                            Err(e) => stop!(e),
                        },
                        // a tensor's number, set where it lies (as `set_path`)
                        Value::NumArray(t) if ok => {
                            if let Err(e) = crow_runtime::core::set_flat_at(t, i, x) {
                                stop!(e)
                            }
                        }
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
                            Ok(k) => std::sync::Arc::make_mut(items)[k].assign(new),
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
                    let Ins::Len { how, known, .. } = &p.code[pc] else { hand_over!() };
                    let row_named = matches!(how.0, Target::Row(_));
                    // any value its row measures, read where it lies (a text
                    // had handed the call over to the boxed program, which
                    // ran the rest of it: k-nucleotide's counting loop ran
                    // boxed after `len[seq]` at its top); a failure, or a
                    // module's own `len`, the boxed program's
                    let n = match &b.value {
                        _ if !b.is_ok() || (!row_named && self.registry.has_function("len")) => None,
                        Value::Array(items) => Some(items.len() as i64),
                        Value::Str(s) if known.is_some() => known.map(|k| crow_runtime::text::len_known(s, &self.frames[self.frames.len() - 1].texts[k as usize])),
                        _ => Self::len_of(&how.0, b, "").ok().and_then(|v| v.plain_int()),
                    };
                    match n {
                        Some(n) => bare.ints[*d as usize] = n,
                        None => hand_over!(),
                    }
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
                TIns::IRet { s } => return Ok(Flow::Return(CrowValue::ok(Value::Int(bare.ints[*s as usize])))),
                TIns::FRet { s } => return Ok(Flow::Return(CrowValue::ok(Value::Float(bare.floats[*s as usize])))),
                TIns::CallT { d, name, args } => {
                    self.line_of(p, pc);
                    let v = match self.call_typed(name, &tc.kinds, bare, args) {
                        Ok(v) => v,
                        Err(e) => stop!(e),
                    };
                    let i = *d as usize;
                    match (tc.kinds[i], v.plain_int(), v.plain_float()) {
                        (RKind::Int, Some(n), _) => bare.ints[i] = n,
                        (RKind::Float, _, Some(x)) => bare.floats[i] = x,
                        // an answer the register cannot hold (a dual number a
                        // function `grad` differentiates computes with): the
                        // boxed program goes on after the call
                        _ => {
                            boxed_sized(regs, p);
                            regs[i].assign(v);
                            self.box_typed(tc, regs, bare, &[*d]);
                            return self.run_code_from(p, regs, pc + 1);
                        }
                    }
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
