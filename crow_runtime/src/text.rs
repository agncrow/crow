//! Text, once: the string builtins and the regular expressions. Both modes
//! call these (the Rust backend prints the call, the interpreter calls it
//! with its values' strings), so a builtin means the same in both. An
//! invalid pattern is an error with the reason the regex crate gives, and
//! only the caller decides whether that stops the program.

use std::sync::atomic::{AtomicUsize, Ordering};

/// What is known of a text's start: how many of its first bytes are ASCII —
/// each one character, so a position among them is a byte offset. A text
/// read by position many times (a loop over a sequence's windows) keeps one
/// beside it — both backends do, for a local whose text does not change
/// while they hold it — and each read extends it only as far as the read
/// needs: the first read checks the text up to where it reads, and a read
/// within what is known is a byte offset. (A slice had walked the text
/// character by character from its start, so a loop over a long text's
/// windows was quadratic: k-nucleotide's 125 million bases could not be
/// counted by slices.) One made for a single read costs what the read
/// costs: the text checked up to where it reads.
#[derive(Debug, Default)]
pub struct Known(AtomicUsize);

impl Known {
    pub const fn new() -> Known {
        Known(AtomicUsize::new(0))
    }

    /// A new text: nothing known of it.
    pub fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }

    /// The ASCII bytes the text starts with, checked at least `need` bytes
    /// in, or as far as a byte that is not ASCII.
    fn ascii(&self, s: &str, need: usize) -> usize {
        let known = self.0.load(Ordering::Relaxed).min(s.len());
        let need = need.min(s.len());
        if need <= known {
            return known;
        }
        let p = known + first_non_ascii(&s.as_bytes()[known..need]);
        self.0.fetch_max(p, Ordering::Relaxed);
        p
    }
}

impl Clone for Known {
    fn clone(&self) -> Known {
        Known(AtomicUsize::new(self.0.load(Ordering::Relaxed)))
    }
}

/// Where the first byte that is not ASCII is (the length when none is),
/// checked 32 bytes at a time.
fn first_non_ascii(b: &[u8]) -> usize {
    let mut i = 0;
    while i + 32 <= b.len() && b[i..i + 32].is_ascii() {
        i += 32;
    }
    i + b[i..].iter().position(|c| !c.is_ascii()).unwrap_or(b.len() - i)
}

/// The positions of a text's slice by character, counted from its end when
/// negative as a list's are (`core::slice_bounds`): the text's characters
/// counted only then, its known ASCII start a byte each.
fn from_end(s: &str, known: &Known, start: i64, end: i64) -> (i64, i64) {
    if start >= 0 && end >= 0 {
        return (start, end);
    }
    let p = known.ascii(s, s.len());
    let n = if p == s.len() { p } else { p + s[p..].chars().count() };
    let (a, b) = crate::core::slice_bounds(start, end, n);
    (a as i64, b as i64)
}

/// `slice[text, from, to]` by character — counted from the end when
/// negative, then clamped to the text, empty when `from` is not before
/// `to` — read with what is known of its start.
pub fn slice_known(s: &str, known: &Known, start: i64, end: i64) -> String {
    let (start, end) = from_end(s, known, start, end);
    let st = start.max(0) as usize;
    let en = end.max(0) as usize;
    if st >= en {
        return String::new();
    }
    let p = known.ascii(s, en);
    if en <= p {
        return s[st..en].to_string();
    }
    if p == s.len() {
        return if st >= p { String::new() } else { s[st..].to_string() };
    }
    // past the ASCII start: its characters one byte each, the rest walked
    let mut out = String::new();
    if st < p {
        out.push_str(&s[st..p]);
        out.extend(s[p..].chars().take(en - p));
    } else {
        out.extend(s[p..].chars().skip(st - p).take(en - st));
    }
    out
}

/// `slice[text, from, to] eq other`, the slice compared where it lies
/// within what is known of the text's start (and made only past it).
pub fn slice_is(s: &str, known: &Known, start: i64, end: i64, other: &str) -> bool {
    let (start, end) = from_end(s, known, start, end);
    let st = start.max(0) as usize;
    let en = end.max(0) as usize;
    if st >= en {
        return other.is_empty();
    }
    let p = known.ascii(s, en);
    if en <= p {
        return &s[st..en] == other;
    }
    if p == s.len() {
        return if st >= p { other.is_empty() } else { &s[st..] == other };
    }
    slice_known(s, known, start, end) == other
}

/// `text[i] eq other`: the character at a position compared where it lies
/// (no text made of it), or why there is none — what `char_at_known`
/// answers, compared.
#[inline]
pub fn char_is(s: &str, known: &Known, i: i64, other: &str) -> Result<bool, String> {
    if i >= 0 {
        let k = i as usize;
        if k < known.ascii(s, k.saturating_add(1)) {
            let b = other.as_bytes();
            return Ok(b.len() == 1 && s.as_bytes()[k] == b[0]);
        }
    }
    char_at_known(s, known, i).map(|c| c == other)
}

/// `occurs[text, part]`: how many times the part occurs in the text, not
/// overlapping (`occurs['aaaa', 'aa']` is 2), as Python's `str.count` and
/// Go's `strings.Count` count, at about the cost of reading the text: one
/// byte, or two different bytes (which cannot overlap themselves, so every
/// place counts), by a compare the compiler runs on many bytes at once; a
/// longer part by memchr's search. An empty part counts nothing a program
/// means: refused.
pub fn count(s: &str, part: &str) -> Result<i64, String> {
    let h = s.as_bytes();
    Ok(match part.as_bytes() {
        [] => return Err("occurs[] counts a part of at least one character".to_string()),
        [c] => counted(h, h, |x, _| x == *c),
        [a, b] if a != b => {
            if h.len() < 2 {
                return Ok(0);
            }
            counted(&h[..h.len() - 1], &h[1..], |x, y| (x == *a) & (y == *b))
        }
        p => memchr::memmem::find_iter(h, p).count(),
    } as i64)
}

/// How many places `hit` holds at, of two byte slices read side by side:
/// each chunk's count kept in a byte (a chunk is shorter than 256, so it
/// cannot wrap — added unchecked, as a program's build checks every add
/// otherwise), so the compare runs a vector's width of bytes at a time.
fn counted(xs: &[u8], ys: &[u8], hit: impl Fn(u8, u8) -> bool) -> usize {
    let mut total = 0usize;
    for (cx, cy) in xs.chunks(240).zip(ys.chunks(240)) {
        let mut n = 0u8;
        for (&x, &y) in cx.iter().zip(cy) {
            n = n.wrapping_add(hit(x, y) as u8);
        }
        total += n as usize;
    }
    total
}

/// `translate[text, from, to]`: each character of `from` in the text
/// replaced by the character at its place in `to`, in one pass — the
/// complement of DNA is `translate[s, 'ACGT', 'TGCA']`. A text of ASCII
/// maps its bytes through a table; any other, its characters. The two
/// must have as many characters.
pub fn translate(s: &str, from: &str, to: &str) -> Result<String, String> {
    translate_taken(s.to_string(), from, to)
}

/// `translate`, of a text nothing else holds: its bytes changed where they
/// lie when the text and both maps are ASCII.
pub fn translate_taken(s: String, from: &str, to: &str) -> Result<String, String> {
    let (nf, nt) = (from.chars().count(), to.chars().count());
    if nf != nt {
        return Err(format!("translate[] maps each character of from to the one at its place in to: '{}' has {}, '{}' has {}", from, nf, to, nt));
    }
    if s.is_ascii() && from.is_ascii() && to.is_ascii() {
        let mut table: [u8; 256] = [0; 256];
        for (i, b) in table.iter_mut().enumerate() {
            *b = i as u8;
        }
        for (f, t) in from.bytes().zip(to.bytes()) {
            table[f as usize] = t;
        }
        let mut bytes = s.into_bytes();
        for b in bytes.iter_mut() {
            *b = table[*b as usize];
        }
        // (ASCII mapped through ASCII stays UTF-8)
        return String::from_utf8(bytes).map_err(|e| format!("translate[]: {}", e));
    }
    let map: std::collections::HashMap<char, char> = from.chars().zip(to.chars()).collect();
    Ok(s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect())
}

/// `len[text]`: its characters, counted with what is known of its start.
pub fn len_known(s: &str, known: &Known) -> i64 {
    let p = known.ascii(s, s.len());
    (p + s[p..].chars().count()) as i64
}

/// `text[i]`: the character at a position, counted from the end when
/// negative, or why there is none.
pub fn char_at_known(s: &str, known: &Known, i: i64) -> Result<String, String> {
    if i >= 0 {
        let k = i as usize;
        if k < known.ascii(s, k.saturating_add(1)) {
            return Ok(s[k..k + 1].to_string());
        }
    }
    let n = len_known(s, known) as usize;
    let k = crate::stops::char_at(i, n)?;
    let p = known.ascii(s, k + 1);
    Ok(if k < p { s[k..k + 1].to_string() } else { s[p..].chars().nth(k - p).map(String::from).unwrap_or_default() })
}

/// `split[text, separator]`: the pieces between each separator. A
/// separator of one character is searched as a character (the byte
/// search), where the substring search cost a parser four times as much.
#[inline]
pub fn split(s: &str, sep: &str) -> Vec<String> {
    let mut chars = sep.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => s.split(c).map(|p| p.to_string()).collect(),
        _ => s.split(sep).map(|p| p.to_string()).collect(),
    }
}

/// `trim[text]`: without the whitespace at either end.
#[inline]
pub fn trim(s: &str) -> String {
    s.trim().to_string()
}

/// `replace[text, old, new]`: every `old` replaced by `new`. One ASCII
/// character for another is a loop over the bytes: the standard library's
/// own path for it ran three times slower inside a program linked with fat
/// LTO, the interpreter (linked without) ten times faster on a long text.
pub fn replace(s: &str, old: &str, new: &str) -> String {
    if let Some((o, n)) = ascii_pair(old, new) {
        let bytes: Vec<u8> = s.bytes().map(|b| if b == o { n } else { b }).collect();
        return String::from_utf8(bytes).unwrap_or_else(|_| s.replace(old, new));
    }
    s.replace(old, new)
}

/// The two bytes of a replacement of one ASCII character by another.
fn ascii_pair(old: &str, new: &str) -> Option<(u8, u8)> {
    match (old.as_bytes(), new.as_bytes()) {
        (&[o], &[n]) if o.is_ascii() && n.is_ascii() => Some((o, n)),
        _ => None,
    }
}

/// `replace` of a text nothing else holds (`s = replace[s, …]`, or the
/// answer of another call): one ASCII character for another is written
/// where the text lies — an ASCII byte is never part of another
/// character, so the text stays UTF-8 — and anything else is `replace`'s.
pub fn replace_taken(s: String, old: &str, new: &str) -> String {
    if let Some((o, n)) = ascii_pair(old, new) {
        let mut bytes = s.into_bytes();
        for b in bytes.iter_mut() {
            if *b == o {
                *b = n;
            }
        }
        return String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    }
    replace(&s, old, new)
}

/// `upper[text]`.
#[inline]
pub fn upper(s: &str) -> String {
    s.to_uppercase()
}

/// `upper` of a text nothing else holds: ASCII text is raised where it
/// lies, any other is `upper`'s.
pub fn upper_taken(mut s: String) -> String {
    if s.is_ascii() {
        s.make_ascii_uppercase();
        return s;
    }
    s.to_uppercase()
}

/// `lower[text]`.
#[inline]
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `lower` of a text nothing else holds: ASCII text is lowered where it
/// lies, any other is `lower`'s (a final sigma is the context's).
pub fn lower_taken(mut s: String) -> String {
    if s.is_ascii() {
        s.make_ascii_lowercase();
        return s;
    }
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
    crate::stops::cells("repeat", &[s.len(), n as usize])?;
    Ok(s.repeat(n as usize))
}

/// A pattern compiled, or what is wrong with it on one line: the regex
/// crate draws the pattern over several, and a stop is one line.
pub(crate) fn compile(pattern: &str) -> Result<regex::Regex, String> {
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

/// `len[sub[s, pattern, replacement]]` without the text `sub` would make:
/// the characters of `s`, less each match's, plus each replacement's as
/// `sub` writes it (`$1` its group). regex-redux counted nine patterns of a
/// 50 MB sequence by the length each `sub` left, each a 50 MB copy, nine at
/// once on the cores.
pub fn sub_len(s: &str, pattern: &str, replacement: &str) -> Result<i64, String> {
    let re = compile(pattern)?;
    let mut n = len_known(s, &Known::new());
    if !replacement.contains('$') {
        let r = replacement.chars().count() as i64;
        for m in re.find_iter(s) {
            n += r - m.as_str().chars().count() as i64;
        }
    } else {
        let mut written = String::new();
        for caps in re.captures_iter(s) {
            let whole = caps.get(0).map_or(0, |m| m.as_str().chars().count()) as i64;
            written.clear();
            caps.expand(replacement, &mut written);
            n += written.chars().count() as i64 - whole;
        }
    }
    Ok(n)
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
mod sub_len_tests {
    use super::*;

    /// The length a substitution makes, counted without the text, is the
    /// length of the text it makes: literal and group replacements, empty
    /// matches, characters beyond ASCII, and the same reason for a pattern
    /// that is none
    #[test]
    fn the_length_of_a_substitution_is_its_texts() {
        let s = "The quick brown fox — jumps über the lazy dög";
        for (p, r) in [("o", ""), ("(qu)ick", "$1$1"), ("[aeiouö]", "XY"), ("zzz", ""), ("", "-"), ("ü|ö", "ue"), ("(\\w)(\\w)", "$2$1"), ("$", "!")] {
            let made = sub(s, p, r).expect("a substitution");
            assert_eq!(sub_len(s, p, r), Ok(made.chars().count() as i64), "{} → {}", p, r);
        }
        assert_eq!(sub_len(s, "(", ""), Err(sub(s, "(", "").unwrap_err()));
    }
}

#[cfg(test)]
mod tests {
    /// `occurs` counts as Python's `str.count` does, by every path: one
    /// byte, two different bytes (across the chunks the compare is run in),
    /// a part that overlaps itself, a longer part, a character of two bytes
    #[test]
    fn occurs_counts_what_a_search_finds() {
        let plain = |s: &str, p: &str| -> i64 {
            let (mut n, mut i) = (0, 0);
            while let Some(at) = s[i..].find(p) {
                n += 1;
                i += at + p.len();
            }
            n
        };
        let mut text = String::new();
        let mut x: u64 = 7;
        for _ in 0..5000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            text.push(['A', 'C', 'G', 'T', '\n'][(x >> 33) as usize % 5]);
        }
        for p in ["A", "\n", "AC", "GT", "TA", "AA", "AAA", "GGT", "GGTATT", "ACGTACGT"] {
            assert_eq!(super::count(&text, p).unwrap(), plain(&text, p), "{}", p);
        }
        assert_eq!(super::count("aaaa", "aa").unwrap(), 2);
        assert_eq!(super::count("a", "ab").unwrap(), 0);
        assert_eq!(super::count("", "a").unwrap(), 0);
        assert_eq!(super::count("héllo hé", "é").unwrap(), 2);
        assert!(super::count("abc", "").is_err());
    }

    /// One ASCII character for another, in place or not, is the standard
    /// library's replace; so is every other replacement, and upper, lower
    /// and reverse of a text taken are the copies' (non-ASCII text included)
    #[test]
    fn a_replacement_taken_or_copied_is_the_standard_one() {
        let texts = ["", "ACGT", "aAbBA", "ÄA→A", "naïve café", "A".repeat(1000).as_str(), "ΣΑΣ σας"].map(String::from);
        for s in &texts {
            for (o, n) in [("A", "t"), ("a", "Z"), ("A", "→"), ("→", "A"), ("AB", "x"), ("", "x"), ("A", "")] {
                assert_eq!(replace(s, o, n), s.replace(o, n), "{:?} {} {}", s, o, n);
                assert_eq!(replace_taken(s.clone(), o, n), s.replace(o, n), "{:?} {} {}", s, o, n);
            }
            assert_eq!(upper_taken(s.clone()), s.to_uppercase());
            assert_eq!(lower_taken(s.clone()), s.to_lowercase());
            let backwards: String = s.chars().rev().collect();
            assert_eq!(crate::core::reverse_text(s), backwards);
            assert_eq!(crate::core::reverse_text_taken(s.clone()), backwards);
        }
    }

    /// A slice, a length and a position read with what is known of a
    /// text's start answer what the character walk answered, for ASCII
    /// text, text that is not, and text that starts as one and goes on as
    /// the other; a read within what is known is a byte offset.
    #[test]
    fn a_text_read_with_what_is_known_answers_what_the_walk_answered() {
        // (a negative position counts from the end, then clamps: `slice`'s
        // rule for every sequence, `core::slice_bounds`)
        fn walk(s: &str, a: i64, b: i64) -> String {
            let n = s.chars().count() as i64;
            let at = |i: i64| (if i < 0 { i + n } else { i }).clamp(0, n) as usize;
            let (st, en) = (at(a), at(b));
            if st >= en { String::new() } else { s.chars().skip(st).take(en - st).collect() }
        }
        for s in ["", "acgt", "héllo wörld", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ→é", "→ab", "ab\u{1F600}cd"] {
            let k = Known::new();
            let n = s.chars().count() as i64;
            assert_eq!(len_known(s, &k), n, "{}", s);
            for a in -2..n + 3 {
                for b in -1..n + 4 {
                    assert_eq!(slice_known(s, &k, a, b), walk(s, a, b), "{} [{}, {}]", s, a, b);
                    assert_eq!(slice_known(s, &Known::new(), a, b), walk(s, a, b), "{} [{}, {}] fresh", s, a, b);
                    assert!(slice_is(s, &k, a, b, &walk(s, a, b)), "{} [{}, {}] is", s, a, b);
                    assert_eq!(slice_is(s, &k, a, b, "zz"), walk(s, a, b) == "zz");
                }
                let want = if a >= -n && a < n { Ok(s.chars().nth(if a < 0 { (n + a) as usize } else { a as usize }).map(String::from).unwrap_or_default()) } else { Err(()) };
                assert_eq!(char_at_known(s, &k, a).map_err(|_| ()), want, "{} [{}]", s, a);
            }
        }
        let long: String = "acgt".repeat(10_000);
        let k = Known::new();
        assert_eq!(slice_known(&long, &k, 39_996, 40_000), "acgt");
        assert_eq!(k.0.load(Ordering::Relaxed), 40_000, "checked once, as far as the read went");
        k.reset();
        assert_eq!(k.0.load(Ordering::Relaxed), 0);
    }

    use super::*;

    /// A separator of one character, of several, and of none split as the
    /// substring search does
    #[test]
    fn a_separator_of_any_length_splits_the_same() {
        assert_eq!(split("a\tb\t\tc", "\t"), vec!["a", "b", "", "c"]);
        assert_eq!(split("a, b, c", ", "), vec!["a", "b", "c"]);
        assert_eq!(split("ab", ""), "ab".split("").map(str::to_string).collect::<Vec<_>>());
        assert_eq!(split("", "\n"), vec![""]);
        assert_eq!(split("é|ü", "|"), vec!["é", "ü"]);
    }

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
