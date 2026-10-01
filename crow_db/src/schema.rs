//! The schema a program declares (spec, db, The schema a program
//! declares). The lowering finds the literal `CREATE TABLE` statements a
//! program hands each connection and gives them to its `connect`; opening
//! the connection checks the database against them here, in the mode the
//! program runs in — the same code in both modes of running, and for
//! `crow db diff`.
//!
//! At connect only the tables the database has are compared: a declared
//! table it lacks is the program's own statement's to make as it runs (a
//! fresh database is no difference), and the tables and columns a database
//! has beyond the declared ones are its own.

use crate::{DbHandle, DbTx};
use crow_runtime::frame::CrowFrame;
use std::collections::BTreeSet;
use std::sync::Mutex;

// ── modes ───────────────────────────────────────────────────────────

/// What opening a connection does with the tables declared on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// What the database lacks is made, under its lock.
    Reconcile,
    /// The catalog is read, and a difference stops the program.
    Verify,
    /// Nothing is read, and every start says so.
    Trust,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s.trim() {
            "reconcile" => Ok(Mode::Reconcile),
            "verify" => Ok(Mode::Verify),
            "trust" => Ok(Mode::Trust),
            other => Err(format!("'{}' is not a schema mode — 'reconcile', 'verify' or 'trust'", other)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Reconcile => "reconcile",
            Mode::Verify => "verify",
            Mode::Trust => "trust",
        }
    }
}

/// The mode a built program starts with, when its crowfile says one.
static MODE: Mutex<Option<Mode>> = Mutex::new(None);

/// Set the mode this program starts with: a built program's crowfile's
/// `schema`. `CROW_SCHEMA` says otherwise for one run.
pub fn set_mode(mode: Mode) {
    *MODE.lock().unwrap_or_else(|p| p.into_inner()) = Some(mode);
}

/// The mode a connection opens in: `CROW_SCHEMA` when set (crow sets it
/// for the programs it runs from their source — reconcile, or the
/// crowfile's), else the mode the program starts with, else verify.
pub fn mode() -> Result<Mode, String> {
    match std::env::var("CROW_SCHEMA") {
        Ok(v) if !v.trim().is_empty() => Mode::parse(&v).map_err(|e| format!("CROW_SCHEMA: {}", e)),
        _ => Ok(MODE.lock().unwrap_or_else(|p| p.into_inner()).unwrap_or(Mode::Verify)),
    }
}

// ── what a statement declares ───────────────────────────────────────

/// A name as a statement writes it. An unquoted name is folded as its
/// database folds names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub text: String,
    pub quoted: bool,
}

/// A column as a `CREATE TABLE` declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: Name,
    /// Its type as written (`INTEGER`, `VARCHAR(20)`, `Nullable(String)`);
    /// empty where none is.
    pub ty: String,
    pub not_null: bool,
    /// It has a value no statement gives it: a default, a serial or an
    /// identity, a generated column.
    pub filled: bool,
    /// A key of its own (`PRIMARY KEY`, `UNIQUE`).
    pub key: bool,
    /// Its definition as written, for `ADD COLUMN`.
    pub def: String,
}

/// A table as a program's `CREATE TABLE` declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The schema (in ClickHouse, the database) the statement names, if any.
    pub schema: Option<Name>,
    pub name: Name,
    /// The statement, as the program wrote it.
    pub create: String,
    /// What its parentheses hold: the columns and the table's constraints.
    pub body: String,
    pub columns: Vec<Column>,
}

impl Table {
    /// The table as a message names it.
    fn shown(&self) -> String {
        match &self.schema {
            Some(s) => format!("{}.{}", s.text, self.name.text),
            None => self.name.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok<'a> {
    /// A bare word: a keyword or an unquoted name.
    Word(&'a str),
    /// A quoted name, without its quotes.
    Quoted(&'a str),
    /// What a pair of parentheses holds.
    Group(&'a str),
    /// A string, a number, an operator.
    Other(&'a str),
}

/// A statement's tokens at its top level, each with where it starts and
/// ends. Comments are skipped; a group is one token.
fn tokens(s: &str) -> Vec<(Tok<'_>, usize, usize)> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c.is_whitespace() {
            continue;
        }
        // a comment: `--` to the line's end, `/*` to `*/`
        let comment_end = match c {
            '-' if s[i + 1..].starts_with('-') => Some(s[i..].find('\n').map_or(s.len(), |n| i + n)),
            '/' if s[i + 1..].starts_with('*') => Some(s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2)),
            _ => None,
        };
        if let Some(end) = comment_end {
            while it.peek().is_some_and(|(j, _)| *j < end) {
                it.next();
            }
            continue;
        }
        match c {
            '"' | '`' | '\'' => {
                // to the closing quote; a doubled quote is the quote itself
                let mut end = None;
                while let Some((j, d)) = it.next() {
                    if d == c {
                        if it.peek().is_some_and(|(_, e)| *e == c) {
                            it.next();
                            continue;
                        }
                        end = Some(j + 1);
                        break;
                    }
                }
                let (inner, end) = match end {
                    Some(e) => (&s[i + 1..e - 1], e),
                    None => (&s[i + 1..], s.len()),
                };
                out.push((if c == '\'' { Tok::Other(&s[i..end]) } else { Tok::Quoted(inner) }, i, end));
            }
            '(' => {
                let mut depth = 1;
                let mut quote: Option<char> = None;
                let mut end = None;
                for (j, d) in it.by_ref() {
                    match quote {
                        Some(q) if d == q => quote = None,
                        Some(_) => {}
                        None => match d {
                            '\'' | '"' | '`' => quote = Some(d),
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(j + 1);
                                    break;
                                }
                            }
                            _ => {}
                        },
                    }
                }
                let (inner, end) = match end {
                    Some(e) => (&s[i + 1..e - 1], e),
                    None => (&s[i + 1..], s.len()),
                };
                out.push((Tok::Group(inner), i, end));
            }
            c if c.is_alphanumeric() || c == '_' || c == '$' => {
                let mut end = i + c.len_utf8();
                while let Some((j, d)) = it.peek().copied() {
                    if !(d.is_alphanumeric() || d == '_' || d == '$') {
                        break;
                    }
                    end = j + d.len_utf8();
                    it.next();
                }
                out.push((Tok::Word(&s[i..end]), i, end));
            }
            _ => out.push((Tok::Other(&s[i..i + c.len_utf8()]), i, i + c.len_utf8())),
        }
    }
    out
}

fn is(t: &Tok<'_>, keyword: &str) -> bool {
    matches!(t, Tok::Word(w) if w.eq_ignore_ascii_case(keyword))
}

fn name_of(t: Tok<'_>) -> Option<Name> {
    match t {
        Tok::Word(w) => Some(Name { text: w.to_string(), quoted: false }),
        Tok::Quoted(q) => Some(Name { text: q.replace("\"\"", "\"").replace("``", "`"), quoted: true }),
        _ => None,
    }
}

/// What a body's top-level commas separate.
fn items(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    for (t, i, j) in tokens(body) {
        if t == Tok::Other(",") {
            if let Some(s) = start.take() {
                out.push(&body[s..end]);
            }
            continue;
        }
        start.get_or_insert(i);
        end = j;
    }
    if let Some(s) = start {
        out.push(&body[s..end]);
    }
    out
}

/// A table's name after `keywords` and an optional `IF [NOT] EXISTS`: its
/// schema (if named), its name, and the tokens' position after them.
fn table_name<'a>(toks: &[(Tok<'a>, usize, usize)], keywords: &[&str]) -> Option<(Option<Name>, Name, usize)> {
    if toks.len() < keywords.len() || !keywords.iter().enumerate().all(|(i, k)| is(&toks[i].0, k)) {
        return None;
    }
    let word = |i: usize, k: &str| toks.get(i).is_some_and(|(t, _, _)| is(t, k));
    let mut i = keywords.len();
    if word(i, "IF") && word(i + 1, "NOT") && word(i + 2, "EXISTS") {
        i += 3;
    } else if word(i, "IF") && word(i + 1, "EXISTS") {
        i += 2;
    }
    let first = name_of(toks.get(i)?.0)?;
    if toks.get(i + 1).is_some_and(|(t, _, _)| *t == Tok::Other(".")) {
        let name = name_of(toks.get(i + 2)?.0)?;
        return Some((Some(first), name, i + 3));
    }
    Some((None, first, i + 1))
}

/// What a `CREATE TABLE` statement declares: `None` for any other
/// statement, and for one that declares no columns (`CREATE TABLE … AS
/// SELECT`).
pub fn parse_table(sql: &str) -> Option<Table> {
    let toks = tokens(sql);
    let (schema, name, mut i) = table_name(&toks, &["CREATE", "TABLE"])?;
    // the column list is the first group (ClickHouse's ON CLUSTER comes
    // before it); an AS first is a table made from a query
    let body = loop {
        match toks.get(i)?.0 {
            Tok::Group(g) => break g,
            t if is(&t, "AS") => return None,
            _ => i += 1,
        }
    };
    let columns = items(body).into_iter().filter_map(parse_column).collect();
    let create = sql.trim().trim_end_matches(';').trim_end().to_string();
    Some(Table { schema, name, create, body: body.to_string(), columns })
}

/// The table a `DROP TABLE` statement drops.
pub fn dropped_table(sql: &str) -> Option<Name> {
    table_name(&tokens(sql), &["DROP", "TABLE"]).map(|(_, name, _)| name)
}

/// The words that end a column's type: its constraints and clauses, in
/// SQLite, Postgres and ClickHouse.
const CLAUSES: &[&str] = &[
    "NOT", "NULL", "DEFAULT", "PRIMARY", "REFERENCES", "UNIQUE", "CHECK", "COLLATE", "GENERATED", "CONSTRAINT", "AUTOINCREMENT", "AUTO_INCREMENT",
    "IDENTITY", "AS", "ON", "CODEC", "TTL", "MATERIALIZED", "ALIAS", "EPHEMERAL", "COMMENT", "SETTINGS",
];

/// The words that begin a table's constraint rather than a column.
const CONSTRAINTS: &[&str] = &["CONSTRAINT", "PRIMARY", "FOREIGN", "UNIQUE", "CHECK", "EXCLUDE", "LIKE", "PERIOD"];

/// The column an item of a table's body declares, or `None` for a
/// constraint.
fn parse_column(item: &str) -> Option<Column> {
    let toks = tokens(item);
    let first = toks.first()?.0;
    if CONSTRAINTS.iter().any(|k| is(&first, k)) {
        return None;
    }
    // MySQL's `KEY idx (a)`, ClickHouse's `INDEX i expr TYPE minmax` and
    // `PROJECTION p (SELECT …)` — where a column may be named key or index
    let group_at = |i: usize| matches!(toks.get(i), Some((Tok::Group(_), _, _)));
    if ["KEY", "INDEX", "PROJECTION"].iter().any(|k| is(&first, k)) && (group_at(1) || group_at(2) || toks.iter().any(|(t, _, _)| is(t, "TYPE"))) {
        return None;
    }
    let name = name_of(first)?;
    let ty_end = toks.iter().skip(1).position(|(t, _, _)| CLAUSES.iter().any(|k| is(t, k))).map_or(toks.len(), |p| p + 1);
    let ty = if ty_end > 1 { item[toks[1].1..toks[ty_end - 1].2].to_string() } else { String::new() };
    let rest = &toks[ty_end..];
    let has = |k: &str| rest.iter().any(|(t, _, _)| is(t, k));
    let primary = has("PRIMARY");
    let not_null = primary || rest.windows(2).any(|w| is(&w[0].0, "NOT") && is(&w[1].0, "NULL"));
    let filled = ["DEFAULT", "GENERATED", "AS", "IDENTITY", "AUTOINCREMENT", "AUTO_INCREMENT", "MATERIALIZED", "ALIAS", "EPHEMERAL"].iter().any(|k| has(k))
        || ty.to_ascii_uppercase().contains("SERIAL");
    Some(Column { name, ty, not_null, filled, key: primary || has("UNIQUE"), def: item.trim().to_string() })
}

// ── kinds ───────────────────────────────────────────────────────────

/// A column type's kind, as Crow reads the cells it holds (`describe`'s
/// words: int, float, str, bool, datetime).
fn kind(backend: &str, ty: &str) -> String {
    match backend {
        #[cfg(feature = "sqlite")]
        "sqlite" => crate::sqlite::kind_of_decl(ty).unwrap_or_default(),
        #[cfg(feature = "clickhouse")]
        "clickhouse" => clickhouse_kind(ty).to_string(),
        _ => postgres_kind(ty).to_string(),
    }
}

/// A type's name folded: lower case, without its modifiers
/// (`numeric(10, 2)`, `timestamp(3) with time zone`), one space between
/// its words.
fn folded(ty: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in ty.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 => out.push(c.to_ascii_lowercase()),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A Postgres type's kind, by the name a statement or the catalog gives
/// it — as `postgres::pg_kind` reads a column of that type.
fn postgres_kind(ty: &str) -> &'static str {
    let t = folded(ty);
    if t.ends_with("[]") || t == "array" {
        return "str";
    }
    match t.as_str() {
        "smallint" | "integer" | "int" | "int2" | "int4" | "int8" | "bigint" | "serial" | "serial2" | "serial4" | "serial8" | "smallserial" | "bigserial" | "oid" => "int",
        "real" | "float" | "float4" | "float8" | "double precision" | "numeric" | "decimal" => "float",
        "boolean" | "bool" => "bool",
        "timestamp" | "timestamptz" | "timestamp without time zone" | "timestamp with time zone" | "date" => "datetime",
        _ => "str",
    }
}

/// A ClickHouse type's kind: a SQL name it takes for one of its types
/// (`INT`, `TEXT`, `DOUBLE`), or its own name as the catalog gives it.
#[cfg(feature = "clickhouse")]
fn clickhouse_kind(ty: &str) -> &'static str {
    match folded(ty).as_str() {
        "int" | "integer" | "tinyint" | "smallint" | "mediumint" | "bigint" | "int1" | "byte" | "signed" | "unsigned" => "int",
        "float" | "double" | "real" | "double precision" | "decimal" | "numeric" | "dec" => "float",
        "bool" | "boolean" => "bool",
        "text" | "varchar" | "char" | "character" | "character varying" | "string" | "tinytext" | "mediumtext" | "longtext" | "blob" | "clob" | "nvarchar"
        | "varbinary" | "binary" => "str",
        "date" | "datetime" | "timestamp" => "datetime",
        _ => crate::clickhouse::ch_kind(ty),
    }
}

// ── the catalog ─────────────────────────────────────────────────────

/// A name as its database holds it: Postgres folds an unquoted name to
/// lower case; SQLite and ClickHouse keep it.
fn held(backend: &str, n: &Name) -> String {
    if backend == "postgres" && !n.quoted { n.text.to_lowercase() } else { n.text.clone() }
}

/// Whether two column names are one: SQLite's names ignore case.
fn same(backend: &str, a: &str, b: &str) -> bool {
    if backend == "sqlite" { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// A name quoted for a statement.
fn quoted(backend: &str, n: &str) -> String {
    if backend == "clickhouse" { format!("`{}`", n.replace('`', "``")) } else { format!("\"{}\"", n.replace('"', "\"\"")) }
}

/// The table as a statement names it.
fn table_ref(backend: &str, t: &Table) -> String {
    let name = quoted(backend, &held(backend, &t.name));
    match &t.schema {
        Some(s) => format!("{}.{}", quoted(backend, &held(backend, s)), name),
        None => name,
    }
}

fn strings(f: &CrowFrame, col: &str) -> Vec<String> {
    f.col_as_strings(col).unwrap_or_default()
}

/// The columns a table has, each with its type, or `None` when the
/// database has no such table. Only the catalog is read.
fn columns_of(tx: &dyn DbTx, backend: &str, t: &Table) -> Result<Option<Vec<(String, String)>>, String> {
    let name = serde_json::Value::String(held(backend, &t.name));
    let schema = t.schema.as_ref().map(|s| serde_json::Value::String(held(backend, s)));
    let (sql, params) = match (backend, schema) {
        // xinfo: a generated column too
        ("sqlite", None) => ("SELECT name, type FROM pragma_table_xinfo(?)", vec![name]),
        ("sqlite", Some(s)) => ("SELECT name, type FROM pragma_table_xinfo(?, ?)", vec![name, s]),
        ("clickhouse", None) => ("SELECT name, type FROM system.columns WHERE database = currentDatabase() AND table = ? ORDER BY position", vec![name]),
        ("clickhouse", Some(s)) => ("SELECT name, type FROM system.columns WHERE database = ? AND table = ? ORDER BY position", vec![s, name]),
        (_, None) => (
            "SELECT column_name::text AS name, COALESCE(domain_name::text, data_type::text) AS type FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = ? ORDER BY ordinal_position",
            vec![name],
        ),
        (_, Some(s)) => (
            "SELECT column_name::text AS name, COALESCE(domain_name::text, data_type::text) AS type FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
            vec![s, name],
        ),
    };
    let f = tx.query(sql, &params)?;
    let names = strings(&f, "name");
    if names.is_empty() {
        return Ok(None);
    }
    let types = strings(&f, "type");
    Ok(Some(names.into_iter().zip(types.into_iter().chain(std::iter::repeat(String::new()))).collect()))
}

/// Whether a table holds a row.
fn has_rows(tx: &dyn DbTx, backend: &str, t: &Table) -> Result<bool, String> {
    Ok(tx.query(&format!("SELECT 1 AS one FROM {} LIMIT 1", table_ref(backend, t)), &[])?.nrows() > 0)
}

// ── differences ─────────────────────────────────────────────────────

/// One way a database differs from what a program declares on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// What differs, in words.
    pub what: String,
    /// The statements that would make the database hold what is declared.
    pub mend: Vec<String>,
    /// Why reconciling does not make it — it would lose data or guess —
    /// or `None` when it does.
    pub refused: Option<String>,
}

/// How the database differs from the tables declared on it. `missing`:
/// whether a declared table it lacks is a difference (`crow db diff`'s
/// draft makes it; at connect the program's own statement does).
fn differences(tx: &dyn DbTx, backend: &str, tables: &[Table], missing: bool) -> Result<Vec<Difference>, String> {
    let mut out = Vec::new();
    for t in tables {
        let Some(has) = columns_of(tx, backend, t)? else {
            if missing {
                out.push(Difference { what: format!("table '{}' is missing", t.shown()), mend: vec![t.create.clone()], refused: None });
            }
            continue;
        };
        let mut lacking: Vec<&Column> = Vec::new();
        for c in &t.columns {
            let want = held(backend, &c.name);
            let Some((_, ty)) = has.iter().find(|(n, _)| same(backend, n, &want)) else {
                lacking.push(c);
                continue;
            };
            // a column declared without a type, or held without one
            // (SQLite), is of any kind
            if c.ty.trim().is_empty() || ty.trim().is_empty() || kind(backend, &c.ty) == kind(backend, ty) {
                continue;
            }
            out.push(Difference {
                what: format!("column '{}' of table '{}' is {} in the database, declared {}", c.name.text, t.shown(), ty, c.ty),
                mend: retype(backend, t, c, &has),
                refused: Some("reconciling does not change a column's kind: the values it holds may not convert".to_string()),
            });
        }
        if lacking.is_empty() {
            continue;
        }
        // SQLite cannot add a key column in place, nor a NOT NULL one without
        // a default; no database can add a NOT NULL column without a value
        // for the rows a table has (a ClickHouse column takes its type's)
        let sqlite_cannot = |c: &Column| backend == "sqlite" && (c.key || (c.not_null && !c.filled));
        let needs_value = |c: &Column| c.not_null && !c.filled && backend != "clickhouse";
        let full = lacking.iter().any(|c| sqlite_cannot(c) || needs_value(c)) && has_rows(tx, backend, t)?;
        if !full && lacking.iter().any(|c| sqlite_cannot(c)) {
            // a table with no rows is made anew, with every column declared
            let names: Vec<String> = lacking.iter().map(|c| format!("'{}'", c.name.text)).collect();
            out.push(Difference {
                what: format!("table '{}' lacks column{} {}", t.shown(), if names.len() == 1 { "" } else { "s" }, names.join(", ")),
                mend: vec![format!("DROP TABLE {}", table_ref(backend, t)), t.create.clone()],
                refused: None,
            });
            continue;
        }
        for c in lacking {
            let refused = if full && needs_value(c) {
                Some("it is NOT NULL without a default, and the table has rows: a value for them would be a guess".to_string())
            } else if full && sqlite_cannot(c) {
                Some("SQLite cannot add a PRIMARY KEY or UNIQUE column to a table that has rows".to_string())
            } else {
                None
            };
            out.push(Difference {
                what: format!("table '{}' lacks column '{}'", t.shown(), c.name.text),
                mend: vec![format!("ALTER TABLE {} ADD COLUMN {}{}", table_ref(backend, t), if backend == "clickhouse" { "IF NOT EXISTS " } else { "" }, c.def)],
                refused,
            });
        }
    }
    Ok(out)
}

/// The statements that would give a column its declared type.
fn retype(backend: &str, t: &Table, c: &Column, has: &[(String, String)]) -> Vec<String> {
    let table = table_ref(backend, t);
    let col = quoted(backend, &held(backend, &c.name));
    match backend {
        "postgres" => vec![format!("ALTER TABLE {} ALTER COLUMN {} TYPE {} USING {}::{}", table, col, c.ty, col, c.ty)],
        "clickhouse" => vec![format!("ALTER TABLE {} MODIFY COLUMN {} {}", table, col, c.ty)],
        // SQLite changes a column only by making its table anew
        _ => {
            let name = held(backend, &t.name);
            let fresh = quoted(backend, &format!("{}_new", name));
            let kept: Vec<String> =
                has.iter().filter(|(n, _)| t.columns.iter().any(|c| same(backend, n, &held(backend, &c.name)))).map(|(n, _)| quoted(backend, n)).collect();
            vec![
                format!("CREATE TABLE {} ({})", fresh, t.body.trim()),
                format!("INSERT INTO {} ({}) SELECT {} FROM {}", fresh, kept.join(", "), kept.join(", "), table),
                format!("DROP TABLE {}", table),
                format!("ALTER TABLE {} RENAME TO {}", fresh, quoted(backend, &name)),
            ]
        }
    }
}

/// A url as a message shows it: without its password.
pub fn shown_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://")
        && let Some((auth, host)) = rest.split_once('@')
        && let Some((user, _)) = auth.split_once(':')
    {
        return format!("{}://{}@{}", scheme, user, host);
    }
    url.to_string()
}

/// Differences as a stop's words: each (with why reconciling does not
/// make it, when that is the stop), and the statements that would mend it.
fn report(head: String, diffs: &[&Difference], reasons: bool) -> String {
    let mut out = head;
    for d in diffs {
        out.push_str("\n  ");
        out.push_str(&d.what);
        if let (true, Some(why)) = (reasons, &d.refused) {
            out.push_str(" — ");
            out.push_str(why);
        }
        for m in &d.mend {
            out.push_str("\n      ");
            out.push_str(m);
            out.push(';');
        }
    }
    out
}

// ── at connect ──────────────────────────────────────────────────────

/// Statements on a handle, outside any transaction: what verifying reads
/// through.
struct Direct<'a>(&'a DbHandle);

impl DbTx for Direct<'_> {
    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<CrowFrame, String> {
        self.0.query(sql, params)
    }
    fn exec(&self, sql: &str, params: &[serde_json::Value]) -> Result<i64, String> {
        self.0.exec(sql, params)
    }
}

/// Check that the database holds the tables declared on it, in `mode`:
/// reconciling makes what it lacks, or stops with what it will not make,
/// making nothing; verifying stops with every difference.
pub(crate) fn check(handle: &DbHandle, url: &str, tables: &[Table], mode: Mode) -> Result<(), String> {
    let backend = handle.backend_name().to_string();
    match mode {
        Mode::Trust => Ok(()),
        Mode::Verify => {
            let diffs = differences(&Direct(handle), &backend, tables, false)?;
            if diffs.is_empty() {
                return Ok(());
            }
            let head = format!("the database at {} does not hold what the program declares (schema = 'verify'):", shown_url(url));
            Err(report(head, &diffs.iter().collect::<Vec<_>>(), false))
        }
        Mode::Reconcile => handle.schema_lock(&|tx| {
            let diffs = differences(tx, &backend, tables, false)?;
            let refused: Vec<&Difference> = diffs.iter().filter(|d| d.refused.is_some()).collect();
            if !refused.is_empty() {
                let head = format!("the database at {} differs from what the program declares in ways reconciling will not make (schema = 'reconcile'):", shown_url(url));
                return Err(report(head, &refused, true));
            }
            for d in &diffs {
                for m in &d.mend {
                    tx.exec(m, &[])?;
                }
            }
            Ok(())
        }),
    }
}

/// Open a connection — the one `connect` of both modes — and check the
/// database holds the tables the program declares on it (`declared`: the
/// statements declaring them, as the lowering found them), in the mode the
/// program runs in. A `pool_size` of `None` or 0 is the backend's default.
pub fn connect(url: &str, pool_size: Option<usize>, declared: &[&str]) -> Result<DbHandle, String> {
    let handle = crate::connect(url, pool_size.filter(|n| *n > 0))?;
    let tables: Vec<Table> = declared.iter().filter_map(|s| parse_table(s)).collect();
    // (a replay has no database to compare: its reads are what was kept)
    if tables.is_empty() || crate::fixtures::replaying_for(url).is_some() {
        return Ok(handle);
    }
    let mode = mode()?;
    if mode == Mode::Trust {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| eprintln!("schema not verified — schema = 'trust'"));
        return Ok(handle);
    }
    // a database opened new holds no tables: nothing to compare
    if url.strip_prefix("sqlite://").is_some_and(|p| p.is_empty() || p.contains(":memory:")) {
        return Ok(handle);
    }
    // each database once a run for what is declared on it: parts that open
    // one connection check it once
    static CHECKED: Mutex<BTreeSet<(String, Vec<String>)>> = Mutex::new(BTreeSet::new());
    let key = (url.to_string(), declared.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    if CHECKED.lock().unwrap_or_else(|p| p.into_inner()).contains(&key) {
        return Ok(handle);
    }
    check(&handle, url, &tables, mode)?;
    CHECKED.lock().unwrap_or_else(|p| p.into_inner()).insert(key);
    Ok(handle)
}

/// What `crow db diff` prints for one connection: the statements that
/// would make the database at `url` hold what is declared on it —
/// reconciling's, a declared table it lacks included, and, commented out,
/// the changes reconciling will not make. Only the catalog is read; a
/// SQLite file that is not there is not made.
pub fn diff(url: &str, declared: &[&str]) -> Result<String, String> {
    let tables: Vec<Table> = declared.iter().filter_map(|s| parse_table(s)).collect();
    let absent = url.strip_prefix("sqlite://").is_some_and(|p| p.is_empty() || p.contains(":memory:") || !std::path::Path::new(p).exists());
    let diffs = if absent {
        tables.iter().map(|t| Difference { what: format!("table '{}' is missing", t.shown()), mend: vec![t.create.clone()], refused: None }).collect()
    } else {
        let handle = crate::connect(url, Some(1))?;
        let backend = handle.backend_name().to_string();
        differences(&Direct(&handle), &backend, &tables, true)?
    };
    let mut out = String::new();
    for d in &diffs {
        out.push_str(&format!("-- {}\n", d.what));
        match &d.refused {
            None => {
                for m in &d.mend {
                    out.push_str(&format!("{};\n", m));
                }
            }
            Some(why) => {
                out.push_str(&format!("-- not made by reconciling: {}\n", why));
                for m in &d.mend {
                    for line in format!("{};", m).lines() {
                        out.push_str(&format!("-- {}\n", line));
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(sql: &[&str]) -> Vec<Table> {
        sql.iter().filter_map(|s| parse_table(s)).collect()
    }

    /// Against a Postgres of its own (`CROW_TEST_POSTGRES=postgres://…`):
    /// reconciling under the advisory lock, verifying, and the catalog read
    /// with names folded as Postgres folds them.
    #[cfg(feature = "postgres")]
    #[test]
    #[ignore = "needs a Postgres: CROW_TEST_POSTGRES=postgres://user@host:port/db"]
    fn reconciling_a_postgres_database() {
        let url = std::env::var("CROW_TEST_POSTGRES").expect("CROW_TEST_POSTGRES names a scratch database");
        let h = crate::connect(&url, Some(2)).unwrap();
        h.exec("DROP TABLE IF EXISTS crow_schema_cards", &[]).unwrap();
        h.exec("CREATE TABLE crow_schema_cards (id SERIAL PRIMARY KEY, points TEXT)", &[]).unwrap();
        h.exec("INSERT INTO crow_schema_cards (points) VALUES ('three')", &[]).unwrap();
        let declared = tables(&[
            "CREATE TABLE IF NOT EXISTS Crow_Schema_Cards (id SERIAL PRIMARY KEY, points TEXT, title VARCHAR(40), done BOOLEAN NOT NULL DEFAULT false, at TIMESTAMPTZ)",
        ]);
        let refused = check(&h, &url, &declared, Mode::Verify).unwrap_err();
        assert!(refused.contains("table 'Crow_Schema_Cards' lacks column 'title'\n      ALTER TABLE \"crow_schema_cards\" ADD COLUMN title VARCHAR(40);"), "{}", refused);
        check(&h, &url, &declared, Mode::Reconcile).unwrap();
        check(&h, &url, &declared, Mode::Verify).unwrap();
        let done = h.query("SELECT done FROM crow_schema_cards", &[]).unwrap();
        assert_eq!(done.col_as_strings("done").unwrap(), vec!["false"]);
        let declared = tables(&["CREATE TABLE IF NOT EXISTS crow_schema_cards (id SERIAL PRIMARY KEY, points INTEGER, due TEXT NOT NULL)"]);
        let refused = check(&h, &url, &declared, Mode::Reconcile).unwrap_err();
        assert!(refused.contains("lacks column 'due' — it is NOT NULL without a default, and the table has rows"), "{}", refused);
        assert!(refused.contains("column 'points' of table 'crow_schema_cards' is text in the database, declared INTEGER"), "{}", refused);
        assert!(refused.contains("ALTER TABLE \"crow_schema_cards\" ALTER COLUMN \"points\" TYPE INTEGER USING \"points\"::INTEGER;"), "{}", refused);
        // processes starting at once, each with a pool of its own, add a
        // column once: the lock holds the rest until it is made
        h.exec("DROP TABLE crow_schema_cards", &[]).unwrap();
        h.exec("CREATE TABLE crow_schema_cards (id SERIAL PRIMARY KEY)", &[]).unwrap();
        let declared = tables(&["CREATE TABLE IF NOT EXISTS crow_schema_cards (id SERIAL PRIMARY KEY, title TEXT, n INTEGER)"]);
        std::thread::scope(|s| {
            for _ in 0..4 {
                let (url, declared) = (url.clone(), declared.clone());
                s.spawn(move || {
                    let own = DbHandle::new(crate::postgres::CrowPostgresDb::connect(&url, 1).unwrap());
                    check(&own, &url, &declared, Mode::Reconcile).unwrap();
                });
            }
        });
        check(&h, &url, &declared, Mode::Verify).unwrap();
        h.exec("DROP TABLE crow_schema_cards", &[]).unwrap();
    }

    #[test]
    fn a_create_statement_declares_its_table_and_columns() {
        let t = parse_table(
            "CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, \"Title\" TEXT NOT NULL, points NUMERIC(10, 2) DEFAULT 0, \
             board INTEGER REFERENCES boards(id), UNIQUE (board, \"Title\"), FOREIGN KEY (board) REFERENCES boards(id)) STRICT;",
        )
        .expect("a table");
        assert_eq!(t.name, Name { text: "cards".into(), quoted: false });
        assert_eq!(t.schema, None);
        assert_eq!(t.create, "CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, \"Title\" TEXT NOT NULL, points NUMERIC(10, 2) DEFAULT 0, board INTEGER REFERENCES boards(id), UNIQUE (board, \"Title\"), FOREIGN KEY (board) REFERENCES boards(id)) STRICT");
        let cols: Vec<(&str, &str, bool, bool, bool)> = t.columns.iter().map(|c| (c.name.text.as_str(), c.ty.as_str(), c.not_null, c.filled, c.key)).collect();
        assert_eq!(
            cols,
            vec![
                ("id", "INTEGER", true, false, true),
                ("Title", "TEXT", true, false, false),
                ("points", "NUMERIC(10, 2)", false, true, false),
                ("board", "INTEGER", false, false, false),
            ]
        );
        assert!(t.columns[1].name.quoted);
        assert_eq!(t.columns[2].def, "points NUMERIC(10, 2) DEFAULT 0");
        // a schema, ClickHouse's clauses, a column named key
        let t = parse_table("create table analytics.events (ts DateTime64(3, 'UTC'), key String, v Nullable(Float64) CODEC(ZSTD), INDEX i v TYPE minmax GRANULARITY 1) ENGINE = MergeTree ORDER BY (ts, key)").expect("a table");
        assert_eq!(t.schema.map(|s| s.text), Some("analytics".to_string()));
        let cols: Vec<(&str, &str)> = t.columns.iter().map(|c| (c.name.text.as_str(), c.ty.as_str())).collect();
        assert_eq!(cols, vec![("ts", "DateTime64(3, 'UTC')"), ("key", "String"), ("v", "Nullable(Float64)")]);
        // a column with no type, a serial, a comment
        let t = parse_table("CREATE TABLE t (a, id SERIAL NOT NULL, -- the name\n name TEXT)").expect("a table");
        let cols: Vec<(&str, &str, bool)> = t.columns.iter().map(|c| (c.name.text.as_str(), c.ty.as_str(), c.filled)).collect();
        assert_eq!(cols, vec![("a", "", false), ("id", "SERIAL", true), ("name", "TEXT", false)]);
        // what declares no table
        assert_eq!(parse_table("CREATE TABLE t AS SELECT 1 AS one"), None);
        assert_eq!(parse_table("CREATE INDEX i ON t (a)"), None);
        assert_eq!(parse_table("CREATE TEMP TABLE t (a INTEGER)"), None);
        assert_eq!(dropped_table("DROP TABLE IF EXISTS \"Cards\"").map(|n| n.text), Some("Cards".to_string()));
        assert_eq!(dropped_table("DROP INDEX i"), None);
    }

    #[test]
    fn a_type_is_of_the_kind_its_cells_are_read_as() {
        assert_eq!(postgres_kind("timestamp(3) with time zone"), "datetime");
        assert_eq!(postgres_kind("TIMESTAMPTZ"), "datetime");
        assert_eq!(postgres_kind("NUMERIC(10, 2)"), "float");
        assert_eq!(postgres_kind("character varying"), "str");
        assert_eq!(postgres_kind("INTEGER[]"), "str");
        assert_eq!(postgres_kind("BIGSERIAL"), "int");
        #[cfg(feature = "clickhouse")]
        {
            assert_eq!(clickhouse_kind("INT"), "int");
            assert_eq!(clickhouse_kind("Int32"), "int");
            assert_eq!(clickhouse_kind("TEXT"), "str");
            assert_eq!(clickhouse_kind("LowCardinality(Nullable(String))"), "str");
            assert_eq!(clickhouse_kind("DateTime64(3)"), "datetime");
        }
        assert_eq!(shown_url("postgres://ada:secret@db.example/app"), "postgres://ada@db.example/app");
        assert_eq!(shown_url("sqlite://data@2.db"), "sqlite://data@2.db");
    }

    #[cfg(feature = "sqlite")]
    mod sqlite {
        use super::super::*;
        use super::tables;

        /// A file database in a directory of its own, removed when dropped.
        struct Scratch(std::path::PathBuf);

        impl Scratch {
            fn new(name: &str) -> (Scratch, String) {
                let dir = std::env::temp_dir().join(format!("crow_schema_{}_{}", name, std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                let url = format!("sqlite://{}", dir.join("app.db").display());
                (Scratch(dir), url)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn columns(h: &DbHandle, table: &str) -> Vec<String> {
            h.query(&format!("SELECT name FROM pragma_table_info('{}')", table), &[]).unwrap().col_as_strings("name").unwrap_or_default()
        }

        const CARDS: &str = "CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, title TEXT, done INTEGER NOT NULL DEFAULT 0)";

        /// Reconciling adds the columns a table the database has lacks, and
        /// then verifying finds nothing to say; a table it lacks is left to
        /// the program's own statement.
        #[test]
        fn reconciling_adds_the_columns_a_table_lacks() {
            let (_dir, url) = Scratch::new("adds");
            let h = crate::connect(&url, None).unwrap();
            h.exec("CREATE TABLE cards (id INTEGER PRIMARY KEY)", &[]).unwrap();
            h.exec("INSERT INTO cards (id) VALUES (1)", &[]).unwrap();
            let declared = tables(&[CARDS, "CREATE TABLE IF NOT EXISTS boards (id INTEGER PRIMARY KEY)"]);
            let refused = check(&h, &url, &declared, Mode::Verify).unwrap_err();
            assert!(refused.contains("table 'cards' lacks column 'title'\n      ALTER TABLE \"cards\" ADD COLUMN title TEXT;"), "{}", refused);
            assert!(refused.contains("table 'cards' lacks column 'done'"), "{}", refused);
            assert!(!refused.contains("boards"), "{}", refused);
            check(&h, &url, &declared, Mode::Reconcile).unwrap();
            assert_eq!(columns(&h, "cards"), vec!["id", "title", "done"]);
            assert_eq!(columns(&h, "boards"), Vec::<String>::new());
            check(&h, &url, &declared, Mode::Verify).unwrap();
            let done = h.query("SELECT done FROM cards", &[]).unwrap();
            assert_eq!(done.col_as_strings("done").unwrap(), vec!["0"]);
        }

        /// What reconciling cannot make without a guess or a loss stops it,
        /// and nothing is made: a NOT NULL column without a default for a
        /// table with rows, a column of another kind.
        #[test]
        fn reconciling_refuses_what_would_lose_data_or_guess() {
            let (_dir, url) = Scratch::new("refuses");
            let h = crate::connect(&url, None).unwrap();
            h.exec("CREATE TABLE cards (id INTEGER PRIMARY KEY, points TEXT)", &[]).unwrap();
            h.exec("INSERT INTO cards (id, points) VALUES (1, 'three')", &[]).unwrap();
            let declared = tables(&["CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, points TEXT, title TEXT, due TEXT NOT NULL)"]);
            let refused = check(&h, &url, &declared, Mode::Reconcile).unwrap_err();
            assert!(refused.contains("table 'cards' lacks column 'due' — it is NOT NULL without a default, and the table has rows"), "{}", refused);
            assert!(refused.contains("ALTER TABLE \"cards\" ADD COLUMN due TEXT NOT NULL;"), "{}", refused);
            // all or nothing: 'title' was not added either
            assert_eq!(columns(&h, "cards"), vec!["id", "points"]);
            let declared = tables(&["CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, points INTEGER)"]);
            for mode in [Mode::Reconcile, Mode::Verify] {
                let refused = check(&h, &url, &declared, mode).unwrap_err();
                assert!(refused.contains("column 'points' of table 'cards' is TEXT in the database, declared INTEGER"), "{}", refused);
                assert!(refused.contains("CREATE TABLE \"cards_new\" (id INTEGER PRIMARY KEY, points INTEGER);"), "{}", refused);
            }
        }

        /// A column SQLite cannot add in place, to a table with no rows, is
        /// made by making the table anew.
        #[test]
        fn an_empty_table_is_made_anew_for_a_column_sqlite_cannot_add() {
            let (_dir, url) = Scratch::new("anew");
            let h = crate::connect(&url, None).unwrap();
            h.exec("CREATE TABLE cards (id INTEGER PRIMARY KEY)", &[]).unwrap();
            let declared = tables(&["CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, due TEXT NOT NULL)"]);
            check(&h, &url, &declared, Mode::Reconcile).unwrap();
            assert_eq!(columns(&h, "cards"), vec!["id", "due"]);
        }

        /// Two processes starting at once reconcile one after the other: the
        /// second finds the column made.
        #[test]
        fn two_connections_reconcile_one_after_the_other() {
            let (dir, url) = Scratch::new("race");
            let first = crate::connect(&url, None).unwrap();
            first.exec("CREATE TABLE cards (id INTEGER PRIMARY KEY)", &[]).unwrap();
            let path = dir.0.join("app.db").display().to_string();
            let declared = tables(&[CARDS]);
            std::thread::scope(|s| {
                for _ in 0..4 {
                    let (path, url, declared) = (path.clone(), url.clone(), declared.clone());
                    s.spawn(move || {
                        // a connection of its own, as another process has
                        let h = DbHandle::new(crate::sqlite::CrowSqliteDb::open(&path).unwrap());
                        check(&h, &url, &declared, Mode::Reconcile).unwrap();
                    });
                }
            });
            assert_eq!(columns(&first, "cards"), vec!["id", "title", "done"]);
        }

        /// `crow db diff`'s draft: a missing table's statement, a missing
        /// column's, and, commented out, what reconciling will not make.
        #[test]
        fn the_diff_drafts_every_change_and_comments_out_the_unsafe() {
            let (_dir, url) = Scratch::new("diff");
            let h = crate::connect(&url, None).unwrap();
            h.exec("CREATE TABLE cards (id INTEGER PRIMARY KEY, points TEXT)", &[]).unwrap();
            let draft = diff(&url, &[CARDS, "CREATE TABLE IF NOT EXISTS boards (id INTEGER PRIMARY KEY)", "CREATE TABLE IF NOT EXISTS points (p TEXT)"]).unwrap();
            assert!(draft.contains("-- table 'cards' lacks column 'title'\nALTER TABLE \"cards\" ADD COLUMN title TEXT;\n"), "{}", draft);
            assert!(draft.contains("-- table 'boards' is missing\nCREATE TABLE IF NOT EXISTS boards (id INTEGER PRIMARY KEY);\n"), "{}", draft);
            let draft = diff(&url, &["CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, points INTEGER)"]).unwrap();
            assert!(draft.contains("-- not made by reconciling: "), "{}", draft);
            assert!(draft.contains("-- DROP TABLE \"cards\";"), "{}", draft);
            // a file that is not there is not made
            let absent = url.replace("app.db", "absent.db");
            assert!(diff(&absent, &[CARDS]).unwrap().contains("-- table 'cards' is missing"));
            assert!(!std::path::Path::new(absent.trim_start_matches("sqlite://")).exists());
        }
    }
}
