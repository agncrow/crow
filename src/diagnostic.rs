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

/// Where a line begins: inside a string written across lines, and inside
/// that string's interpolation. A line a string's text runs onto is read
/// from inside the string, its code after the quote that closes it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Opening {
    in_str: bool,
    in_interp: bool,
}

/// Where each line of a source begins, in order (`Opening`).
pub fn openings(source: &str) -> Vec<Opening> {
    let mut at = Opening::default();
    source
        .lines()
        .map(|line| {
            let here = at;
            at = code_from(line, here).1;
            here
        })
        .collect()
}

/// A line with what is not code blanked: the text of its strings (a
/// backtick's interpolation inside one stays, being code) and its comment;
/// the line read from where `at` says it begins, and where the next line
/// begins.
pub fn code_from(line: &str, at: Opening) -> (Vec<char>, Opening) {
    let mut out = Vec::new();
    let (mut in_str, mut in_interp, mut escaped) = (at.in_str, at.in_interp, false);
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
    (out, Opening { in_str, in_interp })
}

/// `spans_from` of a line that begins outside any string, as a test
/// writes one.
#[cfg(test)]
pub fn spans(line: &str, message: &str) -> Vec<(usize, usize)> {
    spans_from(line, Opening::default(), message)
}

/// Where on its line a message points: every place the line's code holds
/// one of the message's subjects as a whole word (or the operator), as
/// 0-based character ranges, in order — the line read from where `at`
/// says it begins (`openings`: the line a string written across lines
/// closes on holds code after its quote). Empty when the line holds none.
pub fn spans_from(line: &str, at: Opening, message: &str) -> Vec<(usize, usize)> {
    let code = code_from(line, at).0;
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

/// The ranges a diagnostic underlines on its line, which begins where `at`
/// says (`openings`): its subjects, else the token at the producer's
/// column, else the statement from its indentation.
pub fn underline(line: &str, at: Opening, d: &Diagnostic) -> Vec<(usize, usize)> {
    let found = spans_from(line, at, &d.message);
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
    let opens = openings(source);
    let place = |d: &Diagnostic| -> (bool, String, usize, usize) {
        let col = match (&d.file, d.line) {
            (None, Some(l)) => source.lines().nth(l - 1).and_then(|t| underline(t, opens.get(l - 1).copied().unwrap_or_default(), d).first().map(|m| m.0)).unwrap_or(0),
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
        let at = text.as_deref().and_then(|t| openings(t).get(line - 1).copied()).unwrap_or_default();
        let marks = underline(line_text, at, d);
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

/// A change a refusal names: its text on one line, from `start` to `end`
/// (0-based characters), replaced by `with` — which may hold a line break,
/// or be a whole line inserted (`start` = `end` = 0, `with` ending in one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    /// 1-based.
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub with: String,
    /// What it does, as `crow fix` and the editor's quick fix say it.
    pub what: String,
}

/// The fixes a refusal names, read from its messages — the one reading
/// the editor's quick fixes and `crow fix` share: a slip's nearest name
/// (`did you mean 'score'?`), a missing import (`add `use math``), and a
/// habit taught its Crow form where the form is one (`==` is `eq`, `True`
/// is `true`, `f(1)` is `f[1]`, `**` is `^`, `#` is `---`, `x += 1` is
/// `x = x + 1`, `push[xs, v]` alone is `xs = push[xs, v]`, two statements
/// on a line are two lines, a `;` goes). A message with two ways out
/// (`add `use frame` or `use math``) names no fix: that is the author's.
pub fn fixes(source: &str, text: &str) -> Vec<Fix> {
    let lines: Vec<&str> = source.lines().collect();
    let opens = openings(source);
    let mut out: Vec<Fix> = Vec::new();
    let mut imports: Vec<String> = Vec::new();
    for d in parse(text) {
        if d.file.is_some() {
            continue;
        }
        let Some(line) = d.line else { continue };
        let Some(line_text) = lines.get(line - 1).copied() else { continue };
        let chars: Vec<char> = line_text.chars().collect();
        let m = d.message.as_str();
        let at = d.col.map(|c| c.saturating_sub(1)).filter(|c| *c < chars.len());
        let mut fix = |start: usize, end: usize, with: String, what: String| out.push(Fix { line, start, end, with, what });

        // a slip: the name written, every place on its line, the nearest
        if let Some(near) = m.split("did you mean '").nth(1).and_then(|r| r.split('\'').next()) {
            for (start, end) in spans_from(line_text, opens.get(line - 1).copied().unwrap_or_default(), m) {
                let written: String = chars[start..end].iter().collect();
                fix(start, end, near.to_string(), format!("{} → {}", written, near));
            }
        }
        // an import the line needs, once, when the message names one way
        if !m.contains("` or `use ")
            && let Some(module) = m.split("add `use ").nth(1).and_then(|r| r.split('`').next())
            && !imports.iter().any(|i| i == module)
        {
            imports.push(module.to_string());
        }
        // another language's word, at the place the parser met it
        let swap = m
            .split("the word '")
            .nth(1)
            .and_then(|r| r.split_once("', not '"))
            .or_else(|| m.split("use '").nth(1).and_then(|r| r.split_once("', not '")))
            .map(|(crow, rest)| (crow.to_string(), rest.split('\'').next().unwrap_or("").to_string()));
        if let (Some((crow, other)), Some(at)) = (swap, at) {
            let wrote: String = chars[at..].iter().take(other.chars().count()).collect();
            if !other.is_empty() && wrote == other {
                let end = at + other.chars().count();
                let spaced = if crow.chars().all(char::is_alphabetic) && chars.get(end).is_some_and(|c| !c.is_whitespace()) { format!("{} ", crow) } else { crow.clone() };
                fix(at, end, spaced, format!("{} → {}", other, crow));
            }
        }
        if let Some(at) = at {
            let here: String = chars[at..].iter().collect();
            // a call written with parentheses: its own pair, on its line
            if m.starts_with("calls use square brackets") && chars[at] == '(' {
                let mut depth = 0i32;
                if let Some(close) = (at..chars.len()).find(|&k| {
                    match chars[k] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                }) {
                    fix(close, close + 1, "]".into(), "( ) → [ ]".into());
                    fix(at, at + 1, "[".into(), "( ) → [ ]".into());
                }
            }
            if m.contains("Crow comments start with ---") && here.starts_with('#') {
                fix(at, at + 1, "---".into(), "# → ---".into());
            }
            if m.contains("Crow's power is ^") && here.starts_with("**") {
                fix(at, at + 2, "^".into(), "** → ^".into());
            }
            if m.contains("Crow has no ';'") && here.starts_with(';') {
                fix(at, at + 1, String::new(), "; removed".into());
            }
            if m.contains("Crow strings use single quotes")
                && here.starts_with('"')
                && let Some(close) = (at + 1..chars.len()).find(|&k| chars[k] == '"')
                && !chars[at + 1..close].contains(&'\'')
            {
                fix(close, close + 1, "'".into(), "\"…\" → '…'".into());
                fix(at, at + 1, "'".into(), "\"…\" → '…'".into());
            }
            // two statements on a line: the second on a line of its own
            if m.contains("a line holds one statement") {
                let indent: String = chars.iter().take_while(|c| c.is_whitespace()).collect();
                let mut start = at;
                while start > 0 && chars[start - 1] == ' ' {
                    start -= 1;
                }
                fix(start, at, format!("\n{}", indent), "one statement a line".into());
            }
        }
        // `x += 1`, `x++`: written out
        let indent: String = chars.iter().take_while(|c| c.is_whitespace()).collect();
        let body = line_text.trim();
        let ops = ["+", "-", "*", "/", "%"];
        if m.contains("augmented assignment")
            && let Some(op) = ops.iter().find(|op| body.contains(&format!("{}=", op)))
            && let Some((name, value)) = body.split_once(&format!("{}=", op))
        {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                fix(0, chars.len(), format!("{}{} = {} {} {}", indent, name, name, op, value.trim()), format!("{} {}= … → {} = {} {} …", name, op, name, name, op));
            }
        }
        if m.contains("Crow has no ++")
            && let Some(name) = body.strip_suffix("++")
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            fix(0, chars.len(), format!("{}{} = {} + 1", indent, name, name), format!("{}++ → {} = {} + 1", name, name, name));
        }
        // a value a statement drops, kept in the variable it was made from
        if let Some(keep) = m.split("keep it: ").nth(1)
            && let Some((var, _)) = keep.split_once(" = ")
            && var != "x"
            && var.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            let at = chars.iter().take_while(|c| c.is_whitespace()).count();
            fix(at, at, format!("{} = ", var), format!("keep: {} = …", var));
        }
    }
    // each import below the file's last `use` (or at its top)
    let below = lines.iter().enumerate().rev().find(|(_, l)| l.starts_with("use ")).map_or(0, |(i, _)| i + 1);
    for module in imports {
        out.push(Fix { line: below + 1, start: 0, end: 0, with: format!("use {}\n", module), what: format!("add `use {}`", module) });
    }
    out
}

/// A source with fixes applied: from the end back, so each fix's place is
/// where it was read; one that overlaps another already applied is left.
pub fn apply(source: &str, fixes: &[Fix]) -> (String, Vec<Fix>) {
    let mut lines: Vec<String> = source.split('\n').map(String::from).collect();
    let mut order: Vec<&Fix> = fixes.iter().collect();
    order.sort_by(|a, b| (b.line, b.start, b.end).cmp(&(a.line, a.start, a.end)));
    let mut done: Vec<Fix> = Vec::new();
    for f in order {
        let overlaps = done.iter().any(|d| d.line == f.line && f.start < d.end.max(d.start + 1) && d.start < f.end.max(f.start + 1) && !(f.start == f.end && d.start == d.end && f.start == d.start) && f.what != d.what);
        if overlaps {
            continue;
        }
        let i = f.line - 1;
        if i > lines.len() {
            continue;
        }
        if f.start == 0 && f.end == 0 && f.with.ends_with('\n') {
            lines.insert(i, f.with.trim_end_matches('\n').to_string());
        } else if let Some(l) = lines.get_mut(i) {
            let chars: Vec<char> = l.chars().collect();
            if f.end > chars.len() {
                continue;
            }
            *l = format!("{}{}{}", chars[..f.start].iter().collect::<String>(), f.with, chars[f.end..].iter().collect::<String>());
        }
        done.push(f.clone());
    }
    done.reverse();
    (lines.join("\n"), done)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixes a refusal names, applied: a slip, an import, a habit
    #[test]
    fn a_refusals_fixes_are_read_and_applied() {
        let src = "use frame\nxs = ['a']\nlog[lenn[xs]]\nif xs == 1\n    log[1]\n";
        let text = "line 3: undefined function 'lenn' — did you mean 'len'?\nline 4, col 7: Crow compares with the word 'eq', not '==': a eq b.\nline 3: abs[] of numbers is math's: add `use math`";
        let fs = fixes(src, text);
        let (fixed, done) = apply(src, &fs);
        assert_eq!(fixed, "use frame\nuse math\nxs = ['a']\nlog[len[xs]]\nif xs eq 1\n    log[1]\n");
        assert_eq!(done.iter().map(|f| f.what.as_str()).collect::<Vec<_>>(), vec!["add `use math`", "lenn → len", "== → eq"]);
        // two ways out is no fix
        assert!(fixes("x = load['a.csv']\n", "line 1: load[] is frame's (a frame) or math's (numbers): add `use frame` or `use math`").is_empty());
        // a call's parentheses, a dropped push, two statements, `x += 1`
        let (f, _) = apply("log[len(xs)]\n", &fixes("log[len(xs)]\n", "line 1, col 8: calls use square brackets: len[…], not len(…)"));
        assert_eq!(f, "log[len[xs]]\n");
        let (f, _) = apply("    push[xs, 1]\n", &fixes("    push[xs, 1]\n", "line 1: push[] answers a list, and this line drops it — keep it: xs = push[xs, …]"));
        assert_eq!(f, "    xs = push[xs, 1]\n");
        // (an index by a mask is named by its form, and keeps what it indexes)
        let (f, _) = apply("df[df.age gt 50]\n", &fixes("df[df.age gt 50]\n", "line 1: `f[mask]` answers a frame, and this line drops it — keep it: df = df[…]"));
        assert_eq!(f, "df = df[df.age gt 50]\n");
        let (f, _) = apply("x = 1 y = 2\n", &fixes("x = 1 y = 2\n", "line 1, col 7: a line holds one statement — 'y' starts a second one: put it on a line of its own"));
        assert_eq!(f, "x = 1\ny = 2\n");
        let (f, _) = apply("  x += 1\n", &fixes("  x += 1\n", "line 1, col 5: Crow has no augmented assignment ('+=') — write it out: x = x + 1."));
        assert_eq!(f, "  x = x + 1\n");
        let (f, _) = apply("if !a\n", &fixes("if !a\n", "line 1, col 4: Crow uses the word 'not', not '!': not a."));
        assert_eq!(f, "if not a\n");
    }

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

    /// A string written across lines: the line it closes on is read from
    /// inside it, its code after the closing quote, and the lines after it
    /// from outside it — an interpolation's code stays code, a comment
    /// after the string goes.
    #[test]
    fn a_line_a_string_closes_on_is_read_from_inside_it() {
        let src = "log[join[['first\nsepp'], sepp]]\nlog['a `b\nc` d']\nx = 1 --- c'\ny = 2\n";
        let opens = openings(src);
        assert_eq!(opens.len(), 6);
        assert_eq!(opens[0], Opening::default());
        assert!(opens[1].in_str && !opens[1].in_interp);
        assert_eq!(opens[2], Opening::default());
        assert!(opens[3].in_str && opens[3].in_interp);
        assert_eq!(opens[4], Opening::default());
        assert_eq!(opens[5], Opening::default());
        assert_eq!(spans_from("sepp'], sepp]]", opens[1], "undefined variable 'sepp'"), vec![(8, 12)]);
        assert_eq!(spans_from("c` d']", opens[3], "undefined variable 'c'"), vec![(0, 1)]);
        assert_eq!(spans("sepp'], sepp]]", "undefined variable 'sepp'"), vec![(0, 4)]);
        let shown = render("ex.co", src, "line 2: undefined variable 'sepp'", &|_| None);
        assert_eq!(shown, "ex.co:2:9: undefined variable 'sepp'\n  2 │ sepp'], sepp]]\n    │         ^^^^");
    }

    #[test]
    fn a_message_without_a_place_still_names_its_file() {
        assert_eq!(render("a.co", "x\n", "Crow booleans are lowercase", &|_| None), "a.co: Crow booleans are lowercase");
        // the producer's column, when the line holds no name the message gives
        assert_eq!(render("a.co", "x = 1;\n", "unexpected character ';' at line 1, col 6", &|_| None), "a.co:1:6: unexpected character ';'\n  1 │ x = 1;\n    │      ^");
    }
}
