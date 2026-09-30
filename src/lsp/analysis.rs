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

    // Tokenize. A lexer error poisons the whole file — report it and stop.
    let tokens = match Lexer::new(text).tokenize() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = extract_line_col(&e);
            diagnostics.push(diag(
                line,
                col,
                text,
                strip_location(&e),
                DiagnosticSeverity::ERROR,
            ));
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
    for (line, e) in parse_errors {
        diagnostics.push(diag(
            line,
            0,
            text,
            strip_location(&e),
            DiagnosticSeverity::ERROR,
        ));
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
        None => stmts,
    };

    // Named-argument lowering.
    let stmts = match crate::lower::lower_program(&stmts) {
        Ok(s) => s,
        Err(e) => {
            let (line, _) = extract_line_col(&e);
            diagnostics.push(diag(line, 0, text, strip_location(&e), DiagnosticSeverity::ERROR));
            // Continue with unlowered stmts so hover/completion still work.
            stmts
        }
    };

    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);

    // the lowering refuses what is not a program, one "line N: message"
    // entry per error (not while a part it imports fails: its names would
    // all read as undefined)
    if parts_good && let Err(refusal) = crate::lower_ir::check(&stmts, &ctx) {
        let e = refusal.message();
        for entry in e.lines().filter(|l| !l.trim().is_empty()) {
            let (line, _) = extract_line_col(entry);
            // Point inside multi-line statements at the actual line.
            let line = crate::refine_error_line(text, line, entry);
            diagnostics.push(diag(
                line,
                0,
                text,
                strip_location(entry),
                DiagnosticSeverity::ERROR,
            ));
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

    Analysis {
        source: text.to_string(),
        stmts,
        ctx,
        tokens,
        diagnostics,
        parts,
    }
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

/// Where on its line a message points: the first name it quotes
/// (`undefined function 'lenn'`) or calls (`msgpack[] is web's`) that the
/// line holds as a whole word, as character columns.
fn narrowed(line: &str, message: &str) -> Option<(u32, u32)> {
    let mut names: Vec<&str> = message.split('\'').skip(1).step_by(2).collect();
    if let Some(call) = message.split("[]").next().filter(|c| message.contains("[]") && c.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.')) {
        names.push(call);
    }
    let chars: Vec<char> = line.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    for name in names.into_iter().filter(|n| !n.is_empty() && n.chars().all(|c| word(c) || c == '.')) {
        let want: Vec<char> = name.chars().collect();
        let mut i = 0;
        while i + want.len() <= chars.len() {
            let at_word = chars[i..i + want.len()] == want[..]
                && (i == 0 || !word(chars[i - 1]))
                && chars.get(i + want.len()).is_none_or(|c| !word(*c));
            if at_word {
                return Some((i as u32, (i + want.len()) as u32));
            }
            i += 1;
        }
    }
    None
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
    let (start, end) = narrowed(line_text, &message).unwrap_or((col_0based.max(indent).min(line_len), line_len.max(col_0based + 1)));
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

/// Pull "line N" (and optionally "col M") out of an error message.
fn extract_line_col(msg: &str) -> (usize, u32) {
    let mut line = 1usize;
    let mut col = 0u32;
    if let Some(pos) = msg.find("line ") {
        let rest = &msg[pos + 5..];
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse() {
            line = n;
        }
    }
    if let Some(pos) = msg.find("col ") {
        let rest = &msg[pos + 4..];
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<u32>() {
            col = n.saturating_sub(1); // 1-based → 0-based
        }
    }
    (line, col)
}

/// Editors show the location themselves — drop a leading "line N: " prefix
/// so messages aren't doubled up.
fn strip_location(msg: &str) -> String {
    let trimmed = msg.trim();
    if let Some(rest) = trimmed.strip_prefix("line ") {
        if let Some(colon) = rest.find(':') {
            if rest[..colon].chars().all(|c| c.is_ascii_digit()) {
                return rest[colon + 1..].trim().to_string();
            }
        }
    }
    trimmed.to_string()
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
        assert_eq!(narrowed("    s = x + y", "no name here"), None);
    }
}
