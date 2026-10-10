//! A function's text read by position, kept with what is known of its
//! start (crow_runtime::text::Known).
//!
//! A slice, a position or the length of a text is counted in characters,
//! and the text is UTF-8: reading at character `i` had walked the text
//! from its start, so a loop over a long text's windows was quadratic. A
//! local or a parameter of text that the body slices, indexes or measures
//! keeps a `Known` beside it — made where the local is bound, reset where
//! it is given a new text — and every read extends what is known of its
//! ASCII start only as far as it reads: within it, a character is a byte.

use super::*;
use crate::ir::{Expr, Stmt, Target};
use std::collections::{BTreeSet, HashMap};

/// The texts of a body read by position: parameters and locals of text
/// (every binding of the name a text) that a slice, a position or a length
/// reads.
pub(super) fn indexed_texts(params: &[(String, Ty)], body: &[Stmt]) -> BTreeSet<String> {
    let mut text: HashMap<String, bool> = HashMap::new();
    for (p, t) in params {
        text.insert(p.clone(), matches!(as_inferred(t), Ty::Str));
    }
    bindings(body, &mut text);
    let mut read = BTreeSet::new();
    crate::ir::visit_block(body, &mut |e| {
        if let Some(x) = read_by_position(e) {
            read.insert(x.to_string());
        }
    });
    read.into_iter().filter(|x| text.get(x).copied().unwrap_or(false)).collect()
}

/// Every name a body binds, kept while each binding is a text (a loop's
/// variable is a new text every round, and is left alone).
fn bindings(b: &[Stmt], text: &mut HashMap<String, bool>) {
    for s in b {
        match s {
            Stmt::Let { var, ty, .. } => {
                let t = matches!(as_inferred(ty), Ty::Str);
                *text.entry(var.clone()).or_insert(t) &= t;
            }
            Stmt::For { var, body, .. } => {
                text.insert(var.clone(), false);
                bindings(body, text);
            }
            Stmt::If { then, els, .. } => {
                bindings(then, text);
                if let Some(e) = els {
                    bindings(e, text);
                }
            }
            Stmt::While { body, .. } => bindings(body, text),
            _ => {}
        }
    }
    crate::ir::visit_block(b, &mut |e| {
        if let Expr::ForExpr { var, .. } = e {
            text.insert(var.clone(), false);
        }
    });
}

/// The local a read by position reads: a slice's or a length's text, an
/// indexed text.
fn read_by_position(e: &Expr) -> Option<&str> {
    match e {
        Expr::Call { target: Target::Row(row), args, .. } if matches!(row.id, "core.slice_str" | "core.len_text") && !args.is_empty() => named(&args[0]),
        Expr::Index { base, indices, .. } if indices.len() == 1 && matches!(base.ty().payload(), Ty::Str) => named(base),
        _ => None,
    }
}

fn named(e: &Expr) -> Option<&str> {
    match e {
        Expr::Local(v, _) | Expr::Move(v, _) => Some(v.as_str()),
        Expr::Convert { how: crate::ir::Convert::Unwrap | crate::ir::Convert::Known, expr, .. } => named(expr),
        _ => None,
    }
}

impl Emitter {
    /// The text read by position an expression names, if it names one.
    pub(super) fn indexed_text<'e>(&self, e: &'e Expr) -> Option<&'e str> {
        named(e).filter(|x| self.indexed_texts.contains(*x))
    }

    /// `slice[x, a, b] eq other` of texts: compared where the slice lies,
    /// made only past what is known of the text's start (a slice compared
    /// with a pattern at every window had made a string per window).
    pub(super) fn slice_compared(&mut self, slice: &Expr, other: &Expr) -> Option<String> {
        let Expr::Call { target: Target::Row(row), args, ty, .. } = slice else { return None };
        if row.id != "core.slice_str" || args.len() != 3 || !matches!(ty, Ty::Str) || !matches!(other.ty(), Ty::Str) {
            return None;
        }
        self.needs_crow_runtime = true;
        let known = match self.indexed_text(&args[0]) {
            Some(x) => format!("&{x}__known"),
            None => "&crow_runtime::text::Known::new()".to_string(),
        };
        let base = match self.indexed_text(&args[0]) {
            Some(x) => x.to_string(),
            None => format!("({})", self.expr(&args[0])),
        };
        let a = self.expr(&args[1]);
        let b = self.expr(&args[2]);
        let o = self.expr(other);
        Some(format!("crow_runtime::text::slice_is(&{base}, {known}, ({a}) as i64, ({b}) as i64, &{o})"))
    }

    /// A character of a text compared (`s[i] eq 'A'`), where it lies: no
    /// text is made of it, nor of a literal it meets (two had been made for
    /// every character a loop compared). None for another comparison.
    pub(super) fn char_compared(&mut self, index: &Expr, other: &Expr) -> Option<String> {
        let Expr::Index { base, indices, ty } = index else { return None };
        if indices.len() != 1 || !matches!(ty, Ty::Str) || !matches!(base.ty(), Ty::Str) || !matches!(other.ty(), Ty::Str) {
            return None;
        }
        self.needs_crow_runtime = true;
        let line = self.line_now();
        let (b, known) = match self.indexed_text(base) {
            Some(x) => (x.to_string(), format!("&{x}__known")),
            None => (format!("({})", self.expr(base)), "&crow_runtime::text::Known::new()".to_string()),
        };
        let i = self.expr(&indices[0]);
        let o = match other {
            Expr::Const(crate::ir::Const::Str(s), _) => format!("{:?}", s),
            _ => format!("&*({})", self.expr(other)),
        };
        Some(format!("crow_runtime::text::char_is(&{b}, {known}, ({i}) as i64, {o}).unwrap_or_else(|e| crow_stop({line}, e))"))
    }

    /// A slice or a length of a text read by position, with what is known
    /// of its start.
    pub(super) fn text_read(&mut self, e: &Expr) -> Option<String> {
        let Expr::Call { target: Target::Row(row), args, .. } = e else { return None };
        let x = self.indexed_text(args.first()?)?.to_string();
        self.needs_crow_runtime = true;
        match (row.id, args.len()) {
            ("core.slice_str", 3) => {
                let a = self.expr(&args[1]);
                let b = self.expr(&args[2]);
                Some(format!("crow_runtime::text::slice_known(&{x}, &{x}__known, ({a}) as i64, ({b}) as i64)"))
            }
            ("core.len_text", 1) => Some(format!("crow_runtime::text::len_known(&{x}, &{x}__known)")),
            _ => None,
        }
    }
}
