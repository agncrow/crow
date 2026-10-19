//! Crow names that Rust reserves, or that the compiled program itself
//! declares where the program's code runs (a view's session, `state`). A
//! Crow program may call a variable `move` or a function `impl`, or a
//! local `state`; Rust may not, or not beside the session. The compiled program renames
//! each such name the program declares — every declaration and every read,
//! inside interpolation spans too — to one Rust accepts (`move` → `move_`,
//! longer when that is taken). It runs after the gate, so errors speak in
//! the program's own names, and the Rust backend prints the program's name
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

/// The names the compiled program itself declares where the program's code
/// runs: a view's or a handler's session is `state`.
pub const GENERATED_NAMES: &[&str] = &["state"];

/// The program with every declared name Rust reserves renamed, and the map
/// from each new name back to the program's; `None` when it declares none.
pub fn rename_rust_keywords(stmts: &[Stmt]) -> Option<(Vec<Stmt>, BTreeMap<String, String>)> {
    let mut declared = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for s in stmts {
        collect_stmt(s, &mut declared, &mut seen);
    }
    let reserved: Vec<String> = declared.into_iter().filter(|n| RUST_KEYWORDS.contains(&n.as_str()) || GENERATED_NAMES.contains(&n.as_str())).collect();
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
    // a name is renamed where it is visible: the top level's own names
    // everywhere (a handler reads its facts), a body's own names in that
    // body — a local one body declares renames nothing in another (the
    // ClickHouse driver's local `where`, inlined into a program that uses
    // db, had renamed the program's `where[…]`, a builtin, to `where_`:
    // undefined, in the compiled program alone; fuzz seed 2083)
    let mut global = BTreeSet::new();
    for s in stmts {
        top_level_decls(s, &mut global);
    }
    let mut out = stmts.to_vec();
    for s in out.iter_mut() {
        rename_stmt(s, &renames, &global, &BTreeSet::new());
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

/// The names the top level declares, which every body sees (a fact a
/// handler writes): not those inside a function's, a derivation's or a
/// test's body, which are that body's own.
fn top_level_decls(s: &Stmt, out: &mut BTreeSet<String>) {
    match s {
        Stmt::Assign { name, value } => {
            out.insert(name.clone());
            for_vars_in(value, out);
        }
        Stmt::MultiAssign { names, values } => {
            out.extend(names.iter().cloned());
            for v in values {
                for_vars_in(v, out);
            }
        }
        Stmt::FnDecl { name, .. } | Stmt::Derive { name, .. } => {
            out.insert(name.clone());
        }
        Stmt::For { var, iter, body } => {
            out.insert(var.clone());
            for_vars_in(iter, out);
            for st in body {
                top_level_decls(st, out);
            }
        }
        Stmt::While { condition, body } => {
            for_vars_in(condition, out);
            for st in body {
                top_level_decls(st, out);
            }
        }
        Stmt::If { condition, then_body, else_body } => {
            for_vars_in(condition, out);
            for st in then_body.iter().chain(else_body.iter().flatten()) {
                top_level_decls(st, out);
            }
        }
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => for_vars_in(e, out),
        _ => {}
    }
}

/// The variables a collecting loop binds, inside an expression.
fn for_vars_in(e: &Expr, out: &mut BTreeSet<String>) {
    let mut seen = BTreeSet::new();
    collect_expr(e, out, &mut seen);
}

/// A body's own names: its parameters and what it declares.
fn body_decls(params: &[String], body: &[Stmt]) -> BTreeSet<String> {
    let mut inner: BTreeSet<String> = params.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    for st in body {
        collect_stmt(st, &mut inner, &mut seen);
    }
    inner
}

fn visible(global: &BTreeSet<String>, locals: &BTreeSet<String>) -> BTreeSet<String> {
    global.union(locals).cloned().collect()
}

fn rename(n: &mut String, renames: &BTreeMap<String, String>, visible: &BTreeSet<String>) {
    if visible.contains(n.as_str()) {
        if let Some(to) = renames.get(n.as_str()) {
            *n = to.clone();
        }
    }
}

fn rename_stmt(s: &mut Stmt, renames: &BTreeMap<String, String>, global: &BTreeSet<String>, locals: &BTreeSet<String>) {
    let v = visible(global, locals);
    match s {
        Stmt::Assign { name, value } => {
            rename(name, renames, &v);
            rename_expr(value, renames, &v);
        }
        Stmt::MultiAssign { names, values } => {
            for n in names.iter_mut() {
                rename(n, renames, &v);
            }
            for x in values.iter_mut() {
                rename_expr(x, renames, &v);
            }
        }
        Stmt::FnDecl { name, params, body, .. } => {
            rename(name, renames, &v);
            let inner = body_decls(params, body);
            let vi = visible(global, &inner);
            for p in params.iter_mut() {
                rename(p, renames, &vi);
            }
            for st in body.iter_mut() {
                rename_stmt(st, renames, global, &inner);
            }
        }
        Stmt::Derive { name, body, .. } => {
            rename(name, renames, &v);
            let inner = body_decls(&[], body);
            for st in body.iter_mut() {
                rename_stmt(st, renames, global, &inner);
            }
        }
        Stmt::TestBlock { params, body, .. } => {
            let inner = body_decls(params, body);
            let vi = visible(global, &inner);
            for p in params.iter_mut() {
                rename(p, renames, &vi);
            }
            for st in body.iter_mut() {
                rename_stmt(st, renames, global, &inner);
            }
        }
        Stmt::For { var, iter, body } => {
            rename(var, renames, &v);
            rename_expr(iter, renames, &v);
            for st in body.iter_mut() {
                rename_stmt(st, renames, global, locals);
            }
        }
        Stmt::While { condition, body } => {
            rename_expr(condition, renames, &v);
            for st in body.iter_mut() {
                rename_stmt(st, renames, global, locals);
            }
        }
        Stmt::If { condition, then_body, else_body } => {
            rename_expr(condition, renames, &v);
            for st in then_body.iter_mut() {
                rename_stmt(st, renames, global, locals);
            }
            if let Some(eb) = else_body {
                for st in eb.iter_mut() {
                    rename_stmt(st, renames, global, locals);
                }
            }
        }
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => rename_expr(e, renames, &v),
        Stmt::FieldAssign { object, value, .. } => {
            rename_expr(object, renames, &v);
            rename_expr(value, renames, &v);
        }
        Stmt::IndexAssign { object, index, value } => {
            rename_expr(object, renames, &v);
            rename_expr(index, renames, &v);
            rename_expr(value, renames, &v);
        }
        _ => {}
    }
}

pub(crate) fn rename_expr(e: &mut Expr, renames: &BTreeMap<String, String>, visible: &BTreeSet<String>) {
    match e {
        Expr::Ident(n) => rename(n, renames, visible),
        Expr::Str(s) => {
            if s.contains('`') {
                *s = rename_in_spans(s, renames, visible);
            }
        }
        Expr::BinOp { left, right, .. } => {
            rename_expr(left, renames, visible);
            rename_expr(right, renames, visible);
        }
        Expr::UnaryOp { expr, .. } => rename_expr(expr, renames, visible),
        Expr::Call { callee, args } => {
            rename_expr(callee, renames, visible);
            for a in args.iter_mut() {
                rename_expr(a, renames, visible);
            }
        }
        Expr::FieldAccess { object, .. } => rename_expr(object, renames, visible),
        Expr::Array(items) => {
            for i in items.iter_mut() {
                rename_expr(i, renames, visible);
            }
        }
        Expr::Map(pairs) => {
            for (_, v) in pairs.iter_mut() {
                rename_expr(v, renames, visible);
            }
        }
        Expr::Named { value, .. } => rename_expr(value, renames, visible),
        Expr::ForExpr { var, iter, body } => {
            rename(var, renames, visible);
            rename_expr(iter, renames, visible);
            for st in body.iter_mut() {
                // (the loop's body sees what the expression's scope sees)
                rename_stmt(st, renames, visible, &BTreeSet::new());
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

fn rename_in_spans(s: &str, renames: &BTreeMap<String, String>, visible: &BTreeSet<String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (a, b) in span_names(&chars) {
        let word: String = chars[a..b].iter().collect();
        if let Some(to) = renames.get(&word).filter(|_| visible.contains(&word)) {
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
        assert!(text.contains("Named { name: \"move\", int_key: false, value: Ident(\"loop_\") }"), "a map key keeps its spelling: {}", text);
        assert_eq!(shown.get("impl_").map(String::as_str), Some("impl"));
        assert_eq!(shown.get("move_").map(String::as_str), Some("move"));
    }

    /// A reserved name one body declares renames nothing in another: the
    /// ClickHouse driver's local `where`, inlined into a program that uses
    /// db, had renamed the program's `where[…]` — a builtin call in another
    /// function — to `where_`, undefined in the compiled program alone
    /// (fuzz seed 2083). The top level's names are seen everywhere.
    #[test]
    fn a_reserved_local_of_one_body_renames_nothing_in_another() {
        let src = "fn shown[s]\n    where = s\n    return 'at `where`'\n\nfn pick[xs]\n    return where[xs gt 1.0, xs, 0.0]\n\ntype = 3\nfn twice[]\n    return type * 2\n\nlog[shown['x']]\nlog[pick[[0.5, 1.5]]]\nlog[twice[]]\n";
        let (out, shown) = rename_rust_keywords(&stmts(src)).expect("renames");
        let text = format!("{:?}", out);
        assert!(text.contains("Assign { name: \"where_\""), "the driver's local is renamed where it lives: {}", text);
        assert!(text.contains("`where_`"), "and in its own body's span: {}", text);
        assert!(text.contains("callee: Ident(\"where\")"), "the builtin call in another body keeps its name: {}", text);
        assert!(text.contains("Ident(\"type_\")") && text.contains("Assign { name: \"type_\""), "a top-level name is renamed everywhere: {}", text);
        assert_eq!(shown.get("where_").map(String::as_str), Some("where"));
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

    /// A view's local named `state` would stand beside the compiled view's
    /// own session: it is renamed as a reserved name is (the build failed
    /// on `state.view_exit` of the program's string)
    #[test]
    fn a_local_named_like_the_session_is_renamed() {
        let src = "use web, ui\nn = 0\nview v[]\n    state = span['idle']\n    return stack[state, p['`n`']]\n";
        let (out, shown) = rename_rust_keywords(&stmts(src)).expect("renames");
        let text = format!("{:?}", out);
        assert!(text.contains("Assign { name: \"state_\""), "{}", text);
        assert!(!text.contains("Ident(\"state\")"), "{}", text);
        assert_eq!(shown.get("state_").map(String::as_str), Some("state"));
    }
}
