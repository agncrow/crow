/// The hazard detectors. Each takes the extracted SQL facts (and, where
/// needed, the introspected schema) and produces Hazards for the report.
/// Detectors fire conservatively: no hazard without demonstrable evidence.

use super::schema::Schema;
use super::sql_facts::{Facts, ParamLit, SqlCall};
use super::{Hazard, HazardKind};

/// 1f — SQL-vs-schema verification plus injection flags.
pub fn verify(facts: &Facts, schema: &Schema, file: &str) -> Vec<Hazard> {
    let mut out = Vec::new();
    for call in &facts.calls {
        if call.interpolated {
            out.push(Hazard {
                kind: HazardKind::InterpolatedSql,
                file: file.into(),
                span: (call.line, call.line),
            });
            continue;
        }
        if call.unparsed {
            continue;
        }
        let Some(table) = &call.table else { continue };
        let Some(ts) = schema.table(table) else {
            out.push(Hazard {
                kind: HazardKind::UnknownTable {
                    table: table.clone(),
                    suggestion: closest(table, schema.tables.keys()),
                },
                file: file.into(),
                span: (call.line, call.line),
            });
            continue;
        };

        let mentioned: Vec<&String> = call
            .select_cols
            .iter()
            .chain(call.insert_cols.iter())
            .chain(call.set_cols.iter().map(|(c, _)| c))
            .chain(call.where_eq.iter().map(|(c, _)| c))
            .collect();
        for col in mentioned {
            if !ts.has_column(col) {
                out.push(Hazard {
                    kind: HazardKind::UnknownColumn {
                        table: table.clone(),
                        column: col.clone(),
                        suggestion: closest(col, ts.columns.keys()),
                    },
                    file: file.into(),
                    span: (call.line, call.line),
                });
            }
        }

        if let Some(params) = &call.params {
            if call.placeholders != params.len() {
                out.push(Hazard {
                    kind: HazardKind::ParamCountMismatch {
                        placeholders: call.placeholders,
                        given: params.len(),
                    },
                    file: file.into(),
                    span: (call.line, call.line),
                });
            } else {
                out.extend(param_type_mismatches(call, params, ts, table, file));
            }
        }
    }
    out
}

fn param_type_mismatches(
    call: &SqlCall,
    params: &[ParamLit],
    ts: &super::schema::TableSchema,
    table: &str,
    file: &str,
) -> Vec<Hazard> {
    let mut out = Vec::new();
    for (i, slot) in call.slots.iter().enumerate() {
        let (Some(col), Some(lit)) = (slot, params.get(i)) else {
            continue;
        };
        let Some(db_type) = ts.columns.get(col) else {
            continue;
        };
        let numeric_col = db_type.contains("INT") || db_type.contains("REAL")
            || db_type.contains("FLOA") || db_type.contains("DOUB")
            || db_type.contains("NUM") || db_type.contains("DEC");
        let text_col = db_type.contains("TEXT") || db_type.contains("CHAR")
            || db_type.contains("CLOB");
        let used_as = match lit {
            ParamLit::Str(s) if numeric_col && s.parse::<f64>().is_err() => Some("text"),
            ParamLit::Int(_) | ParamLit::Float(_) if text_col => Some("a number"),
            _ => None,
        };
        if let Some(used_as) = used_as {
            out.push(Hazard {
                kind: HazardKind::TypeMismatch {
                    table: table.to_string(),
                    column: col.clone(),
                    db_type: db_type.clone(),
                    used_as: used_as.into(),
                },
                file: file.into(),
                span: (call.line, call.line),
            });
        }
    }
    out
}

/// 1c — one-way latch: a column only ever UPDATEd to a single constant,
/// while a read elsewhere gates work on a *different* value of it.
pub fn latch(facts: &Facts, file: &str) -> Vec<Hazard> {
    use super::sql_facts::{SetVal, SqlKind};
    use std::collections::BTreeMap;

    // (table, col) → (constant values written, first write line, disqualified)
    let mut writes: BTreeMap<(String, String), (Vec<String>, usize, bool)> = BTreeMap::new();
    for call in facts.calls.iter().filter(|c| c.kind == SqlKind::Update) {
        let Some(table) = &call.table else { continue };
        for (col, sv) in &call.set_cols {
            let entry = writes
                .entry((table.clone(), col.clone()))
                .or_insert((Vec::new(), call.line, false));
            match sv {
                SetVal::Constant(v) => entry.0.push(v.clone()),
                _ => entry.2 = true, // written from a variable → not one-way
            }
        }
    }

    let mut out = Vec::new();
    for ((table, col), (values, write_line, disqualified)) in &writes {
        if *disqualified || values.is_empty() {
            continue;
        }
        let first = &values[0];
        if !values.iter().all(|v| v == first) {
            continue; // moves both ways
        }
        // A read gating on a different value of this column.
        let gate = facts.calls.iter().find(|c| {
            c.kind == SqlKind::Select
                && c.table.as_deref() == Some(table.as_str())
                && c.where_eq.iter().any(|(wc, rhs)| {
                    wc == col
                        && matches!(rhs, super::sql_facts::Rhs::Literal(v) if v != first)
                })
        });
        if let Some(gate) = gate {
            out.push(Hazard {
                kind: HazardKind::OneWayLatch {
                    table: table.clone(),
                    column: col.clone(),
                    set_value: first.clone(),
                    write_line: *write_line,
                    gate_line: gate.line,
                },
                file: file.into(),
                span: (*write_line, *write_line),
            });
        }
    }
    out
}

/// 1e — two or more UPDATE/DELETE writes in one scope with no transaction.
/// INSERTs are excluded: sequential appends are a normal logging pattern.
pub fn unpaired_writes(facts: &Facts, file: &str) -> Vec<Hazard> {
    use super::sql_facts::SqlKind;
    use std::collections::BTreeMap;

    let mut by_scope: BTreeMap<Option<String>, Vec<&SqlCall>> = BTreeMap::new();
    for call in &facts.calls {
        if matches!(call.kind, SqlKind::Update | SqlKind::Delete) {
            by_scope.entry(call.func.clone()).or_default().push(call);
        }
    }

    let mut out = Vec::new();
    for (scope, writes) in by_scope {
        if writes.len() < 2 {
            continue;
        }
        if let Some(fn_name) = &scope {
            if facts.transact_fns.contains(fn_name) {
                continue; // transact[conn, fn] wraps this scope
            }
        }
        let mut tables: Vec<String> = writes
            .iter()
            .filter_map(|w| w.table.clone())
            .collect();
        tables.dedup();
        let lines: Vec<usize> = writes.iter().map(|w| w.line).collect();
        out.push(Hazard {
            kind: HazardKind::UnpairedWrites {
                tables,
                write_lines: lines.clone(),
            },
            file: file.into(),
            span: (
                *lines.iter().min().unwrap_or(&0),
                *lines.iter().max().unwrap_or(&0),
            ),
        });
    }
    out
}

/// Nearest name within edit distance 2 — for did-you-mean suggestions.
pub fn closest<'a>(
    target: &str,
    candidates: impl Iterator<Item = &'a String>,
) -> Option<String> {
    candidates
        .map(|c| (levenshtein(target, c), c))
        .filter(|(d, _)| *d > 0 && *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.clone())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}
