/// Rendering for db_check hazards: plain-language messages with a two-actor
/// timeline for concurrency hazards and a ranked fix menu. The exact wording
/// is golden-tested below — detectors must satisfy these messages, not the
/// other way around.

use super::{FixKind, FixOption, Hazard, HazardKind};

/// Render the full report section. Concurrency hazards first, then
/// verification findings. Returns None when there is nothing to say.
pub fn render_all(hazards: &[Hazard], source: &str) -> Option<String> {
    if hazards.is_empty() {
        return None;
    }
    let ordered = ordered(hazards);
    let mut out = String::new();
    for (i, h) in ordered.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&render_hazard(h, i + 1, source));
    }
    Some(out)
}

/// Display order: concurrency hazards first, then verification findings, by
/// line. `crow fix --hazard n` resolves n against this same ordering.
pub fn ordered(hazards: &[Hazard]) -> Vec<&Hazard> {
    let mut ordered: Vec<&Hazard> = hazards.iter().collect();
    ordered.sort_by_key(|h| (!h.kind.is_concurrency(), h.span.0));
    ordered
}

/// Render one hazard, numbered `n` (1-indexed, used by `crow fix --hazard n`).
pub fn render_hazard(h: &Hazard, n: usize, source: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "{}:{} — {}\n",
        h.file,
        span_text(h.span),
        headline(&h.kind)
    ));
    s.push('\n');
    excerpt(&mut s, source, h.span);
    body(&mut s, h);
    let fixes = fix_options(&h.kind);
    if !fixes.is_empty() {
        s.push('\n');
        s.push_str("  fixes, safest first:\n");
        for (i, f) in fixes.iter().enumerate() {
            s.push_str(&format!("  [{}] {}\n", i + 1, f.headline));
            for line in &f.body {
                s.push_str(&format!("      {}\n", line));
            }
        }
        if fixes.iter().any(|f| !matches!(f.kind, FixKind::Advice)) {
            s.push('\n');
            s.push_str(&format!(
                "  apply: crow fix {} --hazard {} --option 1\n",
                h.file, n
            ));
        }
    }
    if h.kind.is_concurrency() {
        s.push_str(&format!(
            "  accept as-is: put `--- crow: single-writer` on the line above line {}\n",
            h.span.0
        ));
    }
    s
}

fn span_text(span: (usize, usize)) -> String {
    if span.0 == span.1 {
        format!("{}", span.0)
    } else {
        format!("{}-{}", span.0, span.1)
    }
}

pub(crate) fn headline(k: &HazardKind) -> String {
    match k {
        HazardKind::WriteSkew { .. } => "two programs can pass this check at the same time".into(),
        HazardKind::LostUpdate { column, .. } => {
            format!("concurrent updates to '{}' can vanish here", column)
        }
        HazardKind::OneWayLatch { column, .. } => {
            format!("'{}' only ever moves one way", column)
        }
        HazardKind::AssumedUnique { .. } => {
            "this code assumes one matching row; the schema allows many".into()
        }
        HazardKind::UnpairedWrites { .. } => "these writes succeed or fail separately".into(),
        HazardKind::UnknownTable { table, suggestion } => match suggestion {
            Some(sg) => format!(
                "table '{}' does not exist in the database — did you mean '{}'? (a query on a DIFFERENT database's connection is out of this check's scope)",
                table, sg
            ),
            None => format!(
                "table '{}' does not exist in the database — check the name, or note this query may target a different database than the checked one",
                table
            ),
        },
        HazardKind::UnknownColumn { table, column, .. } => {
            format!("column '{}.{}' does not exist", table, column)
        }
        HazardKind::TypeMismatch {
            table,
            column,
            db_type,
            used_as,
        } => format!(
            "column '{}.{}' is {} in the database but used as {}",
            table, column, db_type, used_as
        ),
        HazardKind::ParamCountMismatch {
            placeholders,
            given,
        } => format!(
            "this SQL has {} placeholder{} but {} value{} given",
            placeholders,
            plural(*placeholders),
            given,
            plural(*given)
        ),
        HazardKind::InterpolatedSql => {
            "SQL built by string interpolation — injection risk".into()
        }
    }
}

fn excerpt(s: &mut String, source: &str, span: (usize, usize)) {
    let lines: Vec<&str> = source.lines().collect();
    let start = span.0.max(1);
    let end = span.1.min(lines.len()).max(start);
    let mut shown = 0;
    for n in start..=end {
        if n > lines.len() {
            break;
        }
        if shown == 4 && end - start >= 4 {
            s.push_str("      │ …\n");
            break;
        }
        s.push_str(&format!("  {:>3} │ {}\n", n, lines[n - 1]));
        shown += 1;
    }
}

fn body(s: &mut String, h: &Hazard) {
    match &h.kind {
        HazardKind::WriteSkew {
            invariant,
            read_line,
            write_line,
            ..
        } => {
            s.push('\n');
            s.push_str("  what can happen:\n");
            s.push_str(&format!(
                "    1. program A runs the query (line {}) — the check passes\n",
                read_line
            ));
            s.push_str(
                "    2. program B runs the same query — the check passes too (A hasn't written yet)\n",
            );
            s.push_str(&format!(
                "    3. A writes (line {}).  4. B writes.\n",
                write_line
            ));
            s.push_str("    5. the rule the check was protecting is now broken\n");
            s.push('\n');
            s.push_str(&format!(
                "  the database has no rule saying '{}',\n",
                invariant
            ));
            s.push_str(
                "  so this if is the only guard — and an if cannot hold between two programs.\n",
            );
        }
        HazardKind::LostUpdate {
            table,
            column,
            read_line,
            write_line,
        } => {
            s.push('\n');
            s.push_str("  what can happen:\n");
            s.push_str(&format!(
                "    1. program A reads {}.{} (line {}) — say it is 100\n",
                table, column, read_line
            ));
            s.push_str(&format!("    2. program B reads {} — also 100\n", column));
            s.push_str(&format!(
                "    3. A computes and writes its new value (line {})\n",
                write_line
            ));
            s.push_str("    4. B computes from the stale 100 and writes over it\n");
            s.push_str("    5. A's update is gone, silently\n");
        }
        HazardKind::OneWayLatch {
            table,
            column,
            set_value,
            write_line,
            gate_line,
        } => {
            s.push('\n');
            s.push_str(&format!(
                "  '{}.{}' is set to {} (line {}) and never set back; work is gated on it (line {}).\n",
                table, column, set_value, write_line, gate_line
            ));
            s.push_str(
                "  a flag that moves one way records that something happened once — not what is true now.\n",
            );
            s.push_str(
                "  when new work appears after the flag flips, that work is invisible forever.\n",
            );
        }
        HazardKind::AssumedUnique { table, column, access_line } => {
            s.push('\n');
            s.push_str(&format!(
                "  line {} takes one row from this result, but '{}.{}' has no unique rule —\n",
                access_line, table, column
            ));
            s.push_str("  two rows can match, and which one you get is luck.\n");
        }
        HazardKind::UnpairedWrites {
            tables,
            write_lines,
        } => {
            s.push('\n');
            s.push_str(&format!(
                "  a crash between line {} and line {} leaves {} half-updated — the first write applied, the rest lost.\n",
                write_lines.first().unwrap_or(&0),
                write_lines.last().unwrap_or(&0),
                tables.join(", ")
            ));
        }
        HazardKind::UnknownTable { suggestion, .. }
        | HazardKind::UnknownColumn { suggestion, .. } => {
            if let Some(sug) = suggestion {
                s.push_str(&format!("  did you mean '{}'?\n", sug));
            }
        }
        HazardKind::TypeMismatch { .. } | HazardKind::ParamCountMismatch { .. } => {}
        HazardKind::InterpolatedSql => {
            s.push_str(
                "  a value interpolated into SQL text becomes part of the command itself.\n",
            );
            s.push_str("  use ? placeholders and pass values in the params array.\n");
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The ranked fix menu for a hazard. `crow fix` executes the same list,
/// so menu text and applied behavior cannot drift apart.
pub fn fix_options(k: &HazardKind) -> Vec<FixOption> {
    match k {
        HazardKind::WriteSkew {
            table,
            key_cols,
            partial,
            ..
        } => {
            let (name, sql) = unique_index_sql(table, key_cols, partial);
            vec![
                FixOption {
                    headline: format!(
                        "let the database enforce it — generates migrations/{}.sql:",
                        name
                    ),
                    body: vec![
                        format!("  {}", sql),
                        "the losing write then returns an error (kind = 'conflict') instead of breaking the rule.".into(),
                        "protects every writer — including programs that are not this one.".into(),
                    ],
                    kind: FixKind::Migration { name, sql },
                },
                FixOption {
                    headline: "fuse the check and the write into one atomic statement".into(),
                    body: vec![],
                    kind: FixKind::Fusion,
                },
                FixOption {
                    headline: "run the check and the write in a retrying serializable transaction".into(),
                    body: vec![],
                    kind: FixKind::TransactWrap,
                },
            ]
        }
        HazardKind::LostUpdate { column, .. } => vec![
            FixOption {
                headline: "move the arithmetic into the database — one atomic statement:".into(),
                body: vec![format!("  UPDATE … SET {c} = {c} + ? WHERE …", c = column)],
                kind: FixKind::Fusion,
            },
            FixOption {
                headline: "run the read and the write in a retrying serializable transaction".into(),
                body: vec![],
                kind: FixKind::TransactWrap,
            },
        ],
        HazardKind::OneWayLatch { column, .. } => vec![
            FixOption {
                headline: format!(
                    "derive it: replace reads of '{}' with a query of the ground truth it summarizes",
                    column
                ),
                body: vec![],
                kind: FixKind::Advice,
            },
            FixOption {
                headline: "keep the flag, but add a reconciliation job that re-derives it on a schedule".into(),
                body: vec![],
                kind: FixKind::Advice,
            },
        ],
        HazardKind::AssumedUnique { table, column, .. } => {
            let (name, sql) = unique_index_sql(table, &[column.clone()], &[]);
            vec![
                FixOption {
                    headline: format!(
                        "make it actually unique — generates migrations/{}.sql:",
                        name
                    ),
                    body: vec![format!("  {}", sql)],
                    kind: FixKind::Migration { name, sql },
                },
                FixOption {
                    headline: "handle multiple rows explicitly (sort and choose, or aggregate)".into(),
                    body: vec![],
                    kind: FixKind::Advice,
                },
            ]
        }
        HazardKind::UnpairedWrites { .. } => vec![FixOption {
            headline: "run them inside transact[…] so they succeed or fail together".into(),
            body: vec![],
            kind: FixKind::TransactWrap,
        }],
        // Verification findings: the message itself is the fix.
        _ => vec![],
    }
}

/// Build the CREATE UNIQUE INDEX statement and its migration name.
fn unique_index_sql(
    table: &str,
    key_cols: &[String],
    partial: &[(String, String)],
) -> (String, String) {
    let name = if partial.len() == 1 {
        format!("one_{}_per_{}", partial[0].1, key_cols.join("_"))
    } else {
        format!("unique_{}_{}", table, key_cols.join("_"))
    };
    let mut sql = format!(
        "CREATE UNIQUE INDEX {} ON {}({})",
        name,
        table,
        key_cols.join(", ")
    );
    if !partial.is_empty() {
        let preds: Vec<String> = partial
            .iter()
            .map(|(c, v)| format!("{} = '{}'", c, v))
            .collect();
        sql.push_str(&format!(" WHERE {}", preds.join(" AND ")));
    }
    sql.push(';');
    (name, sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(kind: HazardKind, span: (usize, usize)) -> Hazard {
        Hazard {
            kind,
            file: "trading.co".into(),
            span,
        }
    }

    const TRADING_SRC: &str = "\
use db

conn = connect['sqlite://trading.db']

fn hello[]
    log['hi']

fn f[]
    log['x']
\n\
\n\
\n\
open_trades = query[conn, 'SELECT id FROM trades WHERE symbol = ? AND status = ?', [sym, 'active']]
if len[open_trades] eq 0
    query[conn, 'INSERT INTO trades (symbol, qty, entry) VALUES (?, ?, ?)', [sym, qty, price]]
";

    #[test]
    fn write_skew_message() {
        let hz = h(
            HazardKind::WriteSkew {
                table: "trades".into(),
                invariant: "at most one active trades row per symbol".into(),
                key_cols: vec!["symbol".into()],
                partial: vec![("status".into(), "active".into())],
                read_line: 13,
                write_line: 15,
            },
            (13, 15),
        );
        let expected = "\
trading.co:13-15 — two programs can pass this check at the same time

   13 │ open_trades = query[conn, 'SELECT id FROM trades WHERE symbol = ? AND status = ?', [sym, 'active']]
   14 │ if len[open_trades] eq 0
   15 │     query[conn, 'INSERT INTO trades (symbol, qty, entry) VALUES (?, ?, ?)', [sym, qty, price]]

  what can happen:
    1. program A runs the query (line 13) — the check passes
    2. program B runs the same query — the check passes too (A hasn't written yet)
    3. A writes (line 15).  4. B writes.
    5. the rule the check was protecting is now broken

  the database has no rule saying 'at most one active trades row per symbol',
  so this if is the only guard — and an if cannot hold between two programs.

  fixes, safest first:
  [1] let the database enforce it — generates migrations/one_active_per_symbol.sql:
        CREATE UNIQUE INDEX one_active_per_symbol ON trades(symbol) WHERE status = 'active';
      the losing write then returns an error (kind = 'conflict') instead of breaking the rule.
      protects every writer — including programs that are not this one.
  [2] fuse the check and the write into one atomic statement
  [3] run the check and the write in a retrying serializable transaction

  apply: crow fix trading.co --hazard 1 --option 1
  accept as-is: put `--- crow: single-writer` on the line above line 13
";
        assert_eq!(render_hazard(&hz, 1, TRADING_SRC), expected);
    }

    #[test]
    fn lost_update_message() {
        let src = "\
row = query[conn, 'SELECT balance FROM accounts WHERE id = ?', [acct]]
bal = row.balance[0] + amount
query[conn, 'UPDATE accounts SET balance = ? WHERE id = ?', [bal, acct]]
";
        let hz = Hazard {
            kind: HazardKind::LostUpdate {
                table: "accounts".into(),
                column: "balance".into(),
                read_line: 1,
                write_line: 3,
            },
            file: "bank.co".into(),
            span: (1, 3),
        };
        let expected = "\
bank.co:1-3 — concurrent updates to 'balance' can vanish here

    1 │ row = query[conn, 'SELECT balance FROM accounts WHERE id = ?', [acct]]
    2 │ bal = row.balance[0] + amount
    3 │ query[conn, 'UPDATE accounts SET balance = ? WHERE id = ?', [bal, acct]]

  what can happen:
    1. program A reads accounts.balance (line 1) — say it is 100
    2. program B reads balance — also 100
    3. A computes and writes its new value (line 3)
    4. B computes from the stale 100 and writes over it
    5. A's update is gone, silently

  fixes, safest first:
  [1] move the arithmetic into the database — one atomic statement:
        UPDATE … SET balance = balance + ? WHERE …
  [2] run the read and the write in a retrying serializable transaction

  apply: crow fix bank.co --hazard 1 --option 1
  accept as-is: put `--- crow: single-writer` on the line above line 1
";
        assert_eq!(render_hazard(&hz, 1, src), expected);
    }

    #[test]
    fn one_way_latch_message() {
        let src = "\
query[conn, 'UPDATE rooms SET checked_out = 1 WHERE room = ?', [room_no]]
";
        let hz = Hazard {
            kind: HazardKind::OneWayLatch {
                table: "rooms".into(),
                column: "checked_out".into(),
                set_value: "1".into(),
                write_line: 1,
                gate_line: 9,
            },
            file: "hotel.co".into(),
            span: (1, 1),
        };
        let expected = "\
hotel.co:1 — 'checked_out' only ever moves one way

    1 │ query[conn, 'UPDATE rooms SET checked_out = 1 WHERE room = ?', [room_no]]

  'rooms.checked_out' is set to 1 (line 1) and never set back; work is gated on it (line 9).
  a flag that moves one way records that something happened once — not what is true now.
  when new work appears after the flag flips, that work is invisible forever.

  fixes, safest first:
  [1] derive it: replace reads of 'checked_out' with a query of the ground truth it summarizes
  [2] keep the flag, but add a reconciliation job that re-derives it on a schedule
  accept as-is: put `--- crow: single-writer` on the line above line 1
";
        assert_eq!(render_hazard(&hz, 1, src), expected);
    }

    #[test]
    fn assumed_unique_message() {
        let src = "\
u = query[conn, 'SELECT name FROM users WHERE email = ?', [email]]
name = u.name[0]
";
        let hz = Hazard {
            kind: HazardKind::AssumedUnique {
                table: "users".into(),
                column: "email".into(),
                access_line: 2,
            },
            file: "app.co".into(),
            span: (1, 2),
        };
        let expected = "\
app.co:1-2 — this code assumes one matching row; the schema allows many

    1 │ u = query[conn, 'SELECT name FROM users WHERE email = ?', [email]]
    2 │ name = u.name[0]

  line 2 takes one row from this result, but 'users.email' has no unique rule —
  two rows can match, and which one you get is luck.

  fixes, safest first:
  [1] make it actually unique — generates migrations/unique_users_email.sql:
        CREATE UNIQUE INDEX unique_users_email ON users(email);
  [2] handle multiple rows explicitly (sort and choose, or aggregate)

  apply: crow fix app.co --hazard 1 --option 1
  accept as-is: put `--- crow: single-writer` on the line above line 1
";
        assert_eq!(render_hazard(&hz, 1, src), expected);
    }

    #[test]
    fn unpaired_writes_message() {
        let src = "\
query[conn, 'UPDATE accounts SET balance = balance - ? WHERE id = ?', [amt, from_id]]
query[conn, 'UPDATE accounts SET balance = balance + ? WHERE id = ?', [amt, to_id]]
";
        let hz = Hazard {
            kind: HazardKind::UnpairedWrites {
                tables: vec!["accounts".into()],
                write_lines: vec![1, 2],
            },
            file: "transfer.co".into(),
            span: (1, 2),
        };
        let expected = "\
transfer.co:1-2 — these writes succeed or fail separately

    1 │ query[conn, 'UPDATE accounts SET balance = balance - ? WHERE id = ?', [amt, from_id]]
    2 │ query[conn, 'UPDATE accounts SET balance = balance + ? WHERE id = ?', [amt, to_id]]

  a crash between line 1 and line 2 leaves accounts half-updated — the first write applied, the rest lost.

  fixes, safest first:
  [1] run them inside transact[…] so they succeed or fail together

  apply: crow fix transfer.co --hazard 1 --option 1
  accept as-is: put `--- crow: single-writer` on the line above line 1
";
        assert_eq!(render_hazard(&hz, 1, src), expected);
    }

    #[test]
    fn verification_messages() {
        let src = "df = query[conn, 'SELECT nmae FROM users']\n";
        let hz = Hazard {
            kind: HazardKind::UnknownColumn {
                table: "users".into(),
                column: "nmae".into(),
                suggestion: Some("name".into()),
            },
            file: "app.co".into(),
            span: (1, 1),
        };
        let expected = "\
app.co:1 — column 'users.nmae' does not exist

    1 │ df = query[conn, 'SELECT nmae FROM users']
  did you mean 'name'?
";
        assert_eq!(render_hazard(&hz, 1, src), expected);
    }
}
