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

/// Run the database checks over a source file. `db_url` comes from
/// `--db` or the crowfile `db` variable; without it only the checks that
/// need no schema run (injection flags).
pub fn run(file: &str, source: &str, db_url: Option<&str>) -> Result<RunResult, String> {
    let stmts = crate::build(source)?;
    let facts = sql_facts::collect(&stmts);
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
        sql_calls: facts.calls.len(),
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
    /// 1f verification detectors over its .crow program.
    fn verify_fixture(name: &str) -> Vec<Hazard> {
        let src = std::fs::read_to_string(format!("tests/db_check/{}.crow", name)).unwrap();
        let ddl =
            std::fs::read_to_string(format!("tests/db_check/{}.schema.sql", name)).unwrap();
        let db = crow_db::connect("sqlite://:memory:", None).unwrap();
        for stmt in ddl.split(';').filter(|s| !s.trim().is_empty()) {
            db.exec(stmt, &[]).unwrap();
        }
        let sch = schema::introspect(&db).unwrap();
        let stmts = crate::build(&src).unwrap();
        let facts = sql_facts::collect(&stmts);
        catalog::verify(&facts, &sch, &format!("{}.crow", name))
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
        let src = std::fs::read_to_string(format!("tests/db_check/{}.crow", name)).unwrap();
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
        suppress(detect(&stmts, &facts, &sch, &format!("{}.crow", name)), &src)
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
        let src = std::fs::read_to_string("tests/db_check/write_skew.crow").unwrap();
        let stmts = crate::build(&src).unwrap();
        let facts = sql_facts::collect(&stmts);

        // Without the constraint: hazard fires from parsed DDL alone.
        let sch = schema::from_sql(&ddl).unwrap();
        let hazards = detect(&stmts, &facts, &sch, "write_skew.crow");
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
        let hazards2 = detect(&stmts, &facts, &sch2, "write_skew.crow");
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
        let src = std::fs::read_to_string("tests/db_check/write_skew.crow").unwrap();
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
            detect(&stmts, &facts, &sch, "write_skew.crow"),
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
