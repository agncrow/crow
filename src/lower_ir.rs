//! Lowering: the parser's tree to the IR (docs/ir-design.md §4).
//!
//! The three front-end passes have already run: named arguments are
//! positional (`lower.rs`), types are inferred (`type_infer.rs`) and the
//! program is valid (`validator.rs`). This pass writes their decisions
//! into the IR and adds the ones the IR makes explicit: which calls are
//! effects and whether their answer is used, where a result is
//! propagated, where an int becomes a float, which read is a variable's
//! last, which loops are parallel, which functions are handlers and
//! what each function can do.

use crate::ast::{BinOp, Expr as A, Stmt as S};
use crate::ir::*;
use crate::type_infer::{CrowType, TypeContext};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Lower a whole (expanded, lowered, inferred) program.
pub fn lower(stmts: &[S], ctx: &mut TypeContext, part_name: &str) -> Result<Program, String> {
    lower_with_known(stmts, ctx, part_name, &HashSet::new())
}

/// Lowering with names already bound by an earlier fragment (the REPL's
/// session): a read of one is a local, an assignment to one is a `set`.
pub fn lower_with_known(stmts: &[S], ctx: &mut TypeContext, part_name: &str, known: &HashSet<String>) -> Result<Program, String> {
    let in_ui = stmts.iter().any(|s| matches!(s, S::Use { path, .. } if path == "ui" || path == "web.ui"));
    let mut l = Lowerer::new(ctx, in_ui);
    l.scopes[0].extend(known.iter().cloned());
    let part = l.lower_part(stmts, part_name)?;
    Ok(Program { parts: vec![part] })
}

pub fn ty_of(t: &CrowType) -> Ty {
    match t {
        CrowType::Int => Ty::Int,
        CrowType::Float => Ty::Float,
        CrowType::Str => Ty::Str,
        CrowType::Bool => Ty::Bool,
        CrowType::Array(inner) => Ty::List(Box::new(ty_of(inner))),
        CrowType::NumArray(_) => Ty::Tensor,
        CrowType::Map(inner) => Ty::Map(Box::new(ty_of(inner))),
        CrowType::Struct(n) | CrowType::OptBoxStruct(n) => Ty::Record(n.clone()),
        CrowType::Frame => Ty::Frame(None),
        CrowType::FrameCol => Ty::Column(Box::new(Ty::Unknown)),
        CrowType::Json => Ty::Json,
        CrowType::Result(inner) => Ty::Result(Box::new(ty_of(inner))),
        CrowType::Fn(ps, r) => Ty::Fn(ps.iter().map(ty_of).collect(), Box::new(ty_of(r))),
        CrowType::DbHandle => Ty::Handle("db".into()),
        CrowType::DbTx => Ty::Handle("tx".into()),
        CrowType::WebApp | CrowType::WebResponse | CrowType::WebRequest | CrowType::WebDb => Ty::Handle("web".into()),
        CrowType::Sparse => Ty::Handle("sparse".into()),
        CrowType::Void => Ty::Unit,
        CrowType::Unknown => Ty::Unknown,
    }
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
        }
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
        };
        let fact_names: BTreeSet<String> = self.ctx.facts.clone();
        let written: BTreeSet<String> = facts_written_by_fns(stmts, &fact_names);
        for s in stmts {
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
                    let sig = self.ctx.structs.get(name).cloned();
                    let fields = match sig {
                        Some(sig) => sig.fields.iter().map(|(n, t)| (n.clone(), ty_of(t))).collect(),
                        None => fields.iter().map(|f| (f.clone(), Ty::Unknown)).collect(),
                    };
                    part.structs.push(StructDef { name: name.clone(), fields });
                }
                S::FnDecl { name, params, body, is_view, .. } => {
                    let f = self.lower_fn(name, params, body, *is_view)?;
                    part.fns.push(f);
                }
                S::Derive { name, fresh, body } => {
                    let d = self.lower_derive(name, *fresh, body)?;
                    part.derives.push(d);
                }
                S::TestBlock { name, params, body } => {
                    let t = self.lower_test(name.clone(), params, body)?;
                    part.tests.push(t);
                }
                S::Assign { name, value } if self.in_ui && fact_names.contains(name) && !self.declared(name) => {
                    self.at = Where::Top;
                    self.current_fn = "<main>".to_string();
                    let default = self.lower_expr(value)?;
                    self.flush_pre_into(&mut part.main);
                    let ty = self.ctx.vars.get(name).map(ty_of).unwrap_or_else(|| default.ty());
                    self.scopes[0].insert(name.clone());
                    part.facts.push(Fact { name: name.clone(), ty, default, session: written.contains(name) });
                }
                other => {
                    self.at = Where::Top;
                    self.current_fn = "<main>".to_string();
                    let lowered = self.lower_stmt(other)?;
                    self.flush_pre_into(&mut part.main);
                    part.main.extend(lowered);
                }
            }
        }
        place_moves_in(&mut part.main, &HashSet::new(), None);
        // what each function reaches, transitively
        let names: Vec<String> = part.fns.iter().map(|f| f.name.clone()).collect();
        for f in &mut part.fns {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo = vec![f.name.clone()];
            let mut effects: BTreeSet<String> = BTreeSet::new();
            while let Some(n) = todo.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(e) = self.direct_effects.get(&n) {
                    effects.extend(e.iter().cloned());
                }
                if let Some(c) = self.calls.get(&n) {
                    for callee in c {
                        if names.contains(callee) {
                            todo.push(callee.clone());
                        }
                    }
                }
            }
            f.effects = effects.into_iter().collect();
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
        let kind = if is_view || self.ctx.views.contains(name) {
            FnKind::View
        } else if self.ctx.view_handlers.contains(name) {
            FnKind::Handler
        } else if assigns_any(body, &self.ctx.facts) {
            FnKind::Helper
        } else {
            FnKind::Pure
        };
        self.ctx.infer_function_body(name, body);
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
        for s in body {
            self.ctx.infer_stmt(s);
        }
        let ret = sig.as_ref().map(|s| ty_of(&s.return_type)).unwrap_or(Ty::Unknown);
        let ret = if kind == FnKind::View { Ty::Node } else { ret };
        self.at = Where::In(kind);
        self.current_fn = name.to_string();
        self.scopes.push(params.iter().cloned().collect());
        let mut block = self.lower_block(body)?;
        self.scopes.pop();
        self.ctx.leave_body(scope);
        let param_set: HashSet<String> = params.iter().cloned().collect();
        place_moves_in(&mut block, &HashSet::new(), Some(&param_set));
        Ok(Fn { name: name.to_string(), params: ps, ret, body: block, kind, effects: vec![] })
    }

    fn lower_derive(&mut self, name: &str, fresh: f64, body: &[S]) -> Result<Derive, String> {
        let saved = self.ctx.vars.clone();
        for s in body {
            self.ctx.infer_stmt(s);
        }
        self.at = Where::Derive;
        self.current_fn = name.to_string();
        self.scopes.push(HashSet::new());
        let mut block = self.lower_block(body)?;
        self.scopes.pop();
        self.ctx.vars = saved;
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
        Ok(Derive { name: name.to_string(), ty, body: block, reads, fresh })
    }

    fn lower_test(&mut self, name: Option<String>, params: &[String], body: &[S]) -> Result<Test, String> {
        let saved = self.ctx.vars.clone();
        for p in params {
            self.ctx.vars.insert(p.clone(), CrowType::Unknown);
        }
        for s in body {
            self.ctx.infer_stmt(s);
        }
        self.at = Where::Test;
        self.current_fn = name.clone().unwrap_or_default();
        self.scopes.push(params.iter().cloned().collect());
        let mut block = self.lower_block(body)?;
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
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn is_fact(&self, name: &str) -> bool {
        self.in_ui && self.ctx.facts.contains(name) && !self.scopes.iter().skip(1).any(|s| s.contains(name))
    }

    fn sees_facts(&self) -> bool {
        matches!(self.at, Where::Top | Where::Derive | Where::Test | Where::In(FnKind::View) | Where::In(FnKind::Handler) | Where::In(FnKind::Helper))
    }

    fn lower_stmt(&mut self, s: &S) -> Result<Vec<Stmt>, String> {
        Ok(match s {
            S::Line(n) => vec![Stmt::Line(*n)],
            S::Assign { name, value } => {
                let expr = self.lower_expr(value)?;
                vec![self.assign(name, expr)]
            }
            S::MultiAssign { names, values } => {
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
                        out.push(self.assign(n, item));
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
                            out.push(self.assign(n, Expr::Local(t, ty)));
                        }
                    }
                }
                out
            }
            S::FieldAssign { object, field, value } => {
                let expr = self.lower_expr(value)?;
                let (var, mut path) = self.path_of(object)?;
                path.push(PathStep::Field(field.clone()));
                vec![Stmt::SetPath { var, path, expr }]
            }
            S::IndexAssign { object, index, value } => {
                let expr = self.lower_expr(value)?;
                let idx = self.lower_expr(index)?;
                let (var, mut path) = self.path_of(object)?;
                path.push(PathStep::Index(idx));
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
                let mut prelude = std::mem::take(&mut self.pre);
                self.scopes.push(HashSet::new());
                let b = self.lower_block(body)?;
                self.scopes.pop();
                prelude.push(Stmt::While { cond, body: b });
                prelude
            }
            S::For { var, iter, body } => {
                let it = self.lower_expr(iter)?;
                let mut prelude = std::mem::take(&mut self.pre);
                let outer: HashSet<String> = self.scopes.iter().flat_map(|s| s.iter().cloned()).collect();
                let parallel = !assigns_any_of(body, &outer) && !reaches_effect(body, self.ctx) && !assigns_any(body, &self.ctx.facts);
                self.scopes.push([var.clone()].into_iter().collect());
                let b = self.lower_block(body)?;
                self.scopes.pop();
                prelude.push(Stmt::For { var: var.clone(), iter: it, body: b, parallel });
                prelude
            }
            S::Return(None) => vec![Stmt::Return(None)],
            S::Return(Some(e)) => {
                let x = self.lower_expr(e)?;
                vec![Stmt::Return(Some(x))]
            }
            S::Break => vec![Stmt::Break],
            S::Continue => vec![Stmt::Continue],
            S::ExprStmt(e) => {
                let mut x = self.lower_expr(e)?;
                if let Expr::Effect { answered, .. } = &mut x {
                    *answered = false;
                }
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

    /// A condition: a fallible one is propagated first (spec §10, Errors
    /// in Conditions), then decided on its payload.
    fn lower_cond(&mut self, c: &A) -> Result<Expr, String> {
        let e = self.lower_expr(c)?;
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
            self.pre.push(Stmt::Propagate { values: vec![t.clone()] });
            return Ok(Expr::Convert {
                how: Convert::Unwrap,
                expr: Box::new(Expr::Local(t, ty.clone())),
                ty: ty.payload().clone(),
            });
        }
        Ok(e)
    }

    fn assign(&mut self, name: &str, expr: Expr) -> Stmt {
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
            return Stmt::Set { var: name.to_string(), expr };
        }
        self.scopes.last_mut().unwrap().insert(name.to_string());
        let ty = match self.ctx.vars.get(name) {
            Some(t) if !matches!(t, CrowType::Unknown) => ty_of(t),
            _ => expr.ty(),
        };
        Stmt::Let { var: name.to_string(), ty, expr }
    }

    fn fresh(&mut self) -> String {
        self.temp += 1;
        format!("t{}", self.temp)
    }

    fn path_of(&mut self, object: &A) -> Result<(String, Vec<PathStep>), String> {
        match object {
            A::Ident(n) => Ok((n.clone(), vec![])),
            A::FieldAccess { object, field } => {
                let (v, mut p) = self.path_of(object)?;
                p.push(PathStep::Field(field.clone()));
                Ok((v, p))
            }
            A::Call { callee, args } => {
                let (v, mut p) = self.path_of(callee)?;
                for a in args {
                    let i = self.lower_expr(a)?;
                    p.push(PathStep::Index(i));
                }
                Ok((v, p))
            }
            _ => Err("lowering: an assignment target must be a variable, a field or an index".into()),
        }
    }

    // ── expressions ─────────────────────────────────────────────────

    fn lower_expr(&mut self, e: &A) -> Result<Expr, String> {
        let cty = self.ctx.infer_expr(e);
        let ty = ty_of(&cty);
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
            A::Ident(name) => self.ident(name, ty),
            A::BinOp { left, op, right } => {
                let mut l = self.lower_expr(left)?;
                let mut r = self.lower_expr(right)?;
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
                Expr::Unary { op: format!("{:?}", op).to_lowercase(), expr: Box::new(x), ty }
            }
            A::Call { callee, args } => self.call(callee, args, ty)?,
            A::FieldAccess { object, field } => {
                match field.as_str() {
                    "ok" | "reason" | "kind" | "trace" => {
                        let base = self.lower_expr(object)?;
                        let which = match field.as_str() {
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
                                return Ok(Expr::Constant(format!("{}.{}", m, field), ty));
                            }
                        }
                        let base = self.lower_expr(object)?;
                        Expr::Field { base: Box::new(base), name: field.clone(), ty }
                    }
                }
            }
            A::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.lower_expr(i)?);
                }
                Expr::ListLit(out, ty)
            }
            A::Map(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let key = match k {
                        A::Ident(n) | A::Str(n) => n.clone(),
                        other => format!("{:?}", other),
                    };
                    out.push((key, self.lower_expr(v)?));
                }
                Expr::MapLit(out, ty)
            }
            A::Named { name, .. } => return Err(format!("lowering: a named argument ('{}') survived the lowering of named arguments", name)),
            A::ForExpr { var, iter, body } => {
                let it = self.lower_expr(iter)?;
                // the statement form's rule decides a collecting loop too
                let outer: HashSet<String> = self.scopes.iter().flat_map(|s| s.iter().cloned()).collect();
                let parallel = !assigns_any_of(body, &outer) && !reaches_effect(body, self.ctx) && !assigns_any(body, &self.ctx.facts);
                self.scopes.push([var.clone()].into_iter().collect());
                let mut b = self.lower_block(body)?;
                self.scopes.pop();
                place_moves_in(&mut b, &HashSet::new(), Some(&[var.clone()].into_iter().collect()));
                Expr::ForExpr { var: var.clone(), iter: Box::new(it), body: b, ty, parallel }
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

    fn call(&mut self, callee: &A, args: &[A], ty: Ty) -> Result<Expr, String> {
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
            let base = self.lower_expr(callee)?;
            let mut idx = Vec::new();
            for a in args {
                idx.push(self.lower_expr(a)?);
            }
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
        // a struct constructor
        if module.is_none() && self.ctx.structs.contains_key(&name) {
            let mut fields = Vec::new();
            for a in args {
                fields.push(self.lower_expr(a)?);
            }
            return Ok(Expr::RecordLit { name, fields });
        }
        // a ui element with its attributes first (lower.rs put them there)
        if module.is_none() && self.in_ui && crate::lower::UI_ELEMENTS.contains(&name.as_str()) {
            let mut attrs = Vec::new();
            let mut children = Vec::new();
            for (i, a) in args.iter().enumerate() {
                if i == 0 {
                    if let A::Map(pairs) = a {
                        for (k, v) in pairs {
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
                        continue;
                    }
                }
                children.push(self.lower_expr(a)?);
            }
            return Ok(Expr::Tree { element: name, attrs, children });
        }
        // arguments: a fallible one reaches the callee as its payload, and
        // an error among them is the call's value (`propagates`); a view
        // is the exception, like the elements it is made of: it renders
        // what it is given (spec §10), so its arguments arrive as they are
        let propagates = !(module.is_none() && (matches!(name.as_str(), "log" | "type" | "error" | "assert") || self.ctx.views.contains(&name)));
        let mut lowered = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            // the harness's click[a, h[x]] holds the handler call for the click
            if module.is_none() && name == "click" && i == 1 {
                if let Some(d) = self.deferred(a)? {
                    lowered.push(d);
                    continue;
                }
            }
            let x = self.lower_expr(a)?;
            if propagates && x.ty().is_result() {
                let rty = x.ty();
                lowered.push(Expr::Convert { how: Convert::Unwrap, expr: Box::new(x), ty: rty.payload().clone() });
            } else {
                lowered.push(x);
            }
        }
        let current = self.current_fn.clone();
        if let Some(m) = module {
            let m = self.ctx.module_aliases.get(&m).cloned().unwrap_or(m);
            if is_effect(&name) {
                // a qualified effect keeps its module: frame.load, db.query
                self.direct_effects.entry(current).or_default().insert(name.clone());
                return Ok(Expr::Effect { name: format!("{}.{}", m, name), args: lowered, ty, answered: true, propagates });
            }
            return Ok(Expr::Call { target: Target::Module(m, name), args: lowered, ty, propagates });
        }
        if self.ctx.functions.contains_key(&name) || self.ctx.specializations.contains_key(&name) {
            let resolved = self.specialised(&name, args);
            self.calls.entry(current).or_default().insert(resolved.clone());
            return Ok(Expr::Call { target: Target::User(resolved), args: lowered, ty, propagates });
        }
        if is_effect(&name) {
            self.direct_effects.entry(current).or_default().insert(name.clone());
            return Ok(Expr::Effect { name, args: lowered, ty, answered: true, propagates });
        }
        if RUNTIME_TABLE.iter().any(|(b, _)| *b == name) {
            return Ok(Expr::Call { target: Target::Runtime(runtime_name(&name)), args: lowered, ty, propagates });
        }
        // a bare name a loaded module provides resolves at run time, by the
        // registry's own rule, as the interpreter resolves it
        Ok(Expr::Call { target: Target::Runtime(name), args: lowered, ty, propagates })
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
                    return Ok(Some(Expr::Deferred { name: f.clone(), args: lowered }));
                }
            }
        }
        Ok(None)
    }

    /// The specialised name of a user function called at these argument
    /// types, when it has one.
    fn specialised(&self, name: &str, args: &[A]) -> String {
        let specs = match self.ctx.specializations.get(name) {
            Some(s) => s,
            None => return name.to_string(),
        };
        let arg_types: Vec<CrowType> = args.iter().map(|a| self.ctx.infer_expr(a)).collect();
        for (params, mangled) in specs {
            if params.len() == arg_types.len() && params.iter().zip(&arg_types).all(|(p, a)| p.compatible(a)) {
                return mangled.clone();
            }
        }
        specs.first().map(|(_, m)| m.clone()).unwrap_or_else(|| name.to_string())
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
                let stmts = crate::lower::lower_program(&stmts).unwrap_or(stmts);
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

// ── analyses over the parser's tree ─────────────────────────────────

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

fn assigns_any_of(body: &[S], names: &HashSet<String>) -> bool {
    let set: BTreeSet<String> = names.iter().cloned().collect();
    assigns_any(body, &set)
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

fn reaches_effect(body: &[S], ctx: &TypeContext) -> bool {
    fn in_expr(e: &A, ctx: &TypeContext) -> bool {
        match e {
            A::Call { callee, args } => {
                let here = match callee.as_ref() {
                    A::Ident(n) => is_effect(n) || ctx.functions.contains_key(n),
                    A::FieldAccess { field, .. } => is_effect(field),
                    _ => false,
                };
                here || args.iter().any(|a| in_expr(a, ctx))
            }
            A::BinOp { left, right, .. } => in_expr(left, ctx) || in_expr(right, ctx),
            A::UnaryOp { expr, .. } => in_expr(expr, ctx),
            A::FieldAccess { object, .. } => in_expr(object, ctx),
            A::Array(items) => items.iter().any(|i| in_expr(i, ctx)),
            A::Map(pairs) => pairs.iter().any(|(k, v)| in_expr(k, ctx) || in_expr(v, ctx)),
            A::Named { value, .. } => in_expr(value, ctx),
            A::ForExpr { iter, body, .. } => in_expr(iter, ctx) || reaches_effect(body, ctx),
            // an interpolation reaches what its spans reach
            A::Str(s) => crate::parallel::str_interpolation_spans(s).iter().any(|e| in_expr(e, ctx)),
            _ => false,
        }
    }
    body.iter().any(|s| match s {
        S::Assign { value, .. } => in_expr(value, ctx),
        S::MultiAssign { values, .. } => values.iter().any(|v| in_expr(v, ctx)),
        S::FieldAssign { value, .. } => in_expr(value, ctx),
        S::IndexAssign { index, value, .. } => in_expr(index, ctx) || in_expr(value, ctx),
        S::ExprStmt(e) | S::Return(Some(e)) => in_expr(e, ctx),
        S::If { condition, then_body, else_body } => in_expr(condition, ctx) || reaches_effect(then_body, ctx) || else_body.as_ref().map_or(false, |b| reaches_effect(b, ctx)),
        S::For { iter, body, .. } => in_expr(iter, ctx) || reaches_effect(body, ctx),
        S::While { condition, body } => in_expr(condition, ctx) || reaches_effect(body, ctx),
        _ => false,
    })
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
        Stmt::Propagate { values } => out.extend(values.iter().cloned()),
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
        assert!(ir.contains("let n: Int = len(unwrap move r)"), "{}", ir);
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
}
