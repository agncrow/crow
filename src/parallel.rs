/// Dependency analysis for automatic parallelism.
/// Groups independent statements into waves that can run in parallel.

use std::collections::HashSet;
use crate::ast::*;

/// Extract variable names read by string interpolation expressions like "hello {name}"
pub fn str_interpolation_reads(s: &str) -> HashSet<String> {
    let mut reads = HashSet::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let start = i + 1;
            let mut depth = 1;
            i += 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '{' { depth += 1; }
                else if chars[i] == '}' { depth -= 1; }
                if depth > 0 { i += 1; }
            }
            let expr_str: String = chars[start..i].iter().collect();
            i += 1;
            // parse the interpolation expression and extract reads
            if let Ok(tokens) = crate::lexer::Lexer::new(&expr_str).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::new(tokens).parse() {
                    if let Some(Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, Stmt::Line(_))) {
                        reads.extend(expr_reads(expr));
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    reads
}

/// Extract all variable names read by an expression
pub fn expr_reads(expr: &Expr) -> HashSet<String> {
    let mut reads = HashSet::new();
    match expr {
        Expr::Ident(name) => { reads.insert(name.clone()); }
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
        Expr::List(items) => {
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

/// Extract variables written by a statement
pub fn stmt_writes(stmt: &Stmt) -> HashSet<String> {
    let mut writes = HashSet::new();
    match stmt {
        Stmt::Assign { name, .. } => { writes.insert(name.clone()); }
        Stmt::MultiAssign { names, .. } => {
            for n in names { writes.insert(n.clone()); }
        }
        Stmt::FnDecl { name, .. } => { writes.insert(name.clone()); }
        Stmt::StructDecl { name, .. } => { writes.insert(name.clone()); }
        _ => {}
    }
    writes
}

/// Check if a statement is a "barrier" that must execute alone and sequentially.
/// Only simple assignments (Assign, MultiAssign) can be parallelized.
pub fn is_barrier(stmt: &Stmt) -> bool {
    matches!(stmt,
        Stmt::If { .. } | Stmt::For { .. } | Stmt::While { .. } |
        Stmt::Match { .. } | Stmt::Return(_) | Stmt::Use { .. } |
        Stmt::FnDecl { .. } | Stmt::StructDecl { .. } | Stmt::Resource { .. } |
        Stmt::FieldAssign { .. } | Stmt::IndexAssign { .. } |
        Stmt::ExprStmt(_)
    )
}

/// Group statements into waves of independent work.
/// Each wave contains statement indices that can safely run in parallel.
pub fn group_into_waves(stmts: &[Stmt]) -> Vec<Vec<usize>> {
    let mut waves: Vec<Vec<usize>> = vec![];
    let mut current_wave: Vec<usize> = vec![];
    let mut wave_reads: HashSet<String> = HashSet::new();
    let mut wave_writes: HashSet<String> = HashSet::new();

    for (i, stmt) in stmts.iter().enumerate() {
        if matches!(stmt, Stmt::Line(_)) {
            continue;
        }
        if is_barrier(stmt) {
            if !current_wave.is_empty() {
                waves.push(current_wave);
                current_wave = vec![];
            }
            waves.push(vec![i]);
            wave_reads = HashSet::new();
            wave_writes = HashSet::new();
            continue;
        }

        let reads = stmt_reads(stmt);
        let writes = stmt_writes(stmt);

        let raw = !reads.is_disjoint(&wave_writes);
        let waw = !writes.is_disjoint(&wave_writes);
        let war = !writes.is_disjoint(&wave_reads);

        if raw || waw || war {
            if !current_wave.is_empty() {
                waves.push(current_wave);
            }
            current_wave = vec![i];
            wave_reads = reads;
            wave_writes = writes;
        } else {
            current_wave.push(i);
            wave_reads.extend(reads);
            wave_writes.extend(writes);
        }
    }

    if !current_wave.is_empty() {
        waves.push(current_wave);
    }

    waves
}

