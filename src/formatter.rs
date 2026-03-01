/// Crow source code formatter
///
/// Normalizes indentation (4 spaces), trailing whitespace,
/// blank lines, and final newline. Works at the source level
/// (not AST) to preserve comments, then chains, and all syntax.

/// Format a Crow source string. Returns formatted source.
/// Validates syntax first by parsing, then formats line-by-line.
pub fn format_source(source: &str) -> Result<String, String> {
    // Validate: parse to check syntax (we don't use the AST)
    crate::build(source)?;

    let indent_size = detect_indent_size(source);
    let mut output = String::new();
    let mut consecutive_blanks: usize = 0;
    let mut started = false; // track if we've emitted any content

    for line in source.lines() {
        let content = line.trim_end();
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

        // Calculate indent level from original whitespace
        let leading = leading_whitespace_len(line);
        let level = if indent_size > 0 { leading / indent_size } else { 0 };

        // Emit with normalized 4-space indentation
        for _ in 0..level {
            output.push_str("    ");
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

/// Detect the indentation unit size used in the source.
/// Returns the smallest non-zero leading whitespace count (in spaces, tabs=4).
fn detect_indent_size(source: &str) -> usize {
    let mut min_indent: Option<usize> = None;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let leading = leading_whitespace_len(line);
        if leading > 0 {
            min_indent = Some(match min_indent {
                Some(current) => gcd(current, leading),
                None => leading,
            });
        }
    }

    min_indent.unwrap_or(4)
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

/// Greatest common divisor.
fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Check if source is already correctly formatted.
/// Returns true if no changes needed.
pub fn is_formatted(source: &str) -> Result<bool, String> {
    let formatted = format_source(source)?;
    Ok(formatted == source)
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

    #[test]
    fn test_normalize_indent_2_to_4() {
        let input = "if true\n  x = 1\n  y = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "if true\n    x = 1\n    y = 2\n");
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
        assert_eq!(result, "--- hello ---\nx = 1\n");
    }

    #[test]
    fn test_already_formatted() {
        let input = "x = 1\ny = 2\n";
        assert!(is_formatted(input).unwrap());
    }

    #[test]
    fn test_nested_indent() {
        let input = "fn foo[]\n  if true\n    x = 1\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "fn foo[]\n    if true\n        x = 1\n");
    }

    #[test]
    fn test_then_chain_preserved() {
        let input = "result = s then trim then upper\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "result = s then trim then upper\n");
    }

    #[test]
    fn test_comment_preserved() {
        let input = "--- hello ---\nx = 1\n--- world ---\ny = 2\n";
        let result = format_source(input).unwrap();
        assert_eq!(result, "--- hello ---\nx = 1\n--- world ---\ny = 2\n");
    }

    #[test]
    fn test_idempotent() {
        let input = "x = 1\n\nfn add[a, b]\n  return a + b\n";
        let first = format_source(input).unwrap();
        let second = format_source(&first).unwrap();
        assert_eq!(first, second);
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

