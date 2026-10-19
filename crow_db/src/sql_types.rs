//! The types SQLite gives a SELECT's computed columns. SQLite declares no
//! type for an expression (`round(x, 2)`, `count(*)`, `a || b`), and the
//! describe probe — the statement stepped once with NULL parameters —
//! answers only when a row comes back: `WHERE session_id = ?` or an empty
//! table gives none. The compiled program then read such a column as text
//! while the interpreter typed it from the rows (finch's blotter `pnl`).
//! This types each such column from the statement itself: the declared
//! types of the columns it reads and SQLite's rules for its operators and
//! functions. Where the statement does not say (a parameter, NULL, a
//! subquery), the column stays unknown and reads as text, as before.

use sqlparser::ast as sql;
use sqlparser::dialect::{PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser as SqlParser;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of each item of a SELECT's list, in order — "int", "float",
/// "str" or `None` where the statement does not say. `columns_of(table)`
/// answers a table's declared columns as (name, declared type). `None`
/// for a statement that is not a SELECT, or whose list has a wildcard (the
/// database types those columns itself).
pub fn select_kinds(text: &str, columns_of: &dyn Fn(&str) -> Vec<(String, String)>) -> Option<Vec<Option<String>>> {
    let stmts = SqlParser::parse_sql(&SQLiteDialect {}, text).ok()?;
    let query = match stmts.first()? {
        sql::Statement::Query(q) => q,
        _ => return None,
    };
    let select = leftmost_select(query.body.as_ref())?;
    let mut scope = Scope { tables: BTreeMap::new(), all: Vec::new() };
    for from in &select.from {
        for relation in std::iter::once(&from.relation).chain(from.joins.iter().map(|j| &j.relation)) {
            if let sql::TableFactor::Table { name, alias, .. } = relation {
                let table = last_part(name)?;
                let columns: Vec<(String, String)> = columns_of(&table)
                    .into_iter()
                    .map(|(n, t)| (n.to_lowercase(), t))
                    .collect();
                if let Some(a) = alias {
                    scope.tables.insert(a.name.value.to_lowercase(), columns.clone());
                }
                scope.tables.insert(table.to_lowercase(), columns.clone());
                scope.all.extend(columns);
            }
        }
    }
    let mut kinds = Vec::with_capacity(select.projection.len());
    for item in &select.projection {
        match item {
            sql::SelectItem::UnnamedExpr(e) | sql::SelectItem::ExprWithAlias { expr: e, .. } => kinds.push(scope.kind(e)),
            _ => return None,
        }
    }
    Some(kinds)
}

/// A compound query's columns are named and typed by its first SELECT.
fn leftmost_select(body: &sql::SetExpr) -> Option<&sql::Select> {
    match body {
        sql::SetExpr::Select(s) => Some(s),
        sql::SetExpr::SetOperation { left, .. } => leftmost_select(left),
        sql::SetExpr::Query(q) => leftmost_select(q.body.as_ref()),
        _ => None,
    }
}

fn last_part(name: &sql::ObjectName) -> Option<String> {
    match name.0.last()? {
        sql::ObjectNamePart::Identifier(i) => Some(i.value.clone()),
        _ => None,
    }
}

/// Two kinds meeting in one column: numbers widen to float, anything with
/// text is text.
fn unify(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), Some(b)) if a == "str" || b == "str" => Some("str".into()),
        (Some(a), Some(b)) if (a == "int" || a == "float") && (b == "int" || b == "float") => Some("float".into()),
        _ => None,
    }
}

struct Scope {
    /// table name or alias → its declared columns (lowercased names)
    tables: BTreeMap<String, Vec<(String, String)>>,
    /// every column of every table, for a name without a qualifier
    all: Vec<(String, String)>,
}

impl Scope {
    /// A column the statement reads, by its declared type. A bare name two
    /// tables declare differently is not decided here.
    fn column(&self, table: Option<&str>, name: &str) -> Option<String> {
        let name = name.to_lowercase();
        let declared: Vec<&String> = match table {
            Some(t) => self.tables.get(&t.to_lowercase())?.iter().filter(|(n, _)| *n == name).map(|(_, t)| t).collect(),
            None => self.all.iter().filter(|(n, _)| *n == name).map(|(_, t)| t).collect(),
        };
        let kinds: Vec<Option<String>> = declared.iter().map(|t| crate::sqlite::kind_of_decl(t)).collect();
        let first = kinds.first()?.clone();
        if kinds.iter().all(|k| *k == first) { first } else { None }
    }

    fn kind(&self, e: &sql::Expr) -> Option<String> {
        use sql::Expr as E;
        let int = || Some("int".to_string());
        match e {
            E::Identifier(i) => self.column(None, &i.value),
            E::CompoundIdentifier(parts) if parts.len() >= 2 => {
                self.column(Some(&parts[parts.len() - 2].value), &parts[parts.len() - 1].value)
            }
            E::Value(v) => match &v.value {
                sql::Value::Number(n, _) => {
                    Some(if n.contains(['.', 'e', 'E']) { "float" } else { "int" }.to_string())
                }
                sql::Value::SingleQuotedString(_) | sql::Value::DoubleQuotedString(_) => Some("str".into()),
                sql::Value::Boolean(_) => int(),
                _ => None,
            },
            E::Nested(inner) => self.kind(inner),
            E::UnaryOp { op: sql::UnaryOperator::Not, .. } => int(),
            E::UnaryOp { expr, .. } => self.kind(expr),
            E::BinaryOp { left, op, right } => {
                use sql::BinaryOperator as B;
                match op {
                    B::StringConcat => Some("str".into()),
                    B::Plus | B::Minus | B::Multiply | B::Divide | B::Modulo => {
                        match (self.kind(left), self.kind(right)) {
                            (Some(l), Some(r)) if l == "float" || r == "float" => Some("float".into()),
                            (Some(l), Some(r)) if l == "int" && r == "int" => int(),
                            _ => None,
                        }
                    }
                    B::Eq | B::NotEq | B::Lt | B::LtEq | B::Gt | B::GtEq | B::And | B::Or => int(),
                    _ => None,
                }
            }
            E::Cast { data_type, .. } => crate::sqlite::kind_of_decl(&data_type.to_string()),
            E::Case { conditions, else_result, .. } => {
                let mut k = None;
                for c in conditions {
                    k = unify(k, self.kind(&c.result));
                }
                if let Some(e) = else_result {
                    k = unify(k, self.kind(e));
                }
                k
            }
            E::IsNull(_) | E::IsNotNull(_) | E::InList { .. } | E::Between { .. } | E::Like { .. }
            | E::ILike { .. } | E::Exists { .. } | E::InSubquery { .. } => int(),
            E::Function(f) => self.function(f),
            _ => None,
        }
    }

    /// SQLite's functions by what they return.
    fn function(&self, f: &sql::Function) -> Option<String> {
        let name = last_part(&f.name)?.to_lowercase();
        let args: Vec<&sql::Expr> = match &f.args {
            sql::FunctionArguments::List(list) => list
                .args
                .iter()
                .filter_map(|a| match a {
                    sql::FunctionArg::Unnamed(sql::FunctionArgExpr::Expr(e)) => Some(e),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let arg = |i: usize| args.get(i).and_then(|e| self.kind(e));
        let all_args = || args.iter().fold(None, |k, e| unify(k, self.kind(e)));
        match name.as_str() {
            "count" | "length" | "octet_length" | "instr" | "unicode" | "random" | "changes"
            | "total_changes" | "last_insert_rowid" | "sign" | "unixepoch" => Some("int".into()),
            "round" | "avg" | "total" | "julianday" | "exp" | "ln" | "log" | "log10" | "log2" | "pow"
            | "power" | "sqrt" | "pi" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
            | "degrees" | "radians" => Some("float".into()),
            "upper" | "lower" | "trim" | "ltrim" | "rtrim" | "substr" | "substring" | "replace"
            | "printf" | "format" | "group_concat" | "string_agg" | "char" | "hex" | "quote" | "date"
            | "time" | "datetime" | "strftime" | "typeof" | "soundex" | "concat" | "concat_ws" => {
                Some("str".into())
            }
            "abs" | "sum" | "ceil" | "ceiling" | "floor" | "trunc" | "nullif" | "likely" | "unlikely" => arg(0),
            "min" | "max" | "coalesce" | "ifnull" => all_args(),
            "iif" => unify(arg(1), arg(2)),
            _ => None,
        }
    }
}

/// Which columns of a statement's rows can hold NULL, in order — one entry
/// per column it returns, wildcards expanded — as the statement and its
/// tables' declarations say. A column read straight from a table can be
/// NULL unless it is declared NOT NULL, and every column of a table an
/// outer join can leave unmatched can be; an expression can be NULL when
/// what it reads can, except where SQL says it cannot (`count`, `coalesce`
/// with an argument that cannot, `IS NULL`, a literal), and a function this
/// does not know can always be. `None` for a statement this does not read:
/// the caller takes every column as able to be NULL. `columns_of(table)`
/// answers a table's columns in declared order, each with whether it is
/// declared NOT NULL (empty for a table it does not know).
pub fn select_nulls(text: &str, postgres: bool, columns_of: &dyn Fn(&str) -> Vec<(String, bool)>) -> Option<Vec<bool>> {
    let stmts = if postgres {
        SqlParser::parse_sql(&PostgreSqlDialect {}, &numbered(text)).ok()?
    } else {
        SqlParser::parse_sql(&SQLiteDialect {}, text).ok()?
    };
    let nulls = Nulls { postgres, columns_of, valued: Default::default() };
    let none = BTreeMap::new();
    let columns = match stmts.first()? {
        sql::Statement::Query(q) => nulls.query(q, &none)?,
        // RETURNING reads the row written: the target table's columns
        sql::Statement::Insert(i) => match &i.table {
            sql::TableObject::TableName(name) => nulls.returning(name, i.returning.as_ref()?)?,
            _ => return None,
        },
        sql::Statement::Update(u) => match &u.table.relation {
            sql::TableFactor::Table { name, .. } => nulls.returning(name, u.returning.as_ref()?)?,
            _ => return None,
        },
        sql::Statement::Delete(d) => {
            let from = match &d.from {
                sql::FromTable::WithFromKeyword(f) | sql::FromTable::WithoutKeyword(f) => f,
            };
            match from.first().map(|t| &t.relation) {
                Some(sql::TableFactor::Table { name, .. }) => nulls.returning(name, d.returning.as_ref()?)?,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(columns.into_iter().map(|(_, can)| can).collect())
}

/// `?` placeholders as Postgres numbers them (`$1`, `$2`, …), outside quotes
fn numbered(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut n, mut quote) = (0, None);
    for c in text.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '?') => {
                n += 1;
                out.push_str(&format!("${}", n));
                continue;
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

/// A table's name as the database is asked for it: its parts joined
fn full_name(name: &sql::ObjectName) -> Option<String> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|p| match p {
            sql::ObjectNamePart::Identifier(i) => Some(i.value.clone()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(parts.join("."))
}

/// The columns a WHERE clause's conjuncts say have a value in every row it
/// keeps, as (table or alias, name): `x IS NOT NULL`, and a comparison
/// (`x = ?`, `x > 0`, `x IN (…)`, `x LIKE …`, `x BETWEEN …`), which a NULL
/// never passes. An expression that is not a column (`d.symbols[1] IS NOT
/// NULL`) goes in `exprs`, as its SQL text.
fn has_values(e: &sql::Expr, out: &mut Vec<(Option<String>, String)>, exprs: &mut BTreeSet<String>) {
    use sql::BinaryOperator as B;
    use sql::Expr as E;
    let mut valued = |x: &sql::Expr| match x {
        E::Identifier(i) => out.push((None, i.value.to_lowercase())),
        E::CompoundIdentifier(p) if p.len() >= 2 => {
            out.push((Some(p[p.len() - 2].value.to_lowercase()), p[p.len() - 1].value.to_lowercase()))
        }
        E::Value(_) => {}
        other => {
            exprs.insert(other.to_string());
        }
    };
    match e {
        E::Nested(x) => has_values(x, out, exprs),
        E::BinaryOp { left, op: B::And, right } => {
            has_values(left, out, exprs);
            has_values(right, out, exprs);
        }
        E::BinaryOp { left, op: B::Eq | B::NotEq | B::Lt | B::Gt | B::LtEq | B::GtEq, right } => {
            valued(left);
            valued(right);
        }
        E::IsNotNull(x) | E::InList { expr: x, .. } | E::Between { expr: x, .. } | E::Like { expr: x, .. } | E::ILike { expr: x, .. } => {
            valued(x)
        }
        _ => {}
    }
}

/// A query's columns by name, each with whether it can be NULL
type Columns = Vec<(String, bool)>;
/// The common table expressions in view, by name
type Ctes = BTreeMap<String, Columns>;

/// A table, a subquery or a CTE the statement reads rows from: the names it
/// is read by and its columns
struct Relation {
    names: Vec<String>,
    columns: Columns,
}

impl Relation {
    /// On the side of an outer join that can be left unmatched: every column
    /// can be NULL
    fn may_be_unmatched(&mut self) {
        for c in &mut self.columns {
            c.1 = true;
        }
    }
}

struct Nulls<'a> {
    postgres: bool,
    columns_of: &'a dyn Fn(&str) -> Vec<(String, bool)>,
    /// the expressions the SELECT being read (innermost last) says have a
    /// value in the rows its WHERE keeps, as their SQL text
    valued: std::cell::RefCell<Vec<BTreeSet<String>>>,
}

impl Nulls<'_> {
    fn table(&self, name: &sql::ObjectName) -> Option<Columns> {
        let declared = (self.columns_of)(&full_name(name)?);
        if declared.is_empty() {
            return None;
        }
        Some(declared.into_iter().map(|(n, not_null)| (n.to_lowercase(), !not_null)).collect())
    }

    fn returning(&self, table: &sql::ObjectName, items: &[sql::SelectItem]) -> Option<Columns> {
        let scope = vec![Relation { names: vec![last_part(table)?.to_lowercase()], columns: self.table(table)? }];
        self.items(items, &scope, false, &BTreeMap::new())
    }

    fn query(&self, q: &sql::Query, outer: &Ctes) -> Option<Columns> {
        let mut ctes = outer.clone();
        if let Some(with) = &q.with {
            // a recursive CTE reads itself
            if with.recursive {
                return None;
            }
            for cte in &with.cte_tables {
                let mut columns = self.query(&cte.query, &ctes)?;
                for (col, named) in columns.iter_mut().zip(&cte.alias.columns) {
                    col.0 = named.name.value.to_lowercase();
                }
                ctes.insert(cte.alias.name.value.to_lowercase(), columns);
            }
        }
        self.body(&q.body, &ctes)
    }

    fn body(&self, body: &sql::SetExpr, ctes: &Ctes) -> Option<Columns> {
        match body {
            sql::SetExpr::Select(s) => self.select(s, ctes),
            sql::SetExpr::Query(q) => self.query(q, ctes),
            sql::SetExpr::SetOperation { left, op, right, .. } => {
                let l = self.body(left, ctes)?;
                let r = self.body(right, ctes)?;
                if l.len() != r.len() {
                    return None;
                }
                // a union's rows come from both sides; the others' from the left
                let both = matches!(op, sql::SetOperator::Union);
                Some(l.into_iter().zip(r).map(|((n, a), (_, b))| (n, a || (both && b))).collect())
            }
            _ => None,
        }
    }

    fn select(&self, s: &sql::Select, ctes: &Ctes) -> Option<Columns> {
        let mut scope: Vec<Relation> = Vec::new();
        for from in &s.from {
            let start = scope.len();
            scope.push(self.relation(&from.relation, ctes)?);
            for j in &from.joins {
                use sql::JoinOperator as J;
                let mut r = self.relation(&j.relation, ctes)?;
                match &j.join_operator {
                    J::Left(_) | J::LeftOuter(_) | J::OuterApply => r.may_be_unmatched(),
                    J::Right(_) | J::RightOuter(_) => scope[start..].iter_mut().for_each(Relation::may_be_unmatched),
                    J::FullOuter(_) => {
                        r.may_be_unmatched();
                        scope[start..].iter_mut().for_each(Relation::may_be_unmatched);
                    }
                    J::Join(_) | J::Inner(_) | J::CrossJoin(_) | J::CrossApply => {}
                    _ => return None,
                }
                scope.push(r);
            }
        }
        // a column the WHERE clause compares, or asks IS NOT NULL of, has a
        // value in every row it keeps, and so does an expression it does
        let mut exprs = BTreeSet::new();
        if let Some(w) = &s.selection {
            let mut valued = Vec::new();
            has_values(w, &mut valued, &mut exprs);
            for (table, name) in valued {
                for r in scope.iter_mut().filter(|r| table.as_ref().map_or(true, |t| r.names.contains(t))) {
                    for c in r.columns.iter_mut().filter(|(n, _)| *n == name) {
                        c.1 = false;
                    }
                }
            }
        }
        let grouped = match &s.group_by {
            sql::GroupByExpr::All(_) => true,
            sql::GroupByExpr::Expressions(e, _) => !e.is_empty(),
        };
        self.valued.borrow_mut().push(exprs);
        let out = self.items(&s.projection, &scope, grouped, ctes);
        self.valued.borrow_mut().pop();
        out
    }

    fn relation(&self, f: &sql::TableFactor, ctes: &Ctes) -> Option<Relation> {
        match f {
            sql::TableFactor::Table { name, alias, .. } => {
                let table = last_part(name)?.to_lowercase();
                let columns = match ctes.get(&table) {
                    Some(c) if name.0.len() == 1 => c.clone(),
                    _ => self.table(name)?,
                };
                let mut names = vec![table];
                if let Some(a) = alias {
                    names.push(a.name.value.to_lowercase());
                }
                Some(Relation { names, columns })
            }
            sql::TableFactor::Derived { subquery, alias, .. } => {
                let mut columns = self.query(subquery, ctes)?;
                let mut names = Vec::new();
                if let Some(a) = alias {
                    names.push(a.name.value.to_lowercase());
                    for (col, named) in columns.iter_mut().zip(&a.columns) {
                        col.0 = named.name.value.to_lowercase();
                    }
                }
                Some(Relation { names, columns })
            }
            _ => None,
        }
    }

    fn items(&self, items: &[sql::SelectItem], scope: &[Relation], grouped: bool, ctes: &Ctes) -> Option<Columns> {
        let mut out = Vec::new();
        for item in items {
            match item {
                sql::SelectItem::UnnamedExpr(e) => {
                    let name = match e {
                        sql::Expr::Identifier(i) => i.value.to_lowercase(),
                        sql::Expr::CompoundIdentifier(p) => p.last().map(|i| i.value.to_lowercase()).unwrap_or_default(),
                        _ => String::new(),
                    };
                    out.push((name, self.expr(e, scope, grouped, ctes)));
                }
                sql::SelectItem::ExprWithAlias { expr, alias } => out.push((alias.value.to_lowercase(), self.expr(expr, scope, grouped, ctes))),
                sql::SelectItem::Wildcard(_) => {
                    for r in scope {
                        out.extend(r.columns.iter().cloned());
                    }
                }
                sql::SelectItem::QualifiedWildcard(sql::SelectItemQualifiedWildcardKind::ObjectName(n), _) => {
                    let name = last_part(n)?.to_lowercase();
                    out.extend(scope.iter().find(|r| r.names.contains(&name))?.columns.iter().cloned());
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// Whether a column the statement reads can be NULL. A name the scope
    /// does not hold (an outer query's column) can be.
    fn column(&self, scope: &[Relation], table: Option<&str>, name: &str) -> bool {
        let name = name.to_lowercase();
        let table = table.map(str::to_lowercase);
        let found: Vec<bool> = scope
            .iter()
            .filter(|r| table.as_ref().map_or(true, |t| r.names.contains(t)))
            .flat_map(|r| r.columns.iter().filter(|(n, _)| *n == name).map(|(_, can)| *can))
            .collect();
        found.is_empty() || found.contains(&true)
    }

    /// Whether an expression can be NULL
    fn expr(&self, e: &sql::Expr, scope: &[Relation], grouped: bool, ctes: &Ctes) -> bool {
        use sql::Expr as E;
        let can = |x: &sql::Expr| self.expr(x, scope, grouped, ctes);
        let can_opt = |x: &Option<Box<sql::Expr>>| x.as_ref().map_or(false, |x| can(x));
        if self.valued.borrow().last().map_or(false, |v| v.contains(&e.to_string())) {
            return false;
        }
        match e {
            E::Identifier(i) => self.column(scope, None, &i.value),
            E::CompoundIdentifier(parts) if parts.len() >= 2 => {
                self.column(scope, Some(&parts[parts.len() - 2].value), &parts[parts.len() - 1].value)
            }
            E::Value(v) => matches!(v.value, sql::Value::Null | sql::Value::Placeholder(_)),
            E::TypedString(_) => false,
            E::Nested(x) | E::UnaryOp { expr: x, .. } => can(x),
            E::BinaryOp { left, op, right } => {
                let either = can(left) || can(right);
                match op {
                    // SQLite divides by zero into NULL
                    sql::BinaryOperator::Divide | sql::BinaryOperator::Modulo if !self.postgres => {
                        either || !matches!(right.as_ref(), E::Value(v) if matches!(&v.value, sql::Value::Number(n, _) if n.parse::<f64>().map_or(false, |x| x != 0.0)))
                    }
                    _ => either,
                }
            }
            E::IsNull(_) | E::IsNotNull(_) | E::IsTrue(_) | E::IsNotTrue(_) | E::IsFalse(_) | E::IsNotFalse(_)
            | E::IsUnknown(_) | E::IsNotUnknown(_) | E::IsDistinctFrom(..) | E::IsNotDistinctFrom(..) | E::Exists { .. } => false,
            E::InList { expr, list, .. } => can(expr) || list.iter().any(|x| can(x)),
            E::Between { expr, low, high, .. } => can(expr) || can(low) || can(high),
            E::Like { expr, pattern, .. } | E::ILike { expr, pattern, .. } => can(expr) || can(pattern),
            E::Cast { kind, expr, .. } => !matches!(kind, sql::CastKind::Cast | sql::CastKind::DoubleColon) || can(expr),
            E::Case { conditions, else_result, .. } => {
                else_result.as_ref().map_or(true, |x| can(x)) || conditions.iter().any(|c| can(&c.result))
            }
            E::Ceil { expr, .. } | E::Floor { expr, .. } | E::Extract { expr, .. } => can(expr),
            E::Substring { expr, substring_from, substring_for, .. } => can(expr) || can_opt(substring_from) || can_opt(substring_for),
            E::Trim { expr, trim_what, .. } => can(expr) || can_opt(trim_what),
            E::Function(f) => self.function(f, scope, grouped, ctes),
            _ => true,
        }
    }

    /// Whether a function's value can be NULL: SQL's scalar functions are
    /// NULL where an argument is; an aggregate over a group of values that
    /// cannot be NULL is not (a group has a row), and without a GROUP BY it
    /// is NULL over no rows at all, but for `count` and `total`
    fn function(&self, f: &sql::Function, scope: &[Relation], grouped: bool, ctes: &Ctes) -> bool {
        let Some(name) = last_part(&f.name).map(|n| n.to_lowercase()) else { return true };
        let args: Vec<&sql::Expr> = match &f.args {
            sql::FunctionArguments::List(list) => list
                .args
                .iter()
                .filter_map(|a| match a {
                    sql::FunctionArg::Unnamed(sql::FunctionArgExpr::Expr(e)) => Some(e),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let can = |x: &sql::Expr| self.expr(x, scope, grouped, ctes);
        let any = || args.iter().any(|a| can(a));
        if f.over.is_some() {
            return !matches!(name.as_str(), "row_number" | "rank" | "dense_rank" | "percent_rank" | "cume_dist" | "ntile" | "count");
        }
        match name.as_str() {
            "count" | "total" | "changes" | "total_changes" | "last_insert_rowid" | "random" | "concat" | "concat_ws"
            | "quote" | "typeof" | "now" => false,
            "coalesce" | "ifnull" => args.iter().all(|a| can(a)),
            "iif" => args.get(1).map_or(true, |a| can(a)) || args.get(2).map_or(true, |a| can(a)),
            // min and max of two or more values are scalar
            "min" | "max" if args.len() >= 2 => any(),
            "sum" | "avg" | "min" | "max" | "group_concat" | "string_agg" => !grouped || any(),
            "abs" | "round" | "upper" | "lower" | "length" | "octet_length" | "trim" | "ltrim" | "rtrim" | "substr"
            | "substring" | "replace" | "instr" | "hex" | "unicode" | "sign" | "ceil" | "ceiling" | "floor" | "trunc"
            | "char" | "printf" | "format" | "lpad" | "rpad" | "left" | "right" | "reverse" | "md5" | "to_char"
            | "date_trunc" | "date_part" => any(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trades(table: &str) -> Vec<(String, String)> {
        let cols: &[(&str, &str)] = match table {
            "trades" => &[("id", "INTEGER"), ("session_id", "INTEGER"), ("symbol", "TEXT"), ("shares", "INTEGER"), ("entry_px", "REAL"), ("exit_px", "REAL"), ("fallback_fill", "INTEGER")],
            "sessions" => &[("id", "INTEGER"), ("started_at", "REAL"), ("mode", "TEXT")],
            _ => &[],
        };
        cols.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()
    }

    fn kinds(text: &str) -> Vec<Option<String>> {
        select_kinds(text, &trades).expect("a select")
    }

    fn k(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    /// finch's blotter: the P&L computed where the rows are is a number,
    /// though no row answers `session_id = ?` when the program is built
    #[test]
    fn a_computed_column_is_typed_from_the_statement() {
        assert_eq!(
            kinds("SELECT symbol, shares, entry_ts, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE session_id = ? ORDER BY id"),
            vec![k("str"), k("int"), None, k("float")]
        );
    }

    /// Aliases, joins, aggregates, text built with `||`, CASE and CAST — the
    /// history query's shapes
    #[test]
    fn the_shapes_of_a_report_query() {
        let got = kinds("SELECT s.id, datetime(s.started_at, 'unixepoch') AS \"when\", count(t.id) AS trades, sum(CASE WHEN t.exit_px > t.entry_px THEN 1 ELSE 0 END) || '/' || 2 AS wl, round(coalesce(sum((t.exit_px - t.entry_px) * t.shares), 0), 2) AS pnl, cast(round(100.0 * 3 / 4) AS INTEGER) AS pct, coalesce(min(t.symbol), '') AS symbol, t.exit_px IS NULL AS open, ? AS label, NULL AS nothing FROM sessions s LEFT JOIN trades t ON t.session_id = s.id GROUP BY s.id");
        assert_eq!(got, vec![k("int"), k("str"), k("int"), k("str"), k("float"), k("int"), k("str"), k("int"), None, None]);
    }

    /// finch's tables as declared: which columns are NOT NULL
    fn declared(table: &str) -> Vec<(String, bool)> {
        let cols: &[(&str, bool)] = match table {
            "trades" => &[("id", true), ("session_id", true), ("symbol", true), ("shares", true), ("entry_px", true), ("exit_ts", false), ("exit_px", false)],
            "sessions" | "main.sessions" => &[("id", true), ("started_at", true), ("ended_at", false), ("mode", true)],
            _ => &[],
        };
        cols.iter().map(|(n, nn)| (n.to_string(), *nn)).collect()
    }

    fn nulls(text: &str) -> Vec<bool> {
        select_nulls(text, false, &declared).expect("a statement read")
    }

    /// A column read straight from a table can be NULL as it is declared;
    /// an expression as what it reads, but for what SQL says
    #[test]
    fn a_column_can_be_null_as_its_table_declares() {
        assert_eq!(
            nulls("SELECT symbol, exit_px, round((exit_px - entry_px) * shares, 2) AS pnl, shares * 2, exit_px IS NULL AS open, coalesce(exit_px, entry_px), ?, 'x', NULL FROM trades WHERE session_id = ?"),
            vec![false, true, true, false, false, false, true, false, true]
        );
        // an aggregate over no rows is NULL, over a group it is not
        assert_eq!(nulls("SELECT count(*), sum(shares), total(exit_px) FROM trades"), vec![false, true, false]);
        assert_eq!(nulls("SELECT symbol, sum(shares), max(exit_px) FROM trades GROUP BY symbol"), vec![false, false, true]);
        // SQLite divides by zero into NULL; Postgres refuses
        assert_eq!(nulls("SELECT shares / 2, shares / shares FROM trades"), vec![false, true]);
        assert_eq!(select_nulls("SELECT shares / shares FROM trades WHERE id = ?", true, &declared), Some(vec![false]));
        // a function this does not know can always be NULL
        assert_eq!(nulls("SELECT datetime(entry_px, 'unixepoch'), upper(symbol) FROM trades"), vec![true, false]);
    }

    /// Every column of the side an outer join can leave unmatched can be NULL
    #[test]
    fn an_outer_join_leaves_a_side_unmatched() {
        let q = "SELECT s.id, s.mode, t.symbol, t.exit_px, count(t.id) AS n FROM sessions s LEFT JOIN trades t ON t.session_id = s.id GROUP BY s.id";
        assert_eq!(nulls(q), vec![false, false, true, true, false]);
        assert_eq!(nulls("SELECT s.mode, t.symbol FROM sessions s RIGHT JOIN trades t ON t.session_id = s.id"), vec![true, false]);
        assert_eq!(nulls("SELECT s.mode, t.symbol FROM sessions s JOIN trades t ON t.session_id = s.id"), vec![false, false]);
        assert_eq!(nulls("SELECT s.mode, t.symbol FROM main.sessions s FULL OUTER JOIN trades t ON t.session_id = s.id"), vec![true, true]);
    }

    /// Wildcards expand in the tables' order; subqueries, CTEs, unions and
    /// RETURNING are read through
    #[test]
    fn wildcards_subqueries_and_returning() {
        assert_eq!(nulls("SELECT * FROM sessions"), vec![false, false, true, false]);
        assert_eq!(nulls("SELECT t.*, s.ended_at FROM trades t JOIN sessions s ON s.id = t.session_id").len(), 8);
        assert_eq!(nulls("SELECT x.px FROM (SELECT coalesce(exit_px, 0) AS px FROM trades) x"), vec![false]);
        assert_eq!(nulls("WITH o AS (SELECT symbol, exit_px FROM trades) SELECT symbol, exit_px FROM o"), vec![false, true]);
        assert_eq!(nulls("SELECT symbol, entry_px FROM trades UNION ALL SELECT mode, ended_at FROM sessions"), vec![false, true]);
        assert_eq!(nulls("INSERT INTO trades (session_id, symbol, shares, entry_px) VALUES (?, ?, ?, ?) RETURNING id, exit_px"), vec![false, true]);
        assert_eq!(nulls("UPDATE sessions SET ended_at = ? WHERE id = ? RETURNING ended_at, mode"), vec![true, false]);
        // the rows WHERE keeps have a value in what it compares
        assert_eq!(nulls("SELECT exit_px, exit_ts FROM trades WHERE exit_px IS NOT NULL AND session_id = ?"), vec![false, true]);
        assert_eq!(nulls("SELECT round(exit_px, 2) AS px, round(exit_ts, 2) FROM trades WHERE round(exit_px, 2) IS NOT NULL"), vec![false, true]);
        assert_eq!(select_nulls("SELECT d.symbols[1] AS symbol FROM trades d WHERE d.symbols[1] IS NOT NULL", true, &declared), Some(vec![false]));
        assert_eq!(nulls("SELECT t.exit_px FROM sessions s LEFT JOIN trades t ON t.session_id = s.id WHERE t.exit_px > 0 OR s.id = 1"), vec![true]);
        assert_eq!(nulls("SELECT s.ended_at, t.symbol FROM sessions s LEFT JOIN trades t ON t.session_id = s.id WHERE s.ended_at < ? AND t.symbol IN ('A', 'B')"), vec![false, false]);
        // a table nothing declares, a write without RETURNING: not read here
        assert!(select_nulls("SELECT a FROM elsewhere", false, &declared).is_none());
        assert!(select_nulls("UPDATE trades SET shares = 1", false, &declared).is_none());
        assert_eq!(numbered("SELECT '?' || ? FROM t WHERE a = ?"), "SELECT '?' || $1 FROM t WHERE a = $2");
    }

    /// A compound query is typed by its first SELECT; a wildcard is left to
    /// the database; a statement that is not a query is not typed here
    #[test]
    fn compound_queries_wildcards_and_writes() {
        assert_eq!(kinds("SELECT entry_px AS y, ? AS label FROM trades UNION ALL SELECT exit_px, ? FROM trades"), vec![k("float"), None]);
        assert!(select_kinds("SELECT * FROM trades", &trades).is_none());
        assert!(select_kinds("UPDATE trades SET shares = 1", &trades).is_none());
        assert_eq!(unify(k("int"), k("float")), k("float"));
        assert_eq!(unify(k("str"), k("int")), k("str"));
    }
}
