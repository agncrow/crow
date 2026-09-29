//! A path as Crow writes it: a `:name` segment is a parameter
//! (`/card/:id`) — an HTTP route's, which its handler reads from
//! `req.params`, and a page's, which names a fact in its address (spec,
//! Core (Server)). Pure text: no I/O.

/// Whether a segment's name is a parameter's: letters, digits and `_`.
fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The names of a path's parameters, in order: `/card/:id/:tab` → `id`,
/// `tab`. Only a whole segment is one: `/at/12:30` has none.
pub fn params(path: &str) -> Vec<String> {
    path.split('/').filter_map(|seg| seg.strip_prefix(':')).filter(|n| is_name(n)).map(|n| n.to_string()).collect()
}

/// The path as axum 0.8 matches it: `{id}` for each `:id`.
pub fn to_axum(path: &str) -> String {
    path.split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(n) if is_name(n) => format!("{{{}}}", n),
            _ => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The path with each parameter's value in its place, percent-encoded
/// (`/card/:id` with `id` 7 is `/card/7`); a parameter `value` has no text
/// for keeps its segment.
pub fn fill(path: &str, value: impl Fn(&str) -> Option<String>) -> String {
    path.split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(n) if is_name(n) => value(n).map(|v| encode(&v)).unwrap_or_else(|| seg.to_string()),
            _ => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Percent-encode a path segment or a query value: letters, digits and
/// `-_.~` stay.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Percent-decode a query's name or value (`+` is a space); a byte
/// sequence that is not UTF-8 is read as UTF-8 lossily.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            // two hex digits follow it
            b'%' if i + 2 < bytes.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query string's pairs, decoded, in order: `day=2026-09-04&sym=IMRN`.
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_s_parameters_are_its_colon_segments() {
        assert_eq!(params("/card/:id/:tab"), vec!["id", "tab"]);
        assert_eq!(params("/at/12:30"), Vec::<String>::new());
        assert_eq!(params("/"), Vec::<String>::new());
        assert_eq!(to_axum("/users/:id"), "/users/{id}");
        assert_eq!(to_axum("/a/:b/:c"), "/a/{b}/{c}");
        assert_eq!(to_axum("/at/12:30"), "/at/12:30");
        assert_eq!(to_axum("/"), "/");
        assert_eq!(to_axum("/_ws/card/:card"), "/_ws/card/{card}");
    }

    #[test]
    fn a_path_is_filled_with_its_values_encoded() {
        let v = |n: &str| match n {
            "id" => Some("7".to_string()),
            "who" => Some("ada lovelace/x".to_string()),
            _ => None,
        };
        assert_eq!(fill("/card/:id", v), "/card/7");
        assert_eq!(fill("/people/:who/:id", v), "/people/ada%20lovelace%2Fx/7");
        assert_eq!(fill("/card/:gone", v), "/card/:gone");
        assert_eq!(fill("/", v), "/");
    }

    #[test]
    fn a_query_is_read_back_decoded() {
        assert_eq!(decode("ada%20lovelace%2Fx"), "ada lovelace/x");
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("caf%C3%A9"), "café");
        assert_eq!(parse_query("?day=2026-09-04&sym=IM%20RN&flag"), vec![
            ("day".to_string(), "2026-09-04".to_string()),
            ("sym".to_string(), "IM RN".to_string()),
            ("flag".to_string(), String::new()),
        ]);
        assert_eq!(decode(&encode("x/y z?&=é")), "x/y z?&=é");
    }
}
