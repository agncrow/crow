//! Frames' columns as a database says them: what a query returns, asked
//! of the program's databases and kept in `.crow/schema` (how columns pass
//! through frame operations and calls is the lowering's, in each frame's
//! type: `lower_ir/frames.rs`).

use super::*;

impl TypeContext {
    /// Infer the type of an expression.
    /// `x = db.connect['url']` (or bare `connect`) binds x to a database.
    pub(super) fn note_connection(&mut self, name: &str, value: &Expr) {
        if let Expr::Call { callee, args } = value {
            let is_connect = match callee.as_ref() {
                Expr::Ident(n) => n == "connect",
                Expr::FieldAccess { object, field } => field == "connect" && matches!(object.as_ref(), Expr::Ident(m) if m == "db"),
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
    pub(crate) fn describe_sql(&mut self, sql: &str, handle: Option<&str>) -> Option<BTreeMap<String, CrowType>> {
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
        // `...` spans → NULL
        let mut scrubbed = String::with_capacity(sql.len());
        let mut in_span = false;
        for c in sql.chars() {
            if c == '`' { if !in_span { scrubbed.push_str("NULL"); } in_span = !in_span; continue; }
            if !in_span { scrubbed.push(c); }
        }
        let mut reasons = Vec::new();
        let mut found: Option<BTreeMap<String, CrowType>> = None;
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
            match crate::type_infer::describe_with_tables(&url, &scrubbed, &self.ddl) {
                Ok(cols) => {
                    schema_record::write(&var, &scrubbed, &cols);
                    let schema: BTreeMap<String, CrowType> = cols.into_iter().map(|(n, kind)| (n, cell_type_of_kind(&kind))).collect();
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

thread_local! {
    static DESCRIBE_HANDLES: std::cell::RefCell<std::collections::HashMap<String, Result<crow_db::DbHandle, String>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// What each database answered for each statement, and when: a test
/// file's programs, each lowered alone, ask again about the same
/// statements within seconds (a run of finch's nav.co sent 147 describes
/// for 19 queries), and a server that is asked less is a server under less
/// load. One cache for the process, whatever thread lowers: `crow check`
/// lowers a part per worker, and a cache per thread sent finch's 26
/// statements 578 times. An answer is kept a minute, so a long-lived
/// process (the language server) sees a changed table soon. A refusal is
/// kept as an answer is: a statement of one database's dialect is asked
/// of every connection a program has when none is named, and the one that
/// refuses it would be asked again by every worker.
static DESCRIBE_ANSWERS: std::sync::Mutex<Option<std::collections::HashMap<(String, String), (std::time::Instant, Result<Vec<(String, String)>, String>)>>> = std::sync::Mutex::new(None);

/// How long a database's answer about a statement is kept
const DESCRIBE_KEPT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(test)]
thread_local! {
    /// How many times this thread asked a database to describe a statement
    pub(super) static DESCRIBES_ASKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Describe a statement against a database URL, keeping one connection per
/// URL for the build. A SQLite file that does not exist yet is not created
/// by asking: only the program makes its database. For a program that makes
/// its own tables, a SQLite database that
/// does not exist yet — or one missing a table the statement reads — is
/// described from a scratch copy the program's `CREATE` statements build,
/// so a new app's columns are typed on its first build as on every later one.
/// `IN ?` as the list of one it is sent as, `IN (?)`: a list parameter's
/// `?` is only ever sent expanded, and no database prepares `IN $2` (finch's
/// headlines, asked about as written, went untyped)
pub(super) fn in_lists_as_one(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '?' => {
                let mut j = i;
                while j > 0 && chars[j - 1].is_whitespace() {
                    j -= 1;
                }
                let after_in = j >= 2
                    && chars[j - 2..j].iter().collect::<String>().eq_ignore_ascii_case("IN")
                    && (j == 2 || !(chars[j - 3].is_ascii_alphanumeric() || chars[j - 3] == '_'));
                if after_in {
                    out.push_str("(?)");
                    continue;
                }
            }
            None => {}
        }
        out.push(c);
    }
    out
}

pub fn describe_with_tables(url: &str, sql: &str, ddl: &[String]) -> Result<Vec<(String, String)>, String> {
    let listed = in_lists_as_one(sql);
    let sql = listed.as_str();
    let sqlite_path = url.strip_prefix("sqlite://");
    let missing = sqlite_path.map_or(false, |p| p == ":memory:" || !std::path::Path::new(p).exists());
    if missing {
        if ddl.is_empty() {
            return Err(format!("{} does not exist yet", url));
        }
        return describe_on(&program_key(url, ddl), "sqlite://:memory:", ddl, sql);
    }
    match describe_on(url, url, &[], sql) {
        Err(e) if sqlite_path.is_some() && !ddl.is_empty() && e.contains("no such table") => describe_on(&program_key(url, ddl), "sqlite://:memory:", ddl, sql),
        other => other,
    }
}

/// The scratch database a program's own CREATE statements build: one per
/// set of statements, so programs lowered in one process — the test suite,
/// the language server — never meet each other's tables. (Keyed by the URL
/// alone, every `sqlite://:memory:` program shared the first one's tables,
/// and a later program's cells went untyped.)
fn program_key(url: &str, ddl: &[String]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ddl.hash(&mut h);
    format!("{}#program#{:016x}", url, h.finish())
}

/// Describe on the build's connection for `key` (opened on `url`, built from
/// `ddl` when it is new).
pub(super) fn describe_on(key: &str, url: &str, ddl: &[String], sql: &str) -> Result<Vec<(String, String)>, String> {
    let asked = (key.to_string(), sql.to_string());
    let kept = DESCRIBE_ANSWERS.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|a| a.get(&asked).filter(|(at, _)| at.elapsed() < DESCRIBE_KEPT).map(|(_, c)| c.clone()));
    if let Some(answer) = kept {
        return answer;
    }
    let answer = describe_asking(key, url, ddl, sql);
    DESCRIBE_ANSWERS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(std::collections::HashMap::new).insert(asked, (std::time::Instant::now(), answer.clone()));
    answer
}

/// The answer of `crow db describe <url> <sql>`, run by this very binary.
fn describe_by_process(url: &str, sql: &str) -> Result<Vec<(String, String)>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find crow itself: {}", e))?;
    // (a test binary is not crow: the database is not reached, as one that
    // does not answer is not)
    if exe.file_stem().and_then(|s| s.to_str()) != Some("crow") {
        return Err("crow db describe runs from the crow binary alone".to_string());
    }
    if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
        eprintln!("DESCRIBE asks {} for `{}…`", crow_db::schema::shown_url(url), sql.chars().take(50).collect::<String>());
    }
    let out = std::process::Command::new(exe).args(["db", "describe", url, sql]).env("CROW_NO_DESCRIBE", "1").output().map_err(|e| format!("cannot run crow db describe: {}", e))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if said.is_empty() { "crow db describe answered nothing".to_string() } else { said });
    }
    let cols: Vec<(String, String)> = serde_json::from_slice(&out.stdout).map_err(|e| format!("crow db describe answered no columns: {}", e))?;
    Ok(cols)
}

/// Ask the database (see `describe_on`, which keeps the answers).
pub(super) fn describe_asking(key: &str, url: &str, ddl: &[String], sql: &str) -> Result<Vec<(String, String)>, String> {
    #[cfg(test)]
    DESCRIBES_ASKED.with(|n| n.set(n.get() + 1));
    // a database whose driver is written in Crow is asked by a process of
    // its own (`crow db describe`): the driver runs in an interpreter, and
    // this lowering, which asks from inside, is not a place to start one
    if url.starts_with("clickhouse://") || url.starts_with("postgres://") || url.starts_with("postgresql://") {
        return describe_by_process(url, sql);
    }
    DESCRIBE_HANDLES.with(|h| {
        let mut map = h.borrow_mut();
        let handle = map.entry(key.to_string()).or_insert_with(|| {
            let db = crow_db::connect(url, Some(1))?;
            for statement in ddl {
                let _ = db.exec(statement, &[]);
            }
            Ok(db)
        });
        match handle {
            Ok(db) => {
                let mut cols = db.describe(sql)?;
                // SQLite types no computed column, and a statement whose
                // parameters match no row shows none: type those from the
                // statement and the tables' declared columns
                if url.starts_with("sqlite://") && cols.iter().any(|(_, k)| k == "unknown") {
                    let columns_of = |table: &str| -> Vec<(String, String)> {
                        let frame = match db.query(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")), &[]) {
                            Ok(f) => f,
                            Err(_) => return Vec::new(),
                        };
                        let text = |name: &str| -> Vec<String> {
                            frame.columns.iter().position(|c| c == name)
                                .and_then(|i| match frame.data[i].as_ref() { crow_runtime::frame::Column::Str(v) => Some(v.clone()), _ => None })
                                .unwrap_or_default()
                        };
                        text("name").into_iter().zip(text("type")).collect()
                    };
                    if let Some(kinds) = crate::sql_types::select_kinds(sql, &columns_of) {
                        if kinds.len() == cols.len() {
                            for ((_, k), typed) in cols.iter_mut().zip(kinds) {
                                if k == "unknown" {
                                    if let Some(t) = typed {
                                        *k = t;
                                    }
                                }
                            }
                        }
                    }
                }
                mark_nullable(db, url, sql, &mut cols);
                Ok(cols)
            }
            Err(e) => Err(e.clone()),
        }
    })
}

/// Mark the columns of a statement's rows that can hold NULL, as its tables'
/// declarations and the statement say (`sql_types::select_nulls`): their
/// kinds end in `?`, and their cells are fallible. SQLite and Postgres
/// describe no column's nullability, so each table's declared NOT NULLs
/// are asked for; a statement that is not read that way can hold NULL in
/// every column. ClickHouse's DESCRIBE says it itself (`Nullable(…)`).
pub(crate) fn mark_nullable(db: &crow_db::DbHandle, url: &str, sql: &str, cols: &mut [(String, String)]) {
    use crow_runtime::frame::Column;
    let postgres = url.starts_with("postgres");
    if !postgres && !url.starts_with("sqlite://") {
        return;
    }
    let read = |frame: &crow_runtime::frame::CrowFrame, name: &str| -> Vec<String> {
        frame.columns.iter().position(|c| c == name).map(|i| match frame.data[i].as_ref() {
            Column::Str(v) => v.clone(),
            Column::Int(v) => v.iter().map(|x| x.to_string()).collect(),
            Column::Bool(v) => v.iter().map(|x| x.to_string()).collect(),
            _ => Vec::new(),
        }).unwrap_or_default()
    };
    let columns_of = |table: &str| -> Vec<(String, bool)> {
        if postgres {
            let q = "SELECT attname::text AS name, attnotnull AS notnull FROM pg_attribute WHERE attrelid = to_regclass(?) AND attnum > 0 AND NOT attisdropped ORDER BY attnum";
            let Ok(frame) = db.query(q, &[serde_json::Value::String(table.to_string())]) else { return Vec::new() };
            read(&frame, "name").into_iter().zip(read(&frame, "notnull")).map(|(n, nn)| (n, nn == "true")).collect()
        } else {
            let t = table.rsplit('.').next().unwrap_or(table);
            let Ok(frame) = db.query(&format!("PRAGMA table_info(\"{}\")", t.replace('"', "\"\"")), &[]) else { return Vec::new() };
            let (names, types, notnull, pk) = (read(&frame, "name"), read(&frame, "type"), read(&frame, "notnull"), read(&frame, "pk"));
            // an INTEGER PRIMARY KEY is the row's id, never NULL
            let keys = pk.iter().filter(|p| *p != "0").count();
            names.into_iter().enumerate().map(|(i, n)| {
                let rowid = keys == 1 && pk.get(i).map_or(false, |p| p != "0") && types.get(i).map_or(false, |t| t.eq_ignore_ascii_case("integer"));
                (n, rowid || notnull.get(i).map_or(false, |x| x == "1"))
            }).collect()
        }
    };
    let nulls = crate::sql_types::select_nulls(sql, postgres, &columns_of).filter(|v| v.len() == cols.len());
    for (i, (_, kind)) in cols.iter_mut().enumerate() {
        if nulls.as_ref().map_or(true, |v| v[i]) && !kind.ends_with('?') {
            kind.push('?');
        }
    }
}

/// A described column's kind as the type of its cells: a column that can
/// hold NULL (`float?`) has fallible cells, a missing one an error.
pub(crate) fn cell_type_of_kind(kind: &str) -> CrowType {
    let (base, nullable) = match kind.strip_suffix('?') {
        Some(b) => (b, true),
        None => (kind, false),
    };
    let t = match base {
        "int" => CrowType::Int,
        "float" | "datetime" => CrowType::Float,
        "bool" => CrowType::Bool,
        _ => CrowType::Str,
    };
    if nullable { CrowType::Result(Box::new(t)) } else { t }
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
    use super::CrowType;
    use std::collections::BTreeMap;

    /// The FNV-1a hash of the parts, a stable file name (crow_db's, which a
    /// replay names its files by too)
    pub(crate) fn fnv(parts: &[&str]) -> String {
        crow_db::fixtures::fnv(parts)
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
    pub(crate) fn read(var: &str, sql: &str) -> Option<BTreeMap<String, CrowType>> {
        let (_, cols) = parse(&std::fs::read_to_string(file_for(var, sql)?).ok()?)?;
        Some(cols.into_iter().map(|(n, k)| (n, super::cell_type_of_kind(&k))).collect())
    }
}
