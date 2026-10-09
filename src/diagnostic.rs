//! A refusal as the program's author reads it: where it is (the file, the
//! line and the column of the text it is about), what it says, and the line
//! with that text underlined (spec, Error Messages). Every command prints a
//! refusal through `render`, and the language server takes its ranges from
//! `spans`: one reading of a message, so the terminal and the editor point
//! at the same text.

/// One entry of a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The part a refusal belongs to (`sim/session.co`), when it is not the
    /// entry's.
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<usize>,
    /// 1-based, when the producer knows it (the lexer and the parser do).
    pub col: Option<usize>,
    pub message: String,
}

/// The entries of a refusal's text, one per line: `[part.co: ]line N[, col
/// M]: message`, or a parser's sentence holding `(line N, col M)` or `at
/// line N[, col M]`. An indented line continues the entry before it.
pub fn parse(text: &str) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        if raw.starts_with(char::is_whitespace)
            && let Some(last) = out.last_mut()
        {
            last.message.push('\n');
            last.message.push_str(raw);
            continue;
        }
        out.push(parse_entry(raw.trim()));
    }
    out
}

fn parse_entry(entry: &str) -> Diagnostic {
    let mut rest = entry.strip_prefix("error: ").unwrap_or(entry);
    let mut file = None;
    if let Some((head, tail)) = rest.split_once(": ")
        && !head.contains(' ')
        && head.ends_with(".co")
    {
        file = Some(head.to_string());
        rest = tail;
    }
    let rest = rest.strip_prefix("error: ").unwrap_or(rest);
    // `line N[, col M]: message`
    if let Some(after) = rest.strip_prefix("line ") {
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(line) = digits.parse::<usize>() {
            let mut tail = &after[digits.len()..];
            let mut col = None;
            if let Some(c) = tail.strip_prefix(", col ") {
                let cd: String = c.chars().take_while(|c| c.is_ascii_digit()).collect();
                col = cd.parse::<usize>().ok();
                tail = &c[cd.len()..];
            }
            if let Some(message) = tail.strip_prefix(": ") {
                return Diagnostic { file, line: Some(line), col, message: message.to_string() };
            }
        }
    }
    // a sentence holding its location: `(line N, col M)` or `at line N[, col M]`
    for (open, close) in [(" (line ", ")"), (" at line ", "")] {
        if let Some(at) = rest.find(open) {
            let after = &rest[at + open.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(line) = digits.parse::<usize>() {
                let mut tail = &after[digits.len()..];
                let mut col = None;
                if let Some(c) = tail.strip_prefix(", col ") {
                    let cd: String = c.chars().take_while(|c| c.is_ascii_digit()).collect();
                    col = cd.parse::<usize>().ok();
                    tail = &c[cd.len()..];
                }
                if let Some(t) = tail.strip_prefix(close) {
                    let message = format!("{}{}", &rest[..at], t);
                    return Diagnostic { file, line: Some(line), col, message: message.trim().to_string() };
                }
            }
        }
    }
    Diagnostic { file, line: None, col: None, message: rest.to_string() }
}

/// What a message quotes: each `'…'` whose opening quote starts a word and
/// whose closing quote ends one — the apostrophe of `frame's` quotes
/// nothing.
pub fn quoted(message: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = message.char_indices().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        let opens = c == '\'' && (i == 0 || !word(chars[i - 1].1));
        if opens && let Some(j) = (i + 1..chars.len()).find(|&j| chars[j].1 == '\'' && chars.get(j + 1).is_none_or(|n| !word(n.1))) {
            out.push(&message[at + 1..chars[j].0]);
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// The text a message is about, as the program wrote it: the calls it
/// names (`frame.new[] is frame's`, `abs[] and max[] of a column …`), else
/// the last name it quotes before its advice (`'f' has no parameter named
/// 'y'` is about `y`). Advice — after ` — `, `: add`, `did you mean` — names
/// what to write, not what was written.
pub fn subjects(message: &str) -> Vec<String> {
    let head = message
        .split(" — ")
        .next()
        .unwrap_or(message)
        .split(": add `use")
        .next()
        .unwrap_or(message)
        .split("did you mean")
        .next()
        .unwrap_or(message);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // `name[]` calls, dotted names included
    let mut calls = Vec::new();
    let chars: Vec<char> = head.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == ']' {
            let mut s = i;
            while s > 0 && (word(chars[s - 1]) || chars[s - 1] == '.') {
                s -= 1;
            }
            let name: String = chars[s..i].iter().collect();
            let name = name.trim_matches('.').to_string();
            if !name.is_empty() && !calls.contains(&name) {
                calls.push(name);
            }
        }
        i += 1;
    }
    if !calls.is_empty() {
        return calls;
    }
    // the last quoted name or operator
    let quoted: Vec<&str> = quoted(head);
    let usable = |q: &&str| {
        !q.is_empty()
            && (q.chars().all(|c| word(c) || c == '.') || (q.len() <= 3 && q.chars().all(|c| "+-*/^<>=!&|:;{}#%".contains(c))))
    };
    quoted.into_iter().rfind(usable).map(|q| vec![q.to_string()]).unwrap_or_default()
}

/// A line with what is not code blanked: the text of its strings (a
/// backtick's interpolation inside one stays, being code) and its comment.
fn code_of(line: &str) -> Vec<char> {
    let mut out = Vec::new();
    let (mut in_str, mut in_interp, mut escaped) = (false, false, false);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !in_str && c == '-' && chars.get(i + 1) == Some(&'-') && chars.get(i + 2) == Some(&'-') {
            out.extend(std::iter::repeat_n(' ', chars.len() - i));
            break;
        }
        if in_str {
            if escaped {
                escaped = false;
                out.push(' ');
            } else if c == '\\' && !in_interp {
                escaped = true;
                out.push(' ');
            } else if c == '`' {
                in_interp = !in_interp;
                out.push(' ');
            } else if c == '\'' && !in_interp {
                in_str = false;
                out.push(' ');
            } else {
                out.push(if in_interp { c } else { ' ' });
            }
        } else if c == '\'' {
            in_str = true;
            out.push(' ');
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Where on its line a message points: every place the line's code holds
/// one of the message's subjects as a whole word (or the operator), as
/// 0-based character ranges, in order. Empty when the line holds none.
pub fn spans(line: &str, message: &str) -> Vec<(usize, usize)> {
    let code = code_of(line);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    for subject in subjects(message) {
        let want: Vec<char> = subject.chars().collect();
        let is_word = want.iter().all(|c| word(*c) || *c == '.');
        let mut i = 0;
        while i + want.len() <= code.len() {
            let here = code[i..i + want.len()] == want[..]
                && (!is_word
                    || ((i == 0 || !word(code[i - 1]))
                        && code.get(i + want.len()).is_none_or(|c| !word(*c))));
            if here {
                found.push((i, i + want.len()));
                i += want.len();
            } else {
                i += 1;
            }
        }
    }
    found.sort();
    found
}

/// The ranges a diagnostic underlines on its line: its subjects, else the
/// token at the producer's column, else the statement from its indentation.
pub fn underline(line: &str, d: &Diagnostic) -> Vec<(usize, usize)> {
    let found = spans(line, &d.message);
    if !found.is_empty() {
        return found;
    }
    let chars: Vec<char> = line.chars().collect();
    if let Some(col) = d.col.filter(|c| *c >= 1 && *c <= chars.len()) {
        let start = col - 1;
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut end = start + 1;
        if word(chars[start]) {
            while end < chars.len() && word(chars[end]) {
                end += 1;
            }
        }
        return vec![(start, end)];
    }
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let end = chars.len().max(indent + 1);
    vec![(indent, end)]
}

/// A refusal, printed: each entry as `file:line:col: message`, then its line
/// with the text it is about underlined; the same message on the same line
/// once; a count when there are several. `read` finds a part's text by the
/// name a refusal gives it.
pub fn render(file: &str, source: &str, text: &str, read: &dyn Fn(&str) -> Option<String>) -> String {
    let mut entries: Vec<Diagnostic> = Vec::new();
    for d in parse(text) {
        if !entries.iter().any(|e| e.file == d.file && e.line == d.line && e.message == d.message) {
            entries.push(d);
        }
    }
    // in the order they are read: by file (the entry's first), line, column
    let place = |d: &Diagnostic| -> (bool, String, usize, usize) {
        let col = match (&d.file, d.line) {
            (None, Some(l)) => source.lines().nth(l - 1).and_then(|t| underline(t, d).first().map(|m| m.0)).unwrap_or(0),
            _ => d.col.unwrap_or(0),
        };
        (d.file.is_some(), d.file.clone().unwrap_or_default(), d.line.unwrap_or(0), col)
    };
    entries.sort_by_key(|d| place(d));
    let mut out = String::new();
    for d in &entries {
        let (name, text) = match &d.file {
            Some(part) => (part.clone(), read(part)),
            None => (file.to_string(), Some(source.to_string())),
        };
        // (a refusal printed without its entry names only its parts)
        let name = if name.is_empty() { "the program".to_string() } else { name };
        let Some(line) = d.line else {
            out.push_str(&format!("{}: {}\n", name, d.message));
            continue;
        };
        let line_text = text.as_deref().and_then(|t| t.lines().nth(line - 1));
        let Some(line_text) = line_text else {
            out.push_str(&format!("{}:{}: {}\n", name, line, d.message));
            continue;
        };
        let marks = underline(line_text, d);
        let col = marks.first().map(|m| m.0 + 1).or(d.col).unwrap_or(1);
        out.push_str(&format!("{}:{}:{}: {}\n", name, line, col, d.message));
        let width = line.to_string().len() + 2;
        out.push_str(&format!("{:>w$} │ {}\n", line, line_text, w = width));
        let chars: Vec<char> = line_text.chars().collect();
        let mut carets = String::new();
        let mut at = 0;
        for (s, e) in &marks {
            while at < *s {
                carets.push(if chars.get(at) == Some(&'\t') { '\t' } else { ' ' });
                at += 1;
            }
            while at < *e {
                carets.push('^');
                at += 1;
            }
        }
        out.push_str(&format!("{:w$} │ {}\n", "", carets, w = width));
    }
    if entries.len() > 1 {
        let mut imports: Vec<String> = Vec::new();
        for d in &entries {
            if let Some(m) = d.message.split("add `use ").nth(1).and_then(|r| r.split('`').next()) {
                let m = format!("use {}", m);
                if !imports.contains(&m) {
                    imports.push(m);
                }
            }
        }
        let fixed = entries.iter().filter(|d| d.message.contains("add `use ")).count();
        if imports.is_empty() {
            out.push_str(&format!("{} mistakes\n", entries.len()));
        } else {
            out.push_str(&format!("{} mistakes · {} fixed by adding a line: {}\n", entries.len(), fixed, imports.join(", ")));
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_location_format_is_read() {
        let d = parse("line 3: undefined variable 'z'");
        assert_eq!((d[0].line, d[0].col, d[0].message.as_str()), (Some(3), None, "undefined variable 'z'"));
        let d = parse("error: line 3: missing argument 'b' in call to 'f'");
        assert_eq!((d[0].line, d[0].message.as_str()), (Some(3), "missing argument 'b' in call to 'f'"));
        let d = parse("sim/session.co: line 60: undefined variable 'q'");
        assert_eq!((d[0].file.as_deref(), d[0].line), (Some("sim/session.co"), Some(60)));
        let d = parse("Crow has no ':' (line 1, col 9) — blocks are defined by indentation");
        assert_eq!((d[0].line, d[0].col, d[0].message.as_str()), (Some(1), Some(9), "Crow has no ':' — blocks are defined by indentation"));
        let d = parse("expected ']', got '(' at line 3");
        assert_eq!((d[0].line, d[0].message.as_str()), (Some(3), "expected ']', got '('"));
        let d = parse("unterminated string at line 1, col 5");
        assert_eq!((d[0].line, d[0].col), (Some(1), Some(5)));
        let d = parse("Crow booleans are lowercase: use 'true', not 'True'.");
        assert_eq!(d[0].line, None);
    }

    #[test]
    fn a_possessive_quotes_nothing() {
        assert_eq!(quoted("frame.new[] is frame's — add `use frame`"), Vec::<&str>::new());
        assert_eq!(quoted("frame has no 'nosuch' — mean[] is math's"), vec!["nosuch"]);
        assert_eq!(quoted("cannot use '+' with strings"), vec!["+"]);
    }

    #[test]
    fn a_message_points_at_what_it_names() {
        let line = "ours = frame.new[node = keys[m], value = values[m]]";
        assert_eq!(spans(line, "frame.new[] is frame's — add `use frame`"), vec![(7, 16)]);
        assert_eq!(spans(line, "undefined variable 'm'"), vec![(29, 30), (48, 49)]);
        // advice names what to write, not what was written
        assert_eq!(spans("log[lenn[xs]]", "undefined function 'lenn' — did you mean 'len'?"), vec![(4, 8)]);
        assert_eq!(spans("log[f[x = 1, y = 2]]", "'f' has no parameter named 'y' (parameters: x)"), vec![(13, 14)]);
        // a name inside a string's text is not the name; inside its interpolation it is
        assert_eq!(spans("log['m is `m`']", "undefined variable 'm'"), vec![(11, 12)]);
        assert_eq!(spans("x = 1 --- m", "undefined variable 'm'"), vec![]);
        // a field is read after its dot
        assert_eq!(spans("log[x.name]", "an int has no field 'name'"), vec![(6, 10)]);
        assert_eq!(spans("pr = max[abs[both.value - both.pagerank]]", "max[] and abs[] of a column are math's — add `use math`"), vec![(5, 8), (9, 12)]);
    }

    #[test]
    fn a_refusal_is_rendered_once_with_its_line_underlined() {
        let src = "ours = frame.new[node = keys[m], value = values[m]]\nboth = join[ours, table, 'node']\n";
        let text = "line 1: frame.new[] is frame's — add `use frame`\nline 1: undefined variable 'm'\nline 1: undefined variable 'm'\nline 2: undefined variable 'table'";
        let shown = render("ex1.co", src, text, &|_| None);
        assert_eq!(
            shown,
            "ex1.co:1:8: frame.new[] is frame's — add `use frame`\n  1 │ ours = frame.new[node = keys[m], value = values[m]]\n    │        ^^^^^^^^^\n\
             ex1.co:1:30: undefined variable 'm'\n  1 │ ours = frame.new[node = keys[m], value = values[m]]\n    │                              ^                  ^\n\
             ex1.co:2:19: undefined variable 'table'\n  2 │ both = join[ours, table, 'node']\n    │                   ^^^^^\n\
             3 mistakes · 1 fixed by adding a line: use frame"
        );
    }

    #[test]
    fn a_message_without_a_place_still_names_its_file() {
        assert_eq!(render("a.co", "x\n", "Crow booleans are lowercase", &|_| None), "a.co: Crow booleans are lowercase");
        // the producer's column, when the line holds no name the message gives
        assert_eq!(render("a.co", "x = 1;\n", "unexpected character ';' at line 1, col 6", &|_| None), "a.co:1:6: unexpected character ';'\n  1 │ x = 1;\n    │      ^");
    }
}
