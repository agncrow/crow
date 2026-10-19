//! How large each int may grow (spec, Numbers That Keep Every Digit). An int
//! is a whole number of any size in both modes; the compiled program holds
//! one in 64 bits where this pass shows it never needs more, and as the
//! runtime's growing int (`Ty::Big`, `crow_runtime::int::Int`) where it
//! cannot. The interpreter's ints grow as they run, so the pass decides a
//! representation, never an answer.
//!
//! A place an int is held — a local, a function's answer, a record's
//! field, a fact, the items of a list or the values of a map held in one of
//! those — has a range: the least and the most it may hold, unbounded where
//! nothing bounds it. A write joins the written value's range into its
//! place's, and a value's range follows from its parts' (a sum from its
//! addends', a remainder by `n` within `n`, a length from 0 to 2^48, an int
//! a builtin answers within 64 bits). The loops refine what a range alone
//! would not: a counter moved by a constant toward a bound its `while`
//! tests stays within the bound (`i = i + 1` under `i lt n`, and `j = j - 1`
//! under `i lt j` within `i`), and a sum — a write that adds to what its
//! place holds (`t = t + e`, `xs[i] = xs[i] + 1`, a fact a handler adds to,
//! a recursion's answer summing its own calls, `make[depth - 1]`) — moves
//! by e at most as many times as its statement runs: the rounds of one entry
//! of its loop where the loop's block sets it afresh, a `for`'s count where
//! no loop holds that `for`, and else fewer than 2^ROUNDS (no part of a
//! program runs 2^48 times). A place whose range passes 64 bits grows;
//! places a list or a map flows between share one range, since they share
//! one representation.
use crate::ir::{Block, Const, Expr, PathStep, Program, Stmt, Target, Ty};
use std::collections::{BTreeSet, HashMap};

/// A count of items: a list holds fewer than 2^48.
const COUNT: i128 = 1 << 48;
/// Bits a count of rounds needs: no part of a program runs 2^48 times (days
/// of computing at a billionth of a second a round), so a sum taken in a
/// loop or over a recursion's calls moves by its addend fewer times than
/// this. A program that ran longer and grew such a sum past 64 bits stops
/// with integer overflow, never a wrong digit.
const ROUNDS: u32 = 48;
/// Past this, a bound is no bound (and no arithmetic on it overflows).
const INF: i128 = 1 << 100;
/// How many times a place's range may grow before the side that grows is
/// taken as unbounded (the rounds then end).
const WIDENINGS: u32 = 12;

/// The values an int may hold, both ends included; `lo > hi` is a place
/// nothing has written yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Iv {
    lo: i128,
    hi: i128,
}

fn sat(x: i128) -> i128 {
    x.clamp(-INF, INF)
}

impl Iv {
    const EMPTY: Iv = Iv { lo: INF, hi: -INF };
    const TOP: Iv = Iv { lo: -INF, hi: INF };
    /// Any 64-bit int: what a builtin answers, a cell, a parsed number.
    const I64: Iv = Iv { lo: i64::MIN as i128, hi: i64::MAX as i128 };
    /// A truth read as an int.
    const BOOL: Iv = Iv { lo: 0, hi: 1 };

    fn new(lo: i128, hi: i128) -> Iv {
        Iv { lo: sat(lo), hi: sat(hi) }
    }

    fn point(n: i128) -> Iv {
        Iv::new(n, n)
    }

    fn is_empty(self) -> bool {
        self.lo > self.hi
    }

    /// Whether every value fits a 64-bit int.
    fn fits(self) -> bool {
        self.is_empty() || (self.lo >= i64::MIN as i128 && self.hi <= i64::MAX as i128)
    }

    fn join(self, o: Iv) -> Iv {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        Iv { lo: self.lo.min(o.lo), hi: self.hi.max(o.hi) }
    }

    fn add(self, o: Iv) -> Iv {
        if self.is_empty() || o.is_empty() {
            return Iv::EMPTY;
        }
        Iv::new(self.lo + o.lo, self.hi + o.hi)
    }

    fn neg(self) -> Iv {
        if self.is_empty() {
            return self;
        }
        Iv { lo: -self.hi, hi: -self.lo }
    }

    fn sub(self, o: Iv) -> Iv {
        self.add(o.neg())
    }

    fn mul(self, o: Iv) -> Iv {
        if self.is_empty() || o.is_empty() {
            return Iv::EMPTY;
        }
        let m = |a: i128, b: i128| a.checked_mul(b).map(sat).unwrap_or(if (a < 0) != (b < 0) { -INF } else { INF });
        let c = [m(self.lo, o.lo), m(self.lo, o.hi), m(self.hi, o.lo), m(self.hi, o.hi)];
        Iv::new(c.iter().copied().min().unwrap_or(-INF), c.iter().copied().max().unwrap_or(INF))
    }

    /// The largest magnitude.
    fn mag(self) -> i128 {
        self.lo.abs().max(self.hi.abs())
    }

    /// A quotient rounded toward zero: no larger than the dividend, and
    /// within the corners where the divisor holds no zero.
    fn div(self, o: Iv) -> Iv {
        if self.is_empty() || o.is_empty() {
            return Iv::EMPTY;
        }
        if o.lo <= 0 && o.hi >= 0 {
            let m = self.mag();
            return Iv::new(-m, m);
        }
        let d = |a: i128, b: i128| if b == 0 { 0 } else { a / b };
        let c = [d(self.lo, o.lo), d(self.lo, o.hi), d(self.hi, o.lo), d(self.hi, o.hi)];
        Iv::new(c.iter().copied().min().unwrap_or(-INF), c.iter().copied().max().unwrap_or(INF))
    }

    /// A remainder: smaller than the divisor, with the dividend's sign.
    fn rem(self, o: Iv) -> Iv {
        if self.is_empty() || o.is_empty() {
            return Iv::EMPTY;
        }
        let m = (o.mag() - 1).max(0);
        let lo = if self.lo >= 0 { 0 } else { (-m).max(self.lo) };
        let hi = if self.hi <= 0 { 0 } else { m.min(self.hi) };
        Iv::new(lo, hi)
    }

    /// A power by a range of exponents of none or more.
    fn pow(self, e: Iv) -> Iv {
        if self.is_empty() || e.is_empty() {
            return Iv::EMPTY;
        }
        if e.lo < 0 {
            // (an int to a negative power is refused as the program runs)
            return Iv::TOP;
        }
        if self.lo >= 0 && self.hi <= 1 {
            return Iv::new(0, 1);
        }
        if self.lo >= -1 && self.hi <= 1 {
            return Iv::new(-1, 1);
        }
        let b = self.mag();
        if e.hi > 128 {
            return Iv::TOP;
        }
        let mut p: i128 = 1;
        for _ in 0..e.hi {
            p = match p.checked_mul(b) {
                Some(x) if x < INF => x,
                _ => return if self.lo < 0 { Iv::TOP } else { Iv::new(0, INF) },
            };
        }
        if self.lo < 0 { Iv::new(-p, p) } else { Iv::new(0, p) }
    }

    /// The range scaled by up to `2^bits` rounds of moving by it (a sum's
    /// drift): its negative part and its positive part, each times the count.
    fn drift(self, bits: u32) -> Iv {
        if self.is_empty() {
            return Iv::point(0);
        }
        let r: i128 = 1i128 << bits.min(100);
        let m = |a: i128| a.checked_mul(r).map(sat).unwrap_or(if a < 0 { -INF } else { INF });
        Iv::new(m(self.lo.min(0)), m(self.hi.max(0)))
    }

    /// The bits a count from 0 to the range's most needs.
    fn count_bits(self) -> u32 {
        if self.is_empty() || self.hi <= 0 {
            return 0;
        }
        (128 - self.hi.leading_zeros()).min(100)
    }
}

/// Where an int is held.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Place {
    /// A function's local or parameter ("" is the top level).
    Local(String, String),
    /// A function's answer.
    Ret(String),
    /// A record's field.
    Field(String, String),
    Fact(String),
    /// The items of a list (the values of a map) held at a place.
    Items(Box<Place>),
    /// A list made where it is used (a literal, a range, a loop's yield).
    Made(usize),
}

/// The places the program holds an int in that may grow past 64 bits.
#[derive(Default, Debug)]
pub struct Widths {
    pub wide: BTreeSet<Place>,
}

impl Widths {
    pub fn grows(&self, p: &Place) -> bool {
        self.wide.contains(p)
    }
}

/// The places' ranges, settled over the whole program.
#[allow(dead_code)]
pub fn widths(program: &Program) -> Widths {
    settle(program).0
}

/// The walk the ranges settled in, kept for the rewrite to ask an
/// expression's range (an intermediate value is no place: `2 ^ 100` grows
/// where nothing holds it).
fn settle(program: &Program) -> (Widths, Walk) {
    let fns: HashMap<String, Vec<String>> = program.parts.iter().flat_map(|p| p.fns.iter()).map(|f| (f.name.clone(), f.params.iter().map(|(p, _)| p.clone()).collect())).collect();
    let structs: HashMap<String, Vec<(String, Ty)>> = program.parts.iter().flat_map(|p| p.structs.iter()).map(|s| (s.name.clone(), s.fields.clone())).collect();
    let mut types: HashMap<(String, String), Ty> = HashMap::new();
    for f in program.parts.iter().flat_map(|p| p.fns.iter()) {
        for (p, ty) in &f.params {
            types.insert((f.name.clone(), p.clone()), ty.clone());
        }
    }
    let mut w = Walk {
        fns,
        structs,
        types,
        temps: HashMap::new(),
        pending_fresh: Default::default(),
        range: HashMap::new(),
        base: HashMap::new(),
        drifts: HashMap::new(),
        grown: HashMap::new(),
        parent: HashMap::new(),
        made: 0,
        changed: false,
        loops: Vec::new(),
        fun: String::new(),
        cur: HashMap::new(),
        written: Default::default(),
        rels: Vec::new(),
        bounds: HashMap::new(),
        thresholds: thresholds_of(program),
        frozen: false,
        soft: Default::default(),
        accs: HashMap::new(),
        acc_parts: HashMap::new(),
    };
    // writes only widen a range, and a range that keeps growing is taken as
    // unbounded: the rounds end
    for _ in 0..400 {
        w.changed = false;
        w.made = 0;
        w.drifts.clear();
        for part in &program.parts {
            for f in &part.facts {
                let r = w.iv_of(&f.default);
                w.widen_base(&Place::Fact(f.name.clone()), r);
                w.flow(&Place::Fact(f.name.clone()), &f.default);
            }
            for f in &part.fns {
                w.body(&f.name, &f.body);
            }
            // a derivation is a function of no parameters, read by name
            for d in &part.derives {
                w.body(&derive_fn(&d.name), &d.body);
            }
            for (i, test) in part.tests.iter().enumerate() {
                w.body(&test_fn(i), &test.body);
            }
            w.body("", &part.main);
        }
        if !w.changed {
            break;
        }
    }
    if std::env::var("CROW_WIDTH_DEBUG").is_ok() {
        let mut all: Vec<(&Place, &Iv)> = w.range.iter().collect();
        all.sort_by(|a, b| a.0.cmp(b.0));
        for (p, r) in all {
            eprintln!("width: {:?} = [{}, {}] base {:?}", p, r.lo, r.hi, w.base.get(p));
        }
    }
    w.frozen = true;
    w.cur.clear();
    w.rels.clear();
    w.bounds.clear();
    w.written.clear();
    let mut wide = BTreeSet::new();
    let places: Vec<Place> = w.range.keys().chain(w.parent.keys()).cloned().collect();
    for p in places {
        if !w.get(&p).fits() {
            wide.insert(p);
        }
    }
    (Widths { wide }, w)
}

/// The name a derivation's places go by (no function is named so).
fn derive_fn(name: &str) -> String {
    format!("derive {}", name)
}

/// The name a test block's places go by.
fn test_fn(i: usize) -> String {
    format!("test {}", i)
}

/// Arithmetic of 64-bit ints whose answer may not fit them: `2 ^ 100`, a
/// product of two counts.
fn wide_arith(walk: &mut Walk, e: &Expr) -> bool {
    match e {
        Expr::Binary { op, ty, .. } if matches!(op.as_str(), "add" | "sub" | "mul" | "pow") && matches!(ty.payload(), Ty::Int) => !walk.iv_of(e).fits(),
        // (a shift whose answer may pass 64 bits: its form that grows)
        Expr::Call { target: Target::Row(row), .. } if row.id == "bits.shift" => !walk.iv_of(e).fits(),
        _ => false,
    }
}

/// Whether any arithmetic in the program may grow past 64 bits where no
/// place holds it (`log[2 ^ 100]`).
fn grows_anywhere(program: &Program, walk: &mut Walk) -> bool {
    let mut any = false;
    for part in &program.parts {
        for f in &part.fns {
            walk.fun = f.name.clone();
            crate::ir::visit_block(&f.body, &mut |x| any |= wide_arith(walk, x));
        }
        walk.fun = String::new();
        crate::ir::visit_block(&part.main, &mut |x| any |= wide_arith(walk, x));
        for f in &part.facts {
            crate::ir::visit_expr(&f.default, &mut |x| any |= wide_arith(walk, x));
        }
        for d in &part.derives {
            walk.fun = derive_fn(&d.name);
            crate::ir::visit_block(&d.body, &mut |x| any |= wide_arith(walk, x));
        }
        for (i, test) in part.tests.iter().enumerate() {
            walk.fun = test_fn(i);
            crate::ir::visit_block(&test.body, &mut |x| any |= wide_arith(walk, x));
        }
    }
    any
}

/// A loop the walk is inside.
struct LoopCtx {
    /// The bits of a `for`'s count of rounds, where no loop holds it.
    rounds: Option<u32>,
    /// The bits of the rounds one entry of the loop runs: a `for` over a
    /// range's or a list's count, a `while` whose counter steps toward its
    /// bound.
    entry: Option<u32>,
    /// The locals the statements before the loop in its block set afresh
    /// (`off = 0` before it): set again before every entry, so a sum the
    /// loop takes of one moves only by the rounds of one entry.
    fresh: std::collections::HashSet<String>,
}

struct Walk {
    /// Each function's parameters, by name.
    fns: HashMap<String, Vec<String>>,
    /// Each record's fields, in their declared order.
    structs: HashMap<String, Vec<(String, Ty)>>,
    /// The declared type of each function's locals and parameters.
    types: HashMap<(String, String), Ty>,
    /// The value of each temporary the lowering made (`_t1`), by function.
    temps: HashMap<(String, String), Expr>,
    /// The locals set afresh before the loop about to be walked.
    pending_fresh: std::collections::HashSet<String>,
    /// Each place's range (at its set's root).
    range: HashMap<Place, Iv>,
    /// What the writes that are no sum put in each place: where a sum starts.
    base: HashMap<Place, Iv>,
    /// This round's drift of each place its sums move (each sum's added).
    drifts: HashMap<Place, Iv>,
    /// How many times each place's range has grown.
    grown: HashMap<Place, u32>,
    /// Places joined so they share one range (a list flowed between them).
    parent: HashMap<Place, Place>,
    made: usize,
    changed: bool,
    loops: Vec<LoopCtx>,
    fun: String,
    /// The range each local of the function being walked holds where the
    /// walk stands — set by the writes before here in its straight line,
    /// narrowed by the conditions it is under; a local not here holds its
    /// place's range (a loop's body drops what it assigns, whose values
    /// come round again).
    cur: HashMap<String, Iv>,
    /// The locals the straight line has written on every path to where the
    /// walk stands (since its loop's round or its function began): a write
    /// of one is no sum coming round a loop.
    written: std::collections::HashSet<String>,
    /// What the conditions the walk stands under say between two locals:
    /// `(a, b, strict)` is a below b (or at most b) — so `b - a` is at
    /// least one (or none) — until either is written.
    rels: Vec<(String, String, bool)>,
    /// The sides of a local's range the conditions the walk stands under
    /// bound with a value of their own — `(below, above)`: `i lt n` bounds
    /// `i` above, `i gte 0` below, `i eq n` both — as against `k ne 0`,
    /// which only moves an end of `k`'s own range by one.
    bounds: HashMap<String, (bool, bool)>,
    /// The bounds a range that keeps growing moves to: the ladder and the
    /// program's own constants, each and its neighbours.
    thresholds: Vec<i128>,
    /// Settled: the rewrite asks ranges, and nothing it asks widens one.
    frozen: bool,
    /// The places (roots) whose range rests on ROUNDS — a sum bounded by no
    /// count the program states, a recursion's answer — or reads one: their
    /// bound holds only as long as no part of the program runs 2^48 times,
    /// so arithmetic on them keeps its check (Convert::Proven).
    soft: std::collections::HashSet<Place>,
    /// The function walked's locals that add its own answers and are its
    /// answer (`accumulators`), each with the bits its count of writes adds
    /// to ROUNDS.
    accs: HashMap<String, u32>,
    /// Every part an accumulator's writes have added, in every pass.
    acc_parts: HashMap<(String, String), Iv>,
}

impl Walk {
    fn root(&self, p: &Place) -> Place {
        let mut cur = p.clone();
        while let Some(next) = self.parent.get(&cur) {
            if *next == cur {
                break;
            }
            cur = next.clone();
        }
        cur
    }

    fn get(&self, p: &Place) -> Iv {
        let r = self.root(p);
        self.range.get(&r).copied().unwrap_or(Iv::EMPTY)
    }

    fn base_of(&self, p: &Place) -> Iv {
        let r = self.root(p);
        self.base.get(&r).copied().unwrap_or(Iv::EMPTY)
    }

    /// A place's range joined with more; one that keeps growing becomes
    /// unbounded on the side it grows.
    fn widen(&mut self, p: &Place, iv: Iv) {
        if self.frozen {
            return;
        }
        let r = self.root(p);
        let cur = self.range.get(&r).copied().unwrap_or(Iv::EMPTY);
        let mut next = cur.join(iv);
        if next != cur {
            let n = self.grown.entry(r.clone()).or_insert(0);
            *n += 1;
            // a range that keeps growing moves to the next of a ladder of
            // bounds (counts, 32, 48, 53, 63 and 64 bits), then to none
            if *n > WIDENINGS && !cur.is_empty() {
                if next.lo < cur.lo {
                    next.lo = self.thresholds.iter().rev().copied().find(|b| *b <= next.lo).unwrap_or(-INF);
                }
                if next.hi > cur.hi {
                    next.hi = self.thresholds.iter().copied().find(|b| *b >= next.hi).unwrap_or(INF);
                }
            }
            self.changed = true;
            self.range.insert(r, next);
        } else if !self.range.contains_key(&r) {
            self.range.insert(r, next);
        }
    }

    /// A write that is no sum: its place's start, and its range.
    fn widen_base(&mut self, p: &Place, iv: Iv) {
        if self.frozen {
            return;
        }
        let r = self.root(p);
        let cur = self.base.get(&r).copied().unwrap_or(Iv::EMPTY);
        self.base.insert(r, cur.join(iv));
        self.widen(p, iv);
    }

    /// Two places share one range from here on.
    fn join(&mut self, a: &Place, b: &Place) {
        if self.frozen {
            return;
        }
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return;
        }
        let ia = self.range.get(&ra).copied().unwrap_or(Iv::EMPTY);
        let ib = self.range.remove(&rb).unwrap_or(Iv::EMPTY);
        let ba = self.base.get(&ra).copied().unwrap_or(Iv::EMPTY);
        let bb = self.base.remove(&rb).unwrap_or(Iv::EMPTY);
        if self.soft.remove(&rb) {
            self.soft.insert(ra.clone());
        }
        self.parent.insert(rb, ra.clone());
        self.range.insert(ra.clone(), ia.join(ib));
        self.base.insert(ra, ba.join(bb));
        self.changed = true;
    }

    fn local(&self, v: &str) -> Place {
        Place::Local(self.fun.clone(), v.to_string())
    }

    /// The place an expression's list (or map) lives at, where it reads
    /// one; a list it makes gets a place of its own.
    fn place_of(&mut self, e: &Expr) -> Option<Place> {
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => Some(self.local(v)),
            Expr::Load(f, _) => Some(Place::Fact(f.clone())),
            Expr::Convert { expr, .. } => self.place_of(expr),
            // (a record made where it is read — `Box[…].v` — lends no type:
            // its own is read, or its field had read as any 64-bit int)
            Expr::Field { base, name, .. } => match Some(base.ty()).as_ref().map(|t| t.payload()) {
                Some(Ty::Record(s)) => Some(Place::Field(s.clone(), name.clone())),
                _ => None,
            },
            Expr::Index { base, .. } => self.place_of(base).map(|p| Place::Items(Box::new(p))),
            Expr::Call { target: Target::User(g), .. } => Some(Place::Ret(g.clone())),
            _ => {
                self.made += 1;
                Some(Place::Made(self.made))
            }
        }
    }

    /// A list or a map flowed into a place: their items share one range, and
    /// what a made list holds is written into it.
    fn flow(&mut self, to: &Place, e: &Expr) {
        if !holds_ints(e.ty_ref()) {
            return;
        }
        let Some(from) = self.place_of(e) else { return };
        self.flow_items(to, &from, e.ty_ref().map(|t| t.payload().clone()).unwrap_or(Ty::Unknown));
        self.made_items(&from, e);
    }

    /// A list handed to a parameter: the parameter's items hold what the
    /// argument's hold, at every depth. Nothing flows back — the callee
    /// has its own copy (a write there never reaches the caller's) — so
    /// the lists two calls hand one function stay apart: `report[ones]`
    /// and `report[twos]`, twos made from ones' items, had joined into one
    /// range that grew round each pass.
    fn flow_into(&mut self, to: &Place, e: &Expr) {
        if !holds_ints(e.ty_ref()) {
            return;
        }
        let Some(from) = self.place_of(e) else { return };
        self.made_items(&from, e);
        self.flow_items_into(to, &from, e.ty_ref().map(|t| t.payload().clone()).unwrap_or(Ty::Unknown));
    }

    fn flow_items_into(&mut self, to: &Place, from: &Place, ty: Ty) {
        if let Ty::List(inner) | Ty::Map(_, inner) = ty {
            let (a, b) = (Place::Items(Box::new(to.clone())), Place::Items(Box::new(from.clone())));
            if self.root(&a) != self.root(&b) {
                let r = self.get(&b);
                self.widen_base(&a, r);
                if self.soft.contains(&self.root(&b)) {
                    self.mark_soft(&a);
                }
            }
            self.flow_items_into(&a, &b, *inner);
        }
    }

    /// The items of two places of a type holding ints, joined at every
    /// depth (a list of lists of ints: its lists' items too).
    fn flow_items(&mut self, to: &Place, from: &Place, ty: Ty) {
        if let Ty::List(inner) | Ty::Map(_, inner) = ty {
            let (a, b) = (Place::Items(Box::new(to.clone())), Place::Items(Box::new(from.clone())));
            self.join(&a, &b);
            self.flow_items(&a, &b, *inner);
        }
    }

    /// The range of a range builtin's items, of its bounds' ranges: within
    /// 64 bits (a bound past them is refused where it is handed), and
    /// fewer than COUNT from its start — no part of a program runs 2^48
    /// times, nor holds a list that long. A range bounded by that alone (a
    /// bound parsed from text, which grows) rests on ROUNDS: `soft` is
    /// marked on the place its items go to.
    fn range_items(&mut self, args: &[Expr], to: &Place) -> Iv {
        let ivs: Vec<Iv> = args.iter().map(|a| self.iv_of(a)).collect();
        // (a range's forms by their count: to a bound, from one to another,
        // and by a step)
        let (r, counted) = match ivs.as_slice() {
            [n] => (Iv::new(0, (n.hi - 1).max(0)), Iv::new(0, COUNT - 1)),
            [a, b] => {
                let r = Iv::new(a.lo, (b.hi - 1).max(a.lo));
                (r, Iv::new(a.lo.max(b.lo.saturating_sub(COUNT)), (b.hi - 1).min(a.hi.saturating_add(COUNT)).max(a.lo)))
            }
            [a, b, _] => (a.join(*b), Iv::I64),
            _ => (Iv::I64, Iv::I64),
        };
        if r.is_empty() {
            return r;
        }
        let bounded = Iv::new(r.lo.max(counted.lo).max(i64::MIN as i128), r.hi.min(counted.hi).min(i64::MAX as i128));
        if bounded != r {
            self.mark_soft(to);
        }
        bounded
    }

    /// What a made list holds, written into its items' place.
    fn made_items(&mut self, at: &Place, e: &Expr) {
        let items = Place::Items(Box::new(at.clone()));
        match e {
            Expr::ListLit(items_e, _) => {
                for i in items_e {
                    let r = self.iv_of(i);
                    self.widen_base(&items, r);
                    self.flow(&items, i);
                }
            }
            Expr::MapLit(pairs, _) => {
                for (_, v) in pairs {
                    let r = self.iv_of(v);
                    self.widen_base(&items, r);
                    self.flow(&items, v);
                }
            }
            Expr::Convert { expr, .. } => self.made_items(at, expr),
            // (each round's value: the body's last line, or each branch's)
            Expr::ForExpr { body, .. } => {
                let mut ys = Vec::new();
                tails_in(body, &mut ys);
                for y in ys {
                    let r = self.iv_of(y);
                    self.widen_base(&items, r);
                    self.flow(&items, y);
                }
            }
            Expr::Call { target: Target::Row(row), args, .. } => match row.props.contains(&crate::table::Prop::Range) {
                // the items of a range lie between its bounds
                true => {
                    let r = self.range_items(args, &items);
                    self.widen_base(&items, r);
                }
                // a list the row makes of the lists it is given (push, concat,
                // slice, sort, reverse…): their items and its own share a range,
                // and a pushed item is written into them
                false => {
                    for a in args {
                        if holds_ints(a.ty_ref()) && same_shape(a.ty_ref(), e.ty_ref()) {
                            if let Some(p) = self.place_of(a) {
                                let ty = e.ty_ref().map(|t| t.payload().clone()).unwrap_or(Ty::Unknown);
                                self.flow_items(at, &p, ty);
                                self.made_items(&p, a);
                            }
                        } else if is_int(a.ty_ref()) && item_of(e.ty_ref()) {
                            self.write(&items, a);
                        }
                    }
                }
            },
            _ => {}
        }
    }

    /// The range of an int expression.
    fn iv_of(&mut self, e: &Expr) -> Iv {
        match e {
            Expr::Const(Const::Int(n), _) => Iv::point(*n as i128),
            Expr::Const(Const::Bool(_), _) => Iv::BOOL,
            Expr::Const(..) => Iv::I64,
            Expr::Local(v, _) | Expr::Move(v, _) => self.local_iv(v),
            Expr::Load(f, _) => self.get(&Place::Fact(f.clone())),
            Expr::Derived(d, _) => self.get(&Place::Ret(derive_fn(d))),
            // (a record made where it is read — `Box[…].v` — lends no type:
            // its own is read, or its field had read as any 64-bit int)
            Expr::Field { base, name, .. } => match Some(base.ty()).as_ref().map(|t| t.payload()) {
                Some(Ty::Record(s)) => self.get(&Place::Field(s.clone(), name.clone())),
                // (a frame's column, a parsed value's field: 64-bit cells)
                _ => Iv::I64,
            },
            Expr::Index { base, .. } => match base.ty_ref().map(|t| t.payload()) {
                Some(Ty::List(_)) | Some(Ty::Map(..)) => match self.place_of(base) {
                    Some(p) => {
                        self.made_items(&p, base);
                        self.get(&Place::Items(Box::new(p)))
                    }
                    None => Iv::TOP,
                },
                // a tensor's numbers are floats: a whole one within 2^53
                Some(Ty::Tensor(..)) => Iv::new(-(1 << 53), 1 << 53),
                _ => Iv::I64,
            },
            Expr::Binary { op, l, r, .. } => {
                let (a, b) = (self.iv_of(l), self.iv_of(r));
                match op.as_str() {
                    "add" => a.add(b),
                    // `b - a` where a condition holds a below b: at least one
                    "sub" => {
                        let d = a.sub(b);
                        match (local_name(l), local_name(r)) {
                            (Some(x), Some(y)) if !d.is_empty() => {
                                let mut d = d;
                                for (lo, hi, strict) in &self.rels {
                                    if *lo == y && *hi == x {
                                        d.lo = d.lo.max(if *strict { 1 } else { 0 });
                                    }
                                    if *lo == x && *hi == y {
                                        d.hi = d.hi.min(if *strict { -1 } else { 0 });
                                    }
                                }
                                d
                            }
                            _ => d,
                        }
                    }
                    "mul" => a.mul(b),
                    "div" => a.div(b),
                    "mod" => a.rem(b),
                    "pow" => a.pow(b),
                    _ => Iv::BOOL,
                }
            }
            Expr::Unary { op, expr, .. } => {
                let a = self.iv_of(expr);
                if op == "neg" { a.neg() } else { Iv::BOOL }
            }
            Expr::Call { target, args, .. } => {
                let mut ivs = Vec::with_capacity(args.len());
                for a in args {
                    ivs.push(self.iv_of(a));
                }
                match target {
                    Target::User(g) => self.get(&Place::Ret(g.clone())),
                    Target::Row(row) => row_iv(row, &ivs),
                    // (a call its values pick a row for, a module's own form:
                    // an int it answers fits 64 bits)
                    _ => Iv::I64,
                }
            }
            Expr::Convert { expr, .. } => self.iv_of(expr),
            _ => Iv::I64,
        }
    }

    /// A local's range where the walk stands.
    fn local_iv(&self, v: &str) -> Iv {
        match self.cur.get(v) {
            Some(r) => *r,
            None => self.get(&self.local(v)),
        }
    }

    /// A function's body (a derivation's, a test's, the top level's),
    /// walked from no local known.
    fn body(&mut self, name: &str, b: &Block) {
        self.fun = name.to_string();
        self.accs = accumulators(name, b);
        self.cur.clear();
        self.written.clear();
        self.rels.clear();
        self.bounds.clear();
        self.block(b);
    }

    /// The locals a condition names, narrowed to where it holds (`truth`)
    /// or fails: `i lt n` holds `i` below `n`'s most and `n` above `i`'s
    /// least; `and`, `or` and `not` as they combine.
    fn refine(&mut self, cond: &Expr, truth: bool) {
        match cond {
            Expr::Binary { op, l, r, .. } if (op == "and" && truth) || (op == "or" && !truth) => {
                self.refine(l, truth);
                self.refine(r, truth);
            }
            Expr::Unary { op, expr, .. } if op == "not" => self.refine(expr, !truth),
            Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "lt" | "lte" | "gt" | "gte" | "eq" | "ne") => {
                let op = if truth { op.clone() } else { negated(op) };
                if let (Some(a), Some(b)) = (local_name(l), local_name(r)) {
                    match op.as_str() {
                        "lt" => self.rels.push((a, b, true)),
                        "lte" => self.rels.push((a, b, false)),
                        "gt" => self.rels.push((b, a, true)),
                        "gte" => self.rels.push((b, a, false)),
                        _ => {}
                    }
                }
                for (side, other, op) in [(l, r, op.clone()), (r, l, flip(&op))] {
                    let (Expr::Local(v, ty) | Expr::Move(v, ty)) = side.as_ref() else { continue };
                    if !matches!(ty.payload(), Ty::Int | Ty::Big) {
                        continue;
                    }
                    let o = self.iv_of(other);
                    let have = self.local_iv(v);
                    if o.is_empty() || have.is_empty() {
                        continue;
                    }
                    // (and the sides it bounds with a value of its own)
                    let (held, below, above) = match op.as_str() {
                        "lt" => (Iv { lo: -INF, hi: o.hi - 1 }, false, true),
                        "lte" => (Iv { lo: -INF, hi: o.hi }, false, true),
                        "gt" => (Iv { lo: o.lo + 1, hi: INF }, true, false),
                        "gte" => (Iv { lo: o.lo, hi: INF }, true, false),
                        "eq" => (o, true, true),
                        // (not one value: an end of the range that is it is not)
                        "ne" if o.lo == o.hi && have.hi == o.lo && have.lo < have.hi => (Iv { lo: -INF, hi: o.lo - 1 }, false, true),
                        "ne" if o.lo == o.hi && have.lo == o.lo && have.lo < have.hi => (Iv { lo: o.lo + 1, hi: INF }, true, false),
                        _ => continue,
                    };
                    let narrowed = Iv { lo: have.lo.max(held.lo), hi: have.hi.min(held.hi) };
                    self.cur.insert(v.clone(), narrowed);
                    let e = self.bounds.entry(v.clone()).or_insert((false, false));
                    *e = (e.0 || below, e.1 || above);
                }
            }
            _ => {}
        }
    }

    fn block(&mut self, b: &Block) {
        for (k, s) in b.iter().enumerate() {
            if matches!(s, Stmt::While { .. } | Stmt::For { .. }) {
                self.pending_fresh = fresh_before(&b[..k]);
            }
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { var, expr, ty } => {
                self.types.insert((self.fun.clone(), var.clone()), ty.clone());
                if var.starts_with("_t") {
                    self.temps.insert((self.fun.clone(), var.clone()), expr.clone());
                }
                self.calls_in(expr);
                let place = self.local(var);
                if is_int(expr.ty_ref()) {
                    let r = match self.accs.get(var).copied() {
                        Some(extra) => {
                            let r = self.accumulated(var, expr, extra);
                            self.widen(&place, r);
                            r
                        }
                        None => self.write_as(&place, expr, true),
                    };
                    self.cur.insert(var.clone(), r);
                }
                self.flow(&place, expr);
                self.written.insert(var.clone());
                self.rels.retain(|(a, b, _)| a != var && b != var);
                // (a value whose range is not the local's own bounds it on
                // both sides: `pc = jump[pc]`)
                if is_int(expr.ty_ref()) && !ranges_from(expr, var) {
                    self.bounds.insert(var.clone(), (true, true));
                } else {
                    self.bounds.remove(var);
                }
            }
            Stmt::Set { var, expr } => {
                self.calls_in(expr);
                let place = self.local(var);
                if is_int(expr.ty_ref()) {
                    let r = match self.accs.get(var).copied() {
                        Some(extra) => {
                            let r = self.accumulated(var, expr, extra);
                            self.widen(&place, r);
                            r
                        }
                        None => self.write_as(&place, expr, true),
                    };
                    self.cur.insert(var.clone(), r);
                }
                self.flow(&place, expr);
                self.written.insert(var.clone());
                self.rels.retain(|(a, b, _)| a != var && b != var);
                // (a value whose range is not the local's own bounds it on
                // both sides: `pc = jump[pc]`)
                if is_int(expr.ty_ref()) && !ranges_from(expr, var) {
                    self.bounds.insert(var.clone(), (true, true));
                } else {
                    self.bounds.remove(var);
                }
            }
            Stmt::SetPath { var, path, expr } => {
                self.calls_in(expr);
                for p in path {
                    if let PathStep::Index(i) = p {
                        self.calls_in(i);
                    }
                }
                let target = self.path_place(var, path);
                if is_int(expr.ty_ref()) {
                    self.write(&target, expr);
                }
                self.flow(&target, expr);
            }
            Stmt::Store { fact, expr } => {
                self.calls_in(expr);
                let place = Place::Fact(fact.clone());
                if is_int(expr.ty_ref()) {
                    self.write(&place, expr);
                }
                self.flow(&place, expr);
            }
            Stmt::Return(Some(e)) => {
                self.calls_in(e);
                let place = Place::Ret(self.fun.clone());
                if is_int(e.ty_ref()) {
                    // (a local the function adds its own answers into is a
                    // sum over its calls, as below: never a base answer)
                    if let Some(v) = answered_local(e).filter(|v| self.accs.contains_key(v)) {
                        let r = self.get(&self.local(&v));
                        self.widen(&place, r);
                        self.mark_soft(&place);
                    } else {
                        match self.recursive_sum(e) {
                            Some(r) => self.widen(&place, r),
                            None => {
                                let r = self.iv_of(e);
                                self.widen_base(&place, r);
                            }
                        }
                    }
                }
                self.flow(&place, e);
            }
            Stmt::Do(e) => self.calls_in(e),
            Stmt::If { cond, then, els } => {
                self.calls_in(cond);
                let before = self.cur.clone();
                let written_before = self.written.clone();
                let rels_before = self.rels.clone();
                let bounds_before = self.bounds.clone();
                self.refine(cond, true);
                self.block(then);
                let then_cur = std::mem::replace(&mut self.cur, before.clone());
                let then_written = std::mem::replace(&mut self.written, written_before);
                let then_rels = std::mem::replace(&mut self.rels, rels_before.clone());
                let then_bounds = std::mem::replace(&mut self.bounds, bounds_before.clone());
                self.refine(cond, false);
                if let Some(e) = els {
                    self.block(e);
                }
                let (then_leaves, else_leaves) = (leaves(then), els.as_ref().is_some_and(|e| leaves(e)));
                // a branch that leaves flows nowhere after: what follows is the
                // other's, its narrowing and all
                if then_leaves && !else_leaves {
                    return;
                }
                if else_leaves && !then_leaves {
                    self.cur = then_cur;
                    self.written = then_written;
                    self.rels = then_rels;
                    self.bounds = then_bounds;
                    return;
                }
                let assigned_branches: std::collections::HashSet<String> = assigned_in(then).into_iter().chain(els.as_ref().map(assigned_in).unwrap_or_default()).collect();
                self.rels = rels_before.into_iter().filter(|(a, b, _)| !assigned_branches.contains(a) && !assigned_branches.contains(b)).collect();
                // (a side either way bounds stays bounded: a branch's own
                // value, or what the conditions bound where it wrote none)
                let else_bounds = std::mem::take(&mut self.bounds);
                let mut bounds: HashMap<String, (bool, bool)> = bounds_before.into_iter().filter(|(v, _)| !assigned_branches.contains(v)).collect();
                for v in &assigned_branches {
                    let t = then_bounds.get(v).copied().unwrap_or((false, false));
                    let e = else_bounds.get(v).copied().unwrap_or((false, false));
                    let both = (t.0 && e.0, t.1 && e.1);
                    if both.0 || both.1 {
                        bounds.insert(v.clone(), both);
                    }
                }
                self.bounds = bounds;
                let else_cur = std::mem::take(&mut self.cur);
                // after the branches: what either left, a local one did not
                // write holding what it held before
                // (a local only one branch wrote, unknown before, is unknown
                // after; a branch's narrowing of one it did not write ends
                // with it)
                let mut keys: Vec<String> = then_cur.keys().chain(else_cur.keys()).filter(|k| before.contains_key(*k) || (then_written.contains(*k) && self.written.contains(*k))).cloned().collect();
                keys.sort();
                keys.dedup();
                for k in keys {
                    let at = |m: &HashMap<String, Iv>, me: &Self| m.get(&k).copied().or_else(|| before.get(&k).copied()).unwrap_or_else(|| me.get(&me.local(&k)));
                    let joined = at(&then_cur, self).join(at(&else_cur, self));
                    self.cur.insert(k, joined);
                }
                // (written after the branches: what both wrote)
                self.written = self.written.intersection(&then_written).cloned().collect();
            }
            Stmt::While { cond, body } => {
                self.calls_in(cond);
                let cmp = compared(cond);
                let cmp = &cmp;
                // the rounds of one entry: a counter stepping toward its bound
                // by a constant moves at most from its least to the bound
                // (from the counter's value where the loop starts: its place's
                // range holds what the loop itself makes of it)
                let entry = cmp.as_ref().and_then(|(l, op, r)| {
                    let (counter, bound, up) = counter_of(l, op, r)?;
                    if !steps_only(body, &counter, up) {
                        return None;
                    }
                    let c = *self.cur.get(&counter)?;
                    let b = self.iv_of(&bound);
                    Some(c.join(b).sub(c.join(b)).count_bits() + 1)
                });
                let fresh = std::mem::take(&mut self.pending_fresh);
                // what the body assigns comes round again: its place's range;
                // the body runs where the condition holds
                let assigned = assigned_in(body);
                for v in &assigned {
                    self.cur.remove(v);
                    self.written.remove(v);
                }
                self.rels.retain(|(a, b, _)| !assigned.contains(a) && !assigned.contains(b));
                self.bounds.retain(|v, _| !assigned.contains(v));
                let (outside, written_outside, rels_outside) = (self.cur.clone(), self.written.clone(), self.rels.clone());
                let bounds_outside = self.bounds.clone();
                self.loops.push(LoopCtx { rounds: None, entry, fresh });
                self.refine(cond, true);
                self.block(body);
                self.loops.pop();
                self.cur = outside;
                self.written = written_outside;
                self.rels = rels_outside;
                self.bounds = bounds_outside;
            }
            Stmt::For { var, iter, body, .. } => {
                self.calls_in(iter);
                // the variable: an item of what the loop walks
                let item = crate::type_infer::TypeContext::loop_item(iter.ty_ref().cloned().unwrap_or(Ty::Unknown));
                self.types.insert((self.fun.clone(), var.clone()), item);
                let place = self.local(var);
                let (r, rounds) = match iter {
                    Expr::Call { target: Target::Row(row), args, .. } if row.props.contains(&crate::table::Prop::Range) => {
                        let r = self.range_items(args, &place);
                        (r, r.sub(r).count_bits() + 1)
                    }
                    _ => {
                        let r = match self.place_of(iter) {
                            Some(p) => {
                                self.made_items(&p, iter);
                                self.get(&Place::Items(Box::new(p)))
                            }
                            None => Iv::I64,
                        };
                        (r, 48)
                    }
                };
                if matches!(iter.ty_ref().map(|t| t.payload()), Some(Ty::List(t)) if matches!(t.as_ref(), Ty::Int | Ty::Big)) || matches!(iter, Expr::Call { target: Target::Row(_), .. }) {
                    self.widen_base(&place, r);
                }
                if let Some(p) = self.place_of(iter)
                    && let Some(Ty::List(inner)) = iter.ty_ref().map(|t| t.payload().clone())
                    && holds_ints(Some(&inner))
                {
                    self.flow_items(&place, &Place::Items(Box::new(p)), *inner);
                }
                // a sum in a loop no other loop holds is bounded by its rounds
                let outermost = self.loops.is_empty();
                let fresh = std::mem::take(&mut self.pending_fresh);
                let mut assigned = assigned_in(body);
                assigned.insert(var.clone());
                for v in &assigned {
                    self.cur.remove(v);
                    self.written.remove(v);
                }
                self.rels.retain(|(a, b, _)| !assigned.contains(a) && !assigned.contains(b));
                self.bounds.retain(|v, _| !assigned.contains(v));
                let (outside, written_outside, rels_outside) = (self.cur.clone(), self.written.clone(), self.rels.clone());
                let bounds_outside = self.bounds.clone();
                if matches!(iter.ty_ref().map(|t| t.payload()), Some(Ty::List(t)) if matches!(t.as_ref(), Ty::Int | Ty::Big)) || matches!(iter, Expr::Call { target: Target::Row(_), .. }) {
                    self.cur.insert(var.clone(), r);
                }
                self.loops.push(LoopCtx { rounds: if outermost { Some(rounds) } else { None }, entry: Some(rounds), fresh });
                self.block(body);
                self.loops.pop();
                self.cur = outside;
                self.written = written_outside;
                self.rels = rels_outside;
                self.bounds = bounds_outside;
            }
            Stmt::Propagate { .. } | Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Line(_) => {}
        }
    }

    /// A write of `e` into `dest`, and the range it writes: a local this
    /// straight line set already (or a condition bounds) is its value's
    /// range — `v = carry`, then `v = v + a[i]` — a sum that comes round a
    /// loop moves by its rounds, any other write is its value's range.
    fn write(&mut self, dest: &Place, e: &Expr) -> Iv {
        self.write_as(dest, e, false)
    }

    /// `write`, `straight` where the statement assigns a local of the
    /// function walked (a local the walk knows — written before in the
    /// line, or bounded by its loop's condition — is its value's range).
    fn write_as(&mut self, dest: &Place, e: &Expr, straight: bool) -> Iv {
        if self.reads_soft(e) {
            self.mark_soft(dest);
        }
        // (a local this round's straight line has written: its write is no
        // sum coming round the loop. Nor is one a condition bounds on the
        // side the write moves it toward — `i = i + 1` under `i lt n`. One
        // a condition only narrowed otherwise is: its narrowed range holds
        // the loop's own drift, and taking the write as straight had joined
        // that drift into where the sums start each round, until the range
        // was taken as unbounded — n-queens' `k`, `if k eq 0 … else k = k -
        // 1`, held as a growing int)
        let local = match dest {
            Place::Local(f, v) if *f == self.fun && self.cur.contains_key(v) => Some(v.clone()),
            _ => None,
        };
        let written = local.as_ref().is_some_and(|v| self.written.contains(v));
        let narrowed = !written && local.as_ref().is_some_and(|v| self.bound_toward(v, e));
        let known = straight && (written || narrowed);
        if !known && let Some(r) = self.summed(dest, e) {
            self.widen(dest, r);
            return r;
        }
        let r = self.iv_of(e);
        // (a write a condition bounds moves away from its other side: that
        // side of its value is the local's own range, which holds the
        // loop's drift — where the sums start takes only the bounded sides,
        // or each round had started them from the last one's end)
        if straight && narrowed && let Some(v) = &local {
            let (below, above) = self.bounds.get(v).copied().unwrap_or((false, false));
            let b = self.base_of(dest);
            let start = if b.is_empty() { r } else { Iv { lo: if below { r.lo } else { r.lo.max(b.lo) }, hi: if above { r.hi } else { r.hi.min(b.hi) } } };
            if !start.is_empty() {
                self.widen_base(dest, start);
            }
            self.widen(dest, r);
            return r;
        }
        self.widen_base(dest, r);
        r
    }

    /// Whether a write of `e` to the local `v` moves it only toward the
    /// sides of its range the conditions it stands under bound with a
    /// value of their own (`bounds`).
    fn bound_toward(&mut self, v: &str, e: &Expr) -> bool {
        let Some(c) = self.cur.get(v).copied() else { return false };
        let (below, above) = self.bounds.get(v).copied().unwrap_or((false, false));
        let r = self.iv_of(e);
        (r.hi <= c.hi || above) && (r.lo >= c.lo || below)
    }

    /// The place an expression reads its int from, where it reads one a
    /// place holds (a local, a fact, a field, an item, `get[m, k, d]`):
    /// nothing is made for an expression of another shape.
    fn read_place(&self, x: &Expr) -> Option<Place> {
        match x {
            Expr::Local(v, _) | Expr::Move(v, _) => Some(self.local(v)),
            Expr::Load(f, _) => Some(Place::Fact(f.clone())),
            Expr::Convert { expr, .. } => self.read_place(expr),
            // (a record made where it is read — `Box[…].v` — lends no type:
            // its own is read, or its field had read as any 64-bit int)
            Expr::Field { base, name, .. } => match Some(base.ty()).as_ref().map(|t| t.payload()) {
                Some(Ty::Record(s)) => Some(Place::Field(s.clone(), name.clone())),
                _ => None,
            },
            Expr::Index { base, .. } => self.read_place(base).map(|p| Place::Items(Box::new(p))),
            _ => None,
        }
    }

    /// Whether an expression reads an int from a place joined to `dest`.
    fn reads_place(&self, x: &Expr, dest: &Place) -> bool {
        let d = self.root(dest);
        let mut any = false;
        crate::ir::visit_expr(x, &mut |y| {
            if let Some(p) = self.read_place(y) {
                any |= self.root(&p) == d;
            }
        });
        any
    }

    /// The bits of the rounds a sum into `dest` is taken: one entry's of the
    /// loop around it where the statements before that loop set `dest`
    /// afresh, the outermost `for`'s over a known count, else ROUNDS.
    fn sum_rounds(&self, dest: &Place) -> u32 {
        let Some(ctx) = self.loops.last() else { return ROUNDS };
        if let Place::Local(f, v) = dest
            && *f == self.fun
            && ctx.fresh.contains(v)
            && let Some(r) = ctx.entry
        {
            return r;
        }
        ctx.rounds.unwrap_or(ROUNDS)
    }

    /// A place's range rests on ROUNDS from here on.
    fn mark_soft(&mut self, p: &Place) {
        if self.frozen {
            return;
        }
        let r = self.root(p);
        if self.soft.insert(r) {
            self.changed = true;
        }
    }

    /// Whether an expression reads a place whose range rests on ROUNDS.
    pub(crate) fn reads_soft(&self, e: &Expr) -> bool {
        let mut any = false;
        crate::ir::visit_expr(e, &mut |y| {
            let p = match y {
                Expr::Call { target: Target::User(g), .. } => Some(Place::Ret(g.clone())),
                Expr::Derived(d, _) => Some(Place::Ret(derive_fn(d))),
                other => self.read_place(other),
            };
            if let Some(p) = p {
                any |= self.soft.contains(&self.root(&p));
            }
        });
        any
    }

    /// A sum into `dest`: a write that adds to what `dest` holds — `t = t +
    /// e`, `xs[i] = xs[j] - e`, `set[m, k, m[k] + 1]` (through the temporary
    /// the lowering made), a fact a handler adds to, `make[depth - 1]` of a
    /// function's own parameter — moves by e each time it runs, as many
    /// times as `sum_rounds` says at most: its range is where the writes
    /// that are no sum start it, moved by every sum's drift this round.
    /// None for a write of any other shape.
    fn summed(&mut self, dest: &Place, e: &Expr) -> Option<Iv> {
        // (the value of a sum that can fail, unwrapped where it is held:
        // `others = unwrap _t2`)
        if let Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } = e {
            return self.summed(dest, expr);
        }
        // a temporary the lowering made for the value is the value
        if let Expr::Local(v, _) | Expr::Move(v, _) = e
            && let Some(def) = self.temps.get(&(self.fun.clone(), v.clone())).cloned()
        {
            return self.summed(dest, &def);
        }
        let Expr::Binary { op, .. } = e else { return None };
        if !matches!(op.as_str(), "add" | "sub") {
            return None;
        }
        fn terms<'e>(e: &'e Expr, neg: bool, out: &mut Vec<(&'e Expr, bool)>) {
            match e {
                Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "add" | "sub") => {
                    terms(l, neg, out);
                    terms(r, if op == "sub" { !neg } else { neg }, out);
                }
                other => out.push((other, neg)),
            }
        }
        let mut parts = Vec::new();
        terms(e, false, &mut parts);
        let d = self.root(dest);
        // `t * c` of a constant c: a sum scaled each round (`b = b * 2 + bit`)
        let scaled = |x: &Expr, me: &Self| -> Option<i128> {
            let Expr::Binary { op, l, r, .. } = x else { return None };
            if op != "mul" {
                return None;
            }
            let own = |y: &Expr| me.read_place(y).is_some_and(|p| me.root(&p) == d);
            match (l.as_ref(), r.as_ref()) {
                (y, Expr::Const(Const::Int(c), _)) | (Expr::Const(Const::Int(c), _), y) if own(y) => Some(*c as i128),
                _ => None,
            }
        };
        let factors: Vec<Option<i128>> = parts.iter().map(|(x, neg)| if *neg { None } else { scaled(x, self) }).collect();
        if factors.iter().filter(|f| f.is_some()).count() == 1 {
            let c = factors.iter().flatten().next().copied().unwrap_or(1).abs();
            let others: Vec<(Expr, bool)> = parts.iter().zip(&factors).filter(|(_, f)| f.is_none()).map(|((x, neg), _)| ((*x).clone(), *neg)).collect();
            if others.iter().any(|(x, _)| self.reads_place(x, dest)) {
                return None;
            }
            let mut step = Iv::point(0);
            for (x, neg) in &others {
                let r = self.iv_of(x);
                step = if *neg { step.sub(r) } else { step.add(r) };
            }
            // after N rounds: c^N times what it started at and N steps
            let bits = self.sum_rounds(dest);
            if bits == ROUNDS {
                self.mark_soft(dest);
            }
            let n: i128 = if bits >= 100 { INF } else { 1i128 << bits };
            let mut m: i128 = 1;
            let mut k = 0i128;
            while k < n && m < INF {
                m = m.checked_mul(c.max(1)).map(sat).unwrap_or(INF);
                k += 1;
                if c <= 1 {
                    break;
                }
            }
            let start = self.base_of(dest);
            if start.is_empty() {
                return Some(Iv::EMPTY);
            }
            let mag = m.checked_mul(start.mag().saturating_add(n.saturating_mul(step.mag()))).map(sat).unwrap_or(INF);
            return Some(Iv::new(-mag, mag));
        }
        let own: Vec<bool> = parts.iter().map(|(x, neg)| !neg && self.read_place(x).is_some_and(|p| self.root(&p) == d)).collect();
        if own.iter().filter(|o| **o).count() != 1 {
            return None;
        }
        let others: Vec<(Expr, bool)> = parts.iter().zip(&own).filter(|(_, o)| !**o).map(|((x, neg), _)| ((*x).clone(), *neg)).collect();
        if others.iter().any(|(x, _)| self.reads_place(x, dest)) {
            return None;
        }
        let mut step = Iv::point(0);
        for (x, neg) in &others {
            let r = self.iv_of(x);
            step = if *neg { step.sub(r) } else { step.add(r) };
        }
        let bits = self.sum_rounds(dest);
        if bits == ROUNDS && self.loops.last().is_none_or(|c| c.rounds != Some(ROUNDS)) {
            self.mark_soft(dest);
        }
        let moved = step.drift(bits);
        let total = self.drifts.get(&d).copied().unwrap_or(Iv::point(0)).add(moved);
        self.drifts.insert(d.clone(), total);
        let start = self.base_of(dest);
        // (a place no write has started yet starts where the sum's own
        // read finds it: nothing, which this round's other writes fill)
        Some(if start.is_empty() { Iv::EMPTY } else { start.add(total) })
    }

    /// The range of an answer that adds the function's own answers to other
    /// terms (`1 + count[t.left] + count[t.right]`): a sum over its calls,
    /// which are fewer than 2^ROUNDS, so within those many of its other terms
    /// and of its other answers. None for an answer of any other shape.
    fn recursive_sum(&mut self, e: &Expr) -> Option<Iv> {
        fn terms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
            match e {
                Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "add" | "sub") => {
                    terms(l, out);
                    terms(r, out);
                }
                Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => terms(expr, out),
                other => out.push(other),
            }
        }
        let mut parts = Vec::new();
        terms(e, &mut parts);
        let me = self.fun.clone();
        let calls_me = |x: &Expr| matches!(x, Expr::Call { target: Target::User(g), .. } if *g == me);
        if !parts.iter().any(|x| calls_me(x)) {
            return None;
        }
        let mut each = self.base_of(&Place::Ret(me.clone()));
        for x in parts {
            if calls_me(x) {
                continue;
            }
            // a term that reads the answer another way is no plain sum
            let mut reads_me = false;
            crate::ir::visit_expr(x, &mut |y| reads_me |= calls_me(y));
            if reads_me {
                return None;
            }
            let r = self.iv_of(x);
            each = each.join(r.neg()).join(r);
        }
        self.mark_soft(&Place::Ret(me));
        Some(each.join(Iv::point(0)).drift(ROUNDS))
    }

    /// The range of a local `accumulators` found: what any call adds to it —
    /// its starts and its other terms — and the function's other answers,
    /// each added fewer than 2^ROUNDS times at each place that adds (no
    /// part of a program runs 2^ROUNDS times), so within those many of the
    /// largest, as `recursive_sum` bounds an answer that adds the calls
    /// itself. Its sums never come round as a loop's drift does (a counter
    /// summed in a loop of a backtracking search had grown each pass until
    /// it was held as a growing int: n-queens, sudoku). The local and the
    /// answer rest on ROUNDS.
    fn accumulated(&mut self, var: &str, e: &Expr, extra: u32) -> Iv {
        let me = self.fun.clone();
        let mut terms = Vec::new();
        signed_terms(e, false, &mut terms);
        let mut part = Iv::EMPTY;
        for (x, _) in terms {
            let own = calls_fun(&me, resolved(x, &self.temps, &me));
            let itself = matches!(x, Expr::Local(v, _) | Expr::Move(v, _) if v == var);
            if !own && !itself {
                let r = self.iv_of(x);
                part = part.join(r).join(r.neg());
            }
        }
        let key = (me.clone(), var.to_string());
        if !self.frozen {
            let had = self.acc_parts.get(&key).copied().unwrap_or(Iv::EMPTY);
            let next = had.join(part);
            if next != had {
                self.acc_parts.insert(key.clone(), next);
                self.changed = true;
            }
        }
        let parts = self.acc_parts.get(&key).copied().unwrap_or(Iv::EMPTY);
        let base = self.base_of(&Place::Ret(me.clone()));
        let place = self.local(var);
        self.mark_soft(&place);
        self.mark_soft(&Place::Ret(me));
        parts.join(base).join(base.neg()).join(Iv::point(0)).drift(ROUNDS + extra)
    }

    /// The place an assignment through a path writes: an item of a list
    /// local, a record's field.
    fn path_place(&mut self, var: &str, path: &[PathStep]) -> Place {
        let mut place = self.local(var);
        let mut ty: Option<Ty> = self.types.get(&(self.fun.clone(), var.to_string())).map(|t| t.payload().clone());
        for p in path {
            let (next_place, next_ty) = match (p, &ty) {
                (PathStep::Field(f), Some(Ty::Record(s))) => {
                    let fty = self.structs.get(s).and_then(|fs| fs.iter().find(|(n, _)| n == f)).map(|(_, t)| t.payload().clone());
                    (Place::Field(s.clone(), f.clone()), fty)
                }
                (PathStep::Index(_), Some(Ty::List(i) | Ty::Map(_, i))) => (Place::Items(Box::new(place)), Some(i.payload().clone())),
                _ => (Place::Items(Box::new(place)), None),
            };
            place = next_place;
            ty = next_ty;
        }
        place
    }

    /// The calls inside an expression: each argument flows into its
    /// parameter; a record made writes its fields.
    fn calls_in(&mut self, e: &Expr) {
        let mut calls: Vec<(String, Vec<Expr>)> = Vec::new();
        crate::ir::visit_expr(e, &mut |x| {
            if let Expr::Call { target: Target::User(g), args, .. } = x {
                calls.push((g.clone(), args.clone()));
            }
        });
        let mut records: Vec<(String, Vec<Expr>)> = Vec::new();
        crate::ir::visit_expr(e, &mut |x| {
            if let Expr::RecordLit { name, fields } = x {
                records.push((name.clone(), fields.clone()));
            }
        });
        for (name, fields) in records {
            let Some(declared) = self.structs.get(&name).cloned() else { continue };
            for ((f, _), v) in declared.iter().zip(&fields) {
                let place = Place::Field(name.clone(), f.clone());
                if is_int(v.ty_ref()) {
                    self.write(&place, v);
                }
                self.flow(&place, v);
            }
        }
        // (and a form that calls a function by its name with the arguments
        // after it: each flows into the parameter it is handed to)
        let mut by_name: Vec<(String, Vec<Expr>)> = Vec::new();
        crate::ir::visit_expr(e, &mut |x| {
            if let Some((f, skipped, rest)) = called_by_name(x) {
                let mut handed: Vec<Expr> = vec![Expr::Const(Const::Unit, Ty::Unit); skipped];
                handed.extend(rest.iter().cloned());
                by_name.push((f.to_string(), handed));
            }
        });
        calls.extend(by_name);
        for (g, args) in calls {
            let Some(params) = self.fns.get(g.as_str()).cloned() else { continue };
            for (p, a) in params.iter().zip(&args) {
                let place = Place::Local(g.clone(), p.clone());
                if is_int(a.ty_ref()) {
                    // (a parameter is the callee's: never this line's write)
                    self.write(&place, a);
                }
                self.flow_into(&place, a);
            }
        }
        // (a collecting loop's body: its assignments and calls)
        if let Expr::ForExpr { var, iter, body, .. } = e {
            let place = self.local(var);
            let r = match iter.as_ref() {
                Expr::Call { target: Target::Row(row), args, .. } if row.props.contains(&crate::table::Prop::Range) => Some(self.range_items(args, &place)),
                _ => self.place_of(iter).map(|p| {
                    self.made_items(&p, iter);
                    self.get(&Place::Items(Box::new(p)))
                }),
            };
            if let Some(r) = r
                && is_int(Some(&crate::type_infer::TypeContext::loop_item(iter.ty_ref().cloned().unwrap_or(Ty::Unknown))))
            {
                self.widen_base(&place, r);
            }
            let mut assigned = assigned_in(body);
            assigned.insert(var.clone());
            for v in &assigned {
                self.cur.remove(v);
                self.written.remove(v);
            }
            self.rels.retain(|(a, b, _)| !assigned.contains(a) && !assigned.contains(b));
            self.bounds.retain(|v, _| !assigned.contains(v));
            let (outside, written_outside, rels_outside) = (self.cur.clone(), self.written.clone(), self.rels.clone());
            let bounds_outside = self.bounds.clone();
            if let Some(r) = r {
                self.cur.insert(var.clone(), r);
            }
            self.loops.push(LoopCtx { rounds: None, entry: None, fresh: Default::default() });
            self.block(body);
            self.loops.pop();
            self.cur = outside;
            self.written = written_outside;
            self.rels = rels_outside;
            self.bounds = bounds_outside;
        }
    }
}

/// The locals a block's statements set afresh — to a value that does not read
/// them — with nothing after that in the block writing them again: what the
/// block holds of each when it reaches the statement after them.
fn fresh_before(stmts: &[Stmt]) -> std::collections::HashSet<String> {
    let mut fresh = std::collections::HashSet::new();
    for s in stmts {
        match s {
            Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } => {
                let mut reads = false;
                crate::ir::visit_expr(expr, &mut |y| reads |= matches!(y, Expr::Local(n, _) | Expr::Move(n, _) if n == var));
                if reads {
                    fresh.remove(var);
                } else {
                    fresh.insert(var.clone());
                }
            }
            // a nested statement may write any of them
            other => {
                let mut written = Vec::new();
                crate::ir::visit_stmts(std::slice::from_ref(other), &mut |x| {
                    if let Stmt::Let { var, .. } | Stmt::Set { var, .. } | Stmt::SetPath { var, .. } = x {
                        written.push(var.clone());
                    }
                });
                for v in written {
                    fresh.remove(&v);
                }
            }
        }
    }
    fresh
}

/// Whether a `while` body moves its counter only by a constant toward its
/// bound (`i = i + 1` under `i lt n`), exactly once: the rounds of one entry
/// are then bounded, and the bound holds where it steps.
fn steps_only(body: &Block, counter: &str, up: bool) -> bool {
    fn scan(b: &Block, counter: &str, up: bool, steps: &mut u32, other: &mut bool) {
        for s in b {
            match s {
                Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } if var == counter => match expr {
                    Expr::Binary { op, l, r, .. }
                        if matches!(l.as_ref(), Expr::Local(n, _) | Expr::Move(n, _) if n == counter)
                            && matches!(r.as_ref(), Expr::Const(Const::Int(k), _) if *k > 0)
                            && ((up && op == "add") || (!up && op == "sub")) =>
                    {
                        *steps += 1
                    }
                    _ => *other = true,
                },
                Stmt::SetPath { var, .. } if var == counter => *other = true,
                Stmt::If { then, els, .. } => {
                    scan(then, counter, up, steps, other);
                    if let Some(e) = els {
                        scan(e, counter, up, steps, other);
                    }
                }
                // a step inside an inner loop may run many times a round
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    let mut written = false;
                    crate::ir::visit_stmts(body, &mut |x| {
                        if let Stmt::Let { var, .. } | Stmt::Set { var, .. } | Stmt::SetPath { var, .. } = x {
                            written |= var == counter;
                        }
                    });
                    if written {
                        *other = true;
                    }
                }
                _ => {}
            }
        }
    }
    let (mut steps, mut other) = (0, false);
    scan(body, counter, up, &mut steps, &mut other);
    steps == 1 && !other
}

/// The local an expression reads as it is, if it is one.
/// The terms of a sum, each with whether it is subtracted: `a + b - c`.
fn signed_terms<'e>(e: &'e Expr, neg: bool, out: &mut Vec<(&'e Expr, bool)>) {
    match e {
        Expr::Binary { op, l, r, .. } if op == "add" => {
            signed_terms(l, neg, out);
            signed_terms(r, neg, out);
        }
        Expr::Binary { op, l, r, .. } if op == "sub" => {
            signed_terms(l, neg, out);
            signed_terms(r, !neg, out);
        }
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => signed_terms(expr, neg, out),
        other => out.push((other, neg)),
    }
}

/// What a term is, looking through the temporary the lowering put a call
/// in (`_t3 = count[n - 1]`, then `total + _t3`).
fn resolved<'e>(x: &'e Expr, temps: &'e HashMap<(String, String), Expr>, fun: &str) -> &'e Expr {
    match x {
        Expr::Local(v, _) | Expr::Move(v, _) if v.starts_with("_t") => match temps.get(&(fun.to_string(), v.clone())) {
            Some(t) => t,
            None => x,
        },
        _ => x,
    }
}

/// Whether an expression is a call of the function.
fn calls_fun(fun: &str, x: &Expr) -> bool {
    let x = match x {
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => expr.as_ref(),
        _ => x,
    };
    matches!(x, Expr::Call { target: Target::User(g), .. } if g == fun)
}

/// The local a `return` answers, through the marks a fallible read takes.
fn answered_local(e: &Expr) -> Option<String> {
    match e {
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => answered_local(expr),
        _ => local_name(e),
    }
}

/// The locals a recursive function adds its own answers into and answers
/// (`total = total + count[n - 1]` in a loop, then `return total`): every
/// write starts it — terms none of which reads it — or adds terms to it,
/// the local read once and not subtracted; one write at least adds the
/// function's own call; the function returns it; and no other term calls
/// the function or reads the local inside. Each with the bits its count of
/// writes adds to ROUNDS (`Walk::accumulated`).
fn accumulators(fun: &str, b: &Block) -> HashMap<String, u32> {
    let mut temps: HashMap<(String, String), Expr> = HashMap::new();
    crate::ir::visit_stmts(b, &mut |s| {
        if let Stmt::Let { var, expr, .. } = s {
            if var.starts_with("_t") {
                temps.insert((fun.to_string(), var.clone()), expr.clone());
            }
        }
    });
    let calls_inside = |x: &Expr| {
        let mut any = false;
        crate::ir::visit_expr(x, &mut |y| any |= calls_fun(fun, y));
        any
    };
    let mut fits: HashMap<String, bool> = HashMap::new();
    let mut writes: HashMap<String, u32> = HashMap::new();
    let mut adds_own: std::collections::HashSet<String> = Default::default();
    let mut answered: std::collections::HashSet<String> = Default::default();
    crate::ir::visit_stmts(b, &mut |s| match s {
        Stmt::Let { var, expr, .. } | Stmt::Set { var, expr } => {
            let is_var = |x: &Expr| matches!(x, Expr::Local(v, _) | Expr::Move(v, _) if v == var);
            let reads_var = |x: &Expr| {
                let mut any = false;
                crate::ir::visit_expr(x, &mut |y| any |= is_var(y));
                any
            };
            let mut terms = Vec::new();
            signed_terms(expr, false, &mut terms);
            let selves: Vec<bool> = terms.iter().filter(|(x, _)| is_var(x)).map(|(_, neg)| *neg).collect();
            let mut own = false;
            let mut plain = true;
            for (x, _) in &terms {
                if is_var(x) {
                    continue;
                }
                let x = resolved(x, &temps, fun);
                if calls_fun(fun, x) {
                    own = true;
                } else if calls_inside(x) || reads_var(x) {
                    plain = false;
                }
            }
            let ok = is_int(expr.ty_ref()) && plain && match selves.as_slice() {
                [] => true,
                [neg] => !neg,
                _ => false,
            };
            *fits.entry(var.clone()).or_insert(true) &= ok;
            *writes.entry(var.clone()).or_insert(0) += 1;
            if ok && own && !selves.is_empty() {
                adds_own.insert(var.clone());
            }
        }
        Stmt::For { var, .. } => {
            fits.insert(var.clone(), false);
        }
        Stmt::Return(Some(e)) => {
            if let Some(v) = answered_local(e) {
                answered.insert(v);
            }
        }
        _ => {}
    });
    fits.into_iter()
        .filter(|(v, ok)| *ok && adds_own.contains(v) && answered.contains(v))
        .map(|(v, _)| {
            let n = writes.get(&v).copied().unwrap_or(1) + 1;
            (v, 32 - n.leading_zeros())
        })
        .collect()
}

fn local_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Local(n, _) | Expr::Move(n, _) => Some(n.clone()),
        Expr::Convert { how: crate::ir::Convert::ToBig | crate::ir::Convert::ToSmall(_), expr, .. } => local_name(expr),
        _ => None,
    }
}

/// Whether a block always leaves: it ends returning, breaking out or going
/// round again (or in an `if` whose branches each do).
fn leaves(b: &Block) -> bool {
    match b.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
        Some(Stmt::Return(_) | Stmt::Break | Stmt::Continue) => true,
        Some(Stmt::If { then, els: Some(e), .. }) => leaves(then) && leaves(e),
        _ => false,
    }
}

/// The bounds a growing range moves to: a ladder of counts and widths, and
/// every int the program writes, with its neighbours.
fn thresholds_of(program: &Program) -> Vec<i128> {
    let mut out: Vec<i128> = vec![0, 1, 1 << 8, 1 << 16, 1 << 31, 1 << 32, 1 << 48, 1 << 49, 1 << 53, 1 << 62, (1 << 63) - 1, 1 << 63, 1 << 64];
    let mut note = |x: &Expr| {
        if let Expr::Const(Const::Int(n), _) = x {
            let n = *n as i128;
            out.extend([n - 1, n, n + 1, -n - 1, -n, -n + 1]);
        }
    };
    for part in &program.parts {
        for f in &part.fns {
            crate::ir::visit_block(&f.body, &mut note);
        }
        crate::ir::visit_block(&part.main, &mut note);
    }
    let mut neg: Vec<i128> = out.iter().map(|x| -x).collect();
    out.append(&mut neg);
    out.sort();
    out.dedup();
    out
}

/// The locals a block assigns, anywhere inside it.
fn assigned_in(b: &Block) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    crate::ir::visit_stmts(b, &mut |s| match s {
        Stmt::Let { var, .. } | Stmt::Set { var, .. } | Stmt::For { var, .. } => {
            out.insert(var.clone());
        }
        _ => {}
    });
    out
}

/// Whether an expression's range is read from the local `v`'s own: an
/// item read at `v` is not (its range is the items'), a sum of `v` is.
fn ranges_from(e: &Expr, v: &str) -> bool {
    match e {
        Expr::Local(n, _) | Expr::Move(n, _) => n == v,
        Expr::Const(..) => false,
        Expr::Index { base, .. } => ranges_from(base, v),
        Expr::Field { base, .. } => ranges_from(base, v),
        Expr::Binary { l, r, .. } => ranges_from(l, v) || ranges_from(r, v),
        Expr::Unary { expr, .. } | Expr::Convert { expr, .. } => ranges_from(expr, v),
        other => {
            let mut any = false;
            crate::ir::visit_expr(other, &mut |x| any |= matches!(x, Expr::Local(n, _) | Expr::Move(n, _) if n == v));
            any
        }
    }
}

/// A comparison's operator where it fails.
fn negated(o: &str) -> String {
    match o {
        "lt" => "gte",
        "lte" => "gt",
        "gt" => "lte",
        "gte" => "lt",
        "eq" => "ne",
        "ne" => "eq",
        other => other,
    }
    .to_string()
}

/// A comparison's operator with its sides swapped.
fn flip(o: &str) -> String {
    match o {
        "lt" => "gt",
        "lte" => "gte",
        "gt" => "lt",
        "gte" => "lte",
        other => other,
    }
    .to_string()
}

/// A `while`'s condition as a comparison of two sides.
fn compared(cond: &Expr) -> Option<(Expr, String, Expr)> {
    let Expr::Binary { op, l, r, .. } = cond else { return None };
    matches!(op.as_str(), "lt" | "lte" | "gt" | "gte").then(|| ((**l).clone(), op.clone(), (**r).clone()))
}

/// The counter a comparison bounds: a local on one side, the other side its
/// bound, and whether it counts up toward it.
fn counter_of(l: &Expr, op: &str, r: &Expr) -> Option<(String, Expr, bool)> {
    let name = |x: &Expr| match x {
        Expr::Local(n, Ty::Int) | Expr::Move(n, Ty::Int) => Some(n.clone()),
        _ => None,
    };
    match op {
        "lt" | "lte" => name(l).map(|n| (n, r.clone(), true)).or_else(|| name(r).map(|n| (n, l.clone(), false))),
        "gt" | "gte" => name(l).map(|n| (n, r.clone(), false)).or_else(|| name(r).map(|n| (n, l.clone(), true))),
        _ => None,
    }
}

/// A form that calls a function of the program by its name with the
/// arguments after it — `stream[feed, a, …]` calls `feed[put, a, …]`,
/// `later[f, a, …]` calls `f[a, …]`: the function, how many of its first
/// parameters the form fills itself (a feed's `put`), and the arguments
/// handed to the rest.
fn called_by_name(x: &Expr) -> Option<(&str, usize, &[Expr])> {
    let (name, args) = match x {
        Expr::Call { target: Target::Runtime(n), args, .. } => (n.as_str(), args.as_slice()),
        Expr::Effect { name, args, .. } => (name.as_str(), args.as_slice()),
        _ => return None,
    };
    // (the row says: a function parameter whose signature takes the rest,
    // the parameters the row fills itself before them — a feed's put)
    let (i, sig) = crate::table::fn_params(name, args.len()).into_iter().find(|(_, s)| s.rest)?;
    match args.get(i) {
        Some(Expr::FnRef(f, _)) => Some((f.as_str(), sig.params.len(), &args[i + 1..])),
        _ => None,
    }
}

fn is_int(t: Option<&Ty>) -> bool {
    matches!(t.map(|t| t.payload()), Some(Ty::Int) | Some(Ty::Big))
}

/// A type that holds ints somewhere inside it (a list of them, a map of
/// lists of them).
fn holds_ints(t: Option<&Ty>) -> bool {
    match t.map(|t| t.payload()) {
        Some(Ty::List(i)) | Some(Ty::Map(_, i)) => is_int(Some(i)) || holds_ints(Some(i)),
        _ => false,
    }
}

fn same_shape(a: Option<&Ty>, b: Option<&Ty>) -> bool {
    matches!((a.map(|t| t.payload()), b.map(|t| t.payload())), (Some(Ty::List(_)), Some(Ty::List(_))) | (Some(Ty::Map(..)), Some(Ty::Map(..))))
}

/// Whether a type is a list (or a map) of ints, an item of which an int
/// argument may be.
fn item_of(t: Option<&Ty>) -> bool {
    matches!(t.map(|t| t.payload()), Some(Ty::List(i)) | Some(Ty::Map(_, i)) if is_int(Some(i)))
}

/// The least power of two above `m` (`m` ≥ 0): what an `or` of numbers up
/// to `m` stays below.
fn pow2_above(m: i128) -> i128 {
    let mut p: i128 = 1;
    while p <= m && p < INF {
        p = p.saturating_mul(2);
    }
    p
}

/// The span of an and/or/xor where a side may be negative: within the
/// powers of two above both sides' sizes, either sign (64 bits where both
/// fit them).
fn bits_span(a: Iv, b: Iv) -> Iv {
    let p = pow2_above(a.mag().max(b.mag()));
    Iv::new(-p, p - 1)
}

/// The range of an int a builtin answers, by the rule its row declares
/// (`IntRange`): a count or a position from -1 to 2^48, a whole number of
/// any size where it reads one from text or a float, the arguments' where
/// the row keeps them, any other a 64-bit int.
fn row_iv(row: &crate::table::Row, ivs: &[Iv]) -> Iv {
    use crate::table::IntRange as R;
    let first = ivs.first().copied().unwrap_or(Iv::I64);
    match row.range {
        R::Count => Iv::new(0, COUNT),
        R::Position => Iv::new(-1, COUNT),
        R::Abs => {
            if first.lo >= 0 { first } else { Iv::new(0, first.mag()) }
        }
        R::Min => match ivs {
            [a, b] => Iv::new(a.lo.min(b.lo), a.hi.min(b.hi)),
            _ => Iv::I64,
        },
        R::Max => match ivs {
            [a, b] => Iv::new(a.lo.max(b.lo), a.hi.max(b.hi)),
            _ => Iv::I64,
        },
        R::Keeps | R::Same => first,
        // an int's bits: and/or/xor of 64-bit ints are 64-bit, and tighter
        // where a side is not negative; a shift moves a range by its count
        R::And => match ivs {
            [a, b] if a.lo >= 0 && b.lo >= 0 => Iv::new(0, a.hi.min(b.hi)),
            [a, _] if a.lo >= 0 => Iv::new(0, a.hi),
            [_, b] if b.lo >= 0 => Iv::new(0, b.hi),
            [a, b] => bits_span(*a, *b),
            _ => Iv::I64,
        },
        R::OrXor => match ivs {
            [a, b] if a.lo >= 0 && b.lo >= 0 => Iv::new(0, pow2_above(a.hi.max(b.hi)) - 1),
            [a, b] => bits_span(*a, *b),
            _ => Iv::I64,
        },
        R::Code => Iv::new(0, 0x10FFFF),
        R::Shift => match ivs {
            [a, n] if n.hi < 100 => {
                // a left shift multiplies by 2^n for n between its ends; a
                // right one moves toward 0 (or -1), rounding down
                let mut r = Iv::EMPTY;
                if n.hi > 0 {
                    let (k0, k1) = (1i128 << n.lo.max(0), 1i128 << n.hi);
                    let ends = [a.lo.saturating_mul(k0), a.lo.saturating_mul(k1), a.hi.saturating_mul(k0), a.hi.saturating_mul(k1)];
                    r = r.join(Iv::new(ends.iter().copied().min().unwrap_or(0), ends.iter().copied().max().unwrap_or(0)));
                }
                if n.lo <= 0 {
                    r = r.join(Iv::new(a.lo.min(if a.lo < 0 { -1 } else { 0 }), a.hi.max(0)));
                }
                r
            }
            _ => Iv::TOP,
        },
        R::Bool => Iv::BOOL,
        R::Any => Iv::TOP,
        R::I64 => Iv::I64,
    }
}

// ── the rewrite: wide places held as `Ty::Big` ──────────────────────

/// The program with every wide place's ints held as `Ty::Big`: each local,
/// parameter, answer, field and fact the pass found wide is declared big,
/// and so are the items of a list held there; an expression reading one is
/// big, arithmetic with a big operand is big (its other operand converted,
/// `Convert::ToBig`), and a big value handed where a 64-bit int is taken — a
/// builtin's count or position, an index — is converted back
/// (`Convert::ToSmall`, refused past 64 bits, in both modes).
pub fn widen(program: &mut Program) {
    let (w, mut walk) = settle(program);
    let _ = grows_anywhere;
    // the declared types first: what each read and each call will meet
    let mut rets: HashMap<String, Ty> = HashMap::new();
    let mut params: HashMap<String, Vec<Ty>> = HashMap::new();
    let mut fields: HashMap<(String, String), Ty> = HashMap::new();
    let mut facts: HashMap<String, Ty> = HashMap::new();
    let mut records: HashMap<String, Vec<Ty>> = HashMap::new();
    #[allow(clippy::type_complexity)]
    let mut grown_handed: Vec<(String, Vec<(String, Ty)>, Ty, Vec<(String, Ty)>, Ty, crate::ir::FnKind, Vec<String>, Vec<String>, usize)> = Vec::new();
    let row_bodies: std::collections::HashSet<String> = crate::table::ROWS.iter().filter(|r| r.is_crow()).map(crate::table::crow_fn_name).collect();
    // (and a function used as a value: the type of the parameter that takes
    // it names its signature, which stays 64-bit — its body converts at its
    // edges, as a builtin's does)
    // (a function a form calls by its name — `stream[feed, a, …]`,
    // `later[f, a, …]` — is no value: its parameters take what it is handed)
    let mut handed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut refs: HashMap<String, usize> = HashMap::new();
    let mut named: HashMap<String, usize> = HashMap::new();
    for part in program.parts.iter() {
        let mut note = |x: &Expr| {
            if let Expr::FnRef(name, _) = x {
                *refs.entry(name.clone()).or_default() += 1;
            }
            if let Some((f, _, _)) = called_by_name(x) {
                *named.entry(f.to_string()).or_default() += 1;
            }
        };
        for f in &part.fns {
            crate::ir::visit_block(&f.body, &mut note);
        }
        crate::ir::visit_block(&part.main, &mut note);
        for d in &part.derives {
            crate::ir::visit_block(&d.body, &mut note);
        }
        for test in &part.tests {
            crate::ir::visit_block(&test.body, &mut note);
        }
        for f in &part.facts {
            crate::ir::visit_expr(&f.default, &mut note);
        }
    }
    handed.extend(refs.into_iter().filter(|(f, n)| *n > named.get(f).copied().unwrap_or(0)).map(|(f, _)| f));
    for part in program.parts.iter_mut() {
        for s in part.structs.iter_mut() {
            for (f, ty) in s.fields.iter_mut() {
                *ty = held(ty, &Place::Field(s.name.clone(), f.clone()), &w);
                fields.insert((s.name.clone(), f.clone()), ty.clone());
            }
            records.insert(s.name.clone(), s.fields.iter().map(|(_, t)| t.clone()).collect());
        }
        for f in part.facts.iter_mut() {
            f.ty = held(&f.ty, &Place::Fact(f.name.clone()), &w);
            facts.insert(f.name.clone(), f.ty.clone());
        }
        for f in part.fns.iter_mut() {
            // a builtin written in Crow keeps its row's contract: its body
            // converts at its edges (an answer past 64 bits is refused where
            // it returns, in both modes)
            if row_bodies.contains(&f.name) {
                rets.insert(f.name.clone(), f.ret.clone());
                params.insert(f.name.clone(), f.params.iter().map(|(_, t)| t.clone()).collect());
                continue;
            }
            let before = (f.params.clone(), f.ret.clone());
            for (p, ty) in f.params.iter_mut() {
                *ty = held(ty, &Place::Local(f.name.clone(), p.clone()), &w);
            }
            f.ret = held(&f.ret, &Place::Ret(f.name.clone()), &w);
            rets.insert(f.name.clone(), f.ret.clone());
            params.insert(f.name.clone(), f.params.iter().map(|(_, t)| t.clone()).collect());
            if handed.contains(&f.name) && (f.params.iter().any(|(_, t)| has_big(t)) || has_big(&f.ret)) {
                grown_handed.push((f.name.clone(), before.0, before.1, f.params.clone(), f.ret.clone(), f.kind.clone(), f.effects.clone(), f.tables_written.clone(), crate::ir::first_line(&f.body)));
            }
        }
    }
    // a function handed as a value whose signature grew: the value is an
    // adapter of the 64-bit signature the function type that takes it names
    // (`f__handed`), which calls the function and answers its value in 64
    // bits — refused past them, alike in both modes — while a call of the
    // function by its name keeps every digit
    let mut adapters: HashMap<String, String> = HashMap::new();
    for (name, old_params, old_ret, new_params, new_ret, kind, effects, tables_written, line) in grown_handed {
        let wrapper = format!("{}__handed", name);
        let args: Vec<Expr> = old_params.iter().zip(&new_params).map(|((p, old), (_, new))| to_type(Expr::Local(p.clone(), old.clone()), new)).collect();
        let call = Expr::Call { target: Target::User(name.clone()), args, ty: new_ret.clone(), propagates: false };
        let mut answer = to_type(call, &old_ret);
        name_refusal(&mut answer, crate::ir::Who::Handed);
        let f = crate::ir::Fn { name: wrapper.clone(), params: old_params.clone(), ret: old_ret.clone(), body: vec![Stmt::Line(line), Stmt::Return(Some(answer))], kind, effects, tables_written, recursive: false };
        rets.insert(wrapper.clone(), old_ret);
        params.insert(wrapper.clone(), old_params.iter().map(|(_, t)| t.clone()).collect());
        if let Some(part) = program.parts.iter_mut().find(|p| p.fns.iter().any(|g| g.name == name)) {
            part.fns.push(f);
        }
        adapters.insert(name, wrapper);
    }
    if !adapters.is_empty() {
        let mut swap = |x: &mut Expr| {
            if let Expr::FnRef(name, _) = x
                && let Some(w) = adapters.get(name.as_str())
            {
                *name = w.clone();
            }
        };
        for part in program.parts.iter_mut() {
            for f in part.fns.iter_mut() {
                crate::ir::visit_block_mut(&mut f.body, &mut swap);
            }
            crate::ir::visit_block_mut(&mut part.main, &mut swap);
            for d in part.derives.iter_mut() {
                crate::ir::visit_block_mut(&mut d.body, &mut swap);
            }
            for test in part.tests.iter_mut() {
                crate::ir::visit_block_mut(&mut test.body, &mut swap);
            }
        }
    }
    let mut derives: HashMap<String, Ty> = HashMap::new();
    for part in program.parts.iter_mut() {
        for d in part.derives.iter_mut() {
            d.ty = held(&d.ty, &Place::Ret(derive_fn(&d.name)), &w);
            derives.insert(d.name.clone(), d.ty.clone());
        }
    }
    let sigs = Sigs { rets, params, fields, facts, records, derives };
    for part in program.parts.iter_mut() {
        for f in part.facts.iter_mut() {
            walk.fun = String::new();
            let mut r = Retype { w: &w, walk: &mut walk, sigs: &sigs, fun: String::new(), vars: vec![HashMap::new()] };
            let ty = f.ty.clone();
            r.expr(&mut f.default);
            f.default = to_type(std::mem::replace(&mut f.default, Expr::Const(Const::Unit, Ty::Unit)), &ty);
        }
        for f in part.fns.iter_mut() {
            walk.fun = f.name.clone();
            let mut r = Retype { w: &w, walk: &mut walk, sigs: &sigs, fun: f.name.clone(), vars: vec![f.params.iter().cloned().collect()] };
            let ret = f.ret.clone();
            r.block(&mut f.body, &ret);
        }
        walk.fun = String::new();
        for d in part.derives.iter_mut() {
            walk.fun = derive_fn(&d.name);
            let mut r = Retype { w: &w, walk: &mut walk, sigs: &sigs, fun: derive_fn(&d.name), vars: vec![HashMap::new()] };
            let ty = d.ty.clone();
            r.block(&mut d.body, &ty);
        }
        for (i, test) in part.tests.iter_mut().enumerate() {
            walk.fun = test_fn(i);
            let mut r = Retype { w: &w, walk: &mut walk, sigs: &sigs, fun: test_fn(i), vars: vec![test.params.iter().cloned().collect()] };
            r.block(&mut test.body, &Ty::Unit);
        }
        walk.fun = String::new();
        let mut r = Retype { w: &w, walk: &mut walk, sigs: &sigs, fun: String::new(), vars: vec![HashMap::new()] };
        r.block(&mut part.main, &Ty::Unit);
    }
}

/// A declared type, its ints big where the place holding them is wide (a
/// list's items at the place's items, and so on inward).
fn held(ty: &Ty, at: &Place, w: &Widths) -> Ty {
    match ty {
        Ty::Int if w.grows(at) => Ty::Big,
        Ty::List(i) => Ty::List(Box::new(held(i, &Place::Items(Box::new(at.clone())), w))),
        Ty::Map(k, i) => Ty::Map(*k, Box::new(held(i, &Place::Items(Box::new(at.clone())), w))),
        Ty::Result(i) => Ty::Result(Box::new(held(i, at, w))),
        other => other.clone(),
    }
}

struct Sigs {
    rets: HashMap<String, Ty>,
    params: HashMap<String, Vec<Ty>>,
    fields: HashMap<(String, String), Ty>,
    facts: HashMap<String, Ty>,
    /// Each record's field types, in the declaration's order.
    records: HashMap<String, Vec<Ty>>,
    derives: HashMap<String, Ty>,
}

struct Retype<'a> {
    w: &'a Widths,
    /// The settled walk: an expression's width.
    walk: &'a mut Walk,
    sigs: &'a Sigs,
    fun: String,
    /// Each block's locals and their types, innermost last.
    vars: Vec<HashMap<String, Ty>>,
}

/// Whether a type holds a big int anywhere in it.
fn has_big(t: &Ty) -> bool {
    match t {
        Ty::Big => true,
        Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => has_big(i),
        _ => false,
    }
}

/// An expression converted to a type it must have: an int where a big one
/// is held, a big one where an int is (refused past 64 bits); a list or a
/// map whose items differ so, converted item by item.
fn to_type(e: Expr, want: &Ty) -> Expr {
    // a payload's read stays outermost: the backends find a call's argument
    // that can fail by it
    if let Expr::Convert { how: how @ (crate::ir::Convert::Unwrap | crate::ir::Convert::Known), expr, .. } = e {
        let inner = to_type(*expr, want);
        let ty = inner.ty().payload().clone();
        return Expr::Convert { how, expr: Box::new(inner), ty };
    }
    // an empty literal is of the type it is held as
    match e {
        Expr::ListLit(items, Ty::List(_)) if items.is_empty() && matches!(want.payload(), Ty::List(_)) => return Expr::ListLit(items, want.payload().clone()),
        Expr::MapLit(pairs, Ty::Map(..)) if pairs.is_empty() && matches!(want.payload(), Ty::Map(..)) => return Expr::MapLit(pairs, want.payload().clone()),
        _ => {}
    }
    let have = e.ty();
    // (a value that can fail converts as its payload, its failure kept)
    let keep = |t: Ty| if have.is_result() && !t.is_result() { Ty::Result(Box::new(t)) } else { t };
    match (have.payload(), want.payload()) {
        (Ty::Int, Ty::Big) => Expr::Convert { how: crate::ir::Convert::ToBig, expr: Box::new(e), ty: keep(Ty::Big) },
        (Ty::Big, Ty::Int) => Expr::Convert { how: crate::ir::Convert::ToSmall(crate::ir::Who::Held), expr: Box::new(e), ty: keep(Ty::Int) },
        // (a tensor's number: the nearest float)
        (Ty::Big, Ty::Float) => Expr::Convert { how: crate::ir::Convert::IntToFloat, expr: Box::new(e), ty: keep(Ty::Float) },
        (Ty::List(a), Ty::List(b)) | (Ty::Map(_, a), Ty::Map(_, b)) if has_big(a) != has_big(b) => {
            let how = if has_big(b) { crate::ir::Convert::ToBig } else { crate::ir::Convert::ToSmall(crate::ir::Who::Held) };
            // (typed as what it makes: items that can fail only where the
            // items given could — typed as a field's list whose items can
            // fail, the record had taken the plain items as such, and the
            // compiled program did not build)
            let item = |b: &Ty| if a.is_result() { b.clone() } else { b.payload().clone() };
            let made = match want.payload() {
                Ty::List(b) => Ty::List(Box::new(item(b))),
                Ty::Map(k, b) => Ty::Map(*k, Box::new(item(b))),
                other => other.clone(),
            };
            Expr::Convert { how, expr: Box::new(e), ty: keep(made) }
        }
        _ => e,
    }
}

impl<'a> Retype<'a> {
    fn var_ty(&self, v: &str) -> Option<Ty> {
        self.vars.iter().rev().find_map(|s| s.get(v).cloned())
    }

    fn declare(&mut self, v: &str, ty: Ty) {
        if let Some(s) = self.vars.last_mut() {
            s.insert(v.to_string(), ty);
        }
    }

    fn block(&mut self, b: &mut Block, ret: &Ty) {
        self.vars.push(HashMap::new());
        for s in b.iter_mut() {
            self.stmt(s, ret);
        }
        self.vars.pop();
    }

    fn stmt(&mut self, s: &mut Stmt, ret: &Ty) {
        match s {
            Stmt::Let { var, ty, expr } => {
                self.expr(expr);
                *ty = held(ty, &Place::Local(self.fun.clone(), var.clone()), self.w);
                let want = ty.clone();
                *expr = to_type(take(expr), &want);
                self.declare(var, want);
            }
            Stmt::Set { var, expr } => {
                self.expr(expr);
                if let Some(want) = self.var_ty(var) {
                    *expr = to_type(take(expr), &want);
                }
            }
            Stmt::SetPath { var, path, expr } => {
                for p in path.iter_mut() {
                    if let PathStep::Index(i) = p {
                        self.expr(i);
                        *i = small_index(take(i));
                    }
                }
                self.expr(expr);
                let mut ty = self.var_ty(var).unwrap_or(Ty::Unknown);
                for p in path.iter() {
                    ty = match (&ty, p) {
                        (Ty::Record(s), PathStep::Field(f)) => self.sigs.fields.get(&(s.clone(), f.clone())).cloned().unwrap_or(Ty::Unknown),
                        (Ty::List(i), PathStep::Index(_)) | (Ty::Map(_, i), PathStep::Index(_)) => (**i).clone(),
                        (Ty::Tensor(..), PathStep::Index(_)) => Ty::Float,
                        _ => Ty::Unknown,
                    };
                }
                if !matches!(ty, Ty::Unknown) {
                    *expr = to_type(take(expr), &ty);
                }
            }
            Stmt::Store { fact, expr } => {
                self.expr(expr);
                if let Some(want) = self.sigs.facts.get(fact) {
                    *expr = to_type(take(expr), want);
                }
            }
            Stmt::Return(Some(e)) => {
                self.expr(e);
                *e = to_type(take(e), ret);
                // (an adapter of a function handed as a value answers 64
                // bits: said so)
                if self.fun.ends_with("__handed") {
                    name_refusal(e, crate::ir::Who::Handed);
                }
            }
            Stmt::Do(e) => self.expr(e),
            Stmt::If { cond, then, els } => {
                self.expr(cond);
                self.block(then, ret);
                if let Some(b) = els {
                    self.block(b, ret);
                }
            }
            Stmt::While { cond, body } => {
                self.expr(cond);
                self.block(body, ret);
            }
            Stmt::For { var, iter, body, .. } => {
                self.expr(iter);
                self.vars.push(HashMap::new());
                // the variable is an item of what the loop walks, whatever
                // else its name is bound to
                let item = crate::type_infer::TypeContext::loop_item(iter.ty());
                self.declare(var, item);
                for s in body.iter_mut() {
                    self.stmt(s, ret);
                }
                self.vars.pop();
            }
            _ => {}
        }
    }

    /// An expression's parts retyped, then its own type from them; and
    /// arithmetic of 64-bit ints whose range the walk proves within 64 bits,
    /// on no bound that rests on ROUNDS, computed without its check
    /// (Convert::Proven: the overflow branch kept LLVM from vectorizing a
    /// loop's arithmetic).
    fn expr(&mut self, e: &mut Expr) {
        self.expr_inner(e);
        if self.proven(e) {
            let inner = take(e);
            *e = Expr::Convert { how: crate::ir::Convert::Proven, expr: Box::new(inner), ty: Ty::Int };
        }
    }

    /// Whether `+`, `-`, `*` or a negation of 64-bit ints cannot pass them.
    fn proven(&mut self, e: &Expr) -> bool {
        let plain_int = |x: &Expr| matches!(x.ty(), Ty::Int);
        let ok = match e {
            Expr::Binary { op, l, r, ty } => matches!(op.as_str(), "add" | "sub" | "mul") && matches!(ty, Ty::Int) && plain_int(l) && plain_int(r),
            Expr::Unary { op, expr, ty } => op == "neg" && matches!(ty, Ty::Int) && plain_int(expr),
            _ => false,
        };
        if !ok {
            return false;
        }
        let r = self.walk.iv_of(e);
        // (strictly inside: the extremes are where a check would matter)
        !r.is_empty() && r.lo > i64::MIN as i128 && r.hi < i64::MAX as i128 && !self.walk.reads_soft(e)
    }

    fn expr_inner(&mut self, e: &mut Expr) {
        // arithmetic of 64-bit ints whose answer may not fit them grows (asked
        // of the expression as the walk saw it, before its parts are retyped)
        let wide = wide_arith(self.walk, e);
        match e {
            Expr::Local(v, ty) | Expr::Move(v, ty) => {
                if let Some(t) = self.var_ty(v) {
                    *ty = t;
                }
            }
            Expr::Load(f, ty) => {
                if let Some(t) = self.sigs.facts.get(f) {
                    *ty = t.clone();
                }
            }
            Expr::Derived(d, ty) => {
                if let Some(t) = self.sigs.derives.get(d) {
                    *ty = t.clone();
                }
            }
            Expr::Field { base, name, ty } => {
                self.expr(base);
                if let Ty::Record(s) = base.ty().payload() {
                    if let Some(t) = self.sigs.fields.get(&(s.clone(), name.clone())) {
                        *ty = t.clone();
                    }
                }
            }
            Expr::Index { base, indices, ty } => {
                self.expr(base);
                // (a function value applied: its arguments are what it takes,
                // ints of 64 bits as a function handed as a value takes them
                // — spec, Numbers That Keep Every Digit — not positions; the
                // refusal had named a position or a key)
                let applied = matches!(base.ty().payload(), Ty::Fn(..));
                for i in indices.iter_mut() {
                    self.expr(i);
                    // (a growing int alone, as a position's: a list of items
                    // nothing typed is no int to narrow)
                    *i = if applied && matches!(i.ty().payload(), Ty::Big) {
                        handed(take(i), crate::table::TakesBig::Refused, crate::ir::Who::HandedArg)
                    } else if applied {
                        take(i)
                    } else {
                        small_index(take(i))
                    };
                }
                match base.ty().payload() {
                    Ty::List(item) | Ty::Map(_, item) if has_big(item) || has_big(ty) => *ty = (**item).clone(),
                    _ => {}
                }
            }
            Expr::Binary { op, l, r, ty } => {
                self.expr(l);
                self.expr(r);
                let big = wide || matches!(l.ty().payload(), Ty::Big) || matches!(r.ty().payload(), Ty::Big);
                let ints = matches!(l.ty().payload(), Ty::Int | Ty::Big) && matches!(r.ty().payload(), Ty::Int | Ty::Big);
                if big && ints {
                    *l = Box::new(to_type(take(l), &Ty::Big));
                    *r = Box::new(to_type(take(r), &Ty::Big));
                    if matches!(op.as_str(), "add" | "sub" | "mul" | "div" | "mod" | "pow") {
                        *ty = big_of(ty);
                    }
                } else if big && op == "pow" && matches!(l.ty().payload(), Ty::Decimal | Ty::Fraction) && matches!(r.ty().payload(), Ty::Big) {
                    // an exact number's power is by an int of 64 bits: a
                    // growing one is refused past them, in one wording (it
                    // had been made a float, and the compiled program had not
                    // built — crow check --self)
                    *r = Box::new(handed(take(r), crate::table::TakesBig::Refused, crate::ir::Who::Held));
                } else if big {
                    // a whole number past 64 bits beside a float: its float
                    for side in [&mut *l, &mut *r] {
                        if matches!(side.ty().payload(), Ty::Big) {
                            **side = Expr::Convert { how: crate::ir::Convert::IntToFloat, expr: Box::new(take(side)), ty: Ty::Float };
                        }
                    }
                }
            }
            Expr::Unary { op, expr, ty } => {
                let wide = op == "neg" && matches!(ty.payload(), Ty::Int) && !self.walk.iv_of(expr).neg().fits();
                self.expr(expr);
                if wide && matches!(expr.ty().payload(), Ty::Int) {
                    **expr = to_type(take(expr), &Ty::Big);
                }
                if op == "neg" && matches!(expr.ty().payload(), Ty::Big) {
                    *ty = big_of(ty);
                }
            }
            Expr::Call { target, args, ty, .. } => {
                for a in args.iter_mut() {
                    self.expr(a);
                }
                match target {
                    Target::User(g) => {
                        if let Some(ps) = self.sigs.params.get(g.as_str()) {
                            for (a, p) in args.iter_mut().zip(ps) {
                                *a = to_type(take(a), p);
                            }
                        }
                        // (the call's own type where its answer does not grow: the
                        // lowering shaped it to its arguments, a tensor's rank)
                        if let Some(r) = self.sigs.rets.get(g.as_str())
                            && has_big(r)
                        {
                            *ty = match ty {
                                Ty::Result(_) if !r.is_result() => Ty::Result(Box::new(r.clone())),
                                _ => r.clone(),
                            };
                        }
                    }
                    Target::Row(row) => {
                        // `int[n]` of a big int is itself (`float[n]`, its nearest
                        // float, is a row's: the call keeps a failure riding on n)
                        if row.range == crate::table::IntRange::Same && args.len() == 1 && matches!(args[0].ty().payload(), Ty::Big) {
                            // (n that can fail, as the call answered it)
                            let fails = ty.is_result();
                            *e = match take(&mut args[0]) {
                                Expr::Convert { how: crate::ir::Convert::Unwrap, expr, .. } if fails && expr.ty().is_result() => *expr,
                                other => other,
                            };
                            return;
                        }
                        // the row that keeps every digit: where its answer may
                        // grow (`int[text]`, `floor[x]`), or a big int is
                        // handed to one that keeps it (`abs`, `min`, `max`)
                        let grows = (row.range == crate::table::IntRange::Any && row.big.is_some()) || (row.range == crate::table::IntRange::Shift && wide);
                        if let Some(big) = crate::table::big_form(row.id)
                            && (grows || args.iter().any(|a| has_big(&a.ty())))
                        {
                            let name = row.name;
                            *row = big;
                            for (a, k) in args.iter_mut().zip(big.params) {
                                match k {
                                    crate::table::Kind::Big => *a = to_type(take(a), &Ty::Big),
                                    crate::table::Kind::Bigs => *a = to_type(take(a), &Ty::List(Box::new(Ty::Big))),
                                    // (a parameter of 64 bits takes a growing
                                    // int as any row's does: refused past them,
                                    // naming the row — `shift[3, 2 ^ 100]`'s
                                    // amount had reached the growing form as
                                    // it was, an internal error)
                                    k if has_big(&a.ty()) => *a = handed(take(a), k.takes_big(), crate::ir::Who::Row(name)),
                                    _ => {}
                                }
                            }
                            let answer = big.ret.ty();
                            *ty = match ty {
                                Ty::Result(_) => Ty::Result(Box::new(answer)),
                                _ if matches!(big.fails, crate::table::Fails::Value(_)) => Ty::Result(Box::new(answer)),
                                _ => answer,
                            };
                            return;
                        }
                        let kinds = crate::table::kinds_of(row.params, args.len()).unwrap_or_default();
                        // a row generic over its items (`push[xs, x]`, `set[m,
                        // k, v]`): an item takes the collection's item type, and
                        // the answer is a collection of it
                        use crate::table::Kind;
                        let at = kinds.iter().position(|k| matches!(k, Kind::List | Kind::Seq | Kind::MapOf | Kind::MapRef));
                        let mut collection = at.map(|i| args[i].ty().payload().clone());
                        // (a collection made where it is used, its items not yet
                        // known — `map[]`, `[]` — holds what the call says it
                        // answers)
                        if let Some(Ty::List(item) | Ty::Map(_, item)) = &collection
                            && matches!(item.as_ref(), Ty::Unknown)
                        {
                            collection = Some(ty.payload().clone());
                            // (one made here holds a growing item as it is:
                            // `push[[], 2 ^ 100]` had narrowed the item to 64
                            // bits, the answer's ints, and stopped — crow
                            // check --self)
                            let grows = args.iter().zip(&kinds).any(|(a, k)| matches!(k, Kind::Item | Kind::ItemRef) && has_big(&a.ty()));
                            if grows {
                                collection = match collection {
                                    Some(Ty::List(_)) => Some(Ty::List(Box::new(Ty::Big))),
                                    Some(Ty::Map(key, _)) => Some(Ty::Map(key, Box::new(Ty::Big))),
                                    other => other,
                                };
                                if let (Some(i), Some(c)) = (at, collection.clone()) {
                                    match &mut args[i] {
                                        Expr::ListLit(items, t) if items.is_empty() => *t = c,
                                        Expr::MapLit(pairs, t) if pairs.is_empty() => *t = c,
                                        _ => {}
                                    }
                                }
                            }
                        }
                        // (a list or a map of 64-bit ints handed an item that
                        // may grow past them holds every item as it is — spec,
                        // Numbers That Keep Every Digit: a list's items and a
                        // map's values take one as it is; `push[xs, 2 ^ 100]`
                        // had stopped where xs's items were 64-bit)
                        let grows_past = |me: &mut Self, args: &[Expr]| {
                            args.iter().zip(&kinds).any(|(a, k)| matches!(k, Kind::Item | Kind::ItemRef) && matches!(a.ty().payload(), Ty::Big) && !me.walk.iv_of(a).fits())
                        };
                        if let (Some(i), Some(Ty::List(item) | Ty::Map(_, item))) = (at, &collection)
                            && matches!(item.payload(), Ty::Int)
                            && grows_past(self, args)
                        {
                            let grown = match &collection {
                                Some(Ty::Map(key, _)) => Ty::Map(*key, Box::new(Ty::Big)),
                                _ => Ty::List(Box::new(Ty::Big)),
                            };
                            args[i] = to_type(take(&mut args[i]), &grown);
                            collection = Some(grown);
                        }
                        if let Some(Ty::List(item) | Ty::Map(_, item)) = &collection {
                            // (and a collection of 64-bit ints takes an item
                            // in 64 bits: one the rewrite grew where the
                            // analysis bounded it — two loops' `i` share one
                            // place — is checked there, alike in both modes)
                            for (a, k) in args.iter_mut().zip(&kinds) {
                                if matches!(k, Kind::Item | Kind::ItemRef) && (has_big(item) || has_big(&a.ty())) {
                                    *a = to_type(take(a), item);
                                }
                            }
                            if has_big(item) && matches!(row.ret, Kind::List | Kind::MapOf) && let Some(c) = &collection {
                                *ty = if ty.is_result() { Ty::Result(Box::new(c.clone())) } else { c.clone() };
                            }
                        }
                        for (i, a) in args.iter_mut().enumerate() {
                            if !has_big(&a.ty()) {
                                continue;
                            }
                            let takes = kinds.get(i).map(|k| k.takes_big()).unwrap_or(crate::table::TakesBig::Refused);
                            *a = handed(take(a), takes, crate::ir::Who::Row(row.name));
                        }
                    }
                    // a form that calls a function by its name (`stream[feed,
                    // a, …]`): each argument after the function as the
                    // parameter it is handed to takes it, as a call's
                    Target::Runtime(n) if n == "stream" && matches!(args.first(), Some(Expr::FnRef(..))) => {
                        let Some(Expr::FnRef(f, _)) = args.first() else { return };
                        if let Some(ps) = self.sigs.params.get(f.as_str()) {
                            for (a, p) in args.iter_mut().skip(1).zip(ps.iter().skip(1)) {
                                *a = to_type(take(a), p);
                            }
                        }
                    }
                    // a form of its own: one that shows a value (`type`) takes
                    // it as it is; any other, its 64 bits
                    other => {
                        // (a form that takes any value as it is: `type`, `str`)
                        let shows = matches!(other, Target::Runtime(n) if crate::table::rows_named(n.rsplit('.').next().unwrap_or(n)).any(|r| r.params.first().is_some_and(|k| k.takes_big() == crate::table::TakesBig::AsIs)));
                        let takes = if shows { crate::table::TakesBig::AsIs } else { crate::table::TakesBig::Refused };
                        for a in args.iter_mut() {
                            if has_big(&a.ty()) {
                                *a = handed(take(a), takes, crate::ir::Who::Held);
                            }
                        }
                    }
                }
            }
            Expr::Effect { args, row, name, .. } => {
                for a in args.iter_mut() {
                    self.expr(a);
                }
                // (`later[f, a, …]` calls f[a, …] by its name: each argument
                // after the function as the parameter it is handed to takes it)
                if name == "later"
                    && let Some(Expr::FnRef(f, _)) = args.first()
                {
                    if let Some(ps) = self.sigs.params.get(f.as_str()).cloned() {
                        for (a, p) in args.iter_mut().skip(1).zip(&ps) {
                            *a = to_type(take(a), p);
                        }
                    }
                    return;
                }
                // an effect takes a big int as its row's parameter does; `log`
                // shows any value
                let kinds = row.and_then(|r| crate::table::kinds_of(r.params, args.len()));
                let name = row.map(|r| r.name).unwrap_or("log");
                for (i, a) in args.iter_mut().enumerate() {
                    if !has_big(&a.ty()) {
                        continue;
                    }
                    let takes = match &kinds {
                        Some(k) => k.get(i).map(|k| k.takes_big()).unwrap_or(crate::table::TakesBig::Refused),
                        None => crate::table::TakesBig::AsIs,
                    };
                    *a = handed(take(a), takes, crate::ir::Who::Row(name));
                }
            }
            // a literal of whole numbers made a list, one of them growing: a
            // list literal of growing ints (a tensor holds 64-bit numbers)
            Expr::Convert { how: crate::ir::Convert::ToList, expr, .. } if matches!(&**expr, Expr::ListLit(_, Ty::Tensor(..))) => {
                let Expr::ListLit(items, _) = &mut **expr else { return };
                for i in items.iter_mut() {
                    self.expr(i);
                }
                if items.iter().any(|i| matches!(i.ty().payload(), Ty::Big)) {
                    let items: Vec<Expr> = std::mem::take(items).into_iter().map(|i| to_type(i, &Ty::Big)).collect();
                    *e = Expr::ListLit(items, Ty::List(Box::new(Ty::Big)));
                }
            }
            Expr::Convert { expr, ty, how } => {
                self.expr(expr);
                if matches!(how, crate::ir::Convert::Unwrap | crate::ir::Convert::Known) && has_big(&expr.ty()) {
                    *ty = expr.ty().payload().clone();
                }
            }
            Expr::Meta { base, .. } => self.expr(base),
            Expr::Interp(pieces) => {
                for p in pieces.iter_mut() {
                    if let crate::ir::Piece::Expr(x) = p {
                        self.expr(x);
                    }
                }
            }
            Expr::ListLit(items, ty) => {
                for i in items.iter_mut() {
                    self.expr(i);
                }
                // a tensor's whole numbers are held as floats exactly within
                // 64 bits: a big one is its 64 bits, or refused
                if matches!(ty, Ty::Tensor(..)) {
                    for i in items.iter_mut() {
                        if matches!(i.ty().payload(), Ty::Big) {
                            *i = to_type(take(i), &Ty::Int);
                        }
                    }
                }
                if items.iter().any(|i| matches!(i.ty().payload(), Ty::Big)) && let Ty::List(item) = ty {
                    let item = big_of(item);
                    for i in items.iter_mut() {
                        *i = to_type(take(i), &Ty::Big);
                    }
                    *ty = Ty::List(Box::new(item));
                }
            }
            Expr::MapLit(pairs, ty) => {
                for (_, v) in pairs.iter_mut() {
                    self.expr(v);
                }
                if pairs.iter().any(|(_, v)| matches!(v.ty().payload(), Ty::Big)) && let Ty::Map(k, item) = ty {
                    let (k, item) = (*k, big_of(item));
                    for (_, v) in pairs.iter_mut() {
                        *v = to_type(take(v), &Ty::Big);
                    }
                    *ty = Ty::Map(k, Box::new(item));
                }
            }
            Expr::RecordLit { name, fields } => {
                for f in fields.iter_mut() {
                    self.expr(f);
                }
                if let Some(declared) = self.sigs.records.get(name.as_str()) {
                    for (f, ty) in fields.iter_mut().zip(declared) {
                        *f = to_type(take(f), ty);
                    }
                }
            }
            Expr::Tree { attrs, children, .. } => {
                for (_, a) in attrs.iter_mut() {
                    self.expr(a);
                }
                for c in children.iter_mut() {
                    self.expr(c);
                }
            }
            Expr::ForExpr { var, iter, body, ty, .. } => {
                self.expr(iter);
                self.vars.push(HashMap::new());
                let item = crate::type_infer::TypeContext::loop_item(iter.ty());
                self.declare(var, item);
                for s in body.iter_mut() {
                    self.stmt(s, &Ty::Unit);
                }
                // every round's value — each branch's last line — one kind:
                // grown where any grows
                let mut tails = Vec::new();
                tails_of(body, &mut tails);
                if tails.iter().any(|y| matches!(y.ty().payload(), Ty::Big)) && let Ty::List(item) = ty {
                    let grown = big_of(item);
                    for y in tails {
                        *y = to_type(take(y), &grown);
                    }
                    *ty = Ty::List(Box::new(grown));
                }
                self.vars.pop();
            }
            Expr::Deferred { args, .. } => {
                for a in args.iter_mut() {
                    self.expr(a);
                }
            }
            Expr::Const(..) | Expr::Constant(..) | Expr::FnRef(..) => {}
        }
    }

}

/// An argument holding a big int, as its parameter takes one: as it is, as
/// its nearest float (a list's items each), or in 64 bits (refused past
/// them, naming what took it).
fn handed(a: Expr, takes: crate::table::TakesBig, who: crate::ir::Who) -> Expr {
    use crate::table::TakesBig;
    // (beneath a payload's read, as `to_type`)
    if let Expr::Convert { how: how @ (crate::ir::Convert::Unwrap | crate::ir::Convert::Known), expr, .. } = a {
        let inner = handed(*expr, takes, who);
        let ty = inner.ty().payload().clone();
        return Expr::Convert { how, expr: Box::new(inner), ty };
    }
    let ty = a.ty();
    let keep = |t: Ty| if ty.is_result() && !t.is_result() { Ty::Result(Box::new(t)) } else { t };
    match takes {
        TakesBig::AsIs => a,
        TakesBig::AsFloat if matches!(ty.payload(), Ty::Big) => Expr::Convert { how: crate::ir::Convert::IntToFloat, expr: Box::new(a), ty: keep(Ty::Float) },
        TakesBig::AsFloat if matches!(ty.payload(), Ty::List(i) if matches!(i.as_ref(), Ty::Big)) => {
            Expr::Convert { how: crate::ir::Convert::IntsToFloats, expr: Box::new(a), ty: keep(Ty::List(Box::new(Ty::Float))) }
        }
        _ => {
            let want = small_of(&ty);
            Expr::Convert { how: crate::ir::Convert::ToSmall(who), expr: Box::new(a), ty: want }
        }
    }
}

/// The refusal a conversion to 64 bits names, made `who`'s (beneath the
/// reads that wrap it).
fn name_refusal(e: &mut Expr, who: crate::ir::Who) {
    match e {
        Expr::Convert { how: crate::ir::Convert::ToSmall(w), .. } => *w = who,
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => name_refusal(expr, who),
        _ => {}
    }
}

/// `tails_of`, read.
fn tails_in<'b>(body: &'b Block, out: &mut Vec<&'b Expr>) {
    let Some(last) = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) else { return };
    match last {
        Stmt::Do(y) => out.push(y),
        Stmt::If { then, els, .. } => {
            tails_in(then, out);
            if let Some(e) = els {
                tails_in(e, out);
            }
        }
        _ => {}
    }
}

/// A collecting loop's values: the last line of its body, or of each
/// branch where the body ends in an `if`.
fn tails_of<'b>(body: &'b mut Block, out: &mut Vec<&'b mut Expr>) {
    let Some(last) = body.iter_mut().rev().find(|s| !matches!(s, Stmt::Line(_))) else { return };
    match last {
        Stmt::Do(y) => out.push(y),
        Stmt::If { then, els, .. } => {
            tails_of(then, out);
            if let Some(e) = els {
                tails_of(e, out);
            }
        }
        _ => {}
    }
}

/// An item's type with its int grown, a failure it can be kept (a list of
/// ints that can fail holds big ones that can).
fn big_of(t: &Ty) -> Ty {
    match t {
        Ty::Int | Ty::Big => Ty::Big,
        Ty::Result(i) => Ty::Result(Box::new(big_of(i))),
        other => other.clone(),
    }
}

/// A type with its big ints held in 64 bits (what a builtin takes).
fn small_of(t: &Ty) -> Ty {
    match t {
        Ty::Big => Ty::Int,
        Ty::List(i) => Ty::List(Box::new(small_of(i))),
        Ty::Map(k, i) => Ty::Map(*k, Box::new(small_of(i))),
        Ty::Result(i) => Ty::Result(Box::new(small_of(i))),
        other => other.clone(),
    }
}

/// A position or a key: a big one is its 64 bits, or refused.
fn small_index(e: Expr) -> Expr {
    if let Expr::Convert { how: how @ (crate::ir::Convert::Unwrap | crate::ir::Convert::Known), expr, .. } = e {
        let inner = small_index(*expr);
        let ty = inner.ty().payload().clone();
        return Expr::Convert { how, expr: Box::new(inner), ty };
    }
    if matches!(e.ty().payload(), Ty::Big) {
        let ty = if e.ty().is_result() { Ty::Result(Box::new(Ty::Int)) } else { Ty::Int };
        Expr::Convert { how: crate::ir::Convert::ToSmall(crate::ir::Who::Position), expr: Box::new(e), ty }
    } else {
        e
    }
}

fn take(e: &mut Expr) -> Expr {
    std::mem::replace(e, Expr::Const(Const::Unit, Ty::Unit))
}
