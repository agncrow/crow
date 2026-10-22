/// `crow fix` — apply a chosen option from a hazard's fix menu, then
/// self-verify: the file must still parse and a re-run of the checks must no
/// longer report the hazard. Nothing is applied silently; every action taken
/// is reported back.

use crate::ast::{BinOp, Expr, Stmt};
use std::fmt::Write as _;

use super::report;
use super::sql_facts::{self, SqlKind};
use super::{FixKind, Hazard, HazardKind};

/// Where `crow fix` writes a constraint migration, and how it lands: the
/// check's report names the same file and the same command, read the same
/// way. A database's migrations go in `migrations/`, applied as `crow db
/// apply` applies a directory, each recorded in `_crow_migrations` (a fix
/// had run its statement unrecorded, so the next `crow db apply` ran it
/// again: "index … already exists"). An offline schema's — `--db
/// migrations://dir`, or a directory — go in that directory, which is the
/// schema the check reads; there is no database to apply them to (the
/// url had been handed to `connect`: "unknown database URL scheme").
pub struct Migrations {
    pub dir: String,
    /// The database the directory is applied to, when there is one.
    pub apply_to: Option<String>,
    /// The directory is the schema the check reads.
    pub offline: bool,
}

impl Migrations {
    pub fn for_url(db_url: Option<&str>) -> Migrations {
        let offline = |dir: &str| Migrations { dir: dir.to_string(), apply_to: None, offline: true };
        match db_url {
            Some(url) => match url.strip_prefix("migrations://") {
                Some(dir) => offline(dir),
                None if std::path::Path::new(url).is_dir() => offline(url),
                None => Migrations { dir: "migrations".to_string(), apply_to: Some(url.to_string()), offline: false },
            },
            None => Migrations { dir: "migrations".to_string(), apply_to: None, offline: false },
        }
    }

    /// The file the migration named `name` is written as: numbered one past
    /// the directory's highest (`0001_one_active_per_symbol.sql` in an
    /// empty one), as `crow db apply` runs them in name order.
    pub fn next_path(&self, name: &str) -> String {
        let next = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_string_lossy().split('_').next().and_then(|p| p.parse::<u32>().ok()))
                    .max()
                    .unwrap_or(0)
                    + 1
            })
            .unwrap_or(1);
        format!("{}/{:04}_{}.sql", self.dir.trim_end_matches('/'), next, name)
    }
}

pub fn apply(
    file: &str,
    source: &str,
    db_url: Option<&str>,
    hazard_n: usize,
    option_m: usize,
) -> Result<String, String> {
    let result = super::run(file, source, db_url)?;
    let migrations = Migrations::for_url(db_url);
    let ordered = report::ordered(&result.hazards);
    let Some(hazard) = ordered.get(hazard_n.wrapping_sub(1)) else {
        return Err(format!(
            "no hazard #{} — crow check reported {} finding{}",
            hazard_n,
            ordered.len(),
            if ordered.len() == 1 { "" } else { "s" }
        ));
    };
    let options = report::fix_options(&hazard.kind, &migrations);
    let Some(option) = options.get(option_m.wrapping_sub(1)) else {
        return Err(format!(
            "hazard #{} has {} option{} — asked for option {}",
            hazard_n,
            options.len(),
            if options.len() == 1 { "" } else { "s" },
            option_m
        ));
    };

    let mut log = String::new();
    match &option.kind {
        FixKind::Migration { name, sql } => {
            apply_migration(name, sql, &migrations, &mut log)?;
            reverify(file, source, db_url, hazard, &mut log)?;
        }
        FixKind::Fusion => match &hazard.kind {
            HazardKind::LostUpdate { .. } => {
                let new_source = fuse_lost_update(source, hazard)?;
                std::fs::write(file, &new_source)
                    .map_err(|e| format!("could not write {}: {}", file, e))?;
                writeln!(log, "rewrote {} — arithmetic moved into the UPDATE", file).ok();
                reverify(file, &new_source, db_url, hazard, &mut log)?;
            }
            HazardKind::WriteSkew { table, .. } => {
                writeln!(
                    log,
                    "this rewrite is not automated yet — the manual fusion for {}:",
                    table
                )
                .ok();
                writeln!(
                    log,
                    "  replace the query + if + INSERT with one statement:\n  INSERT INTO {} (…) SELECT …\n  WHERE NOT EXISTS (<your check's WHERE clause>)\n  note: on postgres this still needs the option-1 constraint to be race-proof",
                    table
                )
                .ok();
            }
            _ => return Err("no automated fusion for this hazard kind".into()),
        },
        FixKind::TransactWrap => match &hazard.kind {
            HazardKind::UnpairedWrites { write_lines, .. } => {
                let new_source = wrap_in_transact(source, write_lines)?;
                std::fs::write(file, &new_source)
                    .map_err(|e| format!("could not write {}: {}", file, e))?;
                writeln!(log, "rewrote {} — writes wrapped in transact[…]", file).ok();
                reverify(file, &new_source, db_url, hazard, &mut log)?;
            }
            _ => {
                writeln!(
                    log,
                    "not automated for this shape yet — wrap the read and write in a named fn and run it via transact[conn, fn] (serializable mode)"
                )
                .ok();
            }
        },
        FixKind::Advice => {
            writeln!(log, "this option is guidance, not a rewrite:").ok();
            writeln!(log, "  {}", option.headline).ok();
        }
    }
    Ok(log)
}

/// Write the migration as its numbered file, then apply the directory as
/// `crow db apply` does: each file the database has not applied yet, in
/// name order, recorded in `_crow_migrations`.
fn apply_migration(name: &str, sql: &str, migrations: &Migrations, log: &mut String) -> Result<(), String> {
    std::fs::create_dir_all(&migrations.dir).map_err(|e| format!("could not create {}/: {}", migrations.dir, e))?;
    let path = migrations.next_path(name);
    std::fs::write(&path, format!("{}\n", sql)).map_err(|e| format!("could not write {}: {}", path, e))?;
    writeln!(log, "wrote {}", path).ok();
    match &migrations.apply_to {
        Some(url) => {
            let said = crate::driver::apply_migrations(&migrations.dir, url)?;
            writeln!(log, "{} (crow db apply {} {})", said, migrations.dir, url).ok();
        }
        None if migrations.offline => {
            writeln!(log, "the schema read from {}/ holds it now — `crow db apply {} <url>` applies it to a database", migrations.dir, migrations.dir).ok();
        }
        None => {
            writeln!(log, "no --db url — the migration is written, not applied: `crow db apply {} <url>` applies it", migrations.dir).ok();
        }
    }
    Ok(())
}

/// Re-run the checks after a fix; confirm the hazard is gone.
fn reverify(
    file: &str,
    source: &str,
    db_url: Option<&str>,
    fixed: &Hazard,
    log: &mut String,
) -> Result<(), String> {
    // The file must still parse.
    crate::build(source).map_err(|e| format!("fix broke the file — reverted needed: {}", e))?;
    let after = super::run(file, source, db_url)?;
    let same_kind_same_place = after.hazards.iter().any(|h| {
        std::mem::discriminant(&h.kind) == std::mem::discriminant(&fixed.kind)
            && h.span.0 == fixed.span.0
    });
    if same_kind_same_place {
        writeln!(
            log,
            "⚠ re-check: the hazard is still reported — the fix did not take"
        )
        .ok();
    } else {
        writeln!(
            log,
            "✓ re-check: hazard no longer reported ({} finding{} remain)",
            after.hazards.len(),
            if after.hazards.len() == 1 { "" } else { "s" }
        )
        .ok();
    }
    Ok(())
}

/// Canonical lost-update fusion:
///   x = query[.. 'SELECT col FROM t WHERE k = ?' ..]
///   d = x.col[0] + delta
///   query[.. 'UPDATE t SET col = ?' .., [d, ..]]
/// becomes an in-database increment; the derived assign is removed when it
/// has no other use.
fn fuse_lost_update(source: &str, hazard: &Hazard) -> Result<String, String> {
    let HazardKind::LostUpdate {
        column, write_line, ..
    } = &hazard.kind
    else {
        return Err("internal: wrong hazard kind".into());
    };

    // Find the UPDATE call and its parameter variable feeding the column.
    let stmts = crate::build(source)?;
    let facts = sql_facts::collect(&stmts);
    let update = facts
        .calls
        .iter()
        .find(|c| c.line == *write_line && c.kind == SqlKind::Update)
        .ok_or("could not re-locate the UPDATE call")?;
    let slot_idx = update
        .slots
        .iter()
        .position(|s| s.as_deref() == Some(column.as_str()))
        .ok_or("could not locate the column's placeholder")?;
    let Some(sql_facts::ParamLit::Var(Some(param_var))) =
        update.params.as_ref().and_then(|p| p.get(slot_idx))
    else {
        return Err("the UPDATE parameter is not a plain variable".into());
    };

    // Find `param_var = <read>.col[0] + delta` and extract the delta text.
    let delta = find_delta_var(&stmts, param_var, column)
        .ok_or("the derivation is not the canonical read + delta shape")?;

    let mut lines: Vec<String> = source.lines().map(|l| l.to_string()).collect();
    let wl = *write_line - 1; // 0-indexed
    let old = &lines[wl];
    let needle = format!("SET {} = ?", column);
    let replacement = format!("SET {c} = {c} + ?", c = column);
    if !old.contains(&needle) {
        return Err(format!("expected '{}' on line {}", needle, write_line));
    }
    let mut new_line = old.replace(&needle, &replacement);
    // Swap the derived variable for the delta in the params array.
    new_line = replace_word(&new_line, param_var, &delta);
    lines[wl] = new_line;

    // Remove the derivation line if the variable has no remaining use.
    let uses: usize = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| *i != wl && contains_word(l, param_var))
        .map(|(i, _)| i)
        .count();
    if uses == 1 {
        let assign_idx = lines
            .iter()
            .position(|l| {
                contains_word(l, param_var) && l.trim_start().starts_with(&format!("{} =", param_var))
            })
            .ok_or("could not locate the derivation line")?;
        lines.remove(assign_idx);
    }

    Ok(lines.join("\n") + "\n")
}

/// Find `var = <expr>.col[0] + delta` (either operand order) and return the
/// delta expression's source text (identifier only, v1).
fn find_delta_var(stmts: &[Stmt], var: &str, col: &str) -> Option<String> {
    fn search(stmts: &[Stmt], var: &str, col: &str) -> Option<String> {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, value } if name == var => {
                    if let Expr::BinOp { left, op, right } = value {
                        if matches!(op, BinOp::Add) {
                            if is_col_zero_index(left, col) {
                                if let Expr::Ident(d) = right.as_ref() {
                                    return Some(d.clone());
                                }
                            }
                            if is_col_zero_index(right, col) {
                                if let Expr::Ident(d) = left.as_ref() {
                                    return Some(d.clone());
                                }
                            }
                        }
                    }
                }
                Stmt::FnDecl { body, .. } => {
                    if let Some(d) = search(body, var, col) {
                        return Some(d);
                    }
                }
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    if let Some(d) = search(then_body, var, col) {
                        return Some(d);
                    }
                    if let Some(eb) = else_body {
                        if let Some(d) = search(eb, var, col) {
                            return Some(d);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
    search(stmts, var, col)
}

fn is_col_zero_index(e: &Expr, col: &str) -> bool {
    if let Expr::Call { callee, args } = e {
        if args.len() == 1 && matches!(args[0], Expr::Int(0)) {
            if let Expr::FieldAccess { field, .. } = callee.as_ref() {
                return field == col;
            }
        }
    }
    false
}

/// Wrap contiguous write lines in a generated fn + transact call.
fn wrap_in_transact(source: &str, write_lines: &[usize]) -> Result<String, String> {
    let min = *write_lines.iter().min().unwrap();
    let max = *write_lines.iter().max().unwrap();
    if (max - min + 1) != write_lines.len() {
        return Err(
            "the writes are not on consecutive lines — wrap them in a fn + transact[…] manually"
                .into(),
        );
    }
    let lines: Vec<&str> = source.lines().collect();
    let block: Vec<&str> = lines[min - 1..max].to_vec();

    // Connection variable: first arg of the first exec.
    let conn = block[0]
        .split('[')
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .map(|s| s.trim().to_string())
        .ok_or("could not identify the connection variable")?;

    let indent: String = block[0].chars().take_while(|c| c.is_whitespace()).collect();
    let mut fn_name = "run_together".to_string();
    let mut n = 1;
    while source.contains(&format!("fn {}", fn_name)) {
        n += 1;
        fn_name = format!("run_together{}", n);
    }

    let mut replacement = Vec::new();
    replacement.push(format!("{}fn {}[t]", indent, fn_name));
    for l in &block {
        let body_line = l.trim_start().replacen(&format!("[{},", conn), "[t,", 1);
        replacement.push(format!("{}    {}", indent, body_line));
    }
    replacement.push(format!("{}transact[{}, {}]", indent, conn, fn_name));

    let mut out: Vec<String> = lines[..min - 1].iter().map(|l| l.to_string()).collect();
    out.extend(replacement);
    out.extend(lines[max..].iter().map(|l| l.to_string()));
    Ok(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_update_fusion_rewrites_canonically() {
        let src = std::fs::read_to_string("tests/db_check/lost_update.co").unwrap();
        let hazard = Hazard {
            kind: HazardKind::LostUpdate {
                table: "accounts".into(),
                column: "balance".into(),
                read_line: 8,
                write_line: 10,
            },
            file: "lost_update.co".into(),
            span: (8, 10),
        };
        let fused = fuse_lost_update(&src, &hazard).unwrap();
        assert!(fused.contains("SET balance = balance + ? WHERE id = ?"));
        assert!(fused.contains("[amount, acct]"));
        assert!(!fused.contains("bal = "), "derived assign should be removed");
        crate::build(&fused).expect("fused source must still parse");
    }

    /// A constraint migration lands as `crow db apply` lands one: written as
    /// the next numbered file and recorded in `_crow_migrations`, so the
    /// next `crow db apply` of the directory has nothing to run (the fix had
    /// run its statement unrecorded, and the apply ran it again: "index …
    /// already exists"); an offline schema's directory takes the file, and
    /// nothing connects to its `migrations://` url
    #[test]
    fn a_migration_is_written_numbered_and_applied_as_crow_db_apply_does() {
        let dir = std::env::temp_dir().join(format!("crow_fix_migration_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("migrations")).unwrap();
        let db_path = dir.join("dev.db");
        let url = format!("sqlite://{}", db_path.display());
        let db = crow_db::connect(&url, Some(1)).unwrap();
        db.exec("CREATE TABLE trades (id INTEGER PRIMARY KEY, symbol TEXT NOT NULL, status TEXT NOT NULL)", &[]).unwrap();
        std::fs::write(dir.join("migrations/0001_init_notes.sql"), "CREATE TABLE notes (body TEXT)\n").unwrap();
        let migrations = Migrations { dir: dir.join("migrations").display().to_string(), apply_to: Some(url.clone()), offline: false };
        assert!(migrations.next_path("one_active_per_symbol").ends_with("migrations/0002_one_active_per_symbol.sql"));
        let mut log = String::new();
        apply_migration("one_active_per_symbol", "CREATE UNIQUE INDEX one_active_per_symbol ON trades(symbol) WHERE status = 'active';", &migrations, &mut log).unwrap();
        assert!(log.contains("0002_one_active_per_symbol.sql") && log.contains("applied 2 migrations"), "{}", log);
        let applied = db.query("SELECT name FROM _crow_migrations ORDER BY name", &[]).unwrap();
        assert_eq!(applied.nrows(), 2, "both recorded");
        assert_eq!(crate::driver::apply_migrations(&migrations.dir, &url).unwrap(), format!("applied 0 migrations from {} on {}", migrations.dir, url));
        // offline: the directory is the schema, and nothing is connected to
        let offline = Migrations::for_url(Some(&format!("migrations://{}", dir.join("schema").display())));
        assert!(offline.offline && offline.apply_to.is_none());
        let mut log = String::new();
        apply_migration("unique_users_email", "CREATE UNIQUE INDEX unique_users_email ON users(email);", &offline, &mut log).unwrap();
        assert!(dir.join("schema/0001_unique_users_email.sql").exists() && log.contains("crow db apply"), "{}", log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transact_wrap_rewrites_contiguous_writes() {
        let src = std::fs::read_to_string("tests/db_check/unpaired.co").unwrap();
        let wrapped = wrap_in_transact(&src, &[9, 10]).unwrap();
        assert!(wrapped.contains("fn run_together[t]"));
        assert!(wrapped.contains("query[t, 'UPDATE accounts SET balance = balance - ?"));
        assert!(wrapped.contains("transact[conn, run_together]"));
        crate::build(&wrapped).expect("wrapped source must still parse");
    }
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|t| t == word)
}

fn replace_word(haystack: &str, word: &str, with: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    for c in haystack.chars() {
        if c.is_alphanumeric() || c == '_' {
            token.push(c);
        } else {
            if token == word {
                out.push_str(with);
            } else {
                out.push_str(&token);
            }
            token.clear();
            out.push(c);
        }
    }
    if token == word {
        out.push_str(with);
    } else {
        out.push_str(&token);
    }
    out
}
