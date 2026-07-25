/// Decision-slice analysis: dataflow from query results through conditions
/// and derived values into writes.
///
/// Three detectors live here because they share the same walk:
///   1a write skew   — read → condition → guarded INSERT, invariant unenforced
///   1b lost update  — read → arithmetic → UPDATE writing the same column
///   1d assumed-unique — read filtered by a non-unique column → single-row access
///
/// Conservative by design: intra-scope, literal SQL, direct dataflow only.

use crate::ast::{BinOp, Expr, Stmt};
use std::collections::{BTreeMap, BTreeSet};

use super::schema::Schema;
use super::sql_facts::{Facts, ParamLit, Rhs, SetVal, SqlCall, SqlKind};
use super::{Hazard, HazardKind};

pub fn analyze(stmts: &[Stmt], facts: &Facts, schema: &Schema, file: &str) -> Vec<Hazard> {
    let mut ctx = Ctx {
        facts,
        schema,
        file,
        hazards: Vec::new(),
        fired: BTreeSet::new(),
    };
    let mut line = 0usize;
    analyze_scope(stmts, &mut line, &mut ctx);
    ctx.hazards
}

struct Ctx<'a> {
    facts: &'a Facts,
    schema: &'a Schema,
    file: &'a str,
    hazards: Vec<Hazard>,
    /// Dedup key per fired hazard (kind + location).
    fired: BTreeSet<String>,
}

#[derive(Default)]
struct Scope {
    /// var → line of the SELECT its value (transitively) derives from
    tracked: BTreeMap<String, usize>,
    /// var → read line, for vars derived via arithmetic (lost-update signal)
    arith: BTreeMap<String, usize>,
}

fn analyze_scope(stmts: &[Stmt], line: &mut usize, ctx: &mut Ctx) {
    let mut scope = Scope::default();
    walk(stmts, line, &mut scope, ctx);
}

fn walk(stmts: &[Stmt], line: &mut usize, scope: &mut Scope, ctx: &mut Ctx) {
    for stmt in stmts {
        match stmt {
            Stmt::Line(n) => *line = *n,
            Stmt::Assign { name, value } => {
                if let Some(read) = select_at(ctx.facts, *line, name) {
                    scope.tracked.insert(name.clone(), read.line);
                } else {
                    let used = idents_in(value);
                    if let Some(base) = used.iter().find_map(|u| scope.tracked.get(u)).copied()
                    {
                        scope.tracked.insert(name.clone(), base);
                        if has_arithmetic(value) {
                            scope.arith.insert(name.clone(), base);
                        }
                    }
                }
                inspect_exprs_in_stmt(stmt, *line, scope, ctx);
            }
            Stmt::ExprStmt(_) | Stmt::Return(_) => {
                inspect_exprs_in_stmt(stmt, *line, scope, ctx);
            }
            Stmt::FnDecl { body, .. } => {
                let mut fn_line = *line;
                analyze_scope(body, &mut fn_line, ctx);
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let cond_vars = idents_in(condition);
                let base_read = cond_vars
                    .iter()
                    .find_map(|v| scope.tracked.get(v))
                    .copied();
                if let Some(read_line) = base_read {
                    if let Some(read) = call_at(ctx.facts, read_line) {
                        check_guarded_writes(then_body, read, *line, ctx);
                        if let Some(eb) = else_body {
                            check_guarded_writes(eb, read, *line, ctx);
                        }
                    }
                }
                walk(then_body, line, scope, ctx);
                if let Some(eb) = else_body {
                    walk(eb, line, scope, ctx);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                walk(body, line, scope, ctx);
            }
            _ => {}
        }
    }
}

/// Per-statement inspections that need the current scope: lost-update writes
/// and single-row accesses.
fn inspect_exprs_in_stmt(stmt: &Stmt, line: usize, scope: &Scope, ctx: &mut Ctx) {
    // 1b — an UPDATE on this line whose SET placeholder is fed by an
    // arithmetic derivation of a read from the same table/column.
    for call in ctx.facts.calls.iter().filter(|c| c.line == line) {
        if call.kind == SqlKind::Update {
            check_lost_update(call, scope, ctx);
        }
    }
    // 1d — result.col[0] access on a tracked read.
    let mut accesses = Vec::new();
    stmt_exprs(stmt, &mut |e| collect_zero_index_access(e, &mut accesses));
    for var in accesses {
        if let Some(read_line) = scope.tracked.get(&var) {
            if let Some(read) = call_at(ctx.facts, *read_line) {
                check_assumed_unique(read, line, ctx);
            }
        }
    }
}

fn check_lost_update(update: &SqlCall, scope: &Scope, ctx: &mut Ctx) {
    let Some(table) = &update.table else { return };
    for (col, sv) in &update.set_cols {
        if !matches!(sv, SetVal::Placeholder) {
            continue;
        }
        // Which param feeds this column's placeholder?
        let Some(idx) = update
            .slots
            .iter()
            .position(|s| s.as_deref() == Some(col.as_str()))
        else {
            continue;
        };
        let Some(ParamLit::Var(Some(param_var))) =
            update.params.as_ref().and_then(|p| p.get(idx))
        else {
            continue;
        };
        let Some(read_line) = scope.arith.get(param_var) else {
            continue;
        };
        let Some(read) = call_at(ctx.facts, *read_line) else {
            continue;
        };
        if read.table.as_deref() != Some(table.as_str()) {
            continue;
        }
        if !(read.star || read.select_cols.contains(col)) {
            continue;
        }
        let key = format!("lost_update:{}:{}:{}", table, col, update.line);
        if ctx.fired.insert(key) {
            ctx.hazards.push(Hazard {
                kind: HazardKind::LostUpdate {
                    table: table.clone(),
                    column: col.clone(),
                    read_line: read.line,
                    write_line: update.line,
                },
                file: ctx.file.into(),
                span: (read.line, update.line),
            });
        }
    }
}

fn check_assumed_unique(read: &SqlCall, access_line: usize, ctx: &mut Ctx) {
    let Some(table) = &read.table else { return };
    let (key_cols, partial) = split_filters(read);
    if key_cols.len() != 1 || !partial.is_empty() {
        return;
    }
    let col = &key_cols[0];
    let covered = ctx
        .schema
        .table(table)
        .map(|ts| ts.unique_covers(&[col.clone()], &[]))
        .unwrap_or(true); // unknown table already reported by 1f
    if covered {
        return;
    }
    let key = format!("assumed_unique:{}:{}:{}", table, col, read.line);
    if ctx.fired.insert(key) {
        ctx.hazards.push(Hazard {
            kind: HazardKind::AssumedUnique {
                table: table.clone(),
                column: col.clone(),
                access_line,
            },
            file: ctx.file.into(),
            span: (read.line, access_line),
        });
    }
}

/// Writes to the read's table inside a guarded body → write skew unless the
/// schema enforces the invariant implied by the read's WHERE filters.
fn check_guarded_writes(body: &[Stmt], read: &SqlCall, guard_line: usize, ctx: &mut Ctx) {
    let Some(read_table) = &read.table else { return };
    let mut writes = Vec::new();
    collect_write_calls(body, ctx.facts, guard_line, &mut writes);
    for write in writes {
        if write.kind != SqlKind::Insert {
            continue; // v1: the classic guarded-INSERT shape only
        }
        if write.table.as_deref() != Some(read_table.as_str()) {
            continue;
        }
        let (key_cols, partial) = split_filters(read);
        if key_cols.is_empty() {
            continue; // nothing identity-shaped to be unique on
        }
        let covered = ctx
            .schema
            .table(read_table)
            .map(|ts| ts.unique_covers(&key_cols, &partial))
            .unwrap_or(false);
        if covered {
            continue;
        }

        let invariant = if partial.is_empty() {
            format!("at most one {} row per {}", read_table, key_cols.join(" + "))
        } else {
            format!(
                "at most one {} {} row per {}",
                partial
                    .iter()
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                read_table,
                key_cols.join(" + ")
            )
        };

        let key = format!("write_skew:{}:{}:{}", read_table, read.line, write.line);
        if ctx.fired.insert(key) {
            ctx.hazards.push(Hazard {
                kind: HazardKind::WriteSkew {
                    table: read_table.clone(),
                    invariant,
                    key_cols: key_cols.clone(),
                    partial: partial.clone(),
                    read_line: read.line,
                    write_line: write.line,
                },
                file: ctx.file.into(),
                span: (read.line, write.line),
            });
        }
    }
}

/// Split a read's equality filters into identity-shaped key columns (bound to
/// program values) and literal predicate columns. Placeholder filters resolve
/// through the params array: `status = ?` with `'active'` at that position is
/// a literal predicate, not a key.
pub fn split_filters(read: &SqlCall) -> (Vec<String>, Vec<(String, String)>) {
    let mut key_cols = Vec::new();
    let mut partial = Vec::new();
    let mut placeholder_idx = 0usize;
    for (col, rhs) in &read.where_eq {
        match rhs {
            Rhs::Literal(v) => partial.push((col.clone(), v.clone())),
            Rhs::Placeholder => {
                let lit = read
                    .params
                    .as_ref()
                    .and_then(|p| p.get(placeholder_idx))
                    .and_then(|p| match p {
                        ParamLit::Str(s) => Some(s.to_lowercase()),
                        ParamLit::Int(n) => Some(n.to_string()),
                        ParamLit::Float(f) => Some(f.to_string()),
                        ParamLit::Bool(b) => Some(b.to_string()),
                        ParamLit::Var(_) => None,
                    });
                placeholder_idx += 1;
                match lit {
                    Some(v) => partial.push((col.clone(), v)),
                    None => key_cols.push(col.clone()),
                }
            }
            Rhs::Expr => key_cols.push(col.clone()),
        }
    }
    (key_cols, partial)
}

fn collect_write_calls<'a>(
    body: &[Stmt],
    facts: &'a Facts,
    mut line: usize,
    out: &mut Vec<&'a SqlCall>,
) {
    for stmt in body {
        match stmt {
            Stmt::Line(n) => line = *n,
            Stmt::Assign { .. } | Stmt::ExprStmt(_) | Stmt::Return(_) => {
                for call in facts.calls.iter().filter(|c| c.line == line) {
                    if call.kind != SqlKind::Select {
                        out.push(call);
                    }
                }
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_write_calls(then_body, facts, line, out);
                if let Some(eb) = else_body {
                    collect_write_calls(eb, facts, line, out);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                collect_write_calls(body, facts, line, out);
            }
            _ => {}
        }
    }
}

fn select_at<'a>(facts: &'a Facts, line: usize, var: &str) -> Option<&'a SqlCall> {
    facts.calls.iter().find(|c| {
        c.line == line && c.kind == SqlKind::Select && c.result_var.as_deref() == Some(var)
    })
}

fn call_at(facts: &Facts, line: usize) -> Option<&SqlCall> {
    facts.calls.iter().find(|c| c.line == line)
}

/// result.col[0] — indexing the first element of a column of a query result.
fn collect_zero_index_access(expr: &Expr, out: &mut Vec<String>) {
    if let Expr::Call { callee, args } = expr {
        if args.len() == 1 && matches!(args[0], Expr::Int(0)) {
            if let Expr::FieldAccess { object, .. } = callee.as_ref() {
                if let Expr::Ident(var) = object.as_ref() {
                    out.push(var.clone());
                }
            }
        }
    }
}

fn has_arithmetic(expr: &Expr) -> bool {
    match expr {
        Expr::BinOp { left, op, right } => {
            matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
            ) || has_arithmetic(left)
                || has_arithmetic(right)
        }
        Expr::UnaryOp { expr, .. } => has_arithmetic(expr),
        Expr::Call { args, .. } => args.iter().any(has_arithmetic),
        Expr::FieldAccess { object, .. } => has_arithmetic(object),
        _ => false,
    }
}

/// Visit every expression in a statement (shallow — no nested stmt bodies).
fn stmt_exprs(stmt: &Stmt, f: &mut impl FnMut(&Expr)) {
    match stmt {
        Stmt::Assign { value, .. } => visit(value, f),
        Stmt::MultiAssign { values, .. } => {
            for v in values {
                visit(v, f);
            }
        }
        Stmt::ExprStmt(e) => visit(e, f),
        Stmt::Return(Some(e)) => visit(e, f),
        Stmt::If { condition, .. } => visit(condition, f),
        Stmt::While { condition, .. } => visit(condition, f),
        Stmt::For { iter, .. } => visit(iter, f),
        Stmt::FieldAssign { object, value, .. } => {
            visit(object, f);
            visit(value, f);
        }
        Stmt::IndexAssign {
            object,
            index,
            value,
        } => {
            visit(object, f);
            visit(index, f);
            visit(value, f);
        }
        _ => {}
    }
}

fn visit(expr: &Expr, f: &mut impl FnMut(&Expr)) {
    f(expr);
    match expr {
        Expr::BinOp { left, right, .. } => {
            visit(left, f);
            visit(right, f);
        }
        Expr::UnaryOp { expr, .. } => visit(expr, f),
        Expr::Call { callee, args } => {
            visit(callee, f);
            for a in args {
                visit(a, f);
            }
        }
        Expr::FieldAccess { object, .. } => visit(object, f),
        Expr::Array(items) => {
            for i in items {
                visit(i, f);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                visit(k, f);
                visit(v, f);
            }
        }
        Expr::Named { value, .. } => visit(value, f),
        Expr::ForExpr { iter, .. } => visit(iter, f),
        _ => {}
    }
}

/// All identifier names mentioned anywhere in an expression.
fn idents_in(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_idents(expr, &mut out);
    out
}

fn collect_idents(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        Expr::BinOp { left, right, .. } => {
            collect_idents(left, out);
            collect_idents(right, out);
        }
        Expr::UnaryOp { expr, .. } => collect_idents(expr, out),
        Expr::Call { callee, args } => {
            if !matches!(callee.as_ref(), Expr::Ident(_)) {
                collect_idents(callee, out);
            }
            for a in args {
                collect_idents(a, out);
            }
        }
        Expr::FieldAccess { object, .. } => collect_idents(object, out),
        Expr::Array(items) => {
            for i in items {
                collect_idents(i, out);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_idents(k, out);
                collect_idents(v, out);
            }
        }
        Expr::Named { value, .. } => collect_idents(value, out),
        Expr::ForExpr { iter, .. } => collect_idents(iter, out),
        _ => {}
    }
}
