/// Crow source code formatter
///
/// Normalizes indentation (4 spaces, the unit every document, the book,
/// the spec's examples and the standard modules write), trailing
/// whitespace, blank lines, and final newline. Works at the source level
/// (not AST) to preserve comments, then chains, and all syntax.

/// Format a Crow source string. Returns formatted source.
/// Validates syntax first by parsing, then formats line-by-line.
///
/// A level is a block's: a statement line deeper than the line before
/// opens one, a shallower one closes back to the level of its width, so
/// any consistent nesting is read as it is meant whatever its widths. A
/// line inside a bracket continues its statement and moves with it,
/// keeping its alignment; a comment aligned under one does the same. The
/// lines a string written across lines runs onto are its text, left as
/// written — a unit taken from every line's width had multiplied every
/// level by an odd-width continuation, and re-indented a string's text.
pub fn format_source(source: &str) -> Result<String, String> {
    // Validate: parse to check syntax (we don't use the AST)
    crate::build(source)?;

    let lines: Vec<&str> = source.lines().collect();
    let shape = shape_of(&lines);
    let mut output = String::new();
    let mut consecutive_blanks: usize = 0;
    let mut started = false; // track if we've emitted any content
    // the widths of the open levels, the outermost first
    let mut widths: Vec<usize> = vec![0];
    // how far the last statement line moved: a continuation moves as far
    let mut shift: isize = 0;

    for (line, at) in lines.iter().zip(&shape) {
        // the text of a string written across lines: as written
        if at.in_string {
            output.push_str(line);
            output.push('\n');
            started = true;
            consecutive_blanks = 0;
            continue;
        }
        // a string the line opens and runs onto the next keeps its
        // trailing spaces, which are the string's
        let content = if at.runs_on { *line } else { line.trim_end() };
        let trimmed = content.trim_start();

        // Blank line
        if trimmed.is_empty() {
            if started && consecutive_blanks < 1 {
                output.push('\n');
            }
            consecutive_blanks += 1;
            continue;
        }
        consecutive_blanks = 0;
        started = true;

        // Strip cosmetic trailing --- closers on comments (comments end at
        // the newline; old paired closers are inert text)
        let content = strip_comment_closer(content);
        let trimmed = content.trim_start();

        let leading = leading_whitespace_len(line);
        let comment = trimmed.starts_with("---");
        let indent = if at.in_brackets || (comment && !widths.contains(&leading)) {
            (leading as isize + shift).max(0) as usize
        } else if comment {
            // a comment at a level's width is at that level, and opens and
            // closes none (spec, Blocks)
            widths.iter().position(|w| *w == leading).unwrap_or(0) * 4
        } else {
            while widths.len() > 1 && leading < widths[widths.len() - 1] {
                widths.pop();
            }
            if leading > widths[widths.len() - 1] {
                widths.push(leading);
            }
            let to = (widths.len() - 1) * 4;
            shift = to as isize - leading as isize;
            to
        };

        // Emit with normalized 4-space indentation
        for _ in 0..indent {
            output.push(' ');
        }
        output.push_str(trimmed);
        output.push('\n');
    }

    // Trim trailing blank lines, ensure exactly one newline at end
    while output.ends_with("\n\n") {
        output.pop();
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}

/// Where a line begins, for the formatter: inside a string written across
/// lines, inside a bracket another line opened, and whether a string it
/// opens runs onto the next line.
#[derive(Clone, Copy, Default)]
struct Shape {
    in_string: bool,
    in_brackets: bool,
    runs_on: bool,
}

fn shape_of(lines: &[&str]) -> Vec<Shape> {
    use crate::diagnostic::{code_from, Opening};
    let mut at = Opening::default();
    let mut depth: usize = 0;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let (code, next) = code_from(line, at);
        let begins_inside = depth > 0;
        for c in &code {
            match c {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        out.push(Shape { in_string: at != Opening::default(), in_brackets: begins_inside, runs_on: next != Opening::default() });
        at = next;
    }
    out
}

/// Count leading whitespace as equivalent spaces (tab = 4 spaces).
fn leading_whitespace_len(line: &str) -> usize {
    let mut count = 0;
    for ch in line.chars() {
        match ch {
            ' ' => count += 1,
            '\t' => count += 4,
            _ => break,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trailing_whitespace() {
        let input = "x = 1   \ny = 2\t\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\ny = 2\n");
    }

    /// The unit is four spaces, as every document, the book and the
    /// standard modules write (it had been two)
    #[test]
    fn test_normalize_indent_2_to_4() {
        let input = "if true\n  x = 1\n  y = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "if true\n    x = 1\n    y = 2\n");
    }

    #[test]
    fn test_four_spaces_are_kept() {
        let input = "fn f[]\n    if true\n        x = 1\n    return 2\n";
        assert_eq!(format_source(input).unwrap(), input);
    }

    #[test]
    fn test_normalize_indent_tab() {
        let input = "if true\n\tx = 1\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "if true\n    x = 1\n");
    }

    #[test]
    fn test_multiple_blank_lines() {
        let input = "x = 1\n\n\n\ny = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\n\ny = 2\n");
    }

    #[test]
    fn test_leading_blank_lines() {
        let input = "\n\n\nx = 1\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\n");
    }

    #[test]
    fn test_trailing_blank_lines() {
        let input = "x = 1\n\n\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\n");
    }

    #[test]
    fn test_final_newline_added() {
        let input = "x = 1";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\n");
    }

    #[test]
    fn test_preserves_comments() {
        let input = "--- hello ---\nx = 1\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "--- hello\nx = 1\n");
    }

    #[test]
    fn test_already_formatted() {
        let input = "x = 1\ny = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_nested_indent() {
        let input = "fn foo[]\n  if true\n    x = 1\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "fn foo[]\n    if true\n        x = 1\n");
    }

    #[test]
    fn test_then_lines_preserved() {
        let input = "pr = frame.new[a = [1]]\n  then join[pr, t, 'k']\n  then max[pr]\nlog[pr]\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "pr = frame.new[a = [1]]\n    then join[pr, t, 'k']\n    then max[pr]\nlog[pr]\n");
        assert_eq!(format_source(&result).unwrap(), result);
    }

    #[test]
    fn test_then_chain_preserved() {
        let input = "result = s then trim then upper\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "result = s then trim then upper\n");
    }

    #[test]
    fn test_comment_preserved() {
        // Legacy trailing closers are stripped (comments end at the newline)
        let input = "--- hello ---\nx = 1\n--- world ---\ny = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "--- hello\nx = 1\n--- world\ny = 2\n");
    }

    #[test]
    fn test_idempotent() {
        let input = "x = 1\n\nfn add[a, b]\n  return a + b\n";
        let first = format_source(input).unwrap();
        let second = format_source(&first).unwrap();
        assert_eq!(first, second);
    }

    /// A line holding only a comment has no indentation of its own (spec,
    /// Blocks): one aligned under a trailing comment does not set the unit,
    /// so the code is indented as it is without it, and formatting is
    /// formatting once.
    #[test]
    fn test_comment_alone_sets_no_indent() {
        let with = format_source("fn f[x]\n    return x + 1      --- a comment that\n                      --- goes on\nlog[f[1]]\n").unwrap();
        let without = format_source("fn f[x]\n    return x + 1      --- a comment that\nlog[f[1]]\n").unwrap();
        let code: Vec<&str> = with.lines().filter(|l| !l.trim_start().starts_with("---")).collect();
        assert_eq!(code, without.lines().collect::<Vec<_>>());
        assert_eq!(format_source(&with).unwrap(), with);
    }

    /// A continuation aligned at an odd width keeps the levels: the unit
    /// had been every line's widths' common divisor, so one such line
    /// multiplied every level of the file.
    #[test]
    fn test_an_odd_width_continuation_keeps_the_levels() {
        let four = "fn f[a, b]\n    x = g[a,\n          b]\n    return x\n";
        assert_eq!(format_source(four).unwrap(), four);
        let two = "fn f[a, b]\n  x = g[a,\n        b]\n  return x\n";
        assert_eq!(format_source(two).unwrap(), four);
    }

    /// The lines a string written across lines runs onto are its text: as
    /// written, blank lines and leading spaces kept.
    #[test]
    fn test_a_strings_lines_are_left_as_written() {
        let input = "fn f[]\n  s = 'one\n   two  \n\n\nthree'\n  return s\n";
        let out = format_source(input).unwrap();
        assert_eq!(out, "fn f[]\n    s = 'one\n   two  \n\n\nthree'\n    return s\n");
        assert_eq!(format_source(&out).unwrap(), out);
    }

    /// Levels are the nesting's, whatever widths it was written at.
    #[test]
    fn test_levels_follow_the_nesting() {
        let input = "fn f[]\n  if true\n     x = 1\n  return 2\n";
        assert_eq!(format_source(input).unwrap(), "fn f[]\n    if true\n        x = 1\n    return 2\n");
    }

    #[test]
    fn test_for_loop_indent() {
        let input = "for i in [1, 2, 3]\n  log[i]\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "for i in [1, 2, 3]\n    log[i]\n");
    }

    #[test]
    fn test_mixed_content() {
        let input = "\n\nx = 1   \n\n\n\nif true\n  y = 2\n\nlog[x]\n\n\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "x = 1\n\nif true\n    y = 2\n\nlog[x]\n");
    }
}


/// Every program in the repository survives the formatter: formatted, it
/// parses to the same tree (line numbers aside), and formatted again it
/// does not change. The parser has few tests of its own; this holds it,
/// and the formatter, to every program there is.
#[cfg(test)]
mod round_trip {
    use std::path::{Path, PathBuf};

    fn programs(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | ".crow_build" | "crow_modules" | "node_modules" | ".git" | ".crow") {
                    programs(&path, out);
                }
            } else if name.ends_with(".co") {
                out.push(path);
            }
        }
    }

    /// The tree's text with the line markers taken out.
    fn shape(stmts: &[crate::ast::Stmt]) -> String {
        let text = format!("{:?}", stmts);
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find("Line(") {
            out.push_str(&rest[..at]);
            let after = &rest[at + 5..];
            let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
            rest = if after[digits..].starts_with(')') { &after[digits + 1..] } else { after };
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn every_program_round_trips_through_the_formatter() {
        let mut files = Vec::new();
        programs(Path::new(env!("CARGO_MANIFEST_DIR")), &mut files);
        files.sort();
        let (mut checked, mut skipped) = (0, 0);
        let mut problems = Vec::new();
        for path in &files {
            let Ok(src) = std::fs::read_to_string(path) else { continue };
            // (a program the parser refuses is a fixture of a refusal, not this test's)
            let Ok(before) = crate::build(&src) else {
                skipped += 1;
                continue;
            };
            let shown = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(path).display().to_string();
            let formatted = match super::format_source(&src) {
                Ok(f) => f,
                Err(e) => {
                    problems.push(format!("{}: the formatter refuses what the parser took: {}", shown, e));
                    continue;
                }
            };
            match crate::build(&formatted) {
                Ok(after) if shape(&after) != shape(&before) => problems.push(format!("{}: formatted, it parses to a different tree", shown)),
                Ok(_) => {}
                Err(e) => problems.push(format!("{}: formatted, it does not parse: {}", shown, e)),
            }
            match super::format_source(&formatted) {
                Ok(twice) if twice != formatted => problems.push(format!("{}: formatting twice is not formatting once", shown)),
                Ok(_) => {}
                Err(e) => problems.push(format!("{}: the formatted text does not format: {}", shown, e)),
            }
            checked += 1;
        }
        assert!(problems.is_empty(), "{} of {} programs:\n{}", problems.len(), checked, problems.join("\n"));
        assert!(checked >= 300, "{} programs checked, {} skipped", checked, skipped);
    }
}


/// One-time migration: convert legacy {expr} interpolation to `expr` inside
/// string literals (and \{ \} escapes to plain braces). Used by
/// `crow fmt --fix-interpolation`. Character-exact outside strings.
pub fn migrate_interpolation(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if !in_str {
            if c == '\'' {
                in_str = true;
            }
            out.push(c);
            i += 1;
            continue;
        }
        // inside a string literal
        if c == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt == '{' || nxt == '}' {
                out.push(nxt); // \{ -> literal brace
            } else {
                out.push(c);
                out.push(nxt);
            }
            i += 2;
            continue;
        }
        if c == '\'' {
            in_str = false;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '{' || c == '}' {
            out.push('`');
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// If the line contains a comment (`---` outside any string literal) and the
/// comment's text ends with a legacy `---` closer, strip the closer.
fn strip_comment_closer(line: &str) -> String {
    // find first --- outside string literals
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    let mut comment_start: Option<usize> = None;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            if c == '\\' && i + 1 < chars.len() { i += 2; continue; }
            if c == '\'' { in_str = false; }
            i += 1;
            continue;
        }
        if c == '\'' { in_str = true; i += 1; continue; }
        if c == '-' && i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '-' {
            comment_start = Some(i);
            break;
        }
        i += 1;
    }
    let Some(start) = comment_start else {
        return line.to_string();
    };
    let text = &line[line.char_indices().nth(start).map(|(b, _)| b).unwrap_or(0)..];
    let trimmed = text.trim_end();
    // must have content between the opening --- and a trailing ---
    if trimmed.len() > 6 && trimmed.ends_with("---") {
        let inner = &trimmed[3..trimmed.len() - 3];
        if !inner.trim().is_empty() {
            let head = &line[..line.len() - text.len()];
            return format!("{}{}", head, trimmed[..trimmed.len() - 3].trim_end());
        }
    }
    line.to_string()
}

/// `crow fmt --fix-views`: rewrite `fn` to `view` for functions whose bodies
/// build ui trees (call HTML elements). Textual, line-anchored — the parse
/// identifies which declarations qualify, the edit touches only the keyword.
pub fn migrate_views(source: &str) -> String {
    use crate::ast::{Expr, Stmt};

    fn body_uses_elements(stmts: &[Stmt]) -> bool {
        fn expr_uses(e: &Expr) -> bool {
            match e {
                Expr::Call { callee, args } => {
                    let is_el = matches!(callee.as_ref(), Expr::Ident(n) if crate::lower::UI_ELEMENTS.contains(&n.as_str()))
                        || matches!(callee.as_ref(), Expr::FieldAccess { object, field }
                            if matches!(object.as_ref(), Expr::Ident(m) if m == "ui")
                                && crate::lower::UI_ELEMENTS.contains(&field.as_str()));
                    is_el || expr_uses(callee) || args.iter().any(expr_uses)
                }
                Expr::BinOp { left, right, .. } => expr_uses(left) || expr_uses(right),
                Expr::UnaryOp { expr, .. } => expr_uses(expr),
                Expr::Array(items) => items.iter().any(expr_uses),
                Expr::Map(pairs) => pairs.iter().any(|(_, v)| expr_uses(v)),
                Expr::Named { value, .. } => expr_uses(value),
                Expr::FieldAccess { object, .. } => expr_uses(object),
                Expr::ForExpr { iter, body, .. } => expr_uses(iter) || body_uses_elements(body),
                _ => false,
            }
        }
        stmts.iter().any(|s| match s {
            Stmt::Assign { value, .. } => expr_uses(value),
            Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => expr_uses(e),
            Stmt::If { condition, then_body, else_body } => {
                expr_uses(condition)
                    || body_uses_elements(then_body)
                    || else_body.as_ref().map_or(false, |b| body_uses_elements(b))
            }
            Stmt::For { iter, body, .. } => expr_uses(iter) || body_uses_elements(body),
            Stmt::While { condition, body } => expr_uses(condition) || body_uses_elements(body),
            _ => false,
        })
    }

    let mut lexer = crate::lexer::Lexer::new(source);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(_) => return source.to_string(),
    };
    let mut parser = crate::parser::Parser::new(tokens);
    let stmts = match parser.parse() {
        Ok(s) => s,
        Err(_) => return source.to_string(),
    };

    // Names of plain fns that build ui trees
    let mut names: Vec<String> = Vec::new();
    for st in &stmts {
        if let Stmt::FnDecl { name, body, is_view: false, .. } = st {
            if body_uses_elements(body) {
                names.push(name.clone());
            }
        }
    }
    if names.is_empty() {
        return source.to_string();
    }

    let mut out: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        let mut replaced = false;
        for n in &names {
            let pat = format!("fn {}[", n);
            if trimmed.starts_with(&pat) {
                let indent = &line[..line.len() - trimmed.len()];
                out.push(format!("{}view {}", indent, &trimmed[3..]));
                replaced = true;
                break;
            }
        }
        if !replaced {
            out.push(line.to_string());
        }
    }
    let mut result = out.join("\n");
    if source.ends_with('\n') {
        result.push('\n');
    }
    result
}
