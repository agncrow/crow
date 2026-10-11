//! A function's text built by appends, kept as the text itself in the
//! compiled program.
//!
//! Text is built in Crow by appending pieces to a list and joining it:
//! `row = push[row, syms[i]]`, then `join[row, '']`. As a list, each piece
//! is a string of its own: fasta's output had been a one-character string
//! per character — made, copied again by the join, and freed — and ran five
//! times slower than Rust. A local list of text that the body only empties,
//! appends to, joins with one separator and measures needs none of that: it
//! is kept as the joined text and a count of its pieces, an append writing
//! the separator (after the first piece) and the piece where the text
//! lies, and a join reading the text.

use super::*;
use crate::ir::{Expr, Fn, Piece, Stmt, Target};
use std::collections::{BTreeMap, HashMap};

/// The locals of a function kept as text, each with its separator: a list
/// of text bound only to `[]`, given only `[]` or `push[x, piece]`, and read
/// only by `join[x, sep]` with one constant separator and by `len[x]`.
pub(super) fn text_builders(f: &Fn) -> BTreeMap<String, String> {
    let mut w = Walk { seps: HashMap::new(), out: HashMap::new() };
    for (p, _) in &f.params {
        w.out.insert(p.clone(), true);
    }
    w.block(&f.body);
    w.seps
        .into_iter()
        .filter(|(v, _)| !w.out.get(v).copied().unwrap_or(false))
        .map(|(v, sep)| (v, sep.unwrap_or_default()))
        .collect()
}

struct Walk {
    /// Each candidate, with the separator its joins use (none yet).
    seps: HashMap<String, Option<String>>,
    /// The names ruled out: a parameter, or a candidate used otherwise.
    out: HashMap<String, bool>,
}

/// The local an expression names, through an unwrap of a checked value.
fn named(e: &Expr) -> Option<&str> {
    match unwrapped(e) {
        Expr::Local(v, _) | Expr::Move(v, _) => Some(v.as_str()),
        _ => None,
    }
}

fn unwrapped(e: &Expr) -> &Expr {
    match e {
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => expr,
        other => other,
    }
}

fn is_text_list(t: &Ty) -> bool {
    matches!(as_inferred(t), Ty::List(item) if *item == Ty::Str)
}

fn is_empty_list(e: &Expr) -> bool {
    matches!(unwrapped(e), Expr::ListLit(items, _) if items.is_empty())
}

/// `push[x, piece]` of the local `x`: the piece.
pub(super) fn appended<'e>(x: &str, e: &'e Expr) -> Option<&'e Expr> {
    match unwrapped(e) {
        Expr::Call { target: Target::Row(row), args, .. } if row.id == "core.push" && args.len() == 2 && named(&args[0]) == Some(x) && matches!(args[1].ty().payload(), Ty::Str) => Some(&args[1]),
        _ => None,
    }
}

/// `join[x, 'sep']`: the local and the separator.
pub(super) fn joined(e: &Expr) -> Option<(&str, &str)> {
    match unwrapped(e) {
        Expr::Call { target: Target::Row(row), args, .. } if row.id == "core.join" && args.len() == 2 => match (named(&args[0]), unwrapped(&args[1])) {
            (Some(x), Expr::Const(crate::ir::Const::Str(sep), _)) => Some((x, sep.as_str())),
            _ => None,
        },
        _ => None,
    }
}

/// `len[x]`: the local.
pub(super) fn measured(e: &Expr) -> Option<&str> {
    match unwrapped(e) {
        Expr::Call { target: Target::Row(row), args, .. } if row.id == "core.len" && args.len() == 1 => named(&args[0]),
        _ => None,
    }
}

/// Whether an expression names the local anywhere.
fn mentions(e: &Expr, x: &str) -> bool {
    let mut found = false;
    crate::ir::visit_expr(e, &mut |n| {
        if matches!(n, Expr::Local(v, _) | Expr::Move(v, _) if v == x) {
            found = true;
        }
    });
    found
}

impl Walk {
    fn rule_out(&mut self, v: &str) {
        self.out.insert(v.to_string(), true);
    }

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { var, ty, expr } => {
                if is_text_list(ty) && is_empty_list(expr) {
                    self.seps.entry(var.clone()).or_insert(None);
                } else {
                    self.rule_out(var);
                    self.expr(expr);
                }
            }
            Stmt::Set { var, expr } => {
                if is_empty_list(expr) {
                    return;
                }
                if let Some(piece) = appended(var, expr) {
                    if mentions(piece, var) {
                        self.rule_out(var);
                    }
                    self.expr(piece);
                    return;
                }
                self.rule_out(var);
                self.expr(expr);
            }
            Stmt::SetPath { var, path, expr } => {
                self.rule_out(var);
                for p in path {
                    if let PathStep::Index(i) = p {
                        self.expr(i);
                    }
                }
                self.expr(expr);
            }
            Stmt::Store { expr, .. } | Stmt::Do(expr) => self.expr(expr),
            Stmt::Return(Some(e)) => self.expr(e),
            Stmt::If { cond, then, els } => {
                self.expr(cond);
                self.block(then);
                if let Some(e) = els {
                    self.block(e);
                }
            }
            Stmt::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::For { var, iter, body, .. } => {
                self.rule_out(var);
                self.expr(iter);
                self.block(body);
            }
            Stmt::Propagate { values, .. } => {
                for v in values {
                    self.rule_out(v);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr) {
        if let Some((x, sep)) = joined(e) {
            match self.seps.get_mut(x) {
                Some(slot @ None) => *slot = Some(sep.to_string()),
                Some(Some(s)) if s.as_str() == sep => {}
                Some(Some(_)) => self.rule_out(x),
                None => {}
            }
            return;
        }
        if measured(e).is_some() {
            return;
        }
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => {
                if self.seps.contains_key(v.as_str()) {
                    self.rule_out(&v.clone());
                }
            }
            Expr::Call { args, .. } | Expr::Effect { args, .. } | Expr::Deferred { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Convert { expr, .. } | Expr::Unary { expr, .. } => self.expr(expr),
            Expr::Binary { l, r, .. } => {
                self.expr(l);
                self.expr(r);
            }
            Expr::Index { base, indices, .. } => {
                self.expr(base);
                for i in indices {
                    self.expr(i);
                }
            }
            Expr::Field { base, .. } | Expr::Meta { base, .. } => self.expr(base),
            Expr::Interp(pieces) => {
                for p in pieces {
                    if let Piece::Expr(x) = p {
                        self.expr(x);
                    }
                }
            }
            Expr::ListLit(items, _) => {
                for x in items {
                    self.expr(x);
                }
            }
            Expr::MapLit(pairs, _) => {
                for (_, x) in pairs {
                    self.expr(x);
                }
            }
            Expr::RecordLit { fields, .. } => {
                for x in fields {
                    self.expr(x);
                }
            }
            Expr::Tree { attrs, children, .. } => {
                for (_, x) in attrs {
                    self.expr(x);
                }
                for x in children {
                    self.expr(x);
                }
            }
            Expr::ForExpr { var, iter, body, .. } => {
                self.rule_out(var);
                self.expr(iter);
                self.block(body);
            }
            Expr::Const(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => {}
        }
    }
}

impl Emitter {
    /// `x = push[x, piece]` of a text builder: the separator after the
    /// first piece, then the piece, where the text lies.
    pub(super) fn emit_append(&mut self, x: &str, sep: &str, piece: &Expr) {
        let p = self.borrowed_text(piece);
        let sep_part = if sep.is_empty() { String::new() } else { format!("if {x}__count > 0 {{ {x}.push_str({sep:?}); }} ") };
        self.line(&format!("{{ {sep_part}{x}.push_str({p}); {x}__count += 1; }}"));
    }

    /// A text read where it lies, for an append to copy: a builder's text,
    /// an item of a list of text, a local; anything else, made and lent.
    fn borrowed_text(&mut self, e: &Expr) -> String {
        if let Some((x, _)) = joined(e).filter(|(x, sep)| self.text_builders.get(*x).is_some_and(|s| s == sep)) {
            return format!("{x}.as_str()");
        }
        match unwrapped(e) {
            Expr::Index { base, indices, .. } if indices.len() == 1 && matches!(base.ty(), Ty::List(item) if *item == Ty::Str) && matches!(unwrapped(base), Expr::Local(..) | Expr::Move(..)) => {
                let b = self.expr(base);
                let i = self.expr(&indices[0]);
                let at = self.index_at("_i", "_v.len()");
                format!("{{ let _i = {i}; let _v = &{b}; _v[{at}].as_str() }}")
            }
            // (a local holding a value that can fail, unwrapped in the IR
            // once checked, keeps the text in its payload: a tool's schema
            // written by a function that reads a parsed value had been read
            // as the result itself, and the program did not build)
            Expr::Local(v, lt) | Expr::Move(v, lt) if matches!(e.ty().payload(), Ty::Str) => {
                if lt.is_result() {
                    format!("{v}.value.as_str()")
                } else {
                    format!("{v}.as_str()")
                }
            }
            Expr::Const(crate::ir::Const::Str(text), _) => format!("{text:?}"),
            _ => {
                let owned = self.owned_expr(e);
                format!("&({owned})")
            }
        }
    }

    /// A builder's join: its text (taken, where the IR moves it).
    pub(super) fn builder_text(&self, e: &Expr) -> Option<String> {
        let (x, sep) = joined(e)?;
        if self.text_builders.get(x).map(|s| s.as_str()) != Some(sep) {
            return None;
        }
        let Expr::Call { args, .. } = unwrapped(e) else { return None };
        Some(match unwrapped(&args[0]) {
            Expr::Move(..) => format!("std::mem::take(&mut {x})"),
            _ => format!("{x}.clone()"),
        })
    }
}
