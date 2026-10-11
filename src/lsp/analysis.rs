/// Per-document analysis for the language server: run the same pipeline the
/// compiler runs (parse → expand → lower → infer → validate, plus db_check
/// when a database url is configured) and translate every error into an LSP
/// diagnostic. The parser runs in lenient mode so one broken statement never
/// hides the rest of the file.

use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::token::SpannedToken;
use crate::type_infer::TypeContext;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use std::path::Path;

pub struct Analysis {
    pub stmts: Vec<crate::ast::Stmt>,
    pub ctx: TypeContext,
    pub tokens: Vec<SpannedToken>,
    pub diagnostics: Vec<Diagnostic>,
    /// The analyzed document text — signature help walks it to find the
    /// enclosing call at the cursor.
    pub source: String,
    /// The parts the document uses, by the name it calls them (`calc` for
    /// `use lib.calc`), each its file under the project: a qualified name
    /// is defined there.
    pub parts: std::collections::BTreeMap<String, std::path::PathBuf>,
}

pub fn analyze(
    text: &str,
    doc_dir: Option<&Path>,
    db_url: Option<&str>,
    file_label: &str,
) -> Analysis {
    let mut diagnostics = Vec::new();

    crate::note_fn_docs("", text);
    // Tokenize. A lexer error poisons the whole file — report it and stop.
    let tokens = match Lexer::new(text).tokenize() {
        Ok(t) => t,
        Err(e) => {
            diagnostics.extend(refusal(text, &e, 1));
            return Analysis {
                source: text.to_string(),
                stmts: Vec::new(),
                ctx: TypeContext::new(),
                tokens: Vec::new(),
                diagnostics,
                parts: std::collections::BTreeMap::new(),
            };
        }
    };

    // Parse leniently: broken statements become diagnostics, the rest parse.
    let (stmts, parse_errors) = Parser::new(tokens.clone()).parse_lenient();
    // what each broken statement would have defined: a later read of it is
    // the same mistake, not a second one (spec, Error Messages, rule 3)
    let mut unparsed: Vec<String> = Vec::new();
    // a block with no indented body leaves its lines at the margin: a
    // `return` among them is the same mistake
    let unindented = parse_errors.iter().any(|(_, e)| e.contains("has no indented body"));
    for (line, e) in parse_errors.iter().filter(|(_, e)| !(unindented && e.contains("return leaves a function"))) {
        diagnostics.extend(refusal(text, e, *line));
        unparsed.extend(text.lines().nth(line.saturating_sub(1)).map(defined_by).unwrap_or_default());
        // a name the parser said is a keyword's slip (`esle`)
        if let Some(slip) = e.split("'").nth(1).filter(|_| e.contains("is not a word Crow knows")) {
            unparsed.push(slip.to_string());
        }
    }

    // The parts the document uses, by the name it calls them.
    let mut parts = std::collections::BTreeMap::new();
    if let Some(dir) = doc_dir {
        let root = crate::project_root(dir);
        for s in &stmts {
            if let crate::ast::Stmt::Use { path, alias } = s {
                let file = root.join(format!("{}.co", path.replace('.', "/")));
                if file.exists() {
                    let name = alias.clone().unwrap_or_else(|| path.rsplit('.').next().unwrap_or(path).to_string());
                    parts.insert(name, file);
                }
            }
        }
    }

    // Module expansion (missing modules are skipped, never fatal). A part
    // that fails validated alone is a diagnostic at the line that imports
    // it — or the first import, for one a part imports — and the document
    // is analysed as it stands (the server had exited on it).
    let mut parts_good = true;
    let stmts = match doc_dir {
        Some(dir) => {
            let mut visited = std::collections::HashSet::new();
            match crate::expand_module_imports_checked(&stmts, dir, &mut visited) {
                Ok(expanded) => expanded,
                Err(e) => {
                    for entry in e.lines().filter(|l| !l.trim().is_empty()) {
                        diagnostics.push(diag(import_line(&stmts, entry), 0, text, entry.to_string(), DiagnosticSeverity::ERROR));
                    }
                    parts_good = false;
                    stmts
                }
            }
        }
        None => {
            // (no expansion ran: no import of this document's is unresolved,
            // and no part's import stands beside its own)
            crate::UNRESOLVED_USES.with(|u| u.borrow_mut().clear());
            crate::PART_ONLY_IMPORTS.with(|p| p.borrow_mut().clear());
            stmts
        }
    };

    // Named-argument lowering. What it refuses is all the terminal says of
    // the program (its checks need the lowered form), so the editor says
    // the same: the checks are not run on the unlowered statements, which
    // hover and completion still read.
    let mut lowered_whole = true;
    let stmts = match crate::lower::lower_program(&stmts) {
        Ok(s) => s,
        Err(e) => {
            for entry in e.lines().filter(|l| !l.trim().is_empty()) {
                diagnostics.extend(refusal(text, entry, 1));
            }
            lowered_whole = false;
            stmts
        }
    };

    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);

    // the lowering refuses what is not a program, one "line N: message"
    // entry per error (not while a part it imports fails: its names would
    // all read as undefined)
    if parts_good && lowered_whole && let Err(refused) = crate::lower_ir::check(&stmts, &ctx) {
        let e = refused.message();
        // (inside multi-line statements, at the line that holds the name)
        let e = crate::refine_error_lines(text, &e);
        let mut seen: Vec<(Option<usize>, String)> = Vec::new();
        for entry in e.lines().filter(|l| !l.trim().is_empty()) {
            let d = crate::diagnostic::parse(entry).remove(0);
            // the same message on the same line once (spec, Error Messages)
            if seen.contains(&(d.line, d.message.clone())) {
                continue;
            }
            seen.push((d.line, d.message.clone()));
            diagnostics.extend(refusal(text, entry, 1));
        }
    }

    // Schema-aware db checks: hazards appear as warnings with the headline;
    // the full timeline and fix menu stay in `crow check`.
    if let Some(url) = db_url {
        if let Ok(result) = crate::db_check::run(file_label, text, Some(url)) {
            for (i, h) in crate::db_check::report::ordered(&result.hazards)
                .iter()
                .enumerate()
            {
                let mut d = diag(
                    h.span.0,
                    0,
                    text,
                    format!(
                        "{}\n(details: crow check --db …; apply: crow fix {} --hazard {})",
                        crate::db_check::report::headline(&h.kind),
                        file_label,
                        i + 1
                    ),
                    DiagnosticSeverity::WARNING,
                );
                d.source = Some("crow db".into());
                diagnostics.push(d);
            }
        }
    }

    if !unparsed.is_empty() {
        diagnostics.retain(|d| {
            let named = |kind: &str| unparsed.iter().any(|n| d.message.starts_with(&format!("undefined {} '{}'", kind, n)));
            !(named("variable") || named("function"))
        });
    }
    Analysis {
        source: text.to_string(),
        stmts,
        ctx,
        tokens,
        diagnostics,
        parts,
    }
}

/// The names a statement's first line defines, read from its text: `x =
/// …`, `fn f[a, b]` (its parameters too: the lines after a broken header
/// read them), `view v[…]`, `derive d`, `struct S […]`.
fn defined_by(line: &str) -> Vec<String> {
    let t = line.trim_start();
    let word = |s: &str| -> String { s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect() };
    for head in ["fn ", "view ", "derive ", "struct "] {
        if let Some(rest) = t.strip_prefix(head) {
            let rest = rest.trim_start();
            let n = word(rest);
            let mut names: Vec<String> = vec![n.clone()];
            if let Some(params) = rest[n.len()..].trim_start().strip_prefix('[').and_then(|p| p.split(']').next()) {
                names.extend(params.split(',').map(|p| word(p.trim())).filter(|p| !p.is_empty()));
            }
            return names.into_iter().filter(|n| !n.is_empty()).collect();
        }
    }
    let n = word(t);
    let after = t[n.len()..].trim_start();
    if !n.is_empty() && after.starts_with('=') && !after.starts_with("==") { vec![n] } else { Vec::new() }
}

/// The line of the import a part's error belongs to: the `use` naming the
/// part's file (`sim/clock.co: line 3: …` is `use sim.clock`), else the
/// document's first import.
fn import_line(stmts: &[crate::ast::Stmt], error: &str) -> usize {
    let file = error.split(": ").next().unwrap_or("");
    let mut line = 1usize;
    let mut first = None;
    for s in stmts {
        match s {
            crate::ast::Stmt::Line(n) => line = *n,
            crate::ast::Stmt::Use { path, .. } => {
                if file == format!("{}.co", path.replace('.', "/")) {
                    return line;
                }
                first.get_or_insert(line);
            }
            _ => {}
        }
    }
    first.unwrap_or(1)
}

/// A refusal's entry as the editor shows it: read as the terminal reads it
/// (crate::diagnostic), at its line (or `fallback`, 1-based, when it names
/// none), one diagnostic for each place its line holds what it is about.
fn refusal(text: &str, entry: &str, fallback: usize) -> Vec<Diagnostic> {
    let d = crate::diagnostic::parse(entry).into_iter().next().unwrap_or(crate::diagnostic::Diagnostic {
        file: None,
        line: None,
        col: None,
        message: entry.to_string(),
    });
    let line = d.line.unwrap_or(fallback).max(1);
    let line0 = (line - 1) as u32;
    let line_text = text.lines().nth(line0 as usize).unwrap_or("");
    let message = match &d.file {
        Some(part) => format!("{}: {}", part, d.message),
        None => d.message.clone(),
    };
    crate::diagnostic::underline(line_text, &d)
        .into_iter()
        .map(|(start, end)| Diagnostic {
            range: Range {
                start: Position { line: line0, character: start as u32 },
                end: Position { line: line0, character: end as u32 },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("crow".into()),
            message: message.clone(),
            ..Default::default()
        })
        .collect()
}

/// Build a diagnostic. Lines are 1-based on the Crow side, 0-based in LSP.
/// The range is the name the message points at on its line, else the
/// line's text from `col` (0-based), past its indentation, to its end.
fn diag(
    line_1based: usize,
    col_0based: u32,
    text: &str,
    message: String,
    severity: DiagnosticSeverity,
) -> Diagnostic {
    let line0 = line_1based.saturating_sub(1) as u32;
    let line_text = text.lines().nth(line0 as usize).unwrap_or("");
    let line_len = line_text.chars().count() as u32;
    let indent = line_text.chars().take_while(|c| c.is_whitespace()).count() as u32;
    let (start, end) = crate::diagnostic::spans(line_text, &message)
        .first()
        .map(|&(s, e)| (s as u32, e as u32))
        .unwrap_or((col_0based.max(indent).min(line_len), line_len.max(col_0based + 1)));
    Diagnostic {
        range: Range {
            start: Position {
                line: line0,
                character: start,
            },
            end: Position {
                line: line0,
                character: end,
            },
        },
        severity: Some(severity),
        source: Some("crow".into()),
        message,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_file_has_no_diagnostics() {
        let a = analyze("x = 1\nlog['`x`']\n", None, None, "t.co");
        assert!(a.diagnostics.is_empty(), "got {:?}", a.diagnostics);
    }

    #[test]
    fn broken_statement_does_not_hide_later_errors() {
        // Line 1 is unparseable; line 3 uses an undefined variable. Both
        // must be reported — that's what lenient parsing buys.
        let src = "fn = = broken\nx = 1\nlog[undefined_thing]\n";
        let a = analyze(src, None, None, "t.co");
        assert!(
            a.diagnostics.len() >= 2,
            "expected parse error AND validator error, got {:?}",
            a.diagnostics
        );
        assert!(a.diagnostics.iter().any(|d| d.range.start.line == 0));
        assert!(a.diagnostics.iter().any(|d| d.range.start.line == 2));
    }

    #[test]
    fn validator_line_prefix_is_stripped() {
        let a = analyze("log[nope]\n", None, None, "t.co");
        assert_eq!(a.diagnostics.len(), 1);
        let msg = &a.diagnostics[0].message;
        assert!(
            !msg.starts_with("line "),
            "location prefix should be stripped: {}",
            msg
        );
    }

    #[test]
    fn undefined_var_flagged_even_with_use_db() {
        let src = "x = 1\n\nfn broken[ = =\n\nlog[not_defined_anywhere]\n";
        let a = analyze(src, None, None, "t.co");
        assert!(
            a.diagnostics
                .iter()
                .any(|d| d.message.contains("not_defined_anywhere")),
            "expected undefined-variable diagnostic, got {:?}",
            a.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn db_hazards_surface_as_warnings() {
        let src = std::fs::read_to_string("tests/db_check/write_skew.co").unwrap();
        let ddl = std::fs::read_to_string("tests/db_check/write_skew.schema.sql").unwrap();
        let db_path = std::env::temp_dir().join("crow_lsp_test_ws.db");
        let _ = std::fs::remove_file(&db_path);
        let url = format!("sqlite://{}", db_path.display());
        let db = crow_db::connect(&url, None).unwrap();
        for stmt in ddl.split(';').filter(|s| !s.trim().is_empty()) {
            db.exec(stmt, &[]).unwrap();
        }
        drop(db);

        let a = analyze(&src, None, Some(&url), "write_skew.co");
        let warn = a
            .diagnostics
            .iter()
            .find(|d| d.severity == Some(DiagnosticSeverity::WARNING))
            .expect("expected a db hazard warning");
        assert!(
            warn.message.contains("two programs can pass this check"),
            "got: {}",
            warn.message
        );
        assert_eq!(warn.range.start.line, 8); // fixture query line 9, 0-based
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn statements_after_broken_one_still_typecheck() {
        let src = "fn = = broken\ny = 'hello'\n";
        let a = analyze(src, None, None, "t.co");
        // y should still be known to inference despite line 1 being broken.
        assert!(a.ctx.vars.contains_key("y"), "vars: {:?}", a.ctx.vars.keys());
    }

    /// A part the document imports that fails validated alone is a
    /// diagnostic at the line that imports it, and the analysis goes on:
    /// the server had exited on it (process::exit in the expansion)
    #[test]
    fn a_failing_part_is_a_diagnostic_not_an_exit() {
        let dir = std::env::temp_dir().join(format!("crow_lsp_failing_part_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'p'\n").unwrap();
        std::fs::write(dir.join("lib/part.co"), "fn helper[x]\n    return nope[x]\n").unwrap();
        let src = "x = 1\nuse lib.part\nlog[part.helper[1]]\n";
        let a = analyze(src, Some(&dir), None, "main.co");
        assert_eq!(a.diagnostics.len(), 1, "{:?}", a.diagnostics);
        assert_eq!(a.diagnostics[0].range.start.line, 1);
        assert!(a.diagnostics[0].message.contains("lib/part.co: line 2: undefined function 'nope'"), "{:?}", a.diagnostics);
        // mended, the part is validated again (its text changed) and good
        std::fs::write(dir.join("lib/part.co"), "fn helper[x]\n    return x + 1\n").unwrap();
        assert!(analyze(src, Some(&dir), None, "main.co").diagnostics.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A diagnostic points at the name its message names on its line; one
    /// that names none spans the line's text past its indentation
    #[test]
    fn a_diagnostic_points_at_the_name() {
        let a = analyze("x = [1]\ny = lenn[x]\nfn f[t]\n    return t + zzz\n", None, None, "t.co");
        let at = |line: u32| a.diagnostics.iter().find(|d| d.range.start.line == line).map(|d| (d.range.start.character, d.range.end.character));
        assert_eq!(at(1), Some((4, 8)), "{:?}", a.diagnostics);
        assert_eq!(at(3), Some((15, 18)), "{:?}", a.diagnostics);
        assert!(crate::diagnostic::spans("    s = x + y", "no name here").is_empty());
    }
}
