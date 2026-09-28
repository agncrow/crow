//! The IR checked before either backend sees it (docs/bug-classes.md §2).
//!
//! The lowering decides every type once; the interpreter walks what it
//! decided and the Rust backend prints it. A lowering that decides wrongly
//! showed up late and far away: a Rust compile error in the generated
//! program, or a value the interpreter carried where its type said it could
//! not be. This pass walks every body of a lowered program with the
//! bindings in scope and checks that the parts agree with each other: every
//! local read is bound and read at its binding's type, every value fits the
//! slot it is stored in, a decision is made on a boolean that cannot fail,
//! a loop walks something with items, a call hands a function what its
//! parameters take, a conversion converts what it says it does. A program
//! that fails it is a bug in the lowering, reported as one, never a program
//! the user must change.

use std::collections::HashMap;

use crate::ir::*;

/// Whether the lowering checks what it made, and what it does with a
/// finding.
pub enum Mode {
    Off,
    /// Print each finding and go on (`CROW_IR_VERIFY=report`).
    Report,
    /// Refuse the program as the lowering's bug (`CROW_IR_VERIFY=strict`,
    /// and every debug build, so `cargo test` checks every program it
    /// lowers; the harnesses run the release binary strict).
    Strict,
}

pub fn mode() -> Mode {
    match std::env::var("CROW_IR_VERIFY").as_deref() {
        Ok("strict") => Mode::Strict,
        Ok("off") => Mode::Off,
        Ok(_) => Mode::Report,
        Err(_) if cfg!(debug_assertions) => Mode::Strict,
        Err(_) => Mode::Off,
    }
}

/// What the checks found, one line each: the body, the line, what
/// disagrees. Empty for a program whose parts agree.
pub fn verify(program: &Program) -> Vec<String> {
    let mut v = Verifier::new(program);
    for part in &program.parts {
        for f in &part.facts {
            v.body = format!("fact {}", f.name);
            v.line = 0;
            let t = v.expr(&f.default);
            if !fits(&t, &f.ty) {
                v.found(format!("the default ({}) does not fit the fact's type {}", t, f.ty));
            }
        }
        for d in &part.derives {
            v.body = format!("derive {}", d.name);
            v.kind = None;
            v.testing = false;
            v.ret = Some(d.ty.clone());
            v.scopes = vec![HashMap::new()];
            v.block(&d.body);
        }
        // the original of a specialized function is never run or compiled:
        // its clones are, and its signature is every call's types joined
        let specialized: std::collections::HashSet<&str> = part
            .fns
            .iter()
            .filter_map(|f| f.name.rsplit_once("__").filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(b, _)| b))
            .collect();
        for f in &part.fns {
            if specialized.contains(f.name.as_str()) {
                continue;
            }
            v.body = format!("fn {}", f.name);
            v.kind = Some(f.kind);
            v.testing = false;
            v.ret = Some(f.ret.clone());
            v.scopes = vec![f.params.iter().cloned().collect()];
            v.block(&f.body);
        }
        for t in &part.tests {
            v.body = format!("test {}", t.name.clone().unwrap_or_default());
            v.kind = None;
            v.testing = true;
            v.ret = None;
            v.scopes = vec![t.params.iter().map(|p| (p.clone(), Ty::Unknown)).collect()];
            v.block(&t.body);
        }
        v.body = format!("{} main", part.name);
        v.kind = None;
        v.testing = false;
        v.ret = None;
        v.scopes = vec![HashMap::new()];
        v.block(&part.main);
    }
    v.out
}

struct Verifier<'p> {
    fns: HashMap<&'p str, &'p Fn>,
    facts: HashMap<&'p str, &'p Ty>,
    derives: HashMap<&'p str, &'p Ty>,
    /// A script's top-level variables, which its functions read as globals
    /// (a connection, a module's constants).
    globals: HashMap<&'p str, &'p Ty>,
    /// The kind of function whose body is checked: a handler's value goes
    /// to nobody, and a test runs in the interpreter only.
    kind: Option<FnKind>,
    testing: bool,
    /// The locals in scope, innermost last.
    scopes: Vec<HashMap<String, Ty>>,
    /// The body being checked, and the type its `return` hands back.
    body: String,
    ret: Option<Ty>,
    line: usize,
    out: Vec<String>,
}

impl<'p> Verifier<'p> {
    fn new(program: &'p Program) -> Self {
        let mut fns = HashMap::new();
        let mut facts = HashMap::new();
        let mut derives = HashMap::new();
        let mut globals = HashMap::new();
        for part in &program.parts {
            for s in &part.main {
                if let Stmt::Let { var, ty, .. } = s {
                    globals.insert(var.as_str(), ty);
                }
            }
            for f in &part.fns {
                fns.insert(f.name.as_str(), f);
            }
            for f in &part.facts {
                facts.insert(f.name.as_str(), &f.ty);
            }
            for d in &part.derives {
                derives.insert(d.name.as_str(), &d.ty);
            }
        }
        Verifier { fns, facts, derives, globals, kind: None, testing: false, scopes: vec![HashMap::new()], body: String::new(), ret: None, line: 0, out: Vec::new() }
    }

    fn found(&mut self, what: String) {
        self.out.push(format!("{}: line {}: {}", self.body, self.line, what));
    }

    fn local(&self, name: &str) -> Option<&Ty> {
        self.scopes.iter().rev().find_map(|s| s.get(name)).or_else(|| self.globals.get(name).copied())
    }

    fn bind(&mut self, name: &str, ty: Ty) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), ty);
        }
    }

    fn scoped(&mut self, b: &[Stmt], bound: Option<(&str, Ty)>) {
        self.scopes.push(HashMap::new());
        if let Some((n, t)) = bound {
            self.bind(n, t);
        }
        self.block(b);
        self.scopes.pop();
    }

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Line(n) => self.line = *n,
            Stmt::Let { var, ty, expr } => {
                let t = self.expr(expr);
                if !fits(&t, ty) && !sentinel(expr, ty) {
                    self.found(format!("'{}' is declared {} but its first value is {}{}", var, ty, t, what(expr)));
                }
                self.bind(var, ty.clone());
            }
            Stmt::Set { var, expr } => {
                let t = self.expr(expr);
                match self.local(var).cloned() {
                    None if self.facts.contains_key(var.as_str()) => {}
                    None => self.found(format!("'{}' is assigned before it is bound", var)),
                    Some(held) => {
                        if !fits(&t, &held) && !sentinel(expr, &held) {
                            self.found(format!("'{}' holds {} but is assigned {}{}", var, held, t, what(expr)));
                        }
                    }
                }
            }
            Stmt::SetPath { var, path, expr } => {
                self.expr(expr);
                for step in path {
                    if let PathStep::Index(i) = step {
                        self.expr(i);
                    }
                }
                if self.local(var).is_none() && !self.facts.contains_key(var.as_str()) {
                    self.found(format!("'{}' is assigned through before it is bound", var));
                }
            }
            Stmt::Store { fact, expr } => {
                let t = self.expr(expr);
                match self.facts.get(fact.as_str()).copied().cloned() {
                    None => self.found(format!("a store to '{}', which is no fact", fact)),
                    // a value that can fail stops the handler before it
                    // lands: a fact never holds an error (both backends)
                    Some(held) => {
                        if !fits(t.payload(), &held) {
                            self.found(format!("the fact '{}' holds {} but is stored {}", fact, held, t));
                        }
                    }
                }
            }
            Stmt::If { cond, then, els } => {
                self.decision(cond, "an if");
                self.scoped(then, None);
                if let Some(e) = els {
                    self.scoped(e, None);
                }
            }
            Stmt::While { cond, body } => {
                self.decision(cond, "a while");
                self.scoped(body, None);
            }
            Stmt::For { var, iter, body, .. } => {
                let item = self.walked(iter);
                self.scoped(body, Some((var, item)));
            }
            Stmt::Return(e) => {
                let t = e.as_ref().map(|x| self.expr(x)).unwrap_or(Ty::Unit);
                // a handler's or a helper's value goes to nobody
                let valueless = matches!(self.kind, Some(FnKind::Handler | FnKind::Helper));
                if let Some(ret) = self.ret.clone() {
                    if !valueless && !returns(&t, &ret) && !e.as_ref().is_some_and(|x| sentinel(x, &ret)) {
                        self.found(format!("returns {} from a body typed {}", t, ret));
                    }
                }
            }
            Stmt::Break | Stmt::Continue => {}
            Stmt::Do(e) => {
                self.expr(e);
            }
            Stmt::Propagate { values, .. } => {
                for v in values {
                    match self.local(v) {
                        None => self.found(format!("propagates '{}', which is not bound", v)),
                        Some(t) if !t.is_result() && *t != Ty::Unknown => {
                            let t = t.clone();
                            self.found(format!("propagates '{}', which is {} and cannot fail", v, t))
                        }
                        _ => {}
                    }
                }
            }
            Stmt::Stop { value } => {
                if self.local(value).is_none() {
                    self.found(format!("stops on '{}', which is not bound", value));
                }
            }
        }
    }

    /// A condition: a boolean, and one that cannot fail — a propagation
    /// before the decision refuses an error, and the condition reads what
    /// is left.
    fn decision(&mut self, cond: &Expr, what: &str) {
        let t = self.expr(cond);
        if !matches!(t, Ty::Bool | Ty::Unknown | Ty::Json) {
            self.found(format!("{} decides on a value of type {}", what, t));
        }
    }

    /// What a loop walks, and the type of each item.
    fn walked(&mut self, iter: &Expr) -> Ty {
        let t = self.expr(iter);
        match &t {
            Ty::List(e) | Ty::Tensor(e) | Ty::Column(e) => e.as_ref().clone(),
            Ty::Map(_) => Ty::Str,
            Ty::Str | Ty::Markup => Ty::Str,
            Ty::Frame(_) | Ty::Json | Ty::Unknown => Ty::Unknown,
            // refused where it runs, by both backends: `cannot loop over an
            // error value`
            Ty::Result(inner) => match inner.as_ref() {
                Ty::List(e) | Ty::Tensor(e) | Ty::Column(e) => e.as_ref().clone(),
                _ => Ty::Unknown,
            },
            other => {
                let other = other.clone();
                self.found(format!("a loop walks {}, which has no items", other));
                Ty::Unknown
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Ty {
        match e {
            Expr::Const(_, t) | Expr::Constant(_, t) | Expr::FnRef(_, t) => t.clone(),
            Expr::Local(n, t) | Expr::Move(n, t) => {
                match self.local(n).cloned() {
                    None => {
                        // a script's top-level variable read as a global,
                        // a fact read as a local by a body that holds it
                        if !self.facts.contains_key(n.as_str()) {
                            self.found(format!("reads '{}', which is not bound", n));
                        }
                    }
                    Some(bound) => {
                        if !agrees(t, &bound) {
                            self.found(format!("reads '{}' as {}, but it is bound as {}", n, t, bound));
                        }
                    }
                }
                t.clone()
            }
            Expr::Load(n, t) => {
                match self.facts.get(n.as_str()).copied().cloned() {
                    None => self.found(format!("loads '{}', which is no fact", n)),
                    Some(held) => {
                        if !agrees(t, &held) {
                            self.found(format!("loads the fact '{}' as {}, but it holds {}", n, t, held));
                        }
                    }
                }
                t.clone()
            }
            Expr::Derived(n, t) => {
                if !self.derives.contains_key(n.as_str()) && !self.fns.contains_key(n.as_str()) {
                    self.found(format!("reads the derivation '{}', which is not defined", n));
                }
                t.clone()
            }
            Expr::Call { target, args, ty, propagates } => {
                // an argument that can fail is marked with an unwrap for a
                // call that hands the error on: the error is the call's value
                let ts: Vec<Ty> = args
                    .iter()
                    .map(|a| {
                        let t = self.expr(a);
                        match a {
                            Expr::Convert { how: Convert::Unwrap, expr, .. } if expr.ty().is_result() => expr.ty(),
                            _ => t,
                        }
                    })
                    .collect();
                if let Target::User(name) = target {
                    self.user_call(name, &ts, ty, *propagates);
                }
                ty.clone()
            }
            Expr::Effect { args, ty, .. } => {
                // a query's parameter list holds values of every type: each
                // is bound as its own
                for a in args {
                    match a {
                        Expr::ListLit(items, _) => {
                            for i in items {
                                self.expr(i);
                            }
                        }
                        other => {
                            self.expr(other);
                        }
                    }
                }
                ty.clone()
            }
            Expr::Convert { how, expr, ty } => {
                let inner = self.expr(expr);
                self.conversion(*how, &inner, ty);
                ty.clone()
            }
            Expr::Binary { l, r, ty, .. } => {
                self.expr(l);
                self.expr(r);
                ty.clone()
            }
            Expr::Unary { expr, ty, .. } => {
                self.expr(expr);
                ty.clone()
            }
            Expr::Index { base, indices, ty } => {
                self.expr(base);
                for i in indices {
                    self.expr(i);
                }
                ty.clone()
            }
            Expr::Field { base, ty, .. } => {
                self.expr(base);
                ty.clone()
            }
            Expr::Meta { base, .. } => {
                self.expr(base);
                e.ty()
            }
            Expr::Interp(pieces) => {
                for p in pieces {
                    if let Piece::Expr(x) = p {
                        self.expr(x);
                    }
                }
                e.ty()
            }
            Expr::ListLit(items, ty) => {
                let el = match ty {
                    Ty::List(el) | Ty::Tensor(el) => Some(el.as_ref().clone()),
                    _ => None,
                };
                for i in items {
                    let t = self.expr(i);
                    if let Some(el) = &el {
                        if !fits(&t, el) && !numbers(&t, el) {
                            self.found(format!("a list of {} holds an item of type {}", el, t));
                        }
                    }
                }
                ty.clone()
            }
            Expr::MapLit(pairs, ty) => {
                for (_, x) in pairs {
                    self.expr(x);
                }
                ty.clone()
            }
            Expr::RecordLit { fields, .. } => {
                for f in fields {
                    self.expr(f);
                }
                e.ty()
            }
            Expr::Tree { attrs, children, .. } => {
                for (_, x) in attrs {
                    self.expr(x);
                }
                for c in children {
                    self.expr(c);
                }
                Ty::Node
            }
            Expr::ForExpr { var, iter, body, ty, .. } => {
                let item = self.walked(iter);
                self.scoped(body, Some((var, item)));
                ty.clone()
            }
            Expr::Deferred { args, .. } => {
                for a in args {
                    self.expr(a);
                }
                Ty::Unit
            }
        }
    }

    /// A call of one of the program's functions: as many arguments as it
    /// has parameters, each fitting its parameter. An argument that can
    /// fail where the parameter cannot is the call's own error to hand on,
    /// so the call must propagate and be typed as able to fail.
    fn user_call(&mut self, name: &str, args: &[Ty], ty: &Ty, propagates: bool) {
        let Some(f) = self.fns.get(name).copied() else {
            self.found(format!("calls '{}', which the program does not define", name));
            return;
        };
        // a render reads state and never moves it: a view, or a derivation,
        // calls only what writes nothing
        let renders_here = matches!(self.kind, Some(FnKind::View)) || self.body.starts_with("derive ");
        if renders_here && matches!(f.kind, FnKind::Handler | FnKind::Helper) {
            self.found(format!("calls '{}', a {} that writes the session's facts, from a render", name, f.kind.word()));
        }
        if f.params.len() != args.len() {
            self.found(format!("calls '{}' with {} arguments; it takes {}", name, args.len(), f.params.len()));
            return;
        }
        // a view renders a failed argument as its text; a call whose value
        // is nothing drops the error it would hand on (a helper called for
        // what it writes: docs/bug-classes.md, open question 1)
        let renders = f.kind == FnKind::View || (propagates && *ty == Ty::Unit);
        for (i, ((p, pt), at)) in f.params.iter().zip(args).enumerate() {
            if at.is_result() && !pt.is_result() && *pt != Ty::Unknown {
                if !renders && !(propagates && (ty.is_result() || *ty == Ty::Unknown)) {
                    self.found(format!("hands '{}' a value that can fail for its parameter '{}' ({}), and the call does not hand the error on", name, p, pt));
                }
                continue;
            }
            // a test runs in the interpreter, which holds numbers either way
            let numbers_either_way = self.testing && same_numbers(at, pt);
            if !fits(at, pt) && !numbers_either_way {
                self.found(format!("hands '{}' {} for its parameter '{}' (argument {}), which is {}", name, at, p, i + 1, pt));
            }
        }
    }

    fn conversion(&mut self, how: Convert, inner: &Ty, ty: &Ty) {
        let ok = match how {
            Convert::Unwrap | Convert::Known => match inner {
                Ty::Result(p) => agrees(p, ty),
                Ty::Unknown => true,
                // a plain value read through the mark is itself
                other => agrees(other, ty),
            },
            Convert::IntToFloat => matches!(inner.payload(), Ty::Int | Ty::Unknown) && matches!(ty, Ty::Float),
            Convert::ToStr => matches!(ty, Ty::Str | Ty::Markup),
            Convert::ToTensor => matches!(ty, Ty::Tensor(_)),
            Convert::ToList => matches!(ty, Ty::List(_)),
        };
        if !ok {
            self.found(format!("a conversion {:?} of {} claims to give {}", how, inner, ty));
        }
    }
}

/// A value of type `v` stored in a slot of type `slot`: the same type, or
/// one the slot holds as it is — a plain value in a slot that can fail, a
/// string where markup goes, an empty list whose items are not known yet.
/// An int in a float slot is not: the lowering converts it explicitly.
pub fn fits(v: &Ty, slot: &Ty) -> bool {
    if v == slot {
        return true;
    }
    match (v, slot) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        (Ty::Result(a), Ty::Result(b)) => fits(a, b),
        (_, Ty::Result(inner)) => fits(v, inner),
        (Ty::Markup, Ty::Str) | (Ty::Str, Ty::Markup) => true,
        // a view's tree placed where markup goes is its markup
        (Ty::Node, Ty::Markup | Ty::Str) | (Ty::Markup, Ty::Node) => true,
        // an empty list literal handed where numbers go is an empty tensor
        (Ty::List(a), Ty::Tensor(_)) if **a == Ty::Unknown => true,
        (Ty::List(a), Ty::List(b)) | (Ty::Map(a), Ty::Map(b)) | (Ty::Tensor(a), Ty::Tensor(b)) | (Ty::Column(a), Ty::Column(b)) => fits(a, b),
        (Ty::Frame(_), Ty::Frame(_)) => true,
        (Ty::Record(a), Ty::OptRecord(b)) | (Ty::OptRecord(a), Ty::Record(b)) => a == b,
        (Ty::Fn(..), Ty::Fn(..)) => true,
        _ => false,
    }
}

/// What a `return` hands back fits the body's type; an error of another
/// shape passes through with the body's default as its value.
fn returns(v: &Ty, ret: &Ty) -> bool {
    fits(v, ret) || (v.is_result() && ret.is_result())
}

/// A read's type and its binding's agree: the same, or one side not known.
fn agrees(a: &Ty, b: &Ty) -> bool {
    a == b || fits(a, b) && fits(b, a)
}

/// What a value is, where a finding needs it: the call it comes from.
fn what(e: &Expr) -> String {
    match e {
        Expr::Call { target: Target::Runtime(n), .. } => format!(" (from the runtime's {})", n),
        Expr::Call { target: Target::Module(m, n), .. } => format!(" (from {}.{})", m, n),
        Expr::Call { target: Target::User(n), .. } => format!(" (from {})", n),
        Expr::Effect { name, .. } => format!(" (from the effect {})", name),
        _ => String::new(),
    }
}

/// `0`, the empty sentinel, where a recursive link is held.
fn sentinel(e: &Expr, slot: &Ty) -> bool {
    matches!((e, slot.payload()), (Expr::Const(Const::Int(0), _), Ty::OptRecord(_)))
}

/// A tensor and a list of the same numbers.
fn same_numbers(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Tensor(x), Ty::List(y)) | (Ty::List(x), Ty::Tensor(y)) => fits(x, y) || numbers(x, y) || numbers(y, x),
        _ => false,
    }
}

/// Numbers in a list of numbers: an int item of a float list or tensor is
/// the float it widens to.
fn numbers(item: &Ty, el: &Ty) -> bool {
    matches!((item, el), (Ty::Int, Ty::Float))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lowered(src: &str) -> Program {
        let stmts = crate::build(src).expect("parses");
        crate::ir_interp::prepare(&stmts, "t").expect("lowers").0
    }

    #[test]
    fn a_program_whose_parts_agree_passes() {
        let p = lowered("fn add[a, b]\n    return a + b\nx = add[1, 2]\ny = 2.5\ny = x\nlog[y]\nfor i in range[3]\n    log[i]\n");
        assert_eq!(verify(&p), Vec::<String>::new());
    }

    /// A part built without its callers (a test of one file, the language
    /// server) knows nothing of a parameter: a function returning one
    /// returned "nothing", and the local holding its value was declared as
    /// the unit type, which a float variable was then assigned (finch's
    /// sim/autonext.co alone).
    #[test]
    fn a_function_returning_what_it_does_not_know_returns_an_unknown() {
        let p = lowered("fn closing[xs, w]\n    stop = -1.0\n    e = ended[xs, 0, w]\n    stop = e\n    return stop\nfn ended[xs, i, w]\n    if xs[i] lt 0.0\n        return w\n    return xs[i]\n");
        assert_eq!(verify(&p), Vec::<String>::new());
        let ended = p.parts[0].fns.iter().find(|f| f.name == "ended").expect("ended is lowered");
        assert_eq!(ended.ret, Ty::Unknown);
    }

    #[test]
    fn a_read_of_an_unbound_local_is_found() {
        let mut p = lowered("x = 1\nlog[x]\n");
        p.parts[0].main.push(Stmt::Do(Expr::Local("nowhere".into(), Ty::Int)));
        let found = verify(&p);
        assert!(found.iter().any(|f| f.contains("reads 'nowhere', which is not bound")), "{:?}", found);
    }

    #[test]
    fn a_decision_on_a_value_that_can_fail_is_found() {
        let mut p = lowered("x = 1\n");
        p.parts[0].main.push(Stmt::If { cond: Expr::Const(Const::Bool(true), Ty::Result(Box::new(Ty::Bool))), then: vec![], els: None });
        let found = verify(&p);
        assert!(found.iter().any(|f| f.contains("an if decides on a value of type Result(Bool)")), "{:?}", found);
    }

    #[test]
    fn an_int_stored_where_a_float_is_held_is_found() {
        let mut p = lowered("y = 2.5\nlog[y]\n");
        p.parts[0].main.push(Stmt::Set { var: "y".into(), expr: Expr::Const(Const::Int(1), Ty::Int) });
        let found = verify(&p);
        assert!(found.iter().any(|f| f.contains("'y' holds Float but is assigned Int")), "{:?}", found);
    }

    #[test]
    fn a_call_with_the_wrong_arguments_is_found() {
        let mut p = lowered("fn half[x]\n    return x / 2.0\nlog[half[3.0]]\n");
        p.parts[0].main.push(Stmt::Do(Expr::Call { target: Target::User("half".into()), args: vec![], ty: Ty::Float, propagates: true }));
        let found = verify(&p);
        assert!(found.iter().any(|f| f.contains("calls 'half' with 0 arguments; it takes 1")), "{:?}", found);
    }
}
