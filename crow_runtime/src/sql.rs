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
