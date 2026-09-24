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
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser as SqlParser;
use std::collections::BTreeMap;

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
        let kinds: Vec<Option<String>> = declared.iter().map(|t| crow_db::sqlite::kind_of_decl(t)).collect();
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
            E::Cast { data_type, .. } => crow_db::sqlite::kind_of_decl(&data_type.to_string()),
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
