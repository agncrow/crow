//! Reading SQL text for what it touches: the tables a statement reads and
//! writes. The compiler reads a program's literals with it (a derivation's
//! read set, a handler's writes); the database layer reads the statements
//! it runs with it (the interpreter's handlers, whose writes wake readers).

/// The tables a SQL literal reads and writes: (reads, writes). A regex
/// over FROM / JOIN / INSERT INTO / UPDATE / DELETE FROM — every dialect
/// Crow talks to agrees on those words.
pub fn tables_of_sql(sql: &str) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let words: Vec<&str> = sql.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == ';').filter(|w| !w.is_empty()).collect();
    let clean = |w: &str| w.trim_matches(|c: char| c == '`' || c == '"' || c == '\'').to_string();
    let mut i = 0;
    while i < words.len() {
        let w = words[i].to_ascii_uppercase();
        match w.as_str() {
            "FROM" | "JOIN" => {
                if let Some(t) = words.get(i + 1) {
                    let t = clean(t);
                    // FROM (SELECT …) is a subquery, not a table
                    let keyword = matches!(t.to_ascii_uppercase().as_str(), "SELECT" | "VALUES" | "WITH");
                    if !keyword && !t.is_empty() && !t.starts_with('?') && !t.starts_with('\'') && t.chars().next().map_or(false, |c| c.is_alphabetic()) {
                        // DELETE FROM t writes; a FROM after DELETE is the target
                        if i > 0 && words[i - 1].eq_ignore_ascii_case("DELETE") { writes.push(t); } else { reads.push(t); }
                    }
                }
            }
            "INTO" => {
                if i > 0 && words[i - 1].eq_ignore_ascii_case("INSERT") {
                    if let Some(t) = words.get(i + 1) { let t = clean(t); if !t.is_empty() { writes.push(t); } }
                }
            }
            "UPDATE" => {
                if let Some(t) = words.get(i + 1) { let t = clean(t); if !t.is_empty() { writes.push(t); } }
            }
            _ => {}
        }
        i += 1;
    }
    reads.sort(); reads.dedup(); writes.sort(); writes.dedup();
    (reads, writes)
}

/// What a write says about the rows it touches — for invalidating only the
/// derivations that read rows it may have changed (invalidation by key,
/// session-layer-design.md §3.5).
#[derive(Debug, Clone, PartialEq)]
pub struct WriteKeys {
    pub table: String,
    /// A value every row of the write has in a column: an INSERT's values
    /// (one per row), the equalities of an UPDATE's or DELETE's WHERE.
    pub known: Vec<(String, serde_json::Value)>,
    /// Columns an UPDATE sets, or an upsert may: a row can move between any
    /// two values of these.
    pub changed: Vec<String>,
    /// Whether `known` is all the write touches. It is not for a WHERE with
    /// anything but equalities joined by AND, for no WHERE, for an INSERT of
    /// a SELECT: then every reader of the table is touched.
    pub exact: bool,
}

/// The keys of a write (`INSERT`, `UPDATE`, `DELETE`), with its `?`
/// parameters in order; `None` for anything else.
pub fn write_keys(sql: &str, params: &[serde_json::Value]) -> Option<WriteKeys> {
    let toks = tokens(sql);
    let upper: Vec<String> = toks.iter().map(|t| t.to_ascii_uppercase()).collect();
    let at = |i: usize| upper.get(i).map(|s| s.as_str()).unwrap_or("");
    let mut next_param = 0usize;
    let clean = |t: &str| t.trim_matches(|c: char| c == '`' || c == '"').to_string();
    match at(0) {
        "INSERT" | "REPLACE" => {
            let into = upper.iter().position(|t| t == "INTO")?;
            let table = clean(toks.get(into + 1)?);
            let upsert = at(0) == "REPLACE" || at(1) == "OR" || upper.iter().any(|t| t == "CONFLICT" || t == "DUPLICATE");
            // the column list, then each row of VALUES
            let open = toks.iter().skip(into + 2).position(|t| t == "(")? + into + 2;
            let close = toks.iter().skip(open).position(|t| t == ")")? + open;
            let cols: Vec<String> = toks[open + 1..close].iter().filter(|t| *t != ",").map(|t| clean(t)).collect();
            let values = upper.iter().skip(close).position(|t| t == "VALUES").map(|p| p + close);
            let Some(values) = values else {
                // INSERT … SELECT: rows the write does not say
                return Some(WriteKeys { table, known: vec![], changed: cols, exact: false });
            };
            let mut known = Vec::new();
            let mut exact = true;
            let mut i = values + 1;
            while i < toks.len() && toks[i] == "(" {
                let end = toks.iter().skip(i).position(|t| t == ")")? + i;
                let row: Vec<&String> = toks[i + 1..end].iter().filter(|t| *t != ",").collect();
                if row.len() != cols.len() {
                    exact = false;
                }
                for (c, v) in cols.iter().zip(row) {
                    match value_at(v, params, &mut next_param) {
                        Some(v) => known.push((c.clone(), v)),
                        None => exact = false,
                    }
                }
                i = end + 1;
                if toks.get(i).map(|t| t.as_str()) == Some(",") {
                    i += 1;
                }
            }
            // an upsert may change a row that was there under another value
            let changed = if upsert { cols } else { vec![] };
            Some(WriteKeys { table, known, changed, exact })
        }
        "UPDATE" | "DELETE" => {
            let (table, from) = if at(0) == "UPDATE" { (clean(toks.get(1)?), 2) } else { (clean(toks.get(upper.iter().position(|t| t == "FROM")? + 1)?), 0) };
            let where_at = upper.iter().position(|t| t == "WHERE");
            // UPDATE's SET columns come first, and its ? before the WHERE's
            let mut changed = Vec::new();
            if at(0) == "UPDATE" {
                let set = upper.iter().skip(from).position(|t| t == "SET")? + from;
                let end = where_at.unwrap_or(toks.len());
                let mut j = set + 1;
                while j < end {
                    if toks.get(j + 1).map(|t| t.as_str()) == Some("=") {
                        changed.push(clean(&toks[j]));
                        // the value's own parameters are counted, not kept
                        let mut k = j + 2;
                        while k < end && toks[k] != "," {
                            if toks[k] == "?" {
                                next_param += 1;
                            }
                            k += 1;
                        }
                        j = k + 1;
                    } else {
                        j += 1;
                    }
                }
            }
            let Some(w) = where_at else {
                return Some(WriteKeys { table, known: vec![], changed, exact: false });
            };
            // equalities joined by AND, and nothing else, say the rows
            let mut known = Vec::new();
            let mut exact = true;
            let mut j = w + 1;
            while j < toks.len() {
                let is_eq = toks.get(j + 1).map(|t| t.as_str()) == Some("=");
                let joined = matches!(upper.get(j + 3).map(|s| s.as_str()), None | Some("AND") | Some(";"));
                if is_eq && joined {
                    match value_at(&toks[j + 2], params, &mut next_param) {
                        Some(v) => known.push((clean(&toks[j]), v)),
                        None => exact = false,
                    }
                    j += 4;
                } else {
                    exact = false;
                    break;
                }
            }
            Some(WriteKeys { table, known, changed, exact })
        }
        _ => None,
    }
}

/// The rows a query reads, when a simple one says them: one table, and the
/// columns its WHERE fixes by equalities joined with AND, each with the
/// place of its `?` among the query's parameters. `None` for a join, a
/// subquery, or a WHERE with anything else: the query reads the table.
pub fn read_keys(sql: &str) -> Option<(String, Vec<(String, usize)>)> {
    let toks = tokens(sql);
    let upper: Vec<String> = toks.iter().map(|t| t.to_ascii_uppercase()).collect();
    if upper.first().map(|s| s.as_str()) != Some("SELECT") {
        return None;
    }
    if upper.iter().any(|t| matches!(t.as_str(), "JOIN" | "UNION" | "INTERSECT" | "EXCEPT" | "WITH")) || upper.iter().filter(|t| *t == "SELECT").count() > 1 {
        return None;
    }
    let from = upper.iter().position(|t| t == "FROM")?;
    let table = toks.get(from + 1)?.trim_matches(|c: char| c == '`' || c == '"').to_string();
    if table.is_empty() || table == "(" {
        return None;
    }
    let where_at = upper.iter().position(|t| t == "WHERE")?;
    // a second table named after a comma is a join too
    if toks[from + 1..where_at].iter().any(|t| t == ",") {
        return None;
    }
    let placeholders_before = toks[..where_at].iter().filter(|t| *t == "?").count();
    let mut keys = Vec::new();
    let mut index = placeholders_before;
    let mut j = where_at + 1;
    let ends = |t: &str| matches!(t, "ORDER" | "GROUP" | "LIMIT" | "HAVING" | ";");
    while j < toks.len() && !ends(&upper[j]) {
        let is_eq = toks.get(j + 1).map(|t| t.as_str()) == Some("=");
        let joined = upper.get(j + 3).map_or(true, |t| t == "AND" || ends(t));
        if !(is_eq && joined) {
            return None;
        }
        let column = toks[j].trim_matches(|c: char| c == '`' || c == '"');
        // a column of the table, maybe named through its alias
        let column = column.rsplit('.').next().unwrap_or(column).to_string();
        if toks[j + 2] == "?" {
            keys.push((column, index));
            index += 1;
        }
        j += 3;
        if upper.get(j).map(|t| t.as_str()) == Some("AND") {
            j += 1;
        }
    }
    Some((table, keys))
}

/// The value a token stands for: a parameter by its place (`next` counts
/// the parameters met so far), or a literal.
fn value_at(tok: &str, params: &[serde_json::Value], next: &mut usize) -> Option<serde_json::Value> {
    if tok == "?" {
        let v = params.get(*next).cloned();
        *next += 1;
        return v;
    }
    if let Some(s) = tok.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Some(serde_json::Value::String(s.replace("''", "'")));
    }
    if let Ok(n) = tok.parse::<i64>() {
        return Some(serde_json::json!(n));
    }
    if let Ok(x) = tok.parse::<f64>() {
        return Some(serde_json::json!(x));
    }
    None
}

/// SQL's words, strings, `?`, and its punctuation as tokens of their own.
fn tokens(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\'' {
                    if chars.get(j + 1) == Some(&'\'') {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            out.push(chars[i..(j + 1).min(chars.len())].iter().collect());
            i = j + 1;
        } else if matches!(c, '(' | ')' | ',' | '=' | ';' | '?') {
            out.push(c.to_string());
            i += 1;
        } else {
            let mut j = i;
            while j < chars.len() && !chars[j].is_whitespace() && !matches!(chars[j], '(' | ')' | ',' | '=' | ';' | '?' | '\'') {
                j += 1;
            }
            out.push(chars[i..j].iter().collect());
            i = j;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_writes_keys() {
        let k = write_keys("INSERT INTO trades (session_id, symbol, qty) VALUES (?, ?, 10)", &[json!(7), json!("IMRN")]).unwrap();
        assert_eq!(k.table, "trades");
        assert_eq!(k.known, vec![("session_id".into(), json!(7)), ("symbol".into(), json!("IMRN")), ("qty".into(), json!(10))]);
        assert!(k.exact && k.changed.is_empty());

        let k = write_keys("DELETE FROM trades WHERE session_id = ?", &[json!(7)]).unwrap();
        assert_eq!(k.known, vec![("session_id".into(), json!(7))]);
        assert!(k.exact);

        let k = write_keys("UPDATE cards SET stars = ?, title = 'x' WHERE board = ? AND id = ?", &[json!(4), json!(2), json!(9)]).unwrap();
        assert_eq!(k.table, "cards");
        assert_eq!(k.known, vec![("board".into(), json!(2)), ("id".into(), json!(9))]);
        assert_eq!(k.changed, vec!["stars".to_string(), "title".to_string()]);
        assert!(k.exact);

        // what the write does not say: every reader of the table
        assert!(!write_keys("DELETE FROM trades", &[]).unwrap().exact);
        assert!(!write_keys("DELETE FROM trades WHERE qty > ?", &[json!(1)]).unwrap().exact);
        assert!(!write_keys("UPDATE t SET a = 1 WHERE b = ? OR c = ?", &[json!(1), json!(2)]).unwrap().exact);
        assert!(!write_keys("INSERT INTO t (a) SELECT a FROM u", &[]).unwrap().exact);
        let up = write_keys("INSERT OR REPLACE INTO t (id, owner) VALUES (?, ?)", &[json!(1), json!("b")]).unwrap();
        assert_eq!(up.changed, vec!["id".to_string(), "owner".to_string()]);
        assert!(write_keys("SELECT * FROM t", &[]).is_none());
    }

    #[test]
    fn a_reads_keys() {
        assert_eq!(read_keys("SELECT id, title FROM cards WHERE board = ? ORDER BY id"), Some(("cards".to_string(), vec![("board".to_string(), 0)])));
        assert_eq!(
            read_keys("SELECT count(*) AS n FROM trades t WHERE t.session_id = ? AND status = 'open' AND day = ?"),
            Some(("trades".to_string(), vec![("session_id".to_string(), 0), ("day".to_string(), 1)]))
        );
        // a placeholder before the WHERE takes its place
        assert_eq!(read_keys("SELECT ? AS a FROM t WHERE k = ?"), Some(("t".to_string(), vec![("k".to_string(), 1)])));
        // what reads more than a keyed slice of one table
        assert_eq!(read_keys("SELECT * FROM t"), None);
        assert_eq!(read_keys("SELECT * FROM a JOIN b ON a.id = b.id WHERE a.k = ?"), None);
        assert_eq!(read_keys("SELECT * FROM a, b WHERE a.k = ?"), None);
        assert_eq!(read_keys("SELECT * FROM t WHERE k > ?"), None);
        assert_eq!(read_keys("SELECT * FROM (SELECT * FROM t) WHERE k = ?"), None);
    }
}
