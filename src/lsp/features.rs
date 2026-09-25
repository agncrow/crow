/// Editor features answered from an Analysis: completion, hover,
/// go-to-definition, document symbols. All are pure functions over
/// (analysis, position) so they test without any protocol machinery.

use crate::ast::Stmt;
use crate::token::Token;
use crate::type_infer::CrowType;
use lsp_types::{
    CompletionItem, CompletionItemKind, DocumentSymbol, Location, Position, Range, SymbolKind,
    Uri,
};

use super::analysis::Analysis;

const KEYWORDS: &[&str] = &[
    "fn", "view", "struct", "if", "else", "for", "while", "return", "use", "in", "and", "or",
    "not", "true", "false",
];

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
    for b in crate::checker::BUILTINS {
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
    if crate::checker::BUILTINS.contains(&word.as_str()) {
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
pub fn type_name(ty: &CrowType) -> String {
    match ty {
        CrowType::Int => "int".into(),
        CrowType::Float => "float".into(),
        CrowType::Bool => "bool".into(),
        CrowType::Str => "str".into(),
        CrowType::Array(inner) => format!("array of {}", type_name(inner)),
        CrowType::Map(inner) => format!("map of {}", type_name(inner)),
        CrowType::Struct(name) | CrowType::OptBoxStruct(name) => name.clone(),
        CrowType::Fn(params, ret) => format!(
            "fn[{}] -> {}",
            params.iter().map(type_name).collect::<Vec<_>>().join(", "),
            type_name(ret)
        ),
        CrowType::Result(inner) => format!("result of {}", type_name(inner)),
        CrowType::Json => "json".into(),
        CrowType::NumArray(inner) => format!("tensor of {}", type_name(inner)),
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
