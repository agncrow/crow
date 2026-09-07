//! Derivations: the static read sets both execution modes obey.
//!
//! A `derive` block is recomputed when something it read changed. What it
//! reads is decided here, once, from the source — facts and other
//! derivations by name (transitively through the functions it calls), the
//! clock (`now[]`), and the tables its query literals name — so the
//! interpreter and the compiled program recompute at exactly the same
//! moments (derive-design.md §3.2).
use crate::ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct ReadSet {
    pub facts: BTreeSet<String>,
    pub derives: BTreeSet<String>,
    /// reads `now[]` (directly or through a callee): fresh every tick
    pub now: bool,
    /// tables its queries read
    pub tables: BTreeSet<String>,
    /// reads a table this program never writes: a source outside the app,
    /// governed by the freshness window
    pub external: bool,
}

/// The read set of a body, given the program's facts, derivations, the
/// bodies of its functions, and the tables it writes anywhere.
pub fn read_set(
    body: &[Stmt],
    facts: &BTreeSet<String>,
    derives: &BTreeSet<String>,
    fns: &BTreeMap<String, Vec<Stmt>>,
    owned_tables: &BTreeSet<String>,
) -> ReadSet {
    let mut rs = ReadSet::default();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    walk_stmts(body, facts, derives, fns, &mut visited, &mut rs);
    rs.external = rs.tables.iter().any(|t| !owned_tables.contains(t));
    rs
}

fn walk_stmts(stmts: &[Stmt], facts: &BTreeSet<String>, derives: &BTreeSet<String>, fns: &BTreeMap<String, Vec<Stmt>>, visited: &mut BTreeSet<String>, rs: &mut ReadSet) {
    for st in stmts {
        match st {
            Stmt::Assign { value, .. } => walk_expr(value, facts, derives, fns, visited, rs),
            Stmt::MultiAssign { values, .. } => for v in values { walk_expr(v, facts, derives, fns, visited, rs) },
            Stmt::FieldAssign { object, value, .. } => { walk_expr(object, facts, derives, fns, visited, rs); walk_expr(value, facts, derives, fns, visited, rs); }
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk_expr(e, facts, derives, fns, visited, rs),
            Stmt::If { condition, then_body, else_body } => {
                walk_expr(condition, facts, derives, fns, visited, rs);
                walk_stmts(then_body, facts, derives, fns, visited, rs);
                if let Some(eb) = else_body { walk_stmts(eb, facts, derives, fns, visited, rs); }
            }
            Stmt::For { iter, body, .. } => { walk_expr(iter, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
            Stmt::While { condition, body } => { walk_expr(condition, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
            _ => {}
        }
    }
}

/// The expressions inside a string's backtick spans — a derivation read
/// in an interpolation is a read like any other.
pub fn interpolation_exprs(s: &str) -> Vec<Expr> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' { i += 2; continue; }
        if chars[i] == '`' {
            let mut j = i + 1;
            let mut span = String::new();
            while j < chars.len() && chars[j] != '`' { span.push(chars[j]); j += 1; }
            if j >= chars.len() { break; }
            i = j + 1;
            if span.trim().is_empty() { continue; }
            if let Ok(tokens) = crate::lexer::Lexer::new(&span).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::new(tokens).parse() {
                    let stmts = crate::lower::lower_program(&stmts).unwrap_or(stmts);
                    for st in stmts {
                        if let Stmt::ExprStmt(e) = st { out.push(e); }
                    }
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

fn walk_expr(e: &Expr, facts: &BTreeSet<String>, derives: &BTreeSet<String>, fns: &BTreeMap<String, Vec<Stmt>>, visited: &mut BTreeSet<String>, rs: &mut ReadSet) {
    match e {
        Expr::Str(s) if s.contains('`') => {
            for inner in interpolation_exprs(s) { walk_expr(&inner, facts, derives, fns, visited, rs); }
        }
        Expr::Ident(n) => {
            if derives.contains(n) { rs.derives.insert(n.clone()); }
            else if facts.contains(n) { rs.facts.insert(n.clone()); }
        }
        Expr::Call { callee, args } => {
            if let Expr::Ident(n) = callee.as_ref() {
                if n == "now" { rs.now = true; }
                // query[conn, 'sql', ...]: the tables the literal reads
                if n == "query" {
                    if let Some(Expr::Str(sql)) = args.get(1) {
                        for t in tables_of_sql(sql).0 { rs.tables.insert(t); }
                    }
                }
                if let Some(body) = fns.get(n) {
                    if visited.insert(n.clone()) {
                        walk_stmts(body, facts, derives, fns, visited, rs);
                    }
                }
            } else {
                walk_expr(callee, facts, derives, fns, visited, rs);
            }
            for a in args { walk_expr(a, facts, derives, fns, visited, rs); }
        }
        Expr::FieldAccess { object, .. } => walk_expr(object, facts, derives, fns, visited, rs),
        Expr::BinOp { left, right, .. } => { walk_expr(left, facts, derives, fns, visited, rs); walk_expr(right, facts, derives, fns, visited, rs); }
        Expr::UnaryOp { expr, .. } => walk_expr(expr, facts, derives, fns, visited, rs),
        Expr::Array(items) => for i in items { walk_expr(i, facts, derives, fns, visited, rs) },
        Expr::Map(pairs) => for (k, v) in pairs { walk_expr(k, facts, derives, fns, visited, rs); walk_expr(v, facts, derives, fns, visited, rs); },
        Expr::Named { value, .. } => walk_expr(value, facts, derives, fns, visited, rs),
        Expr::ForExpr { iter, body, .. } => { walk_expr(iter, facts, derives, fns, visited, rs); walk_stmts(body, facts, derives, fns, visited, rs); }
        _ => {}
    }
}

/// The tables a SQL literal reads and writes: (reads, writes). A regex
/// over FROM / JOIN / INSERT INTO / UPDATE / DELETE FROM — every dialect
/// Crow talks to agrees on those words.
pub fn tables_of_sql(sql: &str) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let words: Vec<&str> = sql.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == ';').filter(|w| !w.is_empty()).collect();
    let clean = |w: &str| w.trim_matches(|c: char| c == '`' || c == '"' || c == '\'').to_string();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].to_ascii_uppercase();
        match w.as_str() {
            "FROM" | "JOIN" => {
                if let Some(t) = words.get(i + 1) {
                    let t = clean(t);
                    if !t.is_empty() && !t.starts_with('?') && !t.starts_with('\'') && t.chars().next().map_or(false, |c| c.is_alphabetic()) {
                        // DELETE FROM t writes; a FROM after DELETE is the target
                        if i > 0 && words[i - 1].eq_ignore_ascii_case("DELETE") { writes.push(t); } else { reads.push(t); }
                    }
                }
            }
            "INTO" => {
                if i > 0 && words[i - 1].eq_ignore_ascii_case("INSERT") {
                    if let Some(t) = words.get(i + 1) { let t = clean(t); if !t.is_empty() { writes.push(t); } }
                }
            }
            "UPDATE" => {
                if let Some(t) = words.get(i + 1) { let t = clean(t); if !t.is_empty() { writes.push(t); } }
            }
            _ => {}
        }
        i += 1;
    }
    reads.sort(); reads.dedup(); writes.sort(); writes.dedup();
    (reads, writes)
}

/// The functions a body calls, transitively (names in `fns` only).
pub fn called_fns(body: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>) -> BTreeSet<String> {
    fn walk(stmts: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>, out: &mut BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } | Stmt::ExprStmt(value) | Stmt::Return(Some(value)) => walk_e(value, fns, out),
                Stmt::MultiAssign { values, .. } => for v in values { walk_e(v, fns, out) },
                Stmt::FieldAssign { object, value, .. } => { walk_e(object, fns, out); walk_e(value, fns, out); }
                Stmt::If { condition, then_body, else_body } => { walk_e(condition, fns, out); walk(then_body, fns, out); if let Some(eb) = else_body { walk(eb, fns, out); } }
                Stmt::For { iter, body, .. } => { walk_e(iter, fns, out); walk(body, fns, out); }
                Stmt::While { condition, body } => { walk_e(condition, fns, out); walk(body, fns, out); }
                _ => {}
            }
        }
    }
    fn walk_e(e: &Expr, fns: &BTreeMap<String, Vec<Stmt>>, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() {
                    if fns.contains_key(n) && out.insert(n.clone()) {
                        walk(&fns[n], fns, out);
                    }
                }
                walk_e(callee, fns, out);
                for a in args { walk_e(a, fns, out); }
            }
            Expr::Ident(n) => { if fns.contains_key(n) && out.insert(n.clone()) { walk(&fns[n], fns, out); } }
            Expr::Str(s) if s.contains('`') => { for inner in interpolation_exprs(s) { walk_e(&inner, fns, out); } }
            Expr::FieldAccess { object, .. } => walk_e(object, fns, out),
            Expr::BinOp { left, right, .. } => { walk_e(left, fns, out); walk_e(right, fns, out); }
            Expr::UnaryOp { expr, .. } => walk_e(expr, fns, out),
            Expr::Array(items) => for i in items { walk_e(i, fns, out) },
            Expr::Map(pairs) => for (k, v) in pairs { walk_e(k, fns, out); walk_e(v, fns, out); },
            Expr::Named { value, .. } => walk_e(value, fns, out),
            Expr::ForExpr { iter, body, .. } => { walk_e(iter, fns, out); walk(body, fns, out); }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(body, fns, &mut out);
    out
}

/// Every table the program writes (exec / insert literals anywhere).
pub fn owned_tables(stmts: &[Stmt], fns: &BTreeMap<String, Vec<Stmt>>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(stmts: &[Stmt], out: &mut BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } | Stmt::ExprStmt(value) | Stmt::Return(Some(value)) => walk_e(value, out),
                Stmt::MultiAssign { values, .. } => for v in values { walk_e(v, out) },
                Stmt::If { condition, then_body, else_body } => { walk_e(condition, out); walk(then_body, out); if let Some(eb) = else_body { walk(eb, out); } }
                Stmt::For { iter, body, .. } => { walk_e(iter, out); walk(body, out); }
                Stmt::While { condition, body } => { walk_e(condition, out); walk(body, out); }
                Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    fn walk_e(e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() {
                    if n == "exec" || n == "insert" {
                        if let Some(Expr::Str(sql)) = args.get(1) { for t in tables_of_sql(sql).1 { out.insert(t); } }
                        if n == "insert" { if let Some(Expr::Str(t)) = args.get(1) { out.insert(t.clone()); } }
                    }
                }
                walk_e(callee, out);
                for a in args { walk_e(a, out); }
            }
            Expr::FieldAccess { object, .. } => walk_e(object, out),
            Expr::BinOp { left, right, .. } => { walk_e(left, out); walk_e(right, out); }
            Expr::UnaryOp { expr, .. } => walk_e(expr, out),
            Expr::Array(items) => for i in items { walk_e(i, out) },
            Expr::Map(pairs) => for (k, v) in pairs { walk_e(k, out); walk_e(v, out); },
            Expr::Named { value, .. } => walk_e(value, out),
            Expr::ForExpr { iter, body, .. } => { walk_e(iter, out); walk(body, out); }
            _ => {}
        }
    }
    walk(stmts, &mut out);
    for body in fns.values() { walk(body, &mut out); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_tables_read_and_written() {
        let (r, w) = tables_of_sql("SELECT a FROM trades t JOIN sessions s ON s.id = t.session_id WHERE x = ?");
        assert_eq!(r, vec!["sessions", "trades"]);
        assert!(w.is_empty());
        let (_, w) = tables_of_sql("INSERT INTO trades (a) VALUES (?)");
        assert_eq!(w, vec!["trades"]);
        let (_, w) = tables_of_sql("UPDATE trades SET exit_ts = ? WHERE id = ?");
        assert_eq!(w, vec!["trades"]);
        let (_, w) = tables_of_sql("DELETE FROM bookmarks WHERE id = ?");
        assert_eq!(w, vec!["bookmarks"]);
    }
}
