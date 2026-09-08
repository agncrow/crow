//! Regular expressions, once: `matches`, `find`, `sub`. Both modes call
//! these; an invalid pattern is an error with the reason the regex crate
//! gives, and only the caller decides whether that stops the program.

fn compile(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))
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
    fn matches_find_and_sub() {
        assert!(matches("crow-2026", "^[a-z]+-[0-9]+$").unwrap());
        assert!(!matches("Crow", "^[a-z]+$").unwrap());
        assert_eq!(find("order 42 of 7", "([0-9]+) of ([0-9]+)").unwrap(), Some(vec!["42 of 7".into(), "42".into(), "7".into()]));
        assert_eq!(find("none", "[0-9]+").unwrap(), None);
        assert_eq!(find("ab", "(a)(x)?").unwrap(), Some(vec!["a".into(), "a".into(), String::new()]));
        assert_eq!(sub("a1b22", "[0-9]+", "#").unwrap(), "a#b#");
        assert_eq!(sub("2026-09-08", "([0-9]+)-([0-9]+)-([0-9]+)", "$3/$2/$1").unwrap(), "08/09/2026");
        let e = matches("x", "(").unwrap_err();
        assert!(e.starts_with("invalid regex: "), "{}", e);
    }
}
