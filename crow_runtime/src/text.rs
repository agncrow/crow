//! Text, once: the string builtins and the regular expressions. Both modes
//! call these (the Rust backend prints the call, the interpreter calls it
//! with its values' strings), so a builtin means the same in both. An
//! invalid pattern is an error with the reason the regex crate gives, and
//! only the caller decides whether that stops the program.

/// `split[text, separator]`: the pieces between each separator.
#[inline]
pub fn split(s: &str, sep: &str) -> Vec<String> {
    s.split(sep).map(|p| p.to_string()).collect()
}

/// `trim[text]`: without the whitespace at either end.
#[inline]
pub fn trim(s: &str) -> String {
    s.trim().to_string()
}

/// `replace[text, old, new]`: every `old` replaced by `new`.
#[inline]
pub fn replace(s: &str, old: &str, new: &str) -> String {
    s.replace(old, new)
}

/// `upper[text]`.
#[inline]
pub fn upper(s: &str) -> String {
    s.to_uppercase()
}

/// `lower[text]`.
#[inline]
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `starts[text, prefix]`.
#[inline]
pub fn starts(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// `ends[text, suffix]`.
#[inline]
pub fn ends(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// `repeat[text, n]`: the text `n` times; a negative count is refused.
pub fn repeat(s: &str, n: i64) -> Result<String, String> {
    if n < 0 {
        return Err(format!("repeat[] count cannot be negative (got {})", n));
    }
    Ok(s.repeat(n as usize))
}

/// A pattern compiled, or what is wrong with it on one line: the regex
/// crate draws the pattern over several, and a stop is one line.
fn compile(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(pattern).map_err(|e| {
        let said = e.to_string();
        let why = said.lines().last().unwrap_or("").trim().trim_start_matches("error: ");
        format!("invalid regex '{}': {}", pattern, why)
    })
}

/// `matches[text, pattern]`.
pub fn matches(s: &str, pattern: &str) -> Result<bool, String> {
    Ok(compile(pattern)?.is_match(s))
}

/// `find[text, pattern]`: the whole match followed by each group, an
/// unmatched group as the empty string; `None` when nothing matches.
pub fn find(s: &str, pattern: &str) -> Result<Option<Vec<String>>, String> {
    let re = compile(pattern)?;
    Ok(re.captures(s).map(|caps| {
        caps.iter().map(|m| m.map_or(String::new(), |m| m.as_str().to_string())).collect()
    }))
}

/// `sub[text, pattern, replacement]`: every match replaced; `$1` in the
/// replacement is the first group.
pub fn sub(s: &str, pattern: &str, replacement: &str) -> Result<String, String> {
    Ok(compile(pattern)?.replace_all(s, replacement).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_string_builtins() {
        assert_eq!(split("a,b,,c", ","), vec!["a", "b", "", "c"]);
        assert_eq!(trim("  crow \n"), "crow");
        assert_eq!(replace("a-b-c", "-", "+"), "a+b+c");
        assert_eq!(upper("straße"), "STRASSE");
        assert_eq!(lower("ÉCOLE"), "école");
        assert!(starts("crow", "cr") && ends("crow", "ow"));
        assert_eq!(repeat("ab", 3).unwrap(), "ababab");
        assert_eq!(repeat("ab", 0).unwrap(), "");
        assert_eq!(repeat("ab", -1).unwrap_err(), "repeat[] count cannot be negative (got -1)");
    }

    #[test]
    fn matches_find_and_sub() {
        assert!(matches("crow-2026", "^[a-z]+-[0-9]+$").unwrap());
        assert!(!matches("Crow", "^[a-z]+$").unwrap());
        assert_eq!(find("order 42 of 7", "([0-9]+) of ([0-9]+)").unwrap(), Some(vec!["42 of 7".into(), "42".into(), "7".into()]));
        assert_eq!(find("none", "[0-9]+").unwrap(), None);
        assert_eq!(find("ab", "(a)(x)?").unwrap(), Some(vec!["a".into(), "a".into(), String::new()]));
        assert_eq!(sub("a1b22", "[0-9]+", "#").unwrap(), "a#b#");
        assert_eq!(sub("2026-09-08", "([0-9]+)-([0-9]+)-([0-9]+)", "$3/$2/$1").unwrap(), "08/09/2026");
        // one line, naming the pattern and what is wrong with it
        assert_eq!(matches("x", "(").unwrap_err(), "invalid regex '(': unclosed group");
    }
}
