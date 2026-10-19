//! Frames' columns as a database says them: what a query returns, asked
//! of the program's databases through the provider the row declares
//! (`table::Provider`, crow_db's `describe`) and kept in `.crow/schema`
//! (how columns pass through frame operations and calls is the lowering's,
//! in each frame's type: `lower_ir/frames.rs`).

use super::*;

impl TypeContext {
    /// Infer the type of an expression.
    /// `x = db.connect['url']` (or bare `connect`) binds x to a database.
    pub(super) fn note_connection(&mut self, name: &str, value: &Expr) {
        if let Expr::Call { callee, args } = value {
            // (the form that opens a connection says so on its row)
            let is_connect = match callee.as_ref() {
                Expr::Ident(n) => crate::table::opens_connection(None, n),
                Expr::FieldAccess { object, field } => matches!(object.as_ref(), Expr::Ident(m) if crate::table::opens_connection(Some(m), field)),
                _ => false,
            };
            if is_connect {
                // (a URL with a span in it is known only when the program
                // runs: asked with the span's text, the database had
                // answered that the URL is invalid)
                if let Some(Expr::Str(url)) = args.first()
                    && !url.contains('`')
                {
                    self.conn_urls.retain(|(n, _)| n != name);
                    self.conn_urls.push((name.to_string(), url.clone()));
                }
            }
        }
    }

    /// Ask each declared database, in order, what the statement returns.
    /// Interpolated spans and `?` placeholders are unknown at compile
    /// time and stand in as NULL. A database that cannot be reached or
    /// does not understand the statement is skipped; when none answers,
    /// the cells read as text, as before, and the build says so once.
    /// The query's own connection answers when the handle is a known
    /// top-level one; a handle that is a parameter (a helper's `conn`) is
    /// unknown here, and every connection is asked until one answers.
    pub(crate) fn describe_sql(&mut self, provider: &crate::table::Provider, sql: &str, handle: Option<&str>) -> Option<BTreeMap<String, Ty>> {
        if self.conn_urls.is_empty() { return None; }
        if let Some(cached) = self.describe_cache.get(sql) {
            if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
                eprintln!("DESCRIBE cached {:?} for `{}…`", cached, sql.chars().take(50).collect::<String>());
            }
            return cached.clone();
        }
        let own: Vec<(String, String)> = match handle {
            Some(h) => self.conn_urls.iter().filter(|(n, _)| n == h).cloned().collect(),
            None => Vec::new(),
        };
        let candidates: Vec<(String, String)> = if own.is_empty() { self.conn_urls.clone() } else { own };
        let scrubbed = scrubbed(sql);
        let mut reasons = Vec::new();
        let mut found: Option<BTreeMap<String, Ty>> = None;
        let names: Vec<String> = candidates.iter().map(|(v, _)| v.clone()).collect();
        // not asked (CROW_NO_DESCRIBE): no database answers, so what one
        // answered last time, kept in the project, types the cells, as when
        // none can be reached (spec, typed cells); nothing is written
        if !self.describe_queries {
            let kept = names.iter().find_map(|v| schema_record::read(v, &scrubbed));
            self.describe_cache.insert(sql.to_string(), kept.clone());
            return kept;
        }
        for (var, url) in candidates {
            match (provider.columns)(&url, &scrubbed, &self.ddl) {
                Ok(cols) => {
                    schema_record::write(&var, &scrubbed, &cols);
                    let schema: BTreeMap<String, Ty> = cols.into_iter().map(|(n, kind)| (n, cell_type_of_kind(&kind))).collect();
                    found = Some(schema);
                    break;
                }
                // a backend without describe (sqlite) is not a failure to report
                Err(e) if e.contains("describe is not supported") => {}
                Err(e) => reasons.push(format!("{}: {}", var, e)),
            }
        }
        if found.is_none() && !reasons.is_empty() {
            let head: String = sql.chars().take(60).collect();
            // what the database answered last time, kept in the project
            match names.iter().find_map(|v| schema_record::read(v, &scrubbed)) {
                Some(schema) => {
                    self.describe_notes.push(format!("note: the columns of `{}…` are typed from .crow/schema — no database answered ({})", head, reasons.join("; ")));
                    found = Some(schema);
                }
                None => self.describe_notes.push(format!("note: the columns of `{}…` could not be typed at compile time — its cells are dynamic: read them with int[…], float[…] or str[…] ({})", head, reasons.join("; "))),
            }
        }
        if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
            eprintln!("DESCRIBE {:?} via {:?} for `{}…`", found, handle, sql.chars().take(50).collect::<String>());
        }
        self.describe_cache.insert(sql.to_string(), found.clone());
        found
    }

}

/// A statement as a database can be asked about it: each interpolated
/// span (`` `x` ``), known only as the program runs, stands in as NULL.
pub(crate) fn scrubbed(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_span = false;
    for c in sql.chars() {
        if c == '`' {
            if !in_span {
                out.push_str("NULL");
            }
            in_span = !in_span;
            continue;
        }
        if !in_span {
            out.push(c);
        }
    }
    out
}

/// A described column's kind as the type of its cells: a column that can
/// hold NULL (`float?`) has fallible cells, a missing one an error.
pub(crate) fn cell_type_of_kind(kind: &str) -> Ty {
    let (base, nullable) = match kind.strip_suffix('?') {
        Some(b) => (b, true),
        None => (kind, false),
    };
    let t = match base {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "datetime" => Ty::Moment,
        "bool" => Ty::Bool,
        _ => Ty::Str,
    };
    if nullable { Ty::Result(Box::new(t)) } else { t }
}

/// What a program's databases answered about its statements, kept in the
/// project's `.crow/schema`: one file per statement and the connection it
/// was asked on, named by a hash of both (an FNV-1a, the same on every
/// machine and release), holding the statement and its columns — name and
/// kind (`int` `float` `str` `bool` `datetime`), in the statement's order.
/// A build reads it when no database answers — a server down, a machine
/// without the data — so the program is typed as it was; a rehearsal
/// generates rows from it.
pub(crate) mod schema_record {
    use super::Ty;
    use std::collections::BTreeMap;

    /// The FNV-1a hash of the parts, a stable file name (crow_db's, which a
    /// replay names its files by too)
    pub(crate) fn fnv(parts: &[&str]) -> String {
        crow_runtime::fixtures::fnv(parts)
    }

    fn file_for(var: &str, sql: &str) -> Option<std::path::PathBuf> {
        let dir = crate::schema_dir()?;
        Some(dir.join(format!("{}.json", fnv(&[var, sql]))))
    }

    /// Keep what `var` answered for `sql` (written only when it changed).
    pub(crate) fn write(var: &str, sql: &str, columns: &[(String, String)]) {
        let Some(path) = file_for(var, sql) else { return };
        let columns: Vec<[&str; 2]> = columns.iter().map(|(n, k)| [n.as_str(), k.as_str()]).collect();
        let text = match serde_json::to_string_pretty(&serde_json::json!({ "via": var, "sql": sql, "columns": columns })) {
            Ok(t) => t + "\n",
            Err(_) => return,
        };
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, text);
    }

    /// A record's statement and columns, from its file's text.
    pub(crate) fn parse(text: &str) -> Option<(String, Vec<(String, String)>)> {
        crow_db::fixtures::parse_record(text)
    }

    /// What `var` answered for `sql` when it last could, if it is kept.
    pub(crate) fn read(var: &str, sql: &str) -> Option<BTreeMap<String, Ty>> {
        let (_, cols) = parse(&std::fs::read_to_string(file_for(var, sql)?).ok()?)?;
        Some(cols.into_iter().map(|(n, k)| (n, super::cell_type_of_kind(&k))).collect())
    }
}
