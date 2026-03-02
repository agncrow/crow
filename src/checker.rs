/// Automatic verification for Crow programs.
/// Implements `crow check` — purity detection, static checks,
/// edge case generation, and regression snapshots.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::ast::*;
use crate::interpreter::Interpreter;
use crate::type_infer::{CrowType, FnSig, TypeContext};
use crate::value::{CrowValue, Value};

// --- Impure builtins ---
const IMPURE_BUILTINS: &[&str] = &[
    "read", "write", "log", "input", "rand", "time", "sleep", "env", "exit",
    "args",
];

/// All builtin function names (reused from validator for undefined-variable detection).
const BUILTINS: &[&str] = &[
    "log", "assert", "type",
    "abs", "sqrt", "floor", "ceil", "round", "pow", "min", "max",
    "split", "trim", "replace", "upper", "lower", "join", "starts", "ends",
    "repeat", "pad",
    "int", "float", "str",
    "len", "contains", "slice", "push", "pop", "sort", "reverse", "append",
    "range", "map",
    "keys", "values", "has", "set", "del",
    "args", "input",
    "read", "write",
    "rand", "time", "sleep", "env", "exit",
    "json", "encode",
    "error",
    "cpu", "gpu",
];

// --- Issue types ---

#[derive(Debug, Clone)]
pub enum Issue {
    UnusedVariable { func: String, var: String, line: usize },
    UnreachableCode { func: String, line: usize, detail: String },
    UndefinedVariable { func: String, var: String, line: usize },
    EdgeCaseCrash { func: String, input_desc: String, error: String },
    RegressionChange { func: String, input_desc: String, was: String, now: String },
}

impl Issue {
    pub fn summary(&self) -> String {
        match self {
            Issue::UnusedVariable { func, var, .. } =>
                format!("{} has unused variable '{}'", func, var),
            Issue::UnreachableCode { func, detail, .. } =>
                format!("{} has unreachable code — {}", func, detail),
            Issue::UndefinedVariable { func, var, .. } =>
                format!("{} uses undefined variable '{}'", func, var),
            Issue::EdgeCaseCrash { func, input_desc, error } =>
                format!("{} crashes on {} — {}", func, input_desc, error),
            Issue::RegressionChange { func, input_desc, was, now } =>
                format!("{} behavior changed: {} was {}, now {}", func, input_desc, was, now),
        }
    }

    pub fn line(&self) -> usize {
        match self {
            Issue::UnusedVariable { line, .. } => *line,
            Issue::UnreachableCode { line, .. } => *line,
            Issue::UndefinedVariable { line, .. } => *line,
            Issue::EdgeCaseCrash { .. } => 0,
            Issue::RegressionChange { .. } => 0,
        }
    }

    /// Get the function name this issue belongs to.
    #[allow(dead_code)]
    pub fn func(&self) -> &str {
        match self {
            Issue::UnusedVariable { func, .. } => func,
            Issue::UnreachableCode { func, .. } => func,
            Issue::UndefinedVariable { func, .. } => func,
            Issue::EdgeCaseCrash { func, .. } => func,
            Issue::RegressionChange { func, .. } => func,
        }
    }

    /// Get a "try" command the user can paste, if applicable.
    pub fn try_input(&self) -> Option<String> {
        match self {
            Issue::EdgeCaseCrash { input_desc, .. } => Some(input_desc.clone()),
            Issue::RegressionChange { input_desc, .. } => Some(input_desc.clone()),
            _ => None,
        }
    }

    /// Suggest a fix for this issue.
    pub fn suggest(&self) -> String {
        match self {
            Issue::UnusedVariable { var, line, .. } =>
                if *line == 0 {
                    format!("remove parameter '{}'", var)
                } else {
                    format!("remove '{}' or use _ for destructuring", var)
                },
            Issue::UnreachableCode { .. } =>
                "remove the dead code or move it before the return".to_string(),
            Issue::UndefinedVariable { var, .. } =>
                format!("'{}' is not defined — check for typos or add an assignment", var),
            Issue::EdgeCaseCrash { error, .. } => {
                if error.contains("overflow") {
                    "add a bounds check or use saturating arithmetic".to_string()
                } else if error.contains("out of bounds") || error.contains("index") {
                    "check length before indexing, e.g. if len[lst] > 0".to_string()
                } else if error.contains("divide by zero") || error.contains("division by zero") {
                    "check for zero before dividing, e.g. if b != 0".to_string()
                } else {
                    "add a guard to handle this edge case".to_string()
                }
            }
            Issue::RegressionChange { was, now, .. } =>
                format!("was {} — verify the new result {} is correct, then press 'a' to accept", was, now),
        }
    }
}

// --- Purity analysis ---

/// Returns the set of function names that are pure.
pub fn find_pure_functions(ctx: &TypeContext) -> BTreeSet<String> {
    let mut impure: BTreeSet<String> = BTreeSet::new();
    let mut call_graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, sig) in &ctx.functions {
        let mut callees = BTreeSet::new();
        collect_callees_from_stmts(&sig.body, &mut callees);
        call_graph.insert(name.clone(), callees);
    }
    for (name, callees) in &call_graph {
        for c in callees {
            if IMPURE_BUILTINS.contains(&c.as_str()) {
                impure.insert(name.clone());
                break;
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (name, callees) in &call_graph {
            if impure.contains(name) { continue; }
            for c in callees {
                if impure.contains(c) {
                    impure.insert(name.clone());
                    changed = true;
                    break;
                }
            }
        }
    }
    let all: BTreeSet<String> = ctx.functions.keys().cloned().collect();
    all.difference(&impure).cloned().collect()
}

/// Detect functions that are recursive (directly or indirectly via the call graph).
pub fn find_recursive_functions(ctx: &TypeContext) -> BTreeSet<String> {
    let mut call_graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, sig) in &ctx.functions {
        let mut callees = BTreeSet::new();
        collect_callees_from_stmts(&sig.body, &mut callees);
        call_graph.insert(name.clone(), callees);
    }
    let mut recursive = BTreeSet::new();
    for name in ctx.functions.keys() {
        if is_reachable(name, name, &call_graph, &mut BTreeSet::new()) {
            recursive.insert(name.clone());
        }
    }
    recursive
}

/// Check if `target` is reachable from `current` in the call graph.
fn is_reachable(
    target: &str,
    current: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if !visited.insert(current.to_string()) { return false; }
    if let Some(callees) = graph.get(current) {
        for callee in callees {
            if callee == target { return true; }
            if is_reachable(target, callee, graph, visited) { return true; }
        }
    }
    false
}

fn collect_callees_from_stmts(stmts: &[Stmt], callees: &mut BTreeSet<String>) {
    for stmt in stmts { collect_callees_from_stmt(stmt, callees); }
}

fn collect_callees_from_stmt(stmt: &Stmt, callees: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Assign { value, .. } => collect_callees_from_expr(value, callees),
        Stmt::MultiAssign { values, .. } => {
            for v in values { collect_callees_from_expr(v, callees); }
        }
        Stmt::FnDecl { body, .. } => collect_callees_from_stmts(body, callees),
        Stmt::Return(Some(e)) => collect_callees_from_expr(e, callees),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::ExprStmt(e) => collect_callees_from_expr(e, callees),
        Stmt::If { condition, then_body, else_body } => {
            collect_callees_from_expr(condition, callees);
            collect_callees_from_stmts(then_body, callees);
            if let Some(eb) = else_body { collect_callees_from_stmts(eb, callees); }
        }
        Stmt::For { iter, body, .. } => {
            collect_callees_from_expr(iter, callees);
            collect_callees_from_stmts(body, callees);
        }
        Stmt::While { condition, body } => {
            collect_callees_from_expr(condition, callees);
            collect_callees_from_stmts(body, callees);
        }
        Stmt::Match { value, arms } => {
            collect_callees_from_expr(value, callees);
            for arm in arms { collect_callees_from_stmts(&arm.body, callees); }
        }
        Stmt::FieldAssign { object, value, .. } => {
            collect_callees_from_expr(object, callees);
            collect_callees_from_expr(value, callees);
        }
        Stmt::IndexAssign { object, index, value } => {
            collect_callees_from_expr(object, callees);
            collect_callees_from_expr(index, callees);
            collect_callees_from_expr(value, callees);
        }
        _ => {}
    }
}

fn collect_callees_from_expr(expr: &Expr, callees: &mut BTreeSet<String>) {
    match expr {
        Expr::Call { callee, args } => {
            if let Expr::Ident(name) = callee.as_ref() {
                callees.insert(name.clone());
            }
            collect_callees_from_expr(callee, callees);
            for a in args { collect_callees_from_expr(a, callees); }
        }
        Expr::BinOp { left, right, .. } => {
            collect_callees_from_expr(left, callees);
            collect_callees_from_expr(right, callees);
        }
        Expr::UnaryOp { expr: e, .. } => collect_callees_from_expr(e, callees),
        Expr::FieldAccess { object, .. } => collect_callees_from_expr(object, callees),
        Expr::Array(items) => { for i in items { collect_callees_from_expr(i, callees); } }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_callees_from_expr(k, callees);
                collect_callees_from_expr(v, callees);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            collect_callees_from_expr(iter, callees);
            collect_callees_from_stmts(body, callees);
        }
        _ => {}
    }
}

// --- Static checks ---

/// Find unused variables within a function body.
fn find_unused_vars_in_fn(name: &str, params: &[String], body: &[Stmt]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut assigned: Vec<(String, usize)> = Vec::new();
    let mut read: BTreeSet<String> = BTreeSet::new();
    collect_reads_from_stmts(body, &mut read);
    for p in params {
        if !read.contains(p) {
            issues.push(Issue::UnusedVariable {
                func: name.to_string(), var: p.clone(), line: 0,
            });
        }
    }
    collect_assigns_from_stmts(body, &mut assigned);
    for (var, line) in &assigned {
        if !read.contains(var) && var != "_" {
            issues.push(Issue::UnusedVariable {
                func: name.to_string(), var: var.clone(), line: *line,
            });
        }
    }
    issues
}

fn collect_reads_from_stmts(stmts: &[Stmt], reads: &mut BTreeSet<String>) {
    for s in stmts { collect_reads_from_stmt(s, reads); }
}

fn collect_reads_from_stmt(stmt: &Stmt, reads: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Assign { value, .. } => collect_reads_from_expr(value, reads),
        Stmt::MultiAssign { values, .. } => {
            for v in values { collect_reads_from_expr(v, reads); }
        }
        Stmt::FnDecl { body, .. } => collect_reads_from_stmts(body, reads),
        Stmt::Return(Some(e)) => collect_reads_from_expr(e, reads),
        Stmt::ExprStmt(e) => collect_reads_from_expr(e, reads),
        Stmt::If { condition, then_body, else_body } => {
            collect_reads_from_expr(condition, reads);
            collect_reads_from_stmts(then_body, reads);
            if let Some(eb) = else_body { collect_reads_from_stmts(eb, reads); }
        }
        Stmt::For { iter, body, .. } => {
            collect_reads_from_expr(iter, reads);
            collect_reads_from_stmts(body, reads);
        }
        Stmt::While { condition, body } => {
            collect_reads_from_expr(condition, reads);
            collect_reads_from_stmts(body, reads);
        }
        Stmt::Match { value, arms } => {
            collect_reads_from_expr(value, reads);
            for arm in arms { collect_reads_from_stmts(&arm.body, reads); }
        }
        Stmt::FieldAssign { object, value, .. } => {
            collect_reads_from_expr(object, reads);
            collect_reads_from_expr(value, reads);
        }
        Stmt::IndexAssign { object, index, value } => {
            collect_reads_from_expr(object, reads);
            collect_reads_from_expr(index, reads);
            collect_reads_from_expr(value, reads);
        }
        _ => {}
    }
}

fn collect_reads_from_expr(expr: &Expr, reads: &mut BTreeSet<String>) {
    match expr {
        Expr::Ident(name) => { reads.insert(name.clone()); }
        Expr::Str(s) => {
            // Extract reads from string interpolation: '{x.ok}' reads x
            for name in crate::parallel::str_interpolation_reads(s) {
                reads.insert(name);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_reads_from_expr(left, reads);
            collect_reads_from_expr(right, reads);
        }
        Expr::UnaryOp { expr: e, .. } => collect_reads_from_expr(e, reads),
        Expr::Call { callee, args } => {
            collect_reads_from_expr(callee, reads);
            for a in args { collect_reads_from_expr(a, reads); }
        }
        Expr::FieldAccess { object, .. } => collect_reads_from_expr(object, reads),
        Expr::Array(items) => { for i in items { collect_reads_from_expr(i, reads); } }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_reads_from_expr(k, reads);
                collect_reads_from_expr(v, reads);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            collect_reads_from_expr(iter, reads);
            collect_reads_from_stmts(body, reads);
        }
        _ => {}
    }
}

fn collect_assigns_from_stmts(stmts: &[Stmt], assigns: &mut Vec<(String, usize)>) {
    let mut current_line = 0;
    for s in stmts {
        if let Stmt::Line(n) = s { current_line = *n; }
        if let Stmt::Assign { name, .. } = s { assigns.push((name.clone(), current_line)); }
        if let Stmt::MultiAssign { names, .. } = s {
            for n in names { assigns.push((n.clone(), current_line)); }
        }
    }
}

/// Find unreachable code after return statements.
fn find_unreachable_code(name: &str, body: &[Stmt]) -> Vec<Issue> {
    let mut issues = Vec::new();
    check_unreachable(name, body, &mut issues);
    issues
}

fn check_unreachable(name: &str, stmts: &[Stmt], issues: &mut Vec<Issue>) {
    let mut current_line = 0;
    let mut seen_return = false;
    for stmt in stmts {
        if let Stmt::Line(n) = stmt { current_line = *n; continue; }
        if seen_return {
            if matches!(stmt, Stmt::Line(_)) { continue; }
            issues.push(Issue::UnreachableCode {
                func: name.to_string(), line: current_line,
                detail: "code after return can never execute".to_string(),
            });
            return;
        }
        if matches!(stmt, Stmt::Return(_)) { seen_return = true; }
        match stmt {
            Stmt::If { then_body, else_body, .. } => {
                check_unreachable(name, then_body, issues);
                if let Some(eb) = else_body { check_unreachable(name, eb, issues); }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                check_unreachable(name, body, issues);
            }
            _ => {}
        }
    }
}

/// Find undefined variable references in a function body.
/// A variable is undefined if it is read but not a parameter, not assigned before use,
/// not a builtin, and not a user-defined function or struct name.
fn find_undefined_vars_in_fn(
    name: &str, params: &[String], body: &[Stmt], ctx: &TypeContext,
) -> Vec<Issue> {
    let mut defined: BTreeSet<String> = BTreeSet::new();
    // Parameters are defined
    for p in params { defined.insert(p.clone()); }
    // Builtins are defined
    for b in BUILTINS { defined.insert(b.to_string()); }
    // User-defined functions and structs are defined (global scope)
    for fname in ctx.functions.keys() { defined.insert(fname.clone()); }
    for sname in ctx.structs.keys() { defined.insert(sname.clone()); }
    // Boolean literals that parse as identifiers
    defined.insert("true".to_string());
    defined.insert("false".to_string());

    let mut issues = Vec::new();
    collect_undefined_in_stmts(name, body, &defined, &mut issues);
    issues
}

/// Walk statements, tracking local assignments and flagging undefined reads.
fn collect_undefined_in_stmts(
    func: &str, stmts: &[Stmt], outer_defined: &BTreeSet<String>, issues: &mut Vec<Issue>,
) {
    let mut defined = outer_defined.clone();
    let mut current_line = 0usize;
    for stmt in stmts {
        if let Stmt::Line(n) = stmt { current_line = *n; continue; }
        match stmt {
            Stmt::Assign { name, value, .. } => {
                // Check value for undefined reads before adding name to scope
                collect_undefined_in_expr(func, value, &defined, current_line, issues);
                defined.insert(name.clone());
            }
            Stmt::MultiAssign { names, values, .. } => {
                for v in values {
                    collect_undefined_in_expr(func, v, &defined, current_line, issues);
                }
                for n in names { defined.insert(n.clone()); }
            }
            Stmt::ExprStmt(e) => {
                collect_undefined_in_expr(func, e, &defined, current_line, issues);
            }
            Stmt::Return(Some(e)) => {
                collect_undefined_in_expr(func, e, &defined, current_line, issues);
            }
            Stmt::If { condition, then_body, else_body } => {
                collect_undefined_in_expr(func, condition, &defined, current_line, issues);
                collect_undefined_in_stmts(func, then_body, &defined, issues);
                if let Some(eb) = else_body {
                    collect_undefined_in_stmts(func, eb, &defined, issues);
                }
            }
            Stmt::For { var, iter, body, .. } => {
                collect_undefined_in_expr(func, iter, &defined, current_line, issues);
                let mut inner = defined.clone();
                inner.insert(var.clone());
                collect_undefined_in_stmts(func, body, &inner, issues);
            }
            Stmt::While { condition, body } => {
                collect_undefined_in_expr(func, condition, &defined, current_line, issues);
                collect_undefined_in_stmts(func, body, &defined, issues);
            }
            Stmt::IndexAssign { object, index, value } => {
                collect_undefined_in_expr(func, object, &defined, current_line, issues);
                collect_undefined_in_expr(func, index, &defined, current_line, issues);
                collect_undefined_in_expr(func, value, &defined, current_line, issues);
            }
            Stmt::FnDecl { .. } | Stmt::StructDecl { .. } => {
                // Already tracked globally via ctx
            }
            _ => {}
        }
    }
}

/// Check a single expression for undefined variable reads.
fn collect_undefined_in_expr(
    func: &str, expr: &Expr, defined: &BTreeSet<String>, line: usize, issues: &mut Vec<Issue>,
) {
    match expr {
        Expr::Ident(name) => {
            if !defined.contains(name) {
                issues.push(Issue::UndefinedVariable {
                    func: func.to_string(), var: name.clone(), line,
                });
            }
        }
        Expr::Str(s) => {
            for name in crate::parallel::str_interpolation_reads(s) {
                if !defined.contains(&name) {
                    issues.push(Issue::UndefinedVariable {
                        func: func.to_string(), var: name, line,
                    });
                }
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_undefined_in_expr(func, left, defined, line, issues);
            collect_undefined_in_expr(func, right, defined, line, issues);
        }
        Expr::UnaryOp { expr: e, .. } => {
            collect_undefined_in_expr(func, e, defined, line, issues);
        }
        Expr::Call { callee, args } => {
            collect_undefined_in_expr(func, callee, defined, line, issues);
            for a in args { collect_undefined_in_expr(func, a, defined, line, issues); }
        }
        Expr::FieldAccess { object, .. } => {
            collect_undefined_in_expr(func, object, defined, line, issues);
        }
        Expr::Array(items) => {
            for item in items { collect_undefined_in_expr(func, item, defined, line, issues); }
        }
        Expr::Map(entries) => {
            for (k, v) in entries {
                collect_undefined_in_expr(func, k, defined, line, issues);
                collect_undefined_in_expr(func, v, defined, line, issues);
            }
        }
        Expr::ForExpr { var, iter, body } => {
            collect_undefined_in_expr(func, iter, defined, line, issues);
            let mut inner = defined.clone();
            inner.insert(var.clone());
            collect_undefined_in_stmts(func, body, &inner, issues);
        }
        _ => {}
    }
}

/// Run all static checks on functions and top-level code.
pub fn run_static_checks(ctx: &TypeContext, top_level_stmts: &[Stmt]) -> Vec<Issue> {
    let mut issues = Vec::new();
    // Check user-defined functions
    for (name, sig) in &ctx.functions {
        let param_names: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
        issues.extend(find_unused_vars_in_fn(name, &param_names, &sig.body));
        issues.extend(find_unreachable_code(name, &sig.body));
        issues.extend(find_undefined_vars_in_fn(name, &param_names, &sig.body, ctx));
    }
    // Check top-level code
    let top_stmts: Vec<&Stmt> = top_level_stmts.iter()
        .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. }))
        .collect();
    if !top_stmts.is_empty() {
        let top_owned: Vec<Stmt> = top_stmts.into_iter().cloned().collect();
        issues.extend(find_unused_vars_in_fn("<top-level>", &[], &top_owned));
        issues.extend(find_unreachable_code("<top-level>", &top_owned));
        issues.extend(find_undefined_vars_in_fn("<top-level>", &[], &top_owned, ctx));
    }
    issues
}

/// Build a map of function name → declaration line number by scanning for
/// Line markers preceding FnDecl statements.
pub fn find_fn_lines(stmts: &[Stmt]) -> BTreeMap<String, usize> {
    let mut map = BTreeMap::new();
    let mut last_line = 0usize;
    for stmt in stmts {
        match stmt {
            Stmt::Line(n) => { last_line = *n; }
            Stmt::FnDecl { name, .. } => {
                map.insert(name.clone(), last_line);
            }
            _ => {}
        }
    }
    map
}

// --- Edge case generation ---

/// Generate boundary values for a CrowType.
fn boundary_values(ty: &CrowType) -> Option<Vec<CrowValue>> {
    match ty {
        CrowType::Int => Some(vec![
            CrowValue::ok(Value::Int(0)),
            CrowValue::ok(Value::Int(1)),
            CrowValue::ok(Value::Int(-1)),
            CrowValue::ok(Value::Int(i64::MAX)),
            CrowValue::ok(Value::Int(i64::MIN)),
        ]),
        CrowType::Float => Some(vec![
            CrowValue::ok(Value::Float(0.0)),
            CrowValue::ok(Value::Float(1.0)),
            CrowValue::ok(Value::Float(-1.0)),
            CrowValue::ok(Value::Float(0.5)),
            CrowValue::ok(Value::Float(-0.5)),
        ]),
        CrowType::Str => Some(vec![
            CrowValue::ok(Value::Str(String::new())),
            CrowValue::ok(Value::Str("a".into())),
            CrowValue::ok(Value::Str("hello".into())),
        ]),
        CrowType::Bool => Some(vec![
            CrowValue::ok(Value::Bool(true)),
            CrowValue::ok(Value::Bool(false)),
        ]),
        CrowType::Array(inner) => {
            match inner.as_ref() {
                CrowType::Int => Some(vec![
                    CrowValue::ok(Value::Array(vec![])),
                    CrowValue::ok(Value::Array(vec![CrowValue::ok(Value::Int(0))])),
                    CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::Int(1)),
                        CrowValue::ok(Value::Int(2)),
                        CrowValue::ok(Value::Int(3)),
                    ])),
                    CrowValue::ok(Value::Array(vec![CrowValue::ok(Value::Int(1))])),
                    CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::Int(-1)),
                        CrowValue::ok(Value::Int(0)),
                        CrowValue::ok(Value::Int(1)),
                    ])),
                ]),
                CrowType::Str => Some(vec![
                    CrowValue::ok(Value::Array(vec![])),
                    CrowValue::ok(Value::Array(vec![CrowValue::ok(Value::Str(String::new()))])),
                    CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::Str("a".into())),
                        CrowValue::ok(Value::Str("b".into())),
                    ])),
                ]),
                _ => None, // unsupported inner type
            }
        }
        CrowType::NumArray(inner) => {
            // Same boundary values as Array but for numeric arrays
            match inner.as_ref() {
                CrowType::Int | CrowType::Float => Some(vec![
                    CrowValue::ok(Value::Array(vec![])),
                    CrowValue::ok(Value::Array(vec![CrowValue::ok(Value::Int(0))])),
                    CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::Int(1)),
                        CrowValue::ok(Value::Int(2)),
                        CrowValue::ok(Value::Int(3)),
                    ])),
                    CrowValue::ok(Value::Array(vec![CrowValue::ok(Value::Int(1))])),
                    CrowValue::ok(Value::Array(vec![
                        CrowValue::ok(Value::Int(-1)),
                        CrowValue::ok(Value::Int(0)),
                        CrowValue::ok(Value::Int(1)),
                    ])),
                ]),
                _ => None,
            }
        }
        _ => None, // map, struct, etc. — skip
    }
}

/// Check if all params of a function have supported types for edge case generation.
fn can_generate_edge_cases(sig: &FnSig) -> bool {
    sig.params.iter().all(|(_, ty)| {
        boundary_values(ty).is_some()
    })
}

/// Generate all combinations of boundary inputs for a function (capped at 50).
fn generate_input_combos(sig: &FnSig) -> Vec<Vec<CrowValue>> {
    let param_values: Vec<Vec<CrowValue>> = sig.params.iter()
        .filter_map(|(_, ty)| boundary_values(ty))
        .collect();

    if param_values.is_empty() { return vec![vec![]]; }

    let mut combos: Vec<Vec<CrowValue>> = vec![vec![]];
    for vals in &param_values {
        let mut new_combos = Vec::new();
        for combo in &combos {
            for v in vals {
                let mut c = combo.clone();
                c.push(v.clone());
                new_combos.push(c);
            }
        }
        combos = new_combos;
        if combos.len() > 50 { combos.truncate(50); break; }
    }
    combos
}

/// Format a function call with arguments for display.
fn format_call(name: &str, args: &[CrowValue]) -> String {
    let arg_strs: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
    format!("{}[{}]", name, arg_strs.join(", "))
}

/// Run edge case generation on pure functions.
/// Returns (issues, snapshots) where snapshots are input→output pairs.
pub fn run_edge_cases(
    ctx: &TypeContext,
    stmts: &[Stmt],
    pure_fns: &BTreeSet<String>,
    recursive_fns: &BTreeSet<String>,
) -> (Vec<Issue>, BTreeMap<String, Vec<(String, String)>>) {
    let mut issues = Vec::new();
    let mut snapshots: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    // Filter to only fn/struct declarations — skip top-level code to avoid side effects
    let decls_only: Vec<Stmt> = stmts.iter().filter(|s| matches!(s,
        Stmt::FnDecl { .. } | Stmt::StructDecl { .. }
    )).cloned().collect();

    // Suppress panic output during edge case testing
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    for name in pure_fns {
        // Skip recursive functions — edge cases with boundary values cause stack overflow
        if recursive_fns.contains(name) { continue; }
        let sig = match ctx.functions.get(name) {
            Some(s) => s,
            None => continue,
        };
        if !can_generate_edge_cases(sig) { continue; }
        if sig.params.iter().any(|(_, ty)| matches!(ty, CrowType::Unknown)) { continue; }

        let combos = generate_input_combos(sig);
        let mut fn_snapshots = Vec::new();

        for combo in &combos {
            let call_desc = format_call(name, combo);
            let fn_name = name.clone();
            let decls_clone = decls_only.clone();
            let combo_clone = combo.clone();

            // Run in a thread with timeout to handle infinite loops
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let cancel_clone = cancel_flag.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut interp = Interpreter::new();
                    interp.set_cancel_flag(cancel_clone);
                    let _ = interp.run(&decls_clone);
                    let call_expr = Expr::Call {
                        callee: Box::new(Expr::Ident(fn_name)),
                        args: combo_clone.iter().map(|v| value_to_expr(v)).collect(),
                    };
                    interp.eval_expr_pub(&call_expr)
                }));
                let _ = tx.send(result);
            });

            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(Ok(val))) => {
                    let output = format!("{}", val);
                    fn_snapshots.push((call_desc, output));
                }
                Ok(Ok(Err(e))) => {
                    issues.push(Issue::EdgeCaseCrash {
                        func: name.clone(),
                        input_desc: call_desc,
                        error: e,
                    });
                }
                Ok(Err(panic_info)) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "panic during execution".to_string()
                    };
                    issues.push(Issue::EdgeCaseCrash {
                        func: name.clone(),
                        input_desc: call_desc,
                        error: msg,
                    });
                }
                Err(_) => {
                    // Timeout — signal the thread to stop, then wait briefly for cleanup
                    cancel_flag.store(true, Ordering::Relaxed);
                    issues.push(Issue::EdgeCaseCrash {
                        func: name.clone(),
                        input_desc: call_desc,
                        error: "timed out (possible infinite loop)".to_string(),
                    });
                    // Give the thread a moment to notice the cancel flag and exit
                    let _ = handle.join().ok();
                }
            }
        }

        if !fn_snapshots.is_empty() {
            snapshots.insert(name.clone(), fn_snapshots);
        }
    }

    // Restore the previous panic hook
    std::panic::set_hook(prev_hook);

    (issues, snapshots)
}

/// Convert a CrowValue back to an Expr for calling via the interpreter.
fn value_to_expr(val: &CrowValue) -> Expr {
    match &val.value {
        Value::Int(n) => Expr::Int(*n),
        Value::Float(f) => Expr::Float(*f),
        Value::Str(s) => Expr::Str(s.clone()),
        Value::Bool(b) => Expr::Bool(*b),
        Value::Array(items) => {
            Expr::Array(items.iter().map(|v| value_to_expr(v)).collect())
        }
        Value::NumArray(t) => {
            Expr::Array(t.to_vec().iter().map(|v| {
                if *v == (*v as i64) as f64 && v.is_finite() {
                    Expr::Int(*v as i64)
                } else {
                    Expr::Float(*v)
                }
            }).collect())
        }
        _ => Expr::Str(format!("{}", val)),
    }
}

// --- Regression snapshots ---

const SNAPSHOT_DIR: &str = ".crow/snapshots";

/// Load existing snapshots from disk.
fn load_snapshots() -> BTreeMap<String, Vec<(String, String)>> {
    let mut all = BTreeMap::new();
    let dir = std::path::Path::new(SNAPSHOT_DIR);
    if !dir.exists() { return all; }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "snap").unwrap_or(false) {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut pairs = Vec::new();
                    for line in content.lines() {
                        if let Some((input, output)) = line.split_once(" = ") {
                            pairs.push((input.to_string(), output.to_string()));
                        }
                    }
                    all.insert(name, pairs);
                }
            }
        }
    }
    all
}

/// Save snapshots to disk.
fn save_snapshots(snapshots: &BTreeMap<String, Vec<(String, String)>>) {
    let dir = std::path::Path::new(SNAPSHOT_DIR);
    std::fs::create_dir_all(dir).ok();
    for (name, pairs) in snapshots {
        let path = dir.join(format!("{}.snap", name));
        let content: String = pairs.iter()
            .map(|(input, output)| format!("{} = {}", input, output))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, content).ok();
    }
}

/// Compare new snapshots with existing ones, returning regression issues.
fn compare_snapshots(
    old: &BTreeMap<String, Vec<(String, String)>>,
    new: &BTreeMap<String, Vec<(String, String)>>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (name, old_pairs) in old {
        if let Some(new_pairs) = new.get(name) {
            let old_map: BTreeMap<&str, &str> = old_pairs.iter()
                .map(|(i, o)| (i.as_str(), o.as_str())).collect();
            let new_map: BTreeMap<&str, &str> = new_pairs.iter()
                .map(|(i, o)| (i.as_str(), o.as_str())).collect();
            for (input, old_output) in &old_map {
                if let Some(new_output) = new_map.get(input) {
                    if old_output != new_output {
                        issues.push(Issue::RegressionChange {
                            func: name.clone(),
                            input_desc: input.to_string(),
                            was: old_output.to_string(),
                            now: new_output.to_string(),
                        });
                    }
                }
            }
        }
    }
    issues
}

// --- Main check runner ---

/// Result of running `crow check`.
pub struct CheckResult {
    pub pure_fns: BTreeSet<String>,
    pub impure_fns: BTreeSet<String>,
    pub issues: Vec<Issue>,
    pub total_functions: usize,
    pub edge_cases_generated: usize,
    pub snapshots_saved: usize,
    pub snapshots_unchanged: usize,
    /// Map of function name → declaration line number.
    pub fn_lines: BTreeMap<String, usize>,
}

/// Run the full check pipeline on parsed + type-inferred code.
pub fn run_check(
    stmts: &[Stmt],
    ctx: &TypeContext,
    _verbose: bool,
) -> CheckResult {
    let all_fns: BTreeSet<String> = ctx.functions.keys().cloned().collect();
    let pure_fns = find_pure_functions(ctx);
    let impure_fns: BTreeSet<String> = all_fns.difference(&pure_fns).cloned().collect();
    let total_functions = all_fns.len();

    let mut issues = Vec::new();

    // 1. Static checks (functions + top-level code)
    issues.extend(run_static_checks(ctx, stmts));

    // 2. Edge case generation (pure, non-recursive functions only)
    let recursive_fns = find_recursive_functions(ctx);
    let (edge_issues, new_snapshots) = run_edge_cases(ctx, stmts, &pure_fns, &recursive_fns);
    let edge_cases_generated = new_snapshots.values().map(|v| v.len()).sum::<usize>();
    issues.extend(edge_issues);

    // 3. Regression snapshots
    let old_snapshots = load_snapshots();
    let regression_issues = compare_snapshots(&old_snapshots, &new_snapshots);
    let snapshots_unchanged = new_snapshots.len().saturating_sub(
        regression_issues.iter()
            .map(|i| match i { Issue::RegressionChange { func, .. } => func.clone(), _ => String::new() })
            .collect::<BTreeSet<_>>()
            .len()
    );
    issues.extend(regression_issues);

    // Save new snapshots
    let snapshots_saved = new_snapshots.len();
    save_snapshots(&new_snapshots);

    let fn_lines = find_fn_lines(stmts);

    CheckResult {
        pure_fns,
        impure_fns,
        issues,
        total_functions,
        edge_cases_generated,
        snapshots_saved,
        snapshots_unchanged,
        fn_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_infer::TypeContext;

    /// Helper: parse source, run type inference, return (stmts, ctx)
    fn parse_and_infer(src: &str) -> (Vec<Stmt>, TypeContext) {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        (stmts, ctx)
    }

    // === Purity detection ===

    #[test]
    fn test_pure_function() {
        let (_, ctx) = parse_and_infer("fn add[a, b]\n    a + b\nx = add[1, 2]\n");
        let pure = find_pure_functions(&ctx);
        assert!(pure.contains("add"), "add should be pure");
    }

    #[test]
    fn test_impure_function_log() {
        let (_, ctx) = parse_and_infer("fn greet[name]\n    log['hi {name}']\n");
        let pure = find_pure_functions(&ctx);
        assert!(!pure.contains("greet"), "greet calls log, should be impure");
    }

    #[test]
    fn test_impure_transitive() {
        let src = "fn inner[]\n    log['x']\nfn outer[]\n    inner[]\n";
        let (_, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        assert!(!pure.contains("outer"), "outer calls impure inner, should be impure");
        assert!(!pure.contains("inner"), "inner calls log, should be impure");
    }

    #[test]
    fn test_mixed_purity() {
        let src = "fn pure_fn[x]\n    x + 1\nfn impure_fn[x]\n    log[x]\nx = pure_fn[5]\n";
        let (_, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        assert!(pure.contains("pure_fn"));
        assert!(!pure.contains("impure_fn"));
    }

    // === Recursion detection ===

    #[test]
    fn test_direct_recursion() {
        let src = "fn fib[n]\n    if n lte 1\n        return n\n    return fib[n - 1] + fib[n - 2]\nlog[fib[5]]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(recursive.contains("fib"), "fib should be detected as recursive");
    }

    #[test]
    fn test_non_recursive() {
        let src = "fn double[x]\n    x * 2\nx = double[5]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(!recursive.contains("double"), "double should not be recursive");
    }

    #[test]
    fn test_indirect_recursion() {
        let src = "fn a[n]\n    if n lte 0\n        return 0\n    b[n - 1]\nfn b[n]\n    a[n - 1]\nlog[a[3]]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(recursive.contains("a"), "a should be detected as indirectly recursive");
        assert!(recursive.contains("b"), "b should be detected as indirectly recursive");
    }

    // === Static checks: unused variables ===

    #[test]
    fn test_unused_var_in_function() {
        let src = "fn foo[x]\n    temp = 5\n    x + 1\nresult = foo[3]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, .. } if var == "temp")).collect();
        assert!(!unused.is_empty(), "should detect unused 'temp'");
    }

    #[test]
    fn test_unused_param() {
        let src = "fn foo[x, y]\n    x + 1\nresult = foo[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, .. } if var == "y")).collect();
        assert!(!unused.is_empty(), "should detect unused param 'y'");
    }

    #[test]
    fn test_no_false_positive_used_var() {
        let src = "fn add[a, b]\n    a + b\nresult = add[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused_in_add: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { func, .. } if func == "add")).collect();
        assert!(unused_in_add.is_empty(), "add uses both params, no unused vars expected");
    }

    #[test]
    fn test_unused_var_top_level() {
        let src = "x = 42\ny = 10\nlog[y]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, func, .. } if var == "x" && func == "<top-level>")).collect();
        assert!(!unused.is_empty(), "should detect unused top-level 'x'");
    }

    // === Static checks: unreachable code ===

    #[test]
    fn test_unreachable_after_return() {
        let src = "fn foo[x]\n    return x\n    y = 5\nresult = foo[1]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unreachable: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnreachableCode { .. })).collect();
        assert!(!unreachable.is_empty(), "should detect unreachable code after return");
    }

    #[test]
    fn test_no_unreachable_normal() {
        let src = "fn foo[x]\n    y = x + 1\n    return y\nresult = foo[1]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unreachable: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnreachableCode { .. })).collect();
        assert!(unreachable.is_empty(), "no unreachable code expected");
    }

    // === Edge case generation ===

    #[test]
    fn test_edge_case_overflow_detected() {
        let src = "fn add[a, b]\n    a + b\nresult = add[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, _snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        let crashes: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::EdgeCaseCrash { .. })).collect();
        assert!(!crashes.is_empty(), "add should crash on overflow with boundary values");
    }

    #[test]
    fn test_edge_case_empty_array_crash() {
        let src = "fn first[items]\n    return items[0]\nnums = [1, 2]\nresult = first[nums]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, _) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        let crashes: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::EdgeCaseCrash { error, .. } if error.contains("out of bounds"))).collect();
        assert!(!crashes.is_empty(), "first[[]] should crash with index out of bounds");
    }

    #[test]
    fn test_edge_case_skips_recursive() {
        let src = "fn fib[n]\n    if n lte 1\n        return n\n    return fib[n - 1] + fib[n - 2]\nlog[fib[5]]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        assert!(issues.is_empty(), "recursive fib should be skipped, no issues");
        assert!(snapshots.is_empty(), "recursive fib should be skipped, no snapshots");
    }

    #[test]
    fn test_edge_case_skips_impure() {
        let src = "fn greet[name]\n    log['hi {name}']\ngreet['world']\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        assert!(issues.is_empty(), "impure greet should not be tested");
        assert!(snapshots.is_empty(), "impure greet should not be tested");
    }

    // === Full pipeline ===

    #[test]
    fn test_run_check_clean_program() {
        let src = "fn double[x]\n    x * 2\nresult = double[5]\nlog[result]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let result = run_check(&stmts, &ctx, false);
        assert!(result.pure_fns.contains("double"));
        assert!(result.total_functions == 1);
    }
}