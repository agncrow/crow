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

// ── encodings: text written so that a header or a URL can carry it ──
// `encode[text, scheme]` and `decode[text, scheme]` (spec, Encodings): one
// pair of functions both modes call, written here rather than taken from a
// crate, since each is a few lines and the runtime should not grow a
// dependency for them.

/// The base64 alphabet (RFC 4648, the standard one).
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes as base64: the standard alphabet, padded with `=` to a multiple
/// of four, as HTTP's Basic authorization and most APIs want it.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | ((*b as u32) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64's bytes: the standard alphabet or the URL-safe one (`-` and `_`),
/// padded or not, whitespace between its characters skipped (a PEM block,
/// a wrapped API answer). A character outside the alphabet, or a length no
/// encoding has, is the reason it cannot be read.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut sextets: Vec<u8> = Vec::with_capacity(text.len());
    let mut padding = 0;
    for c in text.chars() {
        let v = match c {
            'A'..='Z' => c as u8 - b'A',
            'a'..='z' => c as u8 - b'a' + 26,
            '0'..='9' => c as u8 - b'0' + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => {
                padding += 1;
                continue;
            }
            c if c.is_whitespace() => continue,
            c => return Err(format!("'{}' is not base64: '{}' is not in its alphabet", shortened(text), c)),
        };
        if padding > 0 {
            return Err(format!("'{}' is not base64: '=' pads only its end", shortened(text)));
        }
        sextets.push(v);
    }
    if sextets.len() % 4 == 1 || padding > 2 {
        return Err(format!("'{}' is not base64: its length is not one an encoding has", shortened(text)));
    }
    let mut out = Vec::with_capacity(sextets.len() * 3 / 4);
    for chunk in sextets.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, v)| n | ((*v as u32) << (18 - 6 * i)));
        for i in 0..chunk.len() - 1 {
            out.push(((n >> (16 - 8 * i)) & 255) as u8);
        }
    }
    Ok(out)
}

/// Text as one component of a URL (a query's value, a path's segment):
/// every byte but a letter, a digit and `-` `.` `_` `~` written `%XX`, so a
/// space is `%20` and `&`, `=`, `/` and `?` stand for themselves no more.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// A URL component's text: each `%XX` the byte it stands for; anything
/// else, a `+` too, as it is (as `encode` writes a space, `%20`). A `%` not
/// followed by two hex digits, or bytes that are not text, are the reason
/// it cannot be read.
pub fn url_decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3).filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
            let Some(hex) = hex else {
                return Err(format!("'{}' is not URL-encoded: a % is followed by two hex digits", shortened(s)));
            };
            out.push(u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("'{}' decodes to bytes that are not text (UTF-8)", shortened(s)))
}

/// Text cut for a reason: its first 40 characters.
fn shortened(s: &str) -> String {
    if s.chars().count() <= 40 {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(40).collect::<String>())
    }
}

/// The schemes `encode` and `decode` take.
pub const SCHEMES: &[&str] = &["base64", "url"];

/// Why a scheme is none `encode` or `decode` (`builtin`) knows: the words
/// the program stops with, or is refused with before it runs.
pub fn unknown_scheme(builtin: &str, scheme: &str) -> String {
    format!("{}[] knows no scheme '{}' — {}", builtin, scheme, SCHEMES.join(" or "))
}

/// `encode[text, scheme]`: text as base64 (`'base64'`) or as a URL's
/// component (`'url'`). A scheme it does not know is the program's misuse.
pub fn encode_as(text: &str, scheme: &str) -> Result<String, String> {
    match scheme {
        "base64" => Ok(base64_encode(text.as_bytes())),
        "url" => Ok(url_encode(text)),
        other => Err(unknown_scheme("encode", other)),
    }
}

/// `decode[text, scheme]`: the text `encode[…, scheme]` was given. Text
/// that is not in the scheme, or that decodes to bytes that are not text,
/// is the reason it cannot be read.
pub fn decode_as(text: &str, scheme: &str) -> Result<String, String> {
    match scheme {
        "base64" => String::from_utf8(base64_decode(text)?).map_err(|_| format!("'{}' decodes to bytes that are not text (UTF-8)", shortened(text))),
        "url" => url_decode(text),
        other => Err(unknown_scheme("decode", other)),
    }
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

    #[test]
    fn base64_both_ways() {
        // RFC 4648's test vectors
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(encode_as(plain, "base64").unwrap(), coded);
            assert_eq!(decode_as(coded, "base64").unwrap(), plain);
        }
        assert_eq!(encode_as("user:pässword", "base64").unwrap(), "dXNlcjpww6Rzc3dvcmQ=");
        // unpadded, URL-safe, wrapped
        assert_eq!(decode_as("Zm9vYg", "base64").unwrap(), "foob");
        assert_eq!(base64_decode("-_8").unwrap(), vec![0xfb, 0xff]);
        assert_eq!(base64_decode("+/8=").unwrap(), vec![0xfb, 0xff]);
        assert_eq!(decode_as("Zm9v\nYmFy", "base64").unwrap(), "foobar");
        assert_eq!(base64_encode(&[0, 255, 128]), "AP+A");
        assert_eq!(decode_as("Zm9v!", "base64").unwrap_err(), "'Zm9v!' is not base64: '!' is not in its alphabet");
        assert_eq!(decode_as("Zm9vY", "base64").unwrap_err(), "'Zm9vY' is not base64: its length is not one an encoding has");
        assert_eq!(decode_as("Zg==Zg==", "base64").unwrap_err(), "'Zg==Zg==' is not base64: '=' pads only its end");
        assert_eq!(decode_as("/w==", "base64").unwrap_err(), "'/w==' decodes to bytes that are not text (UTF-8)");
    }

    #[test]
    fn url_components_both_ways() {
        assert_eq!(encode_as("a b&c=d/é?~", "url").unwrap(), "a%20b%26c%3Dd%2F%C3%A9%3F~");
        assert_eq!(decode_as("a%20b%26c%3dd%2F%C3%A9%3F~", "url").unwrap(), "a b&c=d/é?~");
        assert_eq!(decode_as("1+1", "url").unwrap(), "1+1");
        assert_eq!(decode_as("100%", "url").unwrap_err(), "'100%' is not URL-encoded: a % is followed by two hex digits");
        assert_eq!(decode_as("%zz", "url").unwrap_err(), "'%zz' is not URL-encoded: a % is followed by two hex digits");
        assert_eq!(decode_as("%FF", "url").unwrap_err(), "'%FF' decodes to bytes that are not text (UTF-8)");
        assert_eq!(encode_as("x", "hex").unwrap_err(), "encode[] knows no scheme 'hex' — base64 or url");
        assert_eq!(decode_as("x", "rot13").unwrap_err(), "decode[] knows no scheme 'rot13' — base64 or url");
    }
}
