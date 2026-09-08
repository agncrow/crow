//! What an expression reads: the variable names in it, including the
//! ones inside interpolated strings. The checker and the transpiler use
//! it. This module once grouped statements into parallel waves; the
//! spec's Design Note "Statements Run in Order" records why that stopped.

use std::collections::HashSet;
use crate::ast::*;

/// Extract variable names read by string interpolation expressions like "hello {name}"
/// The expressions inside a string's interpolation spans, each parsed as
/// the interpreter parses it: `'item`i` of `n`'` holds `i` and `n`.
pub fn str_interpolation_spans(s: &str) -> Vec<Expr> {
    let mut spans = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Escaped backtick: literal, not interpolation
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            // Interpolation span: scan to closing backtick, quote-aware
            let start = i + 1;
            i += 1;
            let mut in_str = false;
            while i < chars.len() {
                let c = chars[i];
                if in_str {
                    if c == '\\' && i + 1 < chars.len() { i += 2; continue; }
                    if c == '\'' { in_str = false; }
                } else if c == '\'' {
                    in_str = true;
                } else if c == '`' {
                    break;
                }
                i += 1;
            }
            let expr_str: String = chars[start..i.min(chars.len())].iter().collect();
            i += 1;
            if let Ok(tokens) = crate::lexer::Lexer::new(&expr_str).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::new(tokens).parse() {
                    if let Some(Stmt::ExprStmt(expr)) = stmts.into_iter().find(|s| !matches!(s, Stmt::Line(_))) {
                        spans.push(expr);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    spans
}

pub fn str_interpolation_reads(s: &str) -> HashSet<String> {
    let mut reads = HashSet::new();
    for expr in str_interpolation_spans(s) {
        reads.extend(expr_reads(&expr));
    }
    reads
}

/// Extract all variable names read by an expression
pub fn expr_reads(expr: &Expr) -> HashSet<String> {
    let mut reads = HashSet::new();
    match expr {
        Expr::Ident(name) => { reads.insert(name.clone()); }
        Expr::Named { value, .. } => { reads.extend(expr_reads(value)); }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) => {}
        Expr::Str(s) => { reads.extend(str_interpolation_reads(s)); }
        Expr::BinOp { left, right, .. } => {
            reads.extend(expr_reads(left));
            reads.extend(expr_reads(right));
        }
        Expr::UnaryOp { expr: e, .. } => { reads.extend(expr_reads(e)); }
        Expr::Call { callee, args } => {
            reads.extend(expr_reads(callee));
            for a in args { reads.extend(expr_reads(a)); }
        }
        Expr::FieldAccess { object, .. } => { reads.extend(expr_reads(object)); }
        Expr::Array(items) => {
            for item in items { reads.extend(expr_reads(item)); }
        }
        Expr::Map(entries) => {
            for (k, v) in entries {
                reads.extend(expr_reads(k));
                reads.extend(expr_reads(v));
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            reads.extend(expr_reads(iter));
            for s in body { reads.extend(stmt_reads(s)); }
        }
    }
    reads
}

/// Extract variables read by a statement
pub fn stmt_reads(stmt: &Stmt) -> HashSet<String> {
    match stmt {
        Stmt::Assign { value, .. } => expr_reads(value),
        Stmt::MultiAssign { values, .. } => {
            let mut reads = HashSet::new();
            for v in values { reads.extend(expr_reads(v)); }
            reads
        }
        Stmt::ExprStmt(expr) => expr_reads(expr),
        Stmt::Return(Some(expr)) => expr_reads(expr),
        _ => HashSet::new(),
    }
}
