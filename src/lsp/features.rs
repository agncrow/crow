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
    let mut seen = std::collections::BTreeSet::new();
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

    for (name, sig) in &a.ctx.functions {
        push(
            name,
            CompletionItemKind::FUNCTION,
            Some(fn_signature(name, sig)),
            &mut items,
        );
    }
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

pub fn definition(a: &Analysis, pos: Position, uri: &Uri) -> Option<Location> {
    let word = ident_at(a, pos)?;
    let line = decl_line(&a.stmts, &word)?;
    let line0 = line.saturating_sub(1) as u32;
    Some(Location {
        uri: uri.clone(),
        range: Range {
            start: Position { line: line0, character: 0 },
            end: Position { line: line0, character: 0 },
        },
    })
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
        let a = analyze(SRC, None, None, "t.crow");
        let items = completions(&a);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"add"));
        assert!(labels.contains(&"log"));
        assert!(labels.contains(&"fn"));
    }

    #[test]
    fn hover_shows_inferred_fn_signature() {
        let a = analyze(SRC, None, None, "t.crow");
        // cursor on "add" at line 4 (0-based 3), col 8 → inside `add[2, 3]`
        let h = hover(&a, Position { line: 3, character: 9 }).expect("hover");
        assert!(h.contains("fn add[a: int, b: int] -> int"), "got: {}", h);
    }

    #[test]
    fn hover_shows_var_type() {
        let a = analyze(SRC, None, None, "t.crow");
        let h = hover(&a, Position { line: 4, character: 1 }).expect("hover");
        assert_eq!(h, "name: str");
    }

    #[test]
    fn definition_finds_fn_decl_line() {
        let a = analyze(SRC, None, None, "t.crow");
        let uri: Uri = "file:///t.crow".parse().unwrap();
        let loc = definition(&a, Position { line: 3, character: 9 }, &uri).expect("definition");
        assert_eq!(loc.range.start.line, 0);
    }

    #[test]
    fn symbols_outline_fns_and_vars() {
        let a = analyze(SRC, None, None, "t.crow");
        let syms = document_symbols(&a);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"total"));
    }
}
