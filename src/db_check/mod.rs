/// Schema-aware database checks for `crow check` (Level 1).
///
/// Cross-references the program's SQL and the dataflow around it against the
/// actual database schema, and reports concurrency hazards in plain language
/// with a ranked menu of fixes. Advisory by default; `--strict` makes hazards
/// fail the check.

pub mod catalog;
pub mod fix;
pub mod report;
pub mod schema;
pub mod slices;
pub mod sql_facts;

pub struct RunResult {
    pub hazards: Vec<Hazard>,
    pub sql_calls: usize,
    pub schema_checked: bool,
}


/// Map connection variables to their connect[] urls, following one level
/// of parameter passing: `fill_at[ch, ...]` binds fill_at's first param
/// to ch's url when every call site agrees.
fn connection_urls(stmts: &[crate::ast::Stmt]) -> Vec<(String, String)> {
    use crate::ast::{Expr, Stmt};
    let mut direct: Vec<(String, String)> = Vec::new();
    fn scan(stmts: &[Stmt], out: &mut Vec<(String, String)>) {
        for st in stmts {
            match st {
                Stmt::Assign { name, value } => {
                    if let Expr::Call { callee, args } = value {
                        let is_connect = match callee.as_ref() {
                            Expr::Ident(n) => n == "connect",
                            Expr::FieldAccess { field, .. } => field == "connect",
                            _ => false,
                        };
                        if is_connect {
                            if let Some(Expr::Str(u)) = args.first() {
                                out.push((name.clone(), u.clone()));
                            }
                        }
                    }
                }
                Stmt::If { then_body, else_body, .. } => {
                    scan(then_body, out);
                    if let Some(eb) = else_body {
                        scan(eb, out);
                    }
                }
                Stmt::FnDecl { body, .. } => scan(body, out),
                _ => {}
            }
        }
    }
    scan(stmts, &mut direct);

    // One-hop param resolution: for each fn whose param receives exactly
    // one known connection var across all call sites, bind param → url.
    let by_var: std::collections::BTreeMap<&str, &str> = direct
        .iter()
        .map(|(v, u)| (v.as_str(), u.as_str()))
        .collect();
    let mut fn_params: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for st in stmts {
        if let Stmt::FnDecl { name, params, .. } = st {
            fn_params.insert(name.clone(), params.clone());
        }
    }
    let mut param_urls: std::collections::BTreeMap<(String, usize), std::collections::BTreeSet<String>> =
        Default::default();
    fn scan_calls(
        stmts: &[Stmt],
        by_var: &std::collections::BTreeMap<&str, &str>,
        fn_params: &std::collections::BTreeMap<String, Vec<String>>,
        out: &mut std::collections::BTreeMap<(String, usize), std::collections::BTreeSet<String>>,
    ) {
        use crate::ast::{Expr, Stmt};
        fn walk(
            e: &Expr,
            by_var: &std::collections::BTreeMap<&str, &str>,
            fn_params: &std::collections::BTreeMap<String, Vec<String>>,
            out: &mut std::collections::BTreeMap<(String, usize), std::collections::BTreeSet<String>>,
        ) {
            if let Expr::Call { callee, args } = e {
                if let Expr::Ident(fname) = callee.as_ref() {
                    if fn_params.contains_key(fname) {
                        for (i, a) in args.iter().enumerate() {
                            if let Expr::Ident(v) = a {
                                if let Some(u) = by_var.get(v.as_str()) {
                                    out.entry((fname.clone(), i)).or_default().insert(u.to_string());
                                }
                            }
                        }
                    }
                }
                for a in args {
                    walk(a, by_var, fn_params, out);
                }
                walk(callee, by_var, fn_params, out);
            } else if let Expr::BinOp { left, right, .. } = e {
                walk(left, by_var, fn_params, out);
                walk(right, by_var, fn_params, out);
            } else if let Expr::Named { value, .. } = e {
                walk(value, by_var, fn_params, out);
            }
        }
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } => walk(value, by_var, fn_params, out),
                Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, by_var, fn_params, out),
                Stmt::If { condition, then_body, else_body, .. } => {
                    walk(condition, by_var, fn_params, out);
                    scan_calls(then_body, by_var, fn_params, out);
                    if let Some(eb) = else_body {
                        scan_calls(eb, by_var, fn_params, out);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    scan_calls(body, by_var, fn_params, out)
                }
                Stmt::FnDecl { body, .. } => scan_calls(body, by_var, fn_params, out),
                _ => {}
            }
        }
    }
    scan_calls(stmts, &by_var, &fn_params, &mut param_urls);

    let mut all = direct.clone();
    for ((fname, idx), urls) in &param_urls {
        if urls.len() == 1 {
            if let Some(params) = fn_params.get(fname) {
                if let Some(pname) = params.get(*idx) {
                    all.push((pname.clone(), urls.iter().next().unwrap().clone()));
                }
            }
        }
    }
    all
}

/// Run the database checks over a source file. `db_url` comes from
/// `--db` or the crowfile `db` variable; without it only the checks that
/// need no schema run (injection flags).
pub fn run(file: &str, source: &str, db_url: Option<&str>) -> Result<RunResult, String> {
    // Lenient parse: db checks should still cover the healthy statements of
    // a file that has an unrelated syntax error (the LSP lives in that state).
    let tokens = crate::lexer::Lexer::new(source).tokenize()?;
    let (stmts, _parse_errors) = crate::parser::Parser::new(tokens).parse_lenient();
    let facts = sql_facts::collect(&stmts);
    let total_sql_calls = facts.calls.len();
    let mut hazards = Vec::new();
    let mut schema_checked = false;
    match db_url {
        Some(url) => {
            // Offline mode: read the schema from migration files instead of
            // a live database — `--db migrations://dir` or a plain directory.
            let sch = if let Some(dir) = url.strip_prefix("migrations://") {
                schema::from_sql_dir(dir)?
            } else if std::path::Path::new(url).is_dir() {
                schema::from_sql_dir(url)?
            } else {
                let db = crow_db::connect(url, None)?;
                schema::introspect(&db)?
            };
            // Multi-database files: schema checks apply only to queries on
            // the connection whose connect[] url is the checked db. Other
            // connections (and conns not statically resolvable) are out of
            // scope — checking a ClickHouse query against the sqlite schema
            // reports fictions.
            let conn_urls = connection_urls(&stmts);
            let checked: std::collections::BTreeSet<String> = conn_urls
                .iter()
                .filter(|(_, u)| u.as_str() == url)
                .map(|(v, _)| v.clone())
                .collect();
            let facts = if conn_urls.len() > 1 {
                let mut scoped = facts;
                let before = scoped.calls.len();
                scoped.calls.retain(|c| match &c.conn_var {
                    Some(v) => checked.contains(v),
                    None => true,
                });
                let skipped = before - scoped.calls.len();
                if skipped > 0 {
                    eprintln!(
                        "  note: {} quer{} on other connections not checked against '{}' (multi-database file)",
                        skipped,
                        if skipped == 1 { "y" } else { "ies" },
                        url
                    );
                }
                scoped
            } else {
                facts
            };
            hazards.extend(detect(&stmts, &facts, &sch, file));
            schema_checked = true;
        }
        None => {
            // Schema-free findings only.
            for call in &facts.calls {
                if call.interpolated {
                    hazards.push(Hazard {
                        kind: HazardKind::InterpolatedSql,
                        file: file.into(),
                        span: (call.line, call.line),
                    });
                }
            }
        }
    }
    let hazards = suppress(hazards, source);
    Ok(RunResult {
        hazards,
        sql_calls: total_sql_calls,
        schema_checked,
    })
}

/// All detectors over parsed facts + schema. Shared by run() and tests.
fn detect(
    stmts: &[crate::ast::Stmt],
    facts: &sql_facts::Facts,
    sch: &schema::Schema,
    file: &str,
) -> Vec<Hazard> {
    let mut hazards = catalog::verify(facts, sch, file);
    hazards.extend(slices::analyze(stmts, facts, sch, file));
    hazards.extend(catalog::latch(facts, file));
    hazards.extend(catalog::unpaired_writes(facts, file));
    hazards
}

/// Drop concurrency hazards acknowledged with a `--- crow: single-writer`
/// comment on the line above the hazard's first line.
fn suppress(hazards: Vec<Hazard>, source: &str) -> Vec<Hazard> {
    let lines: Vec<&str> = source.lines().collect();
    hazards
        .into_iter()
        .filter(|h| {
            if !h.kind.is_concurrency() {
                return true;
            }
            let above = h.span.0.checked_sub(2).and_then(|i| lines.get(i));
            !matches!(above, Some(l) if l.contains("crow: single-writer"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an in-memory schema from a fixture's .schema.sql and run the
    /// 1f verification detectors over its .co program.
    fn verify_fixture(name: &str) -> Vec<Hazard> {
        let src = std::fs::read_to_string(format!("tests/db_check/{}.co", name)).unwrap();
        let ddl =
            std::fs::read_to_string(format!("tests/db_check/{}.schema.sql", name)).unwrap();
        let db = crow_db::connect("sqlite://:memory:", None).unwrap();
        for stmt in ddl.split(';').filter(|s| !s.trim().is_empty()) {
            db.exec(stmt, &[]).unwrap();
        }
        let sch = schema::introspect(&db).unwrap();
        let stmts = crate::build(&src).unwrap();
        let facts = sql_facts::collect(&stmts);
        catalog::verify(&facts, &sch, &format!("{}.co", name))
    }

    #[test]
    fn verify_fixture_findings() {
        let hazards = verify_fixture("verify");
        let kinds: Vec<String> = hazards.iter().map(|h| format!("{:?}", h.kind)).collect();
        assert!(
            kinds.iter().any(|k| k.contains("UnknownColumn") && k.contains("nmae") && k.contains("name")),
            "expected nmae→name suggestion, got {:?}", kinds
        );
        assert!(
            kinds.iter().any(|k| k.contains("UnknownTable") && k.contains("usrs") && k.contains("users")),
            "expected usrs→users suggestion, got {:?}", kinds
        );
        assert!(
            kinds.iter().any(|k| k.contains("ParamCountMismatch")),
            "expected param count mismatch, got {:?}", kinds
        );
        assert!(
            kinds.iter().any(|k| k.contains("InterpolatedSql")),
            "expected interpolated SQL flag, got {:?}", kinds
        );
    }

    /// Full run (verify + slices) against a fixture with optional extra DDL.
    fn full_check(name: &str, extra_ddl: &str) -> Vec<Hazard> {
        let src = std::fs::read_to_string(format!("tests/db_check/{}.co", name)).unwrap();
        let ddl =
            std::fs::read_to_string(format!("tests/db_check/{}.schema.sql", name)).unwrap();
        let db = crow_db::connect("sqlite://:memory:", None).unwrap();
        for stmt in ddl
            .split(';')
            .chain(extra_ddl.split(';'))
            .filter(|s| !s.trim().is_empty())
        {
            db.exec(stmt, &[]).unwrap();
        }
        let sch = schema::introspect(&db).unwrap();
        let stmts = crate::build(&src).unwrap();
        let facts = sql_facts::collect(&stmts);
        suppress(detect(&stmts, &facts, &sch, &format!("{}.co", name)), &src)
    }

    #[test]
    fn write_skew_fires_without_constraint() {
        let hazards = full_check("write_skew", "");
        let skews: Vec<_> = hazards
            .iter()
            .filter(|h| matches!(h.kind, HazardKind::WriteSkew { .. }))
            .collect();
        assert_eq!(skews.len(), 1, "expected exactly one write-skew hazard");
        if let HazardKind::WriteSkew {
            table,
            key_cols,
            partial,
            invariant,
            ..
        } = &skews[0].kind
        {
            assert_eq!(table, "trades");
            assert_eq!(key_cols, &vec!["symbol".to_string()]);
            assert_eq!(partial, &vec![("status".to_string(), "active".to_string())]);
            assert_eq!(invariant, "at most one active trades row per symbol");
        }
    }

    #[test]
    fn write_skew_silent_with_partial_unique_index() {
        let hazards = full_check(
            "write_skew",
            "CREATE UNIQUE INDEX one_active ON trades(symbol) WHERE status = 'active'",
        );
        assert!(
            !hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::WriteSkew { .. })),
            "partial unique index should satisfy the invariant"
        );
    }

    #[test]
    fn write_skew_silent_with_covering_full_index() {
        let hazards = full_check(
            "write_skew",
            "CREATE UNIQUE INDEX sym_status ON trades(symbol, status)",
        );
        assert!(
            !hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::WriteSkew { .. })),
            "full unique index over key+predicate should satisfy the invariant"
        );
    }

    fn kinds_of(hazards: &[Hazard]) -> Vec<String> {
        hazards.iter().map(|h| format!("{:?}", h.kind)).collect()
    }

    #[test]
    fn lost_update_fires() {
        let hazards = full_check("lost_update", "");
        assert!(
            hazards
                .iter()
                .any(|h| matches!(&h.kind, HazardKind::LostUpdate { table, column, .. }
                    if table == "accounts" && column == "balance")),
            "expected lost-update on accounts.balance, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn latch_fires() {
        let hazards = full_check("latch", "");
        assert!(
            hazards
                .iter()
                .any(|h| matches!(&h.kind, HazardKind::OneWayLatch { table, column, set_value, .. }
                    if table == "rooms" && column == "checked_out" && set_value == "1")),
            "expected one-way latch on rooms.checked_out, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn assumed_unique_fires() {
        let hazards = full_check("assumed_unique", "");
        assert!(
            hazards
                .iter()
                .any(|h| matches!(&h.kind, HazardKind::AssumedUnique { table, column, .. }
                    if table == "users" && column == "email")),
            "expected assumed-unique on users.email, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn assumed_unique_silent_when_unique() {
        let hazards = full_check(
            "assumed_unique",
            "CREATE UNIQUE INDEX uniq_email ON users(email)",
        );
        assert!(
            !hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::AssumedUnique { .. })),
            "unique index should silence assumed-unique, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn unpaired_writes_fires() {
        let hazards = full_check("unpaired", "");
        assert!(
            hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::UnpairedWrites { .. })),
            "expected unpaired-writes, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn offline_schema_from_migration_sql() {
        let ddl = std::fs::read_to_string("tests/db_check/write_skew.schema.sql").unwrap();
        let src = std::fs::read_to_string("tests/db_check/write_skew.co").unwrap();
        let stmts = crate::build(&src).unwrap();
        let facts = sql_facts::collect(&stmts);

        // Without the constraint: hazard fires from parsed DDL alone.
        let sch = schema::from_sql(&ddl).unwrap();
        let hazards = detect(&stmts, &facts, &sch, "write_skew.co");
        assert!(
            hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::WriteSkew { .. })),
            "offline schema should reproduce the write-skew hazard"
        );

        // With the partial unique index in the migrations: silent.
        let ddl2 = format!(
            "{}\nCREATE UNIQUE INDEX one_active ON trades(symbol) WHERE status = 'active';",
            ddl
        );
        let sch2 = schema::from_sql(&ddl2).unwrap();
        let hazards2 = detect(&stmts, &facts, &sch2, "write_skew.co");
        assert!(
            !hazards2
                .iter()
                .any(|h| matches!(h.kind, HazardKind::WriteSkew { .. })),
            "offline partial unique index should silence the hazard"
        );
    }

    #[test]
    fn postgres_introspection_env_gated() {
        // Runs only when CROW_PG_URL points at a reachable postgres and the
        // binary is built with the postgres feature; otherwise it's a no-op.
        let Ok(url) = std::env::var("CROW_PG_URL") else { return };
        let Ok(db) = crow_db::connect(&url, None) else { return };
        let sch = schema::introspect(&db).expect("postgres introspection failed");
        assert_eq!(sch.backend, "postgres");
    }

    #[test]
    fn suppression_pragma_silences() {
        let src = std::fs::read_to_string("tests/db_check/write_skew.co").unwrap();
        // Insert the pragma right above the query line.
        let suppressed_src = src.replace(
            "open_trades = query",
            "--- crow: single-writer\nopen_trades = query",
        );
        let ddl = std::fs::read_to_string("tests/db_check/write_skew.schema.sql").unwrap();
        let db = crow_db::connect("sqlite://:memory:", None).unwrap();
        for stmt in ddl.split(';').filter(|s| !s.trim().is_empty()) {
            db.exec(stmt, &[]).unwrap();
        }
        let sch = schema::introspect(&db).unwrap();
        let stmts = crate::build(&suppressed_src).unwrap();
        let facts = sql_facts::collect(&stmts);
        let hazards = suppress(
            detect(&stmts, &facts, &sch, "write_skew.co"),
            &suppressed_src,
        );
        assert!(
            !hazards
                .iter()
                .any(|h| matches!(h.kind, HazardKind::WriteSkew { .. })),
            "pragma should suppress the write-skew hazard, got {:?}",
            kinds_of(&hazards)
        );
    }

    #[test]
    fn clean_fixture_is_clean() {
        let hazards = verify_fixture("clean");
        assert!(
            hazards.is_empty(),
            "clean fixture should produce no findings, got {:?}",
            hazards.iter().map(|h| format!("{:?}", h.kind)).collect::<Vec<_>>()
        );
    }
}

#[derive(Debug, Clone)]
pub struct Hazard {
    pub kind: HazardKind,
    pub file: String,
    /// 1-indexed source line span (start, end) shown in the excerpt.
    pub span: (usize, usize),
}

#[derive(Debug, Clone)]
pub enum HazardKind {
    /// 1a — check-then-act: a condition derived from a read guards a write,
    /// and no schema constraint enforces the invariant the check implies.
    WriteSkew {
        table: String,
        /// Human phrasing of the implied rule, e.g. "at most one active trades row per symbol".
        invariant: String,
        /// Equality-filter columns bound to program values → the unique key.
        key_cols: Vec<String>,
        /// (column, literal) equality filters that become the partial-index predicate.
        partial: Vec<(String, String)>,
        read_line: usize,
        write_line: usize,
    },
    /// 1b — read-modify-write: a value read from a column flows through
    /// arithmetic back into a write of the same column.
    LostUpdate {
        table: String,
        column: String,
        read_line: usize,
        write_line: usize,
    },
    /// 1c — a column only ever written in one direction, gating reads elsewhere.
    OneWayLatch {
        table: String,
        column: String,
        set_value: String,
        write_line: usize,
        gate_line: usize,
    },
    /// 1d — single-row access on a query whose filter column has no unique rule.
    AssumedUnique {
        table: String,
        column: String,
        access_line: usize,
    },
    /// 1e — consecutive writes with no transaction between them.
    UnpairedWrites {
        tables: Vec<String>,
        write_lines: Vec<usize>,
    },
    /// 1f — SQL-vs-schema verification.
    UnknownTable {
        table: String,
        suggestion: Option<String>,
    },
    UnknownColumn {
        table: String,
        column: String,
        suggestion: Option<String>,
    },
    TypeMismatch {
        table: String,
        column: String,
        db_type: String,
        used_as: String,
    },
    ParamCountMismatch {
        placeholders: usize,
        given: usize,
    },
    InterpolatedSql,
}

/// What `crow fix` will do for a chosen option; `report` renders these,
/// the fixer (M4) executes them. One source of truth for the menu.
#[derive(Debug, Clone)]
pub enum FixKind {
    /// Write a migration file adding a (possibly partial) unique index.
    Migration { name: String, sql: String },
    /// Rewrite the slice into one atomic SQL statement.
    Fusion,
    /// Wrap the slice in a retrying transaction.
    TransactWrap,
    /// No mechanical rewrite — guidance only.
    Advice,
}

#[derive(Debug, Clone)]
pub struct FixOption {
    pub headline: String,
    pub body: Vec<String>,
    pub kind: FixKind,
}

impl HazardKind {
    /// Severity bucket controls report ordering: hazards first, then verification.
    pub fn is_concurrency(&self) -> bool {
        matches!(
            self,
            HazardKind::WriteSkew { .. }
                | HazardKind::LostUpdate { .. }
                | HazardKind::OneWayLatch { .. }
                | HazardKind::AssumedUnique { .. }
                | HazardKind::UnpairedWrites { .. }
        )
    }
}
