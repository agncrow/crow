/// Database schema introspection. Read-only: tables, columns, declared types,
/// and the constraints the database will actually enforce (unique indexes,
/// including partial ones, and primary keys).

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct UniqueRule {
    pub cols: Vec<String>,
    /// Raw predicate text for partial indexes, lowercased, e.g. "status = 'active'".
    pub partial: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TableSchema {
    /// column name → declared type (uppercased, e.g. "INTEGER", "TEXT", "REAL")
    pub columns: BTreeMap<String, String>,
    pub unique: Vec<UniqueRule>,
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub tables: BTreeMap<String, TableSchema>,
    pub backend: String,
}

impl Schema {
    pub fn table(&self, name: &str) -> Option<&TableSchema> {
        self.tables.get(&name.to_lowercase())
    }
}

impl TableSchema {
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.contains_key(&name.to_lowercase())
    }

    /// Is there a unique rule covering exactly these columns (order-free)?
    /// `predicate_cols` are (col, literal) equality filters from the program's
    /// check; a partial index matches when its predicate mentions the same
    /// column/literal pairs.
    pub fn unique_covers(&self, cols: &[String], predicate: &[(String, String)]) -> bool {
        let mut want: Vec<String> = cols.iter().map(|c| c.to_lowercase()).collect();
        want.sort();
        // A full unique index over key ∪ predicate columns also enforces the
        // invariant (one row per key within each predicate value).
        let mut want_wide: Vec<String> = want.clone();
        want_wide.extend(predicate.iter().map(|(c, _)| c.to_lowercase()));
        want_wide.sort();
        want_wide.dedup();
        self.unique.iter().any(|u| {
            let mut have: Vec<String> = u.cols.iter().map(|c| c.to_lowercase()).collect();
            have.sort();
            match &u.partial {
                None => have == want || have == want_wide,
                Some(pred) => {
                    have == want
                        && predicate.iter().all(|(c, v)| {
                            pred.contains(&c.to_lowercase())
                                && pred.contains(&v.to_lowercase())
                        })
                }
            }
        })
    }
}

/// Introspect through an open crow_db handle. Dispatches on backend.
pub fn introspect(db: &crow_db::DbHandle) -> Result<Schema, String> {
    match db.backend_name() {
        "sqlite" => introspect_sqlite(db),
        "postgres" => introspect_postgres(db),
        "clickhouse" => Err(
            "clickhouse does not enforce unique constraints, so schema-aware checks cannot \
             verify invariants against it; run crow check against your sqlite/postgres dev database"
                .to_string(),
        ),
        other => Err(format!(
            "schema introspection for '{}' is not implemented yet",
            other
        )),
    }
}

fn introspect_postgres(db: &crow_db::DbHandle) -> Result<Schema, String> {
    let mut schema = Schema {
        backend: "postgres".into(),
        ..Default::default()
    };
    let cols = db.query(
        "SELECT table_name, column_name, data_type FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        &[],
    )?;
    let tnames = cols.col_as_strings("table_name").unwrap_or_default();
    let cnames = cols.col_as_strings("column_name").unwrap_or_default();
    let dtypes = cols.col_as_strings("data_type").unwrap_or_default();
    for i in 0..tnames.len() {
        schema
            .tables
            .entry(tnames[i].to_lowercase())
            .or_default()
            .columns
            .insert(
                cnames[i].to_lowercase(),
                dtypes.get(i).cloned().unwrap_or_default().to_uppercase(),
            );
    }

    // Unique indexes (covers PRIMARY KEY and UNIQUE constraints too — both
    // are backed by unique indexes), with partial predicates decoded.
    let idx = db.query(
        "SELECT t.relname AS tbl, i.relname AS idx, a.attname AS col, \
                pg_get_expr(ix.indpred, ix.indrelid) AS pred \
         FROM pg_index ix \
         JOIN pg_class t ON t.oid = ix.indrelid \
         JOIN pg_class i ON i.oid = ix.indexrelid \
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
         WHERE ix.indisunique AND t.relkind = 'r' \
           AND t.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = 'public') \
         ORDER BY i.relname",
        &[],
    )?;
    let itbl = idx.col_as_strings("tbl").unwrap_or_default();
    let iidx = idx.col_as_strings("idx").unwrap_or_default();
    let icol = idx.col_as_strings("col").unwrap_or_default();
    let ipred = idx.col_as_strings("pred").unwrap_or_default();
    let mut grouped: BTreeMap<String, (String, Vec<String>, Option<String>)> = BTreeMap::new();
    for i in 0..iidx.len() {
        let entry = grouped.entry(iidx[i].clone()).or_insert_with(|| {
            let pred = ipred.get(i).and_then(|p| {
                let p = p.trim();
                if p.is_empty() || p == "null" {
                    None
                } else {
                    Some(p.to_lowercase())
                }
            });
            (itbl[i].to_lowercase(), Vec::new(), pred)
        });
        entry.1.push(icol[i].to_lowercase());
    }
    for (_, (tbl, cols, partial)) in grouped {
        if let Some(ts) = schema.tables.get_mut(&tbl) {
            ts.unique.push(UniqueRule { cols, partial });
        }
    }
    Ok(schema)
}

/// Offline mode: build the schema from migration files (CREATE TABLE /
/// CREATE UNIQUE INDEX statements) instead of a live database.
pub fn from_sql_dir(dir: &str) -> Result<Schema, String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read migrations dir '{}': {}", dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "sql").unwrap_or(false))
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(format!("no .sql files in '{}'", dir));
    }
    let mut all = String::new();
    for p in entries {
        all.push_str(
            &std::fs::read_to_string(&p)
                .map_err(|e| format!("cannot read {}: {}", p.display(), e))?,
        );
        all.push('\n');
    }
    from_sql(&all)
}

pub fn from_sql(ddl: &str) -> Result<Schema, String> {
    use sqlparser::ast as sql;
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser as SqlParser;

    let mut schema = Schema {
        backend: "offline".into(),
        ..Default::default()
    };
    let stmts = SqlParser::parse_sql(&GenericDialect {}, ddl)
        .map_err(|e| format!("could not parse migration SQL: {}", e))?;
    for stmt in stmts {
        match stmt {
            sql::Statement::CreateTable(ct) => {
                let Some(tname) = last_name(&ct.name.to_string()) else {
                    continue;
                };
                let mut ts = TableSchema::default();
                for col in &ct.columns {
                    let cname = col.name.value.to_lowercase();
                    ts.columns
                        .insert(cname.clone(), col.data_type.to_string().to_uppercase());
                    for opt in &col.options {
                        if matches!(
                            &opt.option,
                            sql::ColumnOption::Unique { .. } | sql::ColumnOption::PrimaryKey { .. }
                        ) {
                            ts.unique.push(UniqueRule {
                                cols: vec![cname.clone()],
                                partial: None,
                            });
                        }
                    }
                }
                for constraint in &ct.constraints {
                    let cols = match constraint {
                        sql::TableConstraint::Unique(u) => Some(&u.columns),
                        sql::TableConstraint::PrimaryKey(pk) => Some(&pk.columns),
                        _ => None,
                    };
                    if let Some(cols) = cols {
                        ts.unique.push(UniqueRule {
                            cols: cols.iter().filter_map(index_col_name).collect(),
                            partial: None,
                        });
                    }
                }
                schema.tables.insert(tname, ts);
            }
            sql::Statement::CreateIndex(ci) => {
                if !ci.unique {
                    continue;
                }
                let Some(tname) = last_name(&ci.table_name.to_string()) else {
                    continue;
                };
                let cols: Vec<String> = ci
                    .columns
                    .iter()
                    .filter_map(index_col_name)
                    .collect();
                let partial = ci.predicate.as_ref().map(|p| p.to_string().to_lowercase());
                if let Some(ts) = schema.tables.get_mut(&tname) {
                    ts.unique.push(UniqueRule { cols, partial });
                }
            }
            _ => {}
        }
    }
    Ok(schema)
}

fn last_name(s: &str) -> Option<String> {
    s.rsplit('.')
        .next()
        .map(|p| p.trim_matches('"').trim_matches('`').to_lowercase())
}

fn index_col_name(c: &sqlparser::ast::IndexColumn) -> Option<String> {
    match &c.column.expr {
        sqlparser::ast::Expr::Identifier(i) => Some(i.value.to_lowercase()),
        _ => None,
    }
}

fn introspect_sqlite(db: &crow_db::DbHandle) -> Result<Schema, String> {
    let mut schema = Schema {
        backend: "sqlite".into(),
        ..Default::default()
    };
    let tables = db.query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        &[],
    )?;
    for tname in tables.col_as_strings("name").unwrap_or_default() {
        let mut ts = TableSchema::default();
        let mut pk_cols: Vec<(i64, String)> = Vec::new();

        let info = db.query(
            &format!("SELECT name, type, pk FROM pragma_table_info('{}')", tname),
            &[],
        )?;
        let names = info.col_as_strings("name").unwrap_or_default();
        let types = info.col_as_strings("type").unwrap_or_default();
        let pks = info.col_as_strings("pk").unwrap_or_default();
        for i in 0..names.len() {
            let ty = types.get(i).cloned().unwrap_or_default().to_uppercase();
            ts.columns.insert(names[i].to_lowercase(), ty);
            let pk_rank: i64 = pks.get(i).and_then(|p| p.parse().ok()).unwrap_or(0);
            if pk_rank > 0 {
                pk_cols.push((pk_rank, names[i].to_lowercase()));
            }
        }
        if !pk_cols.is_empty() {
            pk_cols.sort();
            ts.unique.push(UniqueRule {
                cols: pk_cols.into_iter().map(|(_, c)| c).collect(),
                partial: None,
            });
        }

        let idx = db.query(
            &format!(
                "SELECT name, \"unique\", partial FROM pragma_index_list('{}')",
                tname
            ),
            &[],
        )?;
        let inames = idx.col_as_strings("name").unwrap_or_default();
        let iuniq = idx.col_as_strings("unique").unwrap_or_default();
        let ipart = idx.col_as_strings("partial").unwrap_or_default();
        for i in 0..inames.len() {
            if iuniq.get(i).map(|u| u == "1").unwrap_or(false) {
                let cols_frame = db.query(
                    &format!("SELECT name FROM pragma_index_info('{}')", inames[i]),
                    &[],
                )?;
                let cols: Vec<String> = cols_frame
                    .col_as_strings("name")
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| c.to_lowercase())
                    .collect();
                let partial = if ipart.get(i).map(|p| p == "1").unwrap_or(false) {
                    // Predicate text lives only in the index's CREATE sql.
                    let sql_frame = db.query(
                        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
                        &[serde_json::Value::String(inames[i].clone())],
                    )?;
                    sql_frame
                        .col_as_strings("sql")
                        .unwrap_or_default()
                        .first()
                        .and_then(|sql| {
                            let lower = sql.to_lowercase();
                            lower
                                .find(" where ")
                                .map(|p| lower[p + 7..].trim().to_string())
                        })
                } else {
                    None
                };
                ts.unique.push(UniqueRule { cols, partial });
            }
        }

        schema.tables.insert(tname.to_lowercase(), ts);
    }
    Ok(schema)
}
