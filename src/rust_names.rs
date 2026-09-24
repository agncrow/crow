//! Crow names that Rust reserves. A Crow program may call a variable
//! `move` or a function `impl`; Rust may not. The compiled program renames
//! each such name the program declares — every declaration and every read,
//! inside interpolation spans too — to one Rust accepts (`move` → `move_`,
//! longer when that is taken). It runs after the gate, so errors speak in
//! the program's own names, and the transpiler prints the program's name
//! wherever a name reaches the reader (a trace, a handler's stop). Field
//! names, map keys and named-argument names are never renamed: they are
//! columns, keys and attributes, not Rust identifiers. The interpreter
//! never sees the renamed program.

use crate::ast::{Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

/// Rust's strict and reserved keywords, less the ones that are Crow
/// keywords too (`fn`, `for`, `if`, … never name anything in Crow).
pub const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "const", "crate", "dyn", "enum", "extern", "impl", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "self", "Self", "static", "super", "trait",
    "type", "unsafe", "where", "abstract", "become", "box", "do", "final", "gen", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

/// The program with every declared name Rust reserves renamed, and the map
/// from each new name back to the program's; `None` when it declares none.
pub fn rename_rust_keywords(stmts: &[Stmt]) -> Option<(Vec<Stmt>, BTreeMap<String, String>)> {
    let mut declared = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for s in stmts {
        collect_stmt(s, &mut declared, &mut seen);
    }
    let reserved: Vec<String> = declared.into_iter().filter(|n| RUST_KEYWORDS.contains(&n.as_str())).collect();
    if reserved.is_empty() {
        return None;
    }
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    for name in reserved {
        let mut fresh = format!("{}_", name);
        while seen.contains(&fresh) || renames.values().any(|v| v == &fresh) {
            fresh.push('_');
        }
        renames.insert(name, fresh);
    }
    let mut out = stmts.to_vec();
    for s in out.iter_mut() {
        rename_stmt(s, &renames);
    }
    let shown = renames.into_iter().map(|(from, to)| (to, from)).collect();
    Some((out, shown))
}

fn decl(n: &str, declared: &mut BTreeSet<String>, seen: &mut BTreeSet<String>) {
    declared.insert(n.to_string());
    seen.insert(n.to_string());
}

fn collect_stmt(s: &Stmt, declared: &mut BTreeSet<String>, seen: &mut BTreeSet<String>) {
    match s {
        Stmt::Assign { name, value } => {
            decl(name, declared, seen);
            collect_expr(value, declared, seen);
        }
        Stmt::MultiAssign { names, values } => {
            for n in names {
                decl(n, declared, seen);
            }
            for v in values {
                collect_expr(v, declared, seen);
            }
        }
        Stmt::FnDecl { name, params, body, .. } => {
            decl(name, declared, seen);
            for p in params {
                decl(p, declared, seen);
            }
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::Derive { name, body, .. } => {
            decl(name, declared, seen);
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::TestBlock { params, body, .. } => {
            for p in params {
                decl(p, declared, seen);
            }
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::For { var, iter, body } => {
            decl(var, declared, seen);
            collect_expr(iter, declared, seen);
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::While { condition, body } => {
            collect_expr(condition, declared, seen);
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::If { condition, then_body, else_body } => {
            collect_expr(condition, declared, seen);
            for st in then_body.iter().chain(else_body.iter().flatten()) {
                collect_stmt(st, declared, seen);
            }
        }
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => collect_expr(e, declared, seen),
        Stmt::FieldAssign { object, value, .. } => {
            collect_expr(object, declared, seen);
            collect_expr(value, declared, seen);
        }
        Stmt::IndexAssign { object, index, value } => {
            collect_expr(object, declared, seen);
            collect_expr(index, declared, seen);
            collect_expr(value, declared, seen);
        }
        Stmt::Resource { count, .. } => collect_expr(count, declared, seen),
        Stmt::StructDecl { name, .. } => {
            seen.insert(name.clone());
        }
        _ => {}
    }
}

fn collect_expr(e: &Expr, declared: &mut BTreeSet<String>, seen: &mut BTreeSet<String>) {
    match e {
        Expr::Ident(n) => {
            seen.insert(n.clone());
        }
        Expr::Str(s) => {
            for w in span_words(s) {
                seen.insert(w);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_expr(left, declared, seen);
            collect_expr(right, declared, seen);
        }
        Expr::UnaryOp { expr, .. } => collect_expr(expr, declared, seen),
        Expr::Call { callee, args } => {
            collect_expr(callee, declared, seen);
            for a in args {
                collect_expr(a, declared, seen);
            }
        }
        Expr::FieldAccess { object, .. } => collect_expr(object, declared, seen),
        Expr::Array(items) => {
            for i in items {
                collect_expr(i, declared, seen);
            }
        }
        Expr::Map(pairs) => {
            for (_, v) in pairs {
                collect_expr(v, declared, seen);
            }
        }
        Expr::Named { value, .. } => collect_expr(value, declared, seen),
        Expr::ForExpr { var, iter, body } => {
            declared.insert(var.clone());
            seen.insert(var.clone());
            collect_expr(iter, declared, seen);
            for st in body {
                collect_stmt(st, declared, seen);
            }
        }
        _ => {}
    }
}

fn rename(n: &mut String, renames: &BTreeMap<String, String>) {
    if let Some(to) = renames.get(n.as_str()) {
        *n = to.clone();
    }
}

fn rename_stmt(s: &mut Stmt, renames: &BTreeMap<String, String>) {
    match s {
        Stmt::Assign { name, value } => {
            rename(name, renames);
            rename_expr(value, renames);
        }
        Stmt::MultiAssign { names, values } => {
            for n in names.iter_mut() {
                rename(n, renames);
            }
            for v in values.iter_mut() {
                rename_expr(v, renames);
            }
        }
        Stmt::FnDecl { name, params, body, .. } => {
            rename(name, renames);
            for p in params.iter_mut() {
                rename(p, renames);
            }
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        Stmt::Derive { name, body, .. } => {
            rename(name, renames);
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        Stmt::TestBlock { params, body, .. } => {
            for p in params.iter_mut() {
                rename(p, renames);
            }
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        Stmt::For { var, iter, body } => {
            rename(var, renames);
            rename_expr(iter, renames);
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        Stmt::While { condition, body } => {
            rename_expr(condition, renames);
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        Stmt::If { condition, then_body, else_body } => {
            rename_expr(condition, renames);
            for st in then_body.iter_mut() {
                rename_stmt(st, renames);
            }
            if let Some(eb) = else_body {
                for st in eb.iter_mut() {
                    rename_stmt(st, renames);
                }
            }
        }
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => rename_expr(e, renames),
        Stmt::FieldAssign { object, value, .. } => {
            rename_expr(object, renames);
            rename_expr(value, renames);
        }
        Stmt::IndexAssign { object, index, value } => {
            rename_expr(object, renames);
            rename_expr(index, renames);
            rename_expr(value, renames);
        }
        Stmt::Resource { count, .. } => rename_expr(count, renames),
        _ => {}
    }
}

fn rename_expr(e: &mut Expr, renames: &BTreeMap<String, String>) {
    match e {
        Expr::Ident(n) => rename(n, renames),
        Expr::Str(s) => {
            if s.contains('`') {
                *s = rename_in_spans(s, renames);
            }
        }
        Expr::BinOp { left, right, .. } => {
            rename_expr(left, renames);
            rename_expr(right, renames);
        }
        Expr::UnaryOp { expr, .. } => rename_expr(expr, renames),
        Expr::Call { callee, args } => {
            rename_expr(callee, renames);
            for a in args.iter_mut() {
                rename_expr(a, renames);
            }
        }
        Expr::FieldAccess { object, .. } => rename_expr(object, renames),
        Expr::Array(items) => {
            for i in items.iter_mut() {
                rename_expr(i, renames);
            }
        }
        Expr::Map(pairs) => {
            for (_, v) in pairs.iter_mut() {
                rename_expr(v, renames);
            }
        }
        Expr::Named { value, .. } => rename_expr(value, renames),
        Expr::ForExpr { var, iter, body } => {
            rename(var, renames);
            rename_expr(iter, renames);
            for st in body.iter_mut() {
                rename_stmt(st, renames);
            }
        }
        _ => {}
    }
}

/// The places a name starts inside a string's interpolation spans: a word
/// not preceded by `.` (a field) and not inside a quoted text within the
/// span. Yields (start, end) char offsets.
fn span_names(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_span = false;
    let mut in_text = false;
    while i < chars.len() {
        let c = chars[i];
        if !in_span {
            if c == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                i += 2;
                continue;
            }
            if c == '`' {
                in_span = true;
            }
            i += 1;
            continue;
        }
        if in_text {
            if c == '\\' && i + 1 < chars.len() {
                i += 2;
                continue;
            }
            if c == '\'' {
                in_text = false;
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            in_text = true;
            i += 1;
            continue;
        }
        if c == '`' {
            in_span = false;
            i += 1;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let after_dot = start > 0 && chars[start - 1] == '.';
            let after_word = start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
            if !after_dot && !after_word {
                out.push((start, i));
            }
            continue;
        }
        i += 1;
    }
    out
}

fn span_words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    span_names(&chars).into_iter().map(|(a, b)| chars[a..b].iter().collect()).collect()
}

fn rename_in_spans(s: &str, renames: &BTreeMap<String, String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (a, b) in span_names(&chars) {
        let word: String = chars[a..b].iter().collect();
        if let Some(to) = renames.get(&word) {
            out.extend(chars[last..a].iter());
            out.push_str(to);
            last = b;
        }
    }
    out.extend(chars[last..].iter());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmts(src: &str) -> Vec<Stmt> {
        crate::build(src).expect("parses")
    }

    /// A declared name Rust reserves is renamed everywhere it is read —
    /// inside interpolation spans too — and nothing else is: fields, map
    /// keys, named arguments and the words of a text keep their spelling
    #[test]
    fn a_reserved_name_is_renamed_and_nothing_else() {
        let src = "fn impl[move]\n    ref = move + 1\n    return ref\n\nloop = impl[2]\nm = map[move = loop]\nlog['`loop` and `m.move` and \\'move\\' `len[\\'loop\\']`']\n";
        let (out, shown) = rename_rust_keywords(&stmts(src)).expect("renames");
        let text = format!("{:?}", out);
        assert!(text.contains("FnDecl { name: \"impl_\", params: [\"move_\"]"), "{}", text);
        assert!(text.contains("Assign { name: \"ref_\""), "{}", text);
        assert!(text.contains("Ident(\"move_\")"), "{}", text);
        assert!(text.contains("Assign { name: \"loop_\""), "{}", text);
        assert!(text.contains("`loop_` and `m.move`"), "a span's read is renamed, its field is not: {}", text);
        assert!(text.contains("and 'move' `len['loop']`"), "the text outside a name keeps its words: {}", text);
        assert!(text.contains("Named { name: \"move\", value: Ident(\"loop_\") }"), "a map key keeps its spelling: {}", text);
        assert_eq!(shown.get("impl_").map(String::as_str), Some("impl"));
        assert_eq!(shown.get("move_").map(String::as_str), Some("move"));
    }

    /// The new name never takes one the program already uses; a program
    /// with no reserved names is left alone
    #[test]
    fn a_new_name_is_one_the_program_does_not_use() {
        let (out, shown) = rename_rust_keywords(&stmts("move_ = 1\nmove = move_ + 1\nlog[move]\n")).expect("renames");
        assert_eq!(shown.get("move__").map(String::as_str), Some("move"));
        assert!(format!("{:?}", out).contains("Assign { name: \"move__\""));
        assert!(rename_rust_keywords(&stmts("x = 1\nlog[x]\n")).is_none());
    }
}
