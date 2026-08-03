/// Extract database facts from a Crow program: every `query`/`exec` call with
/// a literal SQL string, parsed into which table it touches, which columns it
/// reads/writes, and how its condition filters — plus the surrounding context
/// (enclosing function, result variable, transact usage) that later detectors
/// need.

use crate::ast::{Expr, Stmt};
use sqlparser::ast as sql;
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser as SqlParser;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum SqlKind {
    Select,
    Insert,
    Update,
    Delete,
    Other,
}

/// Right-hand side of an equality filter `col = X` in a WHERE clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Rhs {
    Placeholder,
    Literal(String),
    Expr,
}

/// Value written to a column in SET / VALUES.
#[derive(Debug, Clone, PartialEq)]
pub enum SetVal {
    Constant(String),
    Placeholder,
    /// col = col + ? / col - ? — already-atomic in-place arithmetic.
    SelfArith,
    Expr,
}

/// A parameter in the Crow-side params array; literals keep their value so
/// filters like `status = ?` with params `[…, 'active']` can be recognized
/// as literal predicates.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamLit {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    /// A variable reference (name kept for dataflow matching) or other expr.
    Var(Option<String>),
}

#[derive(Debug, Clone)]
pub struct SqlCall {
    pub line: usize,
    /// The connection argument (`query[CONN, ...]`) when it's a plain
    /// variable — lets multi-database files scope checks to the checked db.
    pub conn_var: Option<String>,
    /// Enclosing function name; None at top level.
    pub func: Option<String>,
    pub kind: SqlKind,
    pub table: Option<String>,
    /// Projected columns for SELECT (empty when `*`).
    pub select_cols: Vec<String>,
    pub star: bool,
    pub insert_cols: Vec<String>,
    pub set_cols: Vec<(String, SetVal)>,
    /// AND-connected equality filters.
    pub where_eq: Vec<(String, Rhs)>,
    /// Column associated with each `?` placeholder, in order of appearance.
    pub slots: Vec<Option<String>>,
    pub placeholders: usize,
    /// Literal kinds of the Crow-side params array, when it is an array literal.
    pub params: Option<Vec<ParamLit>>,
    /// Variable the result was assigned to (`x = query[...]`).
    pub result_var: Option<String>,
    /// SQL string contains backtick interpolation.
    pub interpolated: bool,
    /// SQL failed to parse (reported nowhere yet — future lint).
    pub unparsed: bool,
}

#[derive(Debug, Default)]
pub struct Facts {
    pub calls: Vec<SqlCall>,
    /// Names of functions passed to transact[…] — their writes are transactional.
    pub transact_fns: BTreeSet<String>,
}

pub fn collect(stmts: &[Stmt]) -> Facts {
    let mut facts = Facts::default();
    walk_stmts(stmts, None, &mut 0, &mut facts);
    facts
}

fn walk_stmts(stmts: &[Stmt], func: Option<&str>, line: &mut usize, facts: &mut Facts) {
    for stmt in stmts {
        match stmt {
            Stmt::Line(n) => *line = *n,
            Stmt::Assign { name, value } => {
                walk_expr(value, func, *line, Some(name), facts);
            }
            Stmt::MultiAssign { values, .. } => {
                for v in values {
                    walk_expr(v, func, *line, None, facts);
                }
            }
            Stmt::FnDecl { name, body, .. } => {
                let mut fn_line = *line;
                walk_stmts(body, Some(name), &mut fn_line, facts);
            }
            Stmt::Return(Some(e)) => walk_expr(e, func, *line, None, facts),
            Stmt::Return(None) => {}
            Stmt::ExprStmt(e) => walk_expr(e, func, *line, None, facts),
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                walk_expr(condition, func, *line, None, facts);
                walk_stmts(then_body, func, line, facts);
                if let Some(eb) = else_body {
                    walk_stmts(eb, func, line, facts);
                }
            }
            Stmt::For { iter, body, .. } => {
                walk_expr(iter, func, *line, None, facts);
                walk_stmts(body, func, line, facts);
            }
            Stmt::While { condition, body } => {
                walk_expr(condition, func, *line, None, facts);
                walk_stmts(body, func, line, facts);
            }
            Stmt::FieldAssign { object, value, .. } => {
                walk_expr(object, func, *line, None, facts);
                walk_expr(value, func, *line, None, facts);
            }
            Stmt::IndexAssign {
                object,
                index,
                value,
            } => {
                walk_expr(object, func, *line, None, facts);
                walk_expr(index, func, *line, None, facts);
                walk_expr(value, func, *line, None, facts);
            }
            _ => {}
        }
    }
}

fn walk_expr(
    expr: &Expr,
    func: Option<&str>,
    line: usize,
    assign_to: Option<&str>,
    facts: &mut Facts,
) {
    match expr {
        Expr::Call { callee, args } => {
            if let Some(name) = call_name(callee) {
                if name == "query" || name == "exec" {
                    if let Some(Expr::Str(sql_text)) = args.get(1) {
                        let mut call = analyze_call(
                            sql_text,
                            args.get(2),
                            line,
                            func,
                            assign_to,
                        );
                        if let Some(Expr::Ident(cv)) = args.first() {
                            call.conn_var = Some(cv.clone());
                        }
                        facts.calls.push(call);
                    }
                } else if name == "transact" {
                    if let Some(Expr::Ident(fn_name)) = args.get(1) {
                        facts.transact_fns.insert(fn_name.clone());
                    }
                }
            }
            walk_expr(callee, func, line, None, facts);
            for a in args {
                walk_expr(a, func, line, None, facts);
            }
        }
        Expr::BinOp { left, right, .. } => {
            walk_expr(left, func, line, None, facts);
            walk_expr(right, func, line, None, facts);
        }
        Expr::UnaryOp { expr, .. } => walk_expr(expr, func, line, None, facts),
        Expr::FieldAccess { object, .. } => walk_expr(object, func, line, None, facts),
        Expr::Array(items) => {
            for i in items {
                walk_expr(i, func, line, None, facts);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                walk_expr(k, func, line, None, facts);
                walk_expr(v, func, line, None, facts);
            }
        }
        Expr::Named { value, .. } => walk_expr(value, func, line, None, facts),
        Expr::ForExpr { iter, body, .. } => {
            walk_expr(iter, func, line, None, facts);
            let mut l = line;
            walk_stmts(body, func, &mut l, facts);
        }
        _ => {}
    }
}

/// `query` / `exec` / `db.query` / `db.exec` — bare or module-qualified.
fn call_name(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Ident(n) => Some(n.as_str()),
        Expr::FieldAccess { object, field } => match object.as_ref() {
            Expr::Ident(m) if m == "db" => Some(field.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn analyze_call(
    sql_text: &str,
    params_arg: Option<&Expr>,
    line: usize,
    func: Option<&str>,
    assign_to: Option<&str>,
) -> SqlCall {
    let mut call = SqlCall {
        line,
        conn_var: None,
        func: func.map(|f| f.to_string()),
        kind: SqlKind::Other,
        table: None,
        select_cols: vec![],
        star: false,
        insert_cols: vec![],
        set_cols: vec![],
        where_eq: vec![],
        slots: vec![],
        placeholders: 0,
        params: None,
        result_var: assign_to.map(|s| s.to_string()),
        interpolated: sql_text.contains('`'),
        unparsed: false,
    };

    call.params = params_arg.map(|p| match p {
        Expr::Array(items) => items
            .iter()
            .map(|i| match i {
                Expr::Int(n) => ParamLit::Int(*n),
                Expr::Float(f) => ParamLit::Float(*f),
                Expr::Str(s) => ParamLit::Str(s.clone()),
                Expr::Bool(b) => ParamLit::Bool(*b),
                Expr::Ident(v) => ParamLit::Var(Some(v.clone())),
                _ => ParamLit::Var(None),
            })
            .collect(),
        _ => vec![],
    });

    if call.interpolated {
        // Interpolated SQL text is not statically parseable — the injection
        // hazard itself is reported; skip structural analysis.
        return call;
    }

    let dialect = SQLiteDialect {};
    let parsed = match SqlParser::parse_sql(&dialect, sql_text) {
        Ok(stmts) if !stmts.is_empty() => stmts,
        _ => {
            call.unparsed = true;
            return call;
        }
    };
    analyze_statement(&parsed[0], &mut call);
    call.placeholders = call.slots.len();
    call
}

fn analyze_statement(stmt: &sql::Statement, call: &mut SqlCall) {
    match stmt {
        sql::Statement::Query(q) => {
            call.kind = SqlKind::Select;
            if let sql::SetExpr::Select(sel) = q.body.as_ref() {
                for item in &sel.projection {
                    match item {
                        sql::SelectItem::UnnamedExpr(e) => {
                            if let Some(c) = ident_of(e) {
                                call.select_cols.push(c);
                            }
                        }
                        sql::SelectItem::ExprWithAlias { expr, .. } => {
                            if let Some(c) = ident_of(expr) {
                                call.select_cols.push(c);
                            }
                        }
                        sql::SelectItem::Wildcard(_) | sql::SelectItem::QualifiedWildcard(..) => {
                            call.star = true;
                        }
                        _ => {}
                    }
                }
                if let Some(t) = sel.from.first() {
                    call.table = table_name(&t.relation);
                }
                if let Some(w) = &sel.selection {
                    collect_where(w, call);
                }
            }
        }
        sql::Statement::Insert(ins) => {
            call.kind = SqlKind::Insert;
            call.table = object_name_text(ins.table.to_string());
            call.insert_cols = ins
                .columns
                .iter()
                .filter_map(|c| object_name_text(c.to_string()))
                .collect();
            if let Some(src) = &ins.source {
                if let sql::SetExpr::Values(vals) = src.body.as_ref() {
                    for row in &vals.rows {
                        for (i, v) in row.iter().enumerate() {
                            if is_placeholder(v) {
                                call.slots.push(call.insert_cols.get(i).cloned());
                            }
                        }
                    }
                }
            }
        }
        sql::Statement::Update(upd) => {
            let sql::Update {
                table,
                assignments,
                selection,
                ..
            } = upd;
            call.kind = SqlKind::Update;
            call.table = table_name(&table.relation);
            for a in assignments {
                let col = assignment_col(a);
                let sv = classify_set_value(&a.value, col.as_deref());
                if is_placeholder(&a.value) || matches!(sv, SetVal::SelfArith) {
                    if matches!(sv, SetVal::Placeholder) {
                        call.slots.push(col.clone());
                    } else if let SetVal::SelfArith = sv {
                        // self-arith consumes one placeholder on its RHS
                        if expr_has_placeholder(&a.value) {
                            call.slots.push(col.clone());
                        }
                    }
                }
                if let Some(c) = col {
                    call.set_cols.push((c, sv));
                }
            }
            if let Some(w) = selection {
                collect_where(w, call);
            }
        }
        sql::Statement::Delete(del) => {
            call.kind = SqlKind::Delete;
            let (sql::FromTable::WithFromKeyword(tables)
            | sql::FromTable::WithoutKeyword(tables)) = &del.from;
            if let Some(t) = tables.first() {
                call.table = table_name(&t.relation);
            }
            if let Some(w) = &del.selection {
                collect_where(w, call);
            }
        }
        _ => {}
    }
}

fn collect_where(e: &sql::Expr, call: &mut SqlCall) {
    match e {
        sql::Expr::BinaryOp { left, op, right } => match op {
            sql::BinaryOperator::And => {
                collect_where(left, call);
                collect_where(right, call);
            }
            sql::BinaryOperator::Eq => {
                if let Some(col) = ident_of(left) {
                    let rhs = if is_placeholder(right) {
                        call.slots.push(Some(col.clone()));
                        Rhs::Placeholder
                    } else if let Some(lit) = literal_text(right) {
                        Rhs::Literal(lit)
                    } else {
                        Rhs::Expr
                    };
                    call.where_eq.push((col, rhs));
                } else if is_placeholder(right) {
                    call.slots.push(None);
                }
            }
            _ => {
                // Non-equality comparisons still consume placeholders.
                if expr_has_placeholder(left) || expr_has_placeholder(right) {
                    call.slots.push(ident_of(left).or_else(|| ident_of(right)));
                }
            }
        },
        sql::Expr::Nested(inner) => collect_where(inner, call),
        _ => {
            if expr_has_placeholder(e) {
                call.slots.push(None);
            }
        }
    }
}

fn ident_of(e: &sql::Expr) -> Option<String> {
    match e {
        sql::Expr::Identifier(i) => Some(i.value.to_lowercase()),
        sql::Expr::CompoundIdentifier(parts) => {
            parts.last().map(|i| i.value.to_lowercase())
        }
        sql::Expr::Nested(inner) => ident_of(inner),
        _ => None,
    }
}

fn is_placeholder(e: &sql::Expr) -> bool {
    matches!(
        e,
        sql::Expr::Value(v) if matches!(&v.value, sql::Value::Placeholder(_))
    )
}

fn literal_text(e: &sql::Expr) -> Option<String> {
    if let sql::Expr::Value(v) = e {
        match &v.value {
            sql::Value::SingleQuotedString(s) | sql::Value::DoubleQuotedString(s) => {
                Some(s.to_lowercase())
            }
            sql::Value::Number(n, _) => Some(n.clone()),
            sql::Value::Boolean(b) => Some(b.to_string()),
            _ => None,
        }
    } else {
        None
    }
}

fn expr_has_placeholder(e: &sql::Expr) -> bool {
    if is_placeholder(e) {
        return true;
    }
    match e {
        sql::Expr::BinaryOp { left, right, .. } => {
            expr_has_placeholder(left) || expr_has_placeholder(right)
        }
        sql::Expr::Nested(inner) => expr_has_placeholder(inner),
        _ => false,
    }
}

fn classify_set_value(e: &sql::Expr, col: Option<&str>) -> SetVal {
    if is_placeholder(e) {
        return SetVal::Placeholder;
    }
    if let Some(lit) = literal_text(e) {
        return SetVal::Constant(lit);
    }
    if let sql::Expr::BinaryOp { left, op, right } = e {
        let arith = matches!(
            op,
            sql::BinaryOperator::Plus | sql::BinaryOperator::Minus
        );
        if arith {
            if let (Some(l), Some(c)) = (ident_of(left), col) {
                if l == c && (is_placeholder(right) || literal_text(right).is_some()) {
                    return SetVal::SelfArith;
                }
            }
        }
    }
    SetVal::Expr
}

fn assignment_col(a: &sql::Assignment) -> Option<String> {
    match &a.target {
        sql::AssignmentTarget::ColumnName(name) => object_name_text(name.to_string()),
        _ => None,
    }
}

fn table_name(tf: &sql::TableFactor) -> Option<String> {
    if let sql::TableFactor::Table { name, .. } = tf {
        object_name_text(name.to_string())
    } else {
        None
    }
}

fn object_name_text(s: String) -> Option<String> {
    s.rsplit('.')
        .next()
        .map(|p| p.trim_matches('"').trim_matches('`').to_lowercase())
}
