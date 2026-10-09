/// Editor features answered from an Analysis: completion, hover,
/// go-to-definition, document symbols. All are pure functions over
/// (analysis, position) so they test without any protocol machinery.

use crate::ast::Stmt;
use crate::token::Token;
use crate::type_infer::Ty;
use lsp_types::{
    CompletionItem, CompletionItemKind, DocumentSymbol, Location, Position, Range, SymbolKind,
    Uri,
};

use super::analysis::Analysis;

use crate::lexer::KEYWORDS;

pub fn completions(a: &Analysis) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for (name, sig) in &a.ctx.functions {
        // Function completion inserts the full call with every parameter as
        // a tab-stopped placeholder — tab through to accept the names
        // (which resolve when same-named facts are in scope: the threading
        // idiom typed for you), or type over any slot. The committed code
        // is always the fully explicit call.
        if seen.insert(name.to_string()) {
            let insert = if sig.params.is_empty() {
                format!("{}[]", name)
            } else {
                let slots: Vec<String> = sig
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, (p, _))| format!("${{{}:{}}}", i + 1, p))
                    .collect();
                format!("{}[{}]", name, slots.join(", "))
            };
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(fn_signature(name, sig)),
                insert_text: Some(insert),
                insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }
    let mut push = |label: &str, kind: CompletionItemKind, detail: Option<String>,
                    items: &mut Vec<CompletionItem>| {
        if seen.insert(label.to_string()) {
            items.push(CompletionItem {
                label: label.to_string(),
                kind: Some(kind),
                detail,
                ..Default::default()
            });
        }
    };

    for (name, s) in &a.ctx.structs {
        push(
            name,
            CompletionItemKind::STRUCT,
            Some(struct_detail(name, s)),
            &mut items,
        );
    }
    for (name, ty) in &a.ctx.vars {
        push(
            name,
            CompletionItemKind::VARIABLE,
            Some(type_name(ty)),
            &mut items,
        );
    }
    for b in crate::lower_ir::BUILTINS.iter() {
        // the rows behind `x in xs` and `key in m` are written as the operator
        if matches!(*b, "contains" | "has") {
            continue;
        }
        push(b, CompletionItemKind::FUNCTION, Some("builtin".into()), &mut items);
    }
    for k in KEYWORDS {
        push(k, CompletionItemKind::KEYWORD, None, &mut items);
    }
    items
}

pub fn hover(a: &Analysis, pos: Position) -> Option<String> {
    let word = ident_at(a, pos)?;
    if let Some(sig) = a.ctx.functions.get(&word) {
        let kind = if a.ctx.views.contains(&word) { "view" } else { "fn" };
        return Some(format!(
            "{} — {}",
            fn_signature(&word, sig).replacen("fn ", &format!("{} ", kind), 1),
            "defined in this file"
        ));
    }
    if let Some(s) = a.ctx.structs.get(&word) {
        return Some(struct_detail(&word, s));
    }
    if let Some(ty) = a.ctx.vars.get(&word) {
        return Some(format!("{}: {}", word, type_name(ty)));
    }
    if crate::lower_ir::BUILTINS.contains(&word.as_str()) {
        return Some(format!("{} — builtin", word));
    }
    None
}

/// What the cursor is on: a name, a part's name qualified by the part
/// (`calc.total`), or the part's name itself (`calc` in `calc.total`).
#[derive(Debug, PartialEq)]
enum Named {
    Plain(String),
    Qualified(String, String),
    Module(String),
}

/// The token index of the identifier at the cursor.
fn token_at(a: &Analysis, pos: Position) -> Option<usize> {
    let line = pos.line as usize + 1;
    let ch = pos.character as usize + 1;
    a.tokens.iter().position(|t| {
        t.line == line && matches!(&t.token, Token::Ident(name) if ch >= t.col && ch <= t.col + name.chars().count())
    })
}

fn named_at(a: &Analysis, pos: Position) -> Option<Named> {
    let i = token_at(a, pos)?;
    let Token::Ident(name) = &a.tokens[i].token else { return None };
    let ident = |j: usize| match a.tokens.get(j).map(|t| &t.token) {
        Some(Token::Ident(n)) => Some(n.clone()),
        _ => None,
    };
    let dot = |j: usize| matches!(a.tokens.get(j).map(|t| &t.token), Some(Token::Dot));
    if i >= 2 && dot(i - 1) {
        if let Some(m) = ident(i - 2) {
            if a.parts.contains_key(&m) {
                return Some(Named::Qualified(m, name.clone()));
            }
        }
    }
    if dot(i + 1) && a.parts.contains_key(name) {
        return Some(Named::Module(name.clone()));
    }
    Some(Named::Plain(name.clone()))
}

fn at_line(uri: Uri, line_1based: usize) -> Location {
    let line0 = line_1based.saturating_sub(1) as u32;
    Location {
        uri,
        range: Range {
            start: Position { line: line0, character: 0 },
            end: Position { line: line0, character: 0 },
        },
    }
}

/// Where a name is declared: in this file, or — qualified by a part
/// (`calc.total`) — in the part's file; the part's name opens the part.
pub fn definition(a: &Analysis, pos: Position, uri: &Uri) -> Option<Location> {
    match named_at(a, pos)? {
        Named::Plain(word) => Some(at_line(uri.clone(), decl_line(&a.stmts, &word)?)),
        Named::Module(m) => {
            let file = a.parts.get(&m)?;
            Some(at_line(file_uri(file)?, 1))
        }
        Named::Qualified(m, word) => {
            let file = a.parts.get(&m)?;
            let text = std::fs::read_to_string(file).ok()?;
            let tokens = crate::lexer::Lexer::new(&text).tokenize().ok()?;
            let (stmts, _) = crate::parser::Parser::new(tokens).parse_lenient();
            Some(at_line(file_uri(file)?, decl_line(&stmts, &word)?))
        }
    }
}

fn file_uri(path: &std::path::Path) -> Option<Uri> {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text: String = abs.to_string_lossy().chars().map(|c| if c == ' ' { "%20".to_string() } else { c.to_string() }).collect();
    format!("file://{}", text).parse().ok()
}

/// Every place the name at the cursor is read or written in this file,
/// its declaration included: a part's name qualified by the part only
/// where it is so qualified, a field (`x.name`) never for a plain name.
pub fn references(a: &Analysis, pos: Position, uri: &Uri) -> Vec<Location> {
    let Some(named) = named_at(a, pos) else { return Vec::new() };
    let ident = |j: usize| match a.tokens.get(j).map(|t| &t.token) {
        Some(Token::Ident(n)) => Some(n.as_str()),
        _ => None,
    };
    let dot = |j: usize| matches!(a.tokens.get(j).map(|t| &t.token), Some(Token::Dot));
    let mut out = Vec::new();
    for (i, t) in a.tokens.iter().enumerate() {
        let Token::Ident(name) = &t.token else { continue };
        let hit = match &named {
            Named::Plain(w) => name == w && !(i >= 1 && dot(i - 1)),
            Named::Qualified(m, w) => name == w && i >= 2 && dot(i - 1) && ident(i - 2) == Some(m.as_str()),
            Named::Module(m) => name == m && dot(i + 1),
        };
        if hit {
            let start = Position { line: t.line.saturating_sub(1) as u32, character: t.col.saturating_sub(1) as u32 };
            let end = Position { line: start.line, character: start.character + name.chars().count() as u32 };
            out.push(Location { uri: uri.clone(), range: Range { start, end } });
        }
    }
    out
}

pub fn document_symbols(a: &Analysis) -> Vec<DocumentSymbol> {
    let mut out = Vec::new();
    let mut line = 0usize;
    for stmt in &a.stmts {
        match stmt {
            Stmt::Line(n) => line = *n,
            // (a standard module's functions, inlined, are not the document's)
            Stmt::FnDecl { name, .. } if crate::modules::is_crow_fn(name) => {}
            Stmt::FnDecl { name, is_view, .. } => {
                out.push(symbol(
                    name,
                    if *is_view { SymbolKind::INTERFACE } else { SymbolKind::FUNCTION },
                    line,
                ));
            }
            Stmt::StructDecl { name, .. } => out.push(symbol(name, SymbolKind::STRUCT, line)),
            Stmt::Assign { name, .. } => out.push(symbol(name, SymbolKind::VARIABLE, line)),
            _ => {}
        }
    }
    out
}

fn symbol(name: &str, kind: SymbolKind, line_1based: usize) -> DocumentSymbol {
    let line0 = line_1based.saturating_sub(1) as u32;
    let range = Range {
        start: Position { line: line0, character: 0 },
        end: Position { line: line0, character: 0 },
    };
    #[allow(deprecated)]
    DocumentSymbol {
        name: name.to_string(),
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: None,
    }
}

/// The identifier token containing the cursor. LSP positions are 0-based;
/// token spans are 1-based (start column).
fn ident_at(a: &Analysis, pos: Position) -> Option<String> {
    let line = pos.line as usize + 1;
    let ch = pos.character as usize + 1;
    for t in &a.tokens {
        if t.line != line {
            continue;
        }
        if let Token::Ident(name) = &t.token {
            if ch >= t.col && ch <= t.col + name.chars().count() {
                return Some(name.clone());
            }
        }
    }
    None
}

/// First declaration line of a name: fn, struct, or first assignment.
fn decl_line(stmts: &[Stmt], word: &str) -> Option<usize> {
    let mut line = 0usize;
    for stmt in stmts {
        match stmt {
            Stmt::Line(n) => line = *n,
            Stmt::FnDecl { name, body, .. } => {
                if name == word {
                    return Some(line);
                }
                if let Some(l) = decl_line(body, word) {
                    return Some(l);
                }
            }
            Stmt::StructDecl { name, .. } | Stmt::Assign { name, .. } if name == word => {
                return Some(line);
            }
            _ => {}
        }
    }
    None
}

fn fn_signature(name: &str, sig: &crate::type_infer::FnSig) -> String {
    let params: Vec<String> = sig
        .params
        .iter()
        .map(|(p, t)| format!("{}: {}", p, type_name(t)))
        .collect();
    format!(
        "fn {}[{}] -> {}",
        name,
        params.join(", "),
        type_name(&sig.return_type)
    )
}

fn struct_detail(name: &str, s: &crate::type_infer::StructSig) -> String {
    let fields: Vec<String> = s
        .fields
        .iter()
        .map(|(f, t)| format!("{}: {}", f, type_name(t)))
        .collect();
    format!("struct {} [{}]", name, fields.join(", "))
}

/// User-facing type names, matching the language's vocabulary.
pub fn type_name(ty: &Ty) -> String {
    match ty {
        Ty::Int => "int".into(),
        Ty::Float => "float".into(),
        Ty::Bool => "bool".into(),
        Ty::Str => "str".into(),
        Ty::List(inner) => format!("array of {}", type_name(inner)),
        Ty::Map(_, inner) => format!("map of {}", type_name(inner)),
        Ty::Record(name) | Ty::OptRecord(name) => name.clone(),
        Ty::Fn(params, ret) => format!(
            "fn[{}] -> {}",
            params.iter().map(type_name).collect::<Vec<_>>().join(", "),
            type_name(ret)
        ),
        Ty::Result(inner) => format!("result of {}", type_name(inner)),
        Ty::Json => "json".into(),
        Ty::Tensor(inner, _) => format!("tensor of {}", type_name(inner)),
        other => format!("{:?}", other).to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analysis::analyze;

    const SRC: &str = "\
fn add[a, b]
    return a + b

total = add[2, 3]
name = 'crow'
";

    #[test]
    fn completions_include_user_fn_builtin_and_keyword() {
        let a = analyze(SRC, None, None, "t.co");
        let items = completions(&a);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"add"));
        assert!(labels.contains(&"log"));
        assert!(labels.contains(&"fn"));
    }

    #[test]
    fn hover_shows_inferred_fn_signature() {
        let a = analyze(SRC, None, None, "t.co");
        // cursor on "add" at line 4 (0-based 3), col 8 → inside `add[2, 3]`
        let h = hover(&a, Position { line: 3, character: 9 }).expect("hover");
        assert!(h.contains("fn add[a: int, b: int] -> int"), "got: {}", h);
    }

    #[test]
    fn hover_shows_var_type() {
        let a = analyze(SRC, None, None, "t.co");
        let h = hover(&a, Position { line: 4, character: 1 }).expect("hover");
        assert_eq!(h, "name: str");
    }

    #[test]
    fn definition_finds_fn_decl_line() {
        let a = analyze(SRC, None, None, "t.co");
        let uri: Uri = "file:///t.co".parse().unwrap();
        let loc = definition(&a, Position { line: 3, character: 9 }, &uri).expect("definition");
        assert_eq!(loc.range.start.line, 0);
    }

    #[test]
    fn symbols_outline_fns_and_vars() {
        let a = analyze(SRC, None, None, "t.co");
        let syms = document_symbols(&a);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"total"));
    }
}

/// Signature help: find the innermost unclosed call at the cursor and
/// report its signature with the active parameter (comma count since the
/// opening bracket). Answered from the last good analysis, like hover.
pub fn signature_help(a: &Analysis, line: u32, character: u32) -> Option<lsp_types::SignatureHelp> {
    let text = a.source.lines().nth(line as usize)?;
    let upto: String = text.chars().take(character as usize).collect();

    // Forward scan for the innermost unclosed '[', string-aware: brackets
    // inside 'string literals' are text, not structure.
    let mut opens: Vec<usize> = Vec::new();
    let mut in_str = false;
    for (i, c) in upto.char_indices() {
        match c {
            '\'' => in_str = !in_str,
            '[' if !in_str => opens.push(i),
            ']' if !in_str => {
                opens.pop();
            }
            _ => {}
        }
    }
    let open_idx = *opens.last()?;

    // The callee name directly before the bracket.
    let head = &upto[..open_idx];
    let name: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if name.is_empty() {
        return None;
    }
    let sig = a.ctx.functions.get(&name)?;

    // Active parameter: top-level commas between the bracket and the cursor.
    let inside = &upto[open_idx + 1..];
    let mut inner_depth = 0i32;
    let mut active = 0u32;
    let mut in_str = false;
    for c in inside.chars() {
        match c {
            '\'' => in_str = !in_str,
            '[' | '(' if !in_str => inner_depth += 1,
            ']' | ')' if !in_str => inner_depth -= 1,
            ',' if !in_str && inner_depth == 0 => active += 1,
            _ => {}
        }
    }

    let params: Vec<lsp_types::ParameterInformation> = sig
        .params
        .iter()
        .map(|(p, ty)| lsp_types::ParameterInformation {
            label: lsp_types::ParameterLabel::Simple(format!("{}: {}", p, type_name(ty))),
            documentation: None,
        })
        .collect();
    Some(lsp_types::SignatureHelp {
        signatures: vec![lsp_types::SignatureInformation {
            label: fn_signature(&name, sig),
            documentation: None,
            parameters: Some(params),
            active_parameter: Some(active),
        }],
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}

#[cfg(test)]
mod param_fill_tests {
    use super::*;

    fn analyzed(src: &str) -> crate::lsp::analysis::Analysis {
        crate::lsp::analysis::analyze(src, None, None, "t.co")
    }

    #[test]
    fn definition_follows_a_part_qualified_name_into_its_file() {
        let dir = std::env::temp_dir().join(format!("crow_lsp_parts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/calc.co"), "--- sums\n\nfn total[x]\n    return x * 2.0\n").unwrap();
        let src = "use lib.calc\nx = calc.total[2.0]\n";
        let a = crate::lsp::analysis::analyze(src, Some(&dir), None, "main.co");
        let here: Uri = "file:///tmp/main.co".parse().unwrap();
        // on `total` in `calc.total`: its declaration, line 3 of the part
        let loc = definition(&a, Position { line: 1, character: 11 }, &here).expect("found");
        assert!(loc.uri.as_str().ends_with("lib/calc.co"), "{:?}", loc.uri);
        assert_eq!(loc.range.start.line, 2);
        // on `calc`: the part's file itself
        let loc = definition(&a, Position { line: 1, character: 5 }, &here).expect("found");
        assert!(loc.uri.as_str().ends_with("lib/calc.co") && loc.range.start.line == 0, "{:?}", loc);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn references_are_the_names_occurrences_not_fields_of_that_name() {
        let a = analyzed("struct P [f]\nfn f[x]\n    return x\ny = f[1]\nz = f[2]\np = P[1]\nw = p.f\n");
        let here: Uri = "file:///tmp/t.co".parse().unwrap();
        let refs = references(&a, Position { line: 1, character: 3 }, &here);
        let lines: Vec<u32> = refs.iter().map(|l| l.range.start.line).collect();
        assert_eq!(lines, vec![0, 1, 3, 4], "{:?}", refs);
    }

    #[test]
    fn function_completion_is_a_tabbed_snippet() {
        let a = analyzed("fn clock_at[base, anchor, speed, live, w_end]\n    return base\nx = clock_at[1.0, 2.0, 3.0, true, 4.0]\n");
        let items = completions(&a);
        let item = items.iter().find(|i| i.label == "clock_at").unwrap();
        assert_eq!(
            item.insert_text.as_deref(),
            Some("clock_at[${1:base}, ${2:anchor}, ${3:speed}, ${4:live}, ${5:w_end}]")
        );
        assert_eq!(item.insert_text_format, Some(lsp_types::InsertTextFormat::SNIPPET));
    }

    #[test]
    fn signature_help_tracks_the_active_parameter() {
        let src = "fn clock_at[base, anchor, speed]\n    return base\nx = clock_at[1.0, 2.0, \n";
        let a = analyzed(src);
        // Cursor after the second comma on line 3 (0-based line 2)
        let h = signature_help(&a, 2, 23).unwrap();
        assert_eq!(h.active_parameter, Some(2));
        let sig = &h.signatures[0];
        assert!(sig.label.starts_with("fn clock_at["), "{}", sig.label);
        // Inside a string, brackets and commas must not confuse it
        let src2 = "fn f[a, b]\n    return a\nx = f['a,[b', \n";
        let a2 = analyzed(src2);
        let h2 = signature_help(&a2, 2, 16).unwrap();
        assert_eq!(h2.active_parameter, Some(1));
    }
}

/// The text between `open` and the next `close` in `s`.
fn between<'s>(s: &'s str, open: &str, close: &str) -> Option<&'s str> {
    let rest = &s[s.find(open)? + open.len()..];
    Some(&rest[..rest.find(close)?])
}

/// The source's text in a single-line range, by character columns.
fn text_in(source: &str, r: Range) -> String {
    let line = source.lines().nth(r.start.line as usize).unwrap_or("");
    line.chars().skip(r.start.character as usize).take(r.end.character.saturating_sub(r.start.character) as usize).collect()
}

/// Quick fixes for the diagnostics the client names: a slip's nearest name
/// put in its place (`undefined function 'lenn' — did you mean 'len'?`),
/// where the diagnostic points at the name written; and an import a
/// function needs (`mean[] is math's: add `use math``), added below the
/// document's last import.
pub fn code_actions(a: &Analysis, uri: &Uri, diagnostics: &[lsp_types::Diagnostic]) -> Vec<lsp_types::CodeActionOrCommand> {
    use lsp_types::{CodeAction, CodeActionKind, CodeActionOrCommand, TextEdit, WorkspaceEdit};
    let fix = |title: String, d: &lsp_types::Diagnostic, edit: TextEdit| {
        CodeActionOrCommand::CodeAction(CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![d.clone()]),
            edit: Some(WorkspaceEdit { changes: Some(std::collections::HashMap::from([(uri.clone(), vec![edit])])), ..Default::default() }),
            is_preferred: Some(true),
            ..Default::default()
        })
    };
    let mut out = Vec::new();
    for d in diagnostics {
        if let (Some(near), Some(written)) = (between(&d.message, "did you mean '", "'?"), between(&d.message, "'", "'"))
            && d.range.start.line == d.range.end.line
            && text_in(&a.source, d.range) == written
        {
            out.push(fix(format!("Change to '{}'", near), d, TextEdit { range: d.range, new_text: near.to_string() }));
        }
        if let Some(module) = between(&d.message, "add `use ", "`") {
            let below = a.source.lines().enumerate().filter(|(_, l)| l.starts_with("use ")).map(|(i, _)| i + 1).last().unwrap_or(0) as u32;
            let at = Position { line: below, character: 0 };
            out.push(fix(format!("Add `use {}`", module), d, TextEdit { range: Range { start: at, end: at }, new_text: format!("use {}\n", module) }));
        }
    }
    out
}

// ── rename ──────────────────────────────────────────────────────────

/// Whether text is a name a program may bind: a letter or `_`, then
/// letters, digits and `_`, and no keyword or builtin.
fn bindable(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && cs.all(|c| c.is_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&s)
        && !crate::lower_ir::BUILTINS.contains(&s)
}

/// The names the document declares at its top level: its functions, views,
/// structs, derivations and top-level variables and facts, as the lines at
/// column 1 declare them.
fn top_level_names(a: &Analysis) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (i, t) in a.tokens.iter().enumerate() {
        if t.col != 1 {
            continue;
        }
        let next = |k: usize| a.tokens.get(i + k).map(|t| &t.token);
        match (&t.token, next(1)) {
            (Token::Fn | Token::View | Token::Struct | Token::Derive, Some(Token::Ident(n))) => {
                out.insert(n.clone());
            }
            (Token::Ident(n), Some(Token::Assign)) => {
                out.insert(n.clone());
            }
            _ => {}
        }
    }
    out
}

/// The top-level declaration whose lines hold `line0` (0-based): its first
/// line, the line after its last, its name and its parameters — a
/// declaration runs to the next line that starts at column 1.
fn enclosing_decl(a: &Analysis, line0: usize) -> Option<(usize, usize, String, Vec<String>)> {
    let lines: Vec<&str> = a.source.lines().collect();
    let starts_body = |l: &str| !l.is_empty() && !l.starts_with(char::is_whitespace);
    let start = (0..=line0.min(lines.len().saturating_sub(1))).rev().find(|&i| starts_body(lines[i]))?;
    let head = lines[start].trim_start();
    let decl = ["fn ", "view ", "test ", "derive "].iter().find(|k| head.starts_with(**k))?;
    let rest = &head[decl.len()..];
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let params: Vec<String> = rest
        .split_once('[')
        .and_then(|(_, r)| r.split_once(']'))
        .map(|(ps, _)| ps.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let end = (start + 1..lines.len()).find(|&i| starts_body(lines[i])).unwrap_or(lines.len());
    Some((start, end, name, params))
}

/// A rename's reach: the lines of the declaration a local or a parameter
/// belongs to, or the whole file for a name declared at the top level.
enum Reach {
    Local { from: usize, to: usize, decl: String, param: bool },
    File,
}

/// The name at the cursor, where it can be renamed, and how far.
fn renameable(a: &Analysis, pos: Position) -> Result<(usize, String, Reach), String> {
    let i = token_at(a, pos).ok_or("no name here")?;
    let Token::Ident(old) = &a.tokens[i].token else { return Err("no name here".into()) };
    if i >= 1 && matches!(a.tokens[i - 1].token, Token::Dot) {
        return Err(format!("'{}' is a field or a part's name: rename it where it is declared", old));
    }
    let top = top_level_names(a);
    if !top.contains(old) && (crate::lower_ir::BUILTINS.contains(&old.as_str()) || a.parts.contains_key(old)) {
        return Err(format!("'{}' is a builtin or a part, and keeps its name", old));
    }
    if let Some((from, to, decl, params)) = enclosing_decl(a, pos.line as usize) {
        let param = params.contains(old);
        let bound_here = a.tokens.iter().any(|t| t.line > from && t.line <= to && matches!(&t.token, Token::Ident(n) if n == old));
        if param || (!top.contains(old) && bound_here) {
            return Ok((i, old.clone(), Reach::Local { from, to, decl, param }));
        }
    }
    if top.contains(old) {
        return Ok((i, old.clone(), Reach::File));
    }
    Err(format!("'{}' is not declared in this file", old))
}

/// Where a rename can start: the range of the name at the cursor, or why
/// it cannot be renamed; none when no name is there.
pub fn prepare_rename(a: &Analysis, pos: Position) -> Option<Result<Range, String>> {
    token_at(a, pos)?;
    Some(renameable(a, pos).map(|(i, old, _)| token_range(&a.tokens[i], &old)))
}

fn token_range(t: &crate::token::SpannedToken, name: &str) -> Range {
    let start = Position { line: t.line.saturating_sub(1) as u32, character: t.col.saturating_sub(1) as u32 };
    Range { start, end: Position { line: start.line, character: start.character + name.chars().count() as u32 } }
}

/// The edits a rename makes: every place the name means the same thing —
/// within its declaration for a local or a parameter (a parameter's name
/// in a named argument of a call to its function too), across the file for
/// a top-level name, and `part.name` in the project's files that import
/// this one. A named argument's key names a parameter, never the value
/// renamed.
#[allow(clippy::mutable_key_type, reason = "lsp_types' own WorkspaceEdit keys its changes by Uri")]
pub fn rename(a: &Analysis, pos: Position, new: &str, uri: &Uri) -> Result<lsp_types::WorkspaceEdit, String> {
    use lsp_types::TextEdit;
    let (_, old, reach) = renameable(a, pos)?;
    if !bindable(new) {
        return Err(format!("'{}' is not a name a program can bind (a keyword or a builtin, or not a name)", new));
    }
    let top = top_level_names(a);
    let taken = match &reach {
        Reach::File => top.contains(new),
        Reach::Local { from, to, .. } => top.contains(new) || a.tokens.iter().any(|t| t.line > *from && t.line <= *to && matches!(&t.token, Token::Ident(n) if n == new)),
    };
    if taken {
        return Err(format!("'{}' is already a name here", new));
    }
    let mut edits: Vec<TextEdit> = Vec::new();
    let mut depth: Vec<Option<String>> = Vec::new(); // each open bracket: the function it calls
    for (i, t) in a.tokens.iter().enumerate() {
        match &t.token {
            Token::LBracket => {
                let callee = match i.checked_sub(1).map(|j| &a.tokens[j].token) {
                    Some(Token::Ident(n)) => Some(n.clone()),
                    _ => None,
                };
                depth.push(callee);
            }
            Token::RBracket => {
                depth.pop();
            }
            Token::Ident(n) if *n == old => {
                let after_dot = i >= 1 && matches!(a.tokens[i - 1].token, Token::Dot);
                let key = !depth.is_empty() && matches!(a.tokens.get(i + 1).map(|t| &t.token), Some(Token::Assign));
                let line0 = t.line.saturating_sub(1);
                let hit = match &reach {
                    Reach::File => !after_dot && !key,
                    Reach::Local { from, to, decl, param } => {
                        let inside = line0 >= *from && line0 < *to;
                        let call_key = *param && key && depth.last().cloned().flatten().as_deref() == Some(decl.as_str());
                        !after_dot && ((inside && !key) || call_key)
                    }
                };
                if hit {
                    edits.push(TextEdit { range: token_range(t, &old), new_text: new.to_string() });
                }
            }
            _ => {}
        }
    }
    let mut changes = std::collections::HashMap::from([(uri.clone(), edits)]);
    if matches!(reach, Reach::File) {
        for (file_uri, file_edits) in qualified_uses(uri, &old, new) {
            changes.insert(file_uri, file_edits);
        }
    }
    Ok(lsp_types::WorkspaceEdit { changes: Some(changes), ..Default::default() })
}

/// `part.name` in every file of the project that imports the document
/// (`use sim.clock`, or `use sim.clock as c` read as `c.name`), each an edit
/// to that file.
fn qualified_uses(uri: &Uri, old: &str, new: &str) -> Vec<(Uri, Vec<lsp_types::TextEdit>)> {
    let path = std::path::PathBuf::from(uri.path().as_str());
    let Some(dir) = path.parent() else { return Vec::new() };
    let root = crate::project_root(dir);
    let Ok(rel) = path.strip_prefix(&root) else { return Vec::new() };
    let module = rel.with_extension("").to_string_lossy().replace('/', ".");
    let mut files = Vec::new();
    let mut todo = vec![root.clone()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "crow_modules" {
                    todo.push(p);
                }
            } else if name.ends_with(".co") && p != path {
                files.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(tokens) = crate::lexer::Lexer::new(&text).tokenize() else { continue };
        // the name the file calls this part by
        let mut alias: Option<String> = None;
        let mut k = 0;
        while k < tokens.len() {
            if matches!(tokens[k].token, Token::Use) {
                let mut j = k + 1;
                let mut dotted = String::new();
                while let Some(Token::Ident(n)) = tokens.get(j).map(|t| &t.token) {
                    dotted.push_str(n);
                    j += 1;
                    if matches!(tokens.get(j).map(|t| &t.token), Some(Token::Dot)) {
                        dotted.push('.');
                        j += 1;
                    } else {
                        break;
                    }
                }
                if dotted == module {
                    let called = match (tokens.get(j).map(|t| &t.token), tokens.get(j + 1).map(|t| &t.token)) {
                        (Some(Token::Ident(a)), Some(Token::Ident(b))) if a == "as" => b.clone(),
                        _ => module.rsplit('.').next().unwrap_or(&module).to_string(),
                    };
                    alias = Some(called);
                }
                k = j;
            } else {
                k += 1;
            }
        }
        let Some(alias) = alias else { continue };
        let edits: Vec<lsp_types::TextEdit> = tokens
            .windows(3)
            .filter(|w| matches!(&w[0].token, Token::Ident(a) if *a == alias) && matches!(w[1].token, Token::Dot) && matches!(&w[2].token, Token::Ident(n) if n == old))
            .map(|w| lsp_types::TextEdit { range: token_range(&w[2], old), new_text: new.to_string() })
            .collect();
        if let (false, Some(u)) = (edits.is_empty(), file_uri(&f)) {
            out.push((u, edits));
        }
    }
    out
}

#[cfg(test)]
mod fix_and_rename_tests {
    use super::*;

    fn analyzed(src: &str) -> crate::lsp::analysis::Analysis {
        crate::lsp::analysis::analyze(src, None, None, "t.co")
    }

    fn edits(e: &lsp_types::WorkspaceEdit, uri: &Uri) -> Vec<(u32, u32, u32, String)> {
        let mut out: Vec<(u32, u32, u32, String)> = e.changes.as_ref().and_then(|c| c.get(uri)).map(|es| es.iter().map(|x| (x.range.start.line, x.range.start.character, x.range.end.character, x.new_text.clone())).collect()).unwrap_or_default();
        out.sort();
        out
    }

    /// A slip's nearest name, and an import a function needs, are quick
    /// fixes for their diagnostics
    #[test]
    fn a_slip_and_a_missing_import_are_quick_fixes() {
        let a = analyzed("x = [1, 2]\ny = lenn[x]\nm = mean[x]\n");
        let here: Uri = "file:///tmp/t.co".parse().unwrap();
        let actions = code_actions(&a, &here, &a.diagnostics);
        let titles: Vec<String> = actions.iter().filter_map(|x| match x { lsp_types::CodeActionOrCommand::CodeAction(c) => Some(c.title.clone()), _ => None }).collect();
        assert!(titles.contains(&"Change to 'len'".to_string()), "{:?}", titles);
        assert!(titles.contains(&"Add `use math`".to_string()), "{:?}", titles);
        let lsp_types::CodeActionOrCommand::CodeAction(slip) = &actions[0] else { panic!() };
        assert_eq!(edits(slip.edit.as_ref().unwrap(), &here), vec![(1, 4, 8, "len".to_string())]);
    }

    /// A local is renamed within its function, never a same-named one of
    /// another function; a parameter with it the named arguments of calls to
    /// its function; a top-level name across the file, never a field or a
    /// named argument's key of that name; a builtin or a field is refused,
    /// and so is a name already in use
    #[test]
    fn a_rename_reaches_what_the_name_means() {
        let src = "fn go[speed]\n    t = speed * 2\n    return t\n\nfn other[t]\n    return t\n\nlog[go[speed = 3]]\nspeed = 4\nlog[fmt[speed, width = 2]]\n";
        let a = analyzed(src);
        let here: Uri = "file:///tmp/t.co".parse().unwrap();
        // the local `t` of go (line 2): its two lines, not other's `t`
        let e = rename(&a, Position { line: 1, character: 4 }, "twice", &here).unwrap();
        assert_eq!(edits(&e, &here), vec![(1, 4, 5, "twice".to_string()), (2, 11, 12, "twice".to_string())]);
        // go's parameter: its body and the key of the call to go, not the
        // top-level variable of the same name
        let e = rename(&a, Position { line: 0, character: 7 }, "pace", &here).unwrap();
        assert_eq!(edits(&e, &here), vec![(0, 6, 11, "pace".to_string()), (1, 8, 13, "pace".to_string()), (7, 7, 12, "pace".to_string())]);
        // the top-level function go, across the file
        let e = rename(&a, Position { line: 7, character: 5 }, "run", &here).unwrap();
        assert_eq!(edits(&e, &here), vec![(0, 3, 5, "run".to_string()), (7, 4, 6, "run".to_string())]);
        assert!(rename(&a, Position { line: 7, character: 5 }, "other", &here).unwrap_err().contains("already a name"));
        assert!(rename(&a, Position { line: 7, character: 5 }, "len", &here).is_err());
        assert!(prepare_rename(&a, Position { line: 9, character: 5 }).unwrap().is_err(), "fmt is a builtin");
    }

    /// A top-level name of a part is renamed where the project's files call
    /// it by the part's name (`calc.total`, `c.total` under an alias)
    #[test]
    fn a_parts_name_is_renamed_where_it_is_called() {
        let dir = std::env::temp_dir().join(format!("crow_lsp_rename_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'r'\n").unwrap();
        let part = "fn total[x]\n    return x * 2\n";
        std::fs::write(dir.join("lib/calc.co"), part).unwrap();
        std::fs::write(dir.join("main.co"), "use lib.calc\nlog[calc.total[2]]\n").unwrap();
        std::fs::write(dir.join("other.co"), "use lib.calc as c\nlog[c.total[3] + c.total[4]]\n").unwrap();
        let a = crate::lsp::analysis::analyze(part, Some(&dir.join("lib")), None, "calc.co");
        let here = file_uri(&dir.join("lib/calc.co")).unwrap();
        let e = rename(&a, Position { line: 0, character: 4 }, "sum", &here).unwrap();
        let changes = e.changes.unwrap();
        assert_eq!(changes.len(), 3, "{:?}", changes.keys().collect::<Vec<_>>());
        let main = changes.iter().find(|(u, _)| u.as_str().ends_with("/main.co")).map(|(_, es)| es.len());
        let other = changes.iter().find(|(u, _)| u.as_str().ends_with("/other.co")).map(|(_, es)| es.len());
        assert_eq!((main, other), (Some(1), Some(2)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

