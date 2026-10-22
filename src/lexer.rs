use crate::token::{SpannedToken, Token};

/// The words the language keeps for itself — every one lexes as its own
/// token, never a name (a test holds the two to each other). The language
/// server completes them, and a refusal of a name one slip from one says it.
pub const KEYWORDS: &[&str] = &[
    "fn", "view", "derive", "return", "if", "else", "for", "in", "while", "struct", "use", "break", "continue", "true", "false",
    "eq", "ne", "gt", "lt", "gte", "lte", "and", "or", "not", "then",
];

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending_tokens: Vec<SpannedToken>,
    at_line_start: bool,
    bracket_depth: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending_tokens: Vec::new(),
            at_line_start: true,
            bracket_depth: 0,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>, String> {
        let mut tokens = Vec::new();

        loop {
            // drain pending tokens first (indents/dedents)
            while let Some(t) = self.pending_tokens.pop() {
                tokens.push(t);
            }

            if self.pos >= self.source.len() {
                // emit dedents for remaining indentation
                while self.indent_stack.len() > 1 {
                    self.indent_stack.pop();
                    tokens.push(self.make_token(Token::Dedent));
                }
                tokens.push(self.make_token(Token::Eof));
                break;
            }

            let ch = self.source[self.pos];

            // handle line start indentation (suppressed inside brackets)
            if self.at_line_start {
                if self.bracket_depth == 0 {
                    self.handle_indentation(&mut tokens)?;
                } else {
                    // Inside brackets: skip leading whitespace, don't emit indent/dedent
                    while self.pos < self.source.len()
                        && (self.source[self.pos] == ' ' || self.source[self.pos] == '\t')
                    {
                        self.pos += 1;
                        self.col += 1;
                    }
                }
                self.at_line_start = false;
                continue;
            }

            // skip spaces and tabs (not at line start)
            if ch == ' ' || ch == '\t' {
                self.advance();
                continue;
            }

            // newline (suppressed inside brackets)
            if ch == '\n' {
                if self.bracket_depth == 0 {
                    tokens.push(self.make_token(Token::Newline));
                }
                self.advance();
                self.line += 1;
                self.col = 1;
                self.at_line_start = true;
                continue;
            }

            // carriage return
            if ch == '\r' {
                self.advance();
                continue;
            }

            // comments: --- ... ---
            if self.check_str("---") {
                self.skip_comment()?;
                continue;
            }

            // string literal
            if ch == '\'' {
                let tok = self.lex_string()?;
                tokens.push(tok);
                continue;
            }

            // number
            if ch.is_ascii_digit() {
                let tok = self.lex_number()?;
                tokens.push(tok);
                continue;
            }

            // identifier or keyword
            if ch.is_ascii_alphabetic() || ch == '_' {
                let tok = self.lex_ident();
                // Python string prefix: f'...', r'...', b'... — a name abutting
                // a quote is never valid Crow, and it's the LLM interpolation reflex.
                if let Token::Ident(name) = &tok.token {
                    if self.peek() == Some('\'')
                        && matches!(name.as_str(), "f" | "r" | "b" | "rb" | "br" | "fr" | "rf")
                    {
                        return Err(format!(
                            "line {}, col {}: '{}' string prefixes don't exist in Crow — \
                             strings interpolate by default with backticks: 'hello `name`'. \
                             Drop the '{}' prefix.",
                            tok.line, tok.col, name, name
                        ));
                    }
                }
                tokens.push(tok);
                continue;
            }

            // Python-ism detection: characters and operator spellings that are
            // never valid Crow but are common reflexes from Python/JS. Caught
            // here so the message can teach the Crow form instead of a bare
            // "unexpected character".
            if let Some(msg) = self.python_ism_at(ch) {
                return Err(msg);
            }

            // two-character tokens
            if ch == '.' {
                match self.peek_at(1) {
                    Some('.') => {
                        let tok = self.make_token(Token::DotDot);
                        self.advance(); self.advance();
                        tokens.push(tok); continue;
                    }
                    Some('*') => {
                        let tok = self.make_token(Token::DotStar);
                        self.advance(); self.advance();
                        tokens.push(tok); continue;
                    }
                    Some('/') => {
                        let tok = self.make_token(Token::DotSlash);
                        self.advance(); self.advance();
                        tokens.push(tok); continue;
                    }
                    Some('^') => {
                        let tok = self.make_token(Token::DotCaret);
                        self.advance(); self.advance();
                        tokens.push(tok); continue;
                    }
                    _ => {
                        // a column's name with a computed piece — `df.lag_`k``,
                        // `df.`name`` — after a dot, a name reads as text does,
                        // a backtick inside it an interpolation (spec, Frames):
                        // one name token holding the raw pieces, which the
                        // lowering interpolates as it does a string's
                        if let Some(raw) = self.column_name_ahead()? {
                            let dot = self.make_token(Token::Dot);
                            self.advance();
                            tokens.push(dot);
                            let tok = self.make_token(Token::Ident(raw.clone()));
                            for _ in raw.chars() {
                                self.advance();
                            }
                            tokens.push(tok);
                            continue;
                        }
                        // (a plain name: the single-char '.' below)
                    }
                }
            }

            // single-character tokens
            let tok = match ch {
                '+' => Token::Plus,
                '-' => Token::Minus,
                '*' => Token::Star,
                '/' => Token::Slash,
                '%' => Token::Percent,
                '^' => Token::Caret,
                '=' => Token::Assign,
                ',' => Token::Comma,
                '.' => Token::Dot,
                '[' => {
                    self.bracket_depth += 1;
                    Token::LBracket
                }
                ']' => {
                    if self.bracket_depth > 0 {
                        self.bracket_depth -= 1;
                    }
                    Token::RBracket
                }
                '(' => Token::LParen,
                ')' => Token::RParen,
                _ => {
                    return Err(format!(
                        "line {}, col {}: '{}' is not a character Crow writes code with",
                        self.line, self.col, ch
                    ))
                }
            };
            let spanned = self.make_token(tok);
            self.advance();
            tokens.push(spanned);
        }

        Ok(tokens)
    }

    fn make_token(&self, token: Token) -> SpannedToken {
        SpannedToken {
            token,
            line: self.line,
            col: self.col,
        }
    }

    fn advance(&mut self) -> Option<char> {
        if self.pos < self.source.len() {
            let ch = self.source[self.pos];
            self.pos += 1;
            self.col += 1;
            Some(ch)
        } else {
            None
        }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.source.get(self.pos + offset).copied()
    }

    /// Recognize Python/JS syntax that has no Crow equivalent and return a
    /// message teaching the Crow form. Returns None for characters that aren't
    /// a known Python-ism (they fall through to normal lexing / the generic
    /// "unexpected character" error).
    fn python_ism_at(&self, ch: char) -> Option<String> {
        let next = self.peek_at(1);
        let at = format!("line {}, col {}:", self.line, self.col);
        let msg = match ch {
            '"' => format!(
                "{at} Crow strings use single quotes: 'text', not \"text\". \
                 Nested quotes escape as \\'."
            ),
            ':' => format!("{at} {}", self.colon_said()),
            '{' | '}' => format!(
                "{at} Crow has no braces — a map is map[key = value], and a block \
                 is the lines indented under its first line"
            ),
            '#' => format!("{at} Crow comments start with ---, not #"),
            ';' => format!("{at} a statement ends with its line — Crow has no ';'"),
            '+' if next == Some('+') => format!("{at} Crow has no ++: write x = x + 1"),
            '*' if next == Some('*') => format!("{at} Crow's power is ^: x ^ 2, not x ** 2"),
            '+' | '-' | '*' | '/' | '%' if next == Some('=') => {
                let op = ch;
                format!(
                    "{at} Crow has no augmented assignment ('{op}=') — write it \
                     out: x = x {op} 1. (The compiler still optimizes this to an \
                     in-place update.)"
                )
            }
            '=' if next == Some('=') => format!(
                "{at} Crow compares with the word 'eq', not '==': a eq b."
            ),
            '!' if next == Some('=') => format!(
                "{at} Crow compares with the word 'ne', not '!=': a ne b."
            ),
            '<' if next == Some('=') => format!(
                "{at} Crow uses the word 'lte', not '<=': a lte b."
            ),
            '>' if next == Some('=') => format!(
                "{at} Crow uses the word 'gte', not '>=': a gte b."
            ),
            '<' => format!("{at} Crow uses the word 'lt', not '<': a lt b."),
            '>' => format!("{at} Crow uses the word 'gt', not '>': a gt b."),
            '&' if next == Some('&') => {
                format!("{at} Crow uses the word 'and', not '&&': a and b.")
            }
            '|' if next == Some('|') => {
                format!("{at} Crow uses the word 'or', not '||': a or b.")
            }
            '!' => format!(
                "{at} Crow uses the word 'not', not '!': not a. \
                 (And 'ne' for inequality: a ne b.)"
            ),
            _ => return None,
        };
        Some(msg)
    }

    /// A colon, by what its line is: Python's block header, a slice, a
    /// dictionary, a lambda, a function's `def`.
    fn colon_said(&self) -> String {
        let start = self.source[..self.pos].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        let before: String = self.source[start..self.pos].iter().collect();
        let first = before.split_whitespace().next().unwrap_or("");
        let parens = if before.contains('(') { " — and calls use square brackets: range[3], not range(3)" } else { "" };
        if before.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w == "lambda") {
            return "Crow has no lambdas — define a named function with fn name[params] and pass its name (see Design Note: No Lambdas)".to_string();
        }
        match first {
            "def" => "Crow declares functions with fn name[params], not def — its body is the lines indented under it, no ':'".to_string(),
            "lambda" => "Crow has no lambdas — define a named function with fn name[params] and pass its name (see Design Note: No Lambdas)".to_string(),
            "for" | "if" | "else" | "elif" | "while" | "fn" | "view" | "test" | "struct" => {
                format!("a block's body is the lines indented under its first line — no ':' after `{}`{}", before.trim(), parens)
            }
            _ if self.bracket_depth > 0 => "Crow has no ':' in an index — a slice is slice[xs, start, end]".to_string(),
            _ if before.contains('{') => "Crow has no '{…}' maps — a map is map[key = value]".to_string(),
            _ => "Crow has no ':' — a block is the lines indented under its first line, and a map is map[key = value]".to_string(),
        }
    }

    fn check_str(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if self.peek_at(i) != Some(ch) {
                return false;
            }
        }
        true
    }

    fn handle_indentation(&mut self, tokens: &mut Vec<SpannedToken>) -> Result<(), String> {
        let indent = loop {
            let mut indent = 0;

            // skip blank lines entirely
            while self.pos < self.source.len() && self.source[self.pos] == '\n' {
                self.pos += 1;
                self.line += 1;
                self.col = 1;
            }

            // count spaces at start of line
            while self.pos < self.source.len() && self.source[self.pos] == ' ' {
                indent += 1;
                self.pos += 1;
                self.col += 1;
            }

            // skip tabs (convert to spaces: 1 tab = 4 spaces)
            while self.pos < self.source.len() && self.source[self.pos] == '\t' {
                indent += 4;
                self.pos += 1;
                self.col += 1;
            }

            // if we hit end of file or another blank line, don't emit indent/dedent
            if self.pos >= self.source.len() {
                return Ok(());
            }
            if self.source[self.pos] == '\n' {
                return Ok(());
            }

            // a line that holds only a comment opens and closes no block,
            // whatever its indentation: it is passed over whole, as a blank
            // line is (spec, Blocks) — a comment continued under a line's
            // trailing one, aligned with it, had been "indented under a
            // line that opens no block", and one at the margin inside a
            // block had ended the block
            if self.check_str("---") {
                self.skip_comment()?;
                if self.pos >= self.source.len() {
                    return Ok(());
                }
                self.pos += 1; // the comment's line break
                self.line += 1;
                self.col = 1;
                continue;
            }
            break indent;
        };

        let current = *self.indent_stack.last().unwrap();

        if indent > current {
            self.indent_stack.push(indent);
            tokens.push(SpannedToken {
                token: Token::Indent,
                line: self.line,
                col: 1,
            });
        } else if indent < current {
            while *self.indent_stack.last().unwrap() > indent {
                self.indent_stack.pop();
                tokens.push(SpannedToken {
                    token: Token::Dedent,
                    line: self.line,
                    col: 1,
                });
            }
            if *self.indent_stack.last().unwrap() != indent {
                return Err(format!(
                    "inconsistent indentation at line {}",
                    self.line
                ));
            }
        }

        Ok(())
    }

    /// Comments run from `---` to the end of the line — no closing delimiter,
    /// so an unclosed comment can never swallow the next line into code
    /// (spec Design Note "Comments End at the Newline"). The newline itself is
    /// left for the main loop to tokenize.
    fn skip_comment(&mut self) -> Result<(), String> {
        // skip opening ---
        self.pos += 3;
        self.col += 3;

        loop {
            if self.pos >= self.source.len() {
                return Ok(());
            }
            if self.source[self.pos] == '\n' {
                return Ok(());
            }
            self.pos += 1;
            self.col += 1;
        }
    }

    fn lex_string(&mut self) -> Result<SpannedToken, String> {
        let line = self.line;
        let col = self.col;
        self.advance(); // skip opening '

        let mut s = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(format!(
                        "unterminated string at line {}, col {}",
                        line, col
                    ))
                }
                Some('\'') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance(); // skip backslash
                    match self.peek() {
                        Some('n') => { s.push('\n'); self.advance(); }
                        Some('t') => { s.push('\t'); self.advance(); }
                        Some('r') => { s.push('\r'); self.advance(); }
                        Some('\\') => { s.push('\\'); self.advance(); }
                        Some('\'') => { s.push('\''); self.advance(); }
                        // Escaped backtick: kept as \` in the raw string so
                        // interpolation treats it as a literal character
                        Some('`') => { s.push('\\'); s.push('`'); self.advance(); }
                        // any other character after a backslash is itself, the
                        // backslash kept: a regular expression's `\d`, `\.` and
                        // `\|` are written as they are (strings are inert)
                        Some(ch) => {
                            s.push('\\');
                            s.push(ch);
                            self.advance();
                            self.held_line_break(ch);
                        }
                        None => {
                            return Err(format!(
                                "unterminated string at line {}, col {}",
                                line, col
                            ));
                        }
                    }
                }
                Some(ch) => {
                    s.push(ch);
                    self.advance();
                    self.held_line_break(ch);
                }
            }
        }

        Ok(SpannedToken {
            token: Token::Str(s),
            line,
            col,
        })
    }

    /// A line break a string holds as written is a line of the source: the
    /// lines after the string are counted on, and its next line's columns
    /// from its start (a stop after a three-line string had named a line
    /// two short, in both modes).
    fn held_line_break(&mut self, ch: char) {
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        }
    }

    fn lex_number(&mut self) -> Result<SpannedToken, String> {
        let line = self.line;
        let col = self.col;
        let mut num_str = String::new();
        let mut is_float = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                num_str.push(ch);
                self.advance();
            } else if ch == '.' && !is_float {
                // check if next char is a digit (not ..)
                if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                    is_float = true;
                    num_str.push(ch);
                    self.advance();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let token = if is_float {
            Token::Float(num_str.parse::<f64>().map_err(|e| {
                format!("invalid float '{}' at line {}: {}", num_str, line, e)
            })?)
        } else {
            // (digits past 64 bits are the int they write: an int grows —
            // they had been refused in Rust's words, "number too large to
            // fit in target type")
            match num_str.parse::<i64>() {
                Ok(n) => Token::Int(n),
                Err(_) => Token::Whole(num_str),
            }
        };

        Ok(SpannedToken {
            token,
            line,
            col,
        })
    }

    /// The column name after a dot when it holds an interpolation piece
    /// (`lag_`k``, `` `name` ``): its raw text, backticks kept, as a string
    /// keeps its own; None where the dot is followed by a plain name or by
    /// anything else. A piece is read as a string's is: to the closing
    /// backtick outside a quoted text, a backslash escaping the next
    /// character inside one.
    fn column_name_ahead(&self) -> Result<Option<String>, String> {
        let mut i = 1;
        let mut raw = String::new();
        let mut pieces = 0;
        loop {
            match self.peek_at(i) {
                Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                    raw.push(c);
                    i += 1;
                }
                Some('`') => {
                    raw.push('`');
                    i += 1;
                    let mut in_text = false;
                    loop {
                        match self.peek_at(i) {
                            None | Some('\n') => {
                                return Err(format!(
                                    "line {}, col {}: a column's name holds an interpolation that is not closed — close it with `",
                                    self.line, self.col
                                ));
                            }
                            Some('\\') if in_text => {
                                raw.push('\\');
                                i += 1;
                                if let Some(n) = self.peek_at(i) {
                                    raw.push(n);
                                    i += 1;
                                }
                            }
                            Some('\'') => {
                                in_text = !in_text;
                                raw.push('\'');
                                i += 1;
                            }
                            Some('`') if !in_text => {
                                raw.push('`');
                                i += 1;
                                break;
                            }
                            Some(c) => {
                                raw.push(c);
                                i += 1;
                            }
                        }
                    }
                    pieces += 1;
                }
                _ => break,
            }
        }
        Ok(if pieces == 0 { None } else { Some(raw) })
    }

    fn lex_ident(&mut self) -> SpannedToken {
        let line = self.line;
        let col = self.col;
        let mut ident = String::new();

        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ident.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        let token = match ident.as_str() {
            "fn" => Token::Fn,
            "view" => Token::View,
            "derive" => Token::Derive,
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "for" => Token::For,
            "in" => Token::In,
            "while" => Token::While,
            "struct" => Token::Struct,
            "use" => Token::Use,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "true" => Token::True,
            "false" => Token::False,
            "eq" => Token::Eq,
            "ne" => Token::Ne,
            "gt" => Token::Gt,
            "lt" => Token::Lt,
            "gte" => Token::Gte,
            "lte" => Token::Lte,
            "and" => Token::And,
            "or" => Token::Or,
            "not" => Token::Not,
            "then" => Token::Then,
            "_" => Token::Underscore,
            _ => Token::Ident(ident),
        };

        SpannedToken { token, line, col }
    }
}


#[cfg(test)]
mod keyword_tests {
    /// Every keyword the list names lexes as a keyword, not a name.
    #[test]
    fn every_keyword_is_one() {
        for k in super::KEYWORDS {
            let tokens = super::Lexer::new(k).tokenize().unwrap();
            assert!(!matches!(tokens[0].token, crate::token::Token::Ident(_)), "{} lexes as a name", k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(src: &str) -> String {
        Lexer::new(src).tokenize().err().unwrap_or_default()
    }

    #[test]
    fn double_quote_teaches_single_quotes() {
        assert!(err("x = \"hi\"").contains("single quotes"));
    }

    #[test]
    fn augmented_assign_teaches_explicit_form() {
        assert!(err("x += 1").contains("x = x + 1"));
        assert!(err("x -= 1").contains("augmented assignment"));
    }

    #[test]
    fn symbol_operators_teach_word_operators() {
        assert!(err("a == b").contains("'eq'"));
        assert!(err("a != b").contains("'ne'"));
        assert!(err("a < b").contains("'lt'"));
        assert!(err("a >= b").contains("'gte'"));
        assert!(err("a && b").contains("'and'"));
        assert!(err("a || b").contains("'or'"));
        assert!(err("not a\nb = !c").contains("'not'"));
    }

    #[test]
    fn colon_teaches_indentation() {
        assert!(err("g = lambda x: x").contains("Crow has no lambdas"));
        assert!(err("for i in range[3]:").contains("no ':' after `for i in range[3]`"));
        assert!(err("x = xs[1:3]").contains("slice[xs, start, end]"));
    }

    #[test]
    fn python_string_prefix_teaches_backticks() {
        assert!(err("x = f'hi'").contains("backticks"));
        assert!(err("x = r'raw'").contains("prefix"));
    }

    /// A string written across lines holds its line breaks, and the tokens
    /// after it are on the lines they are on: a stop and a refusal after a
    /// three-line string had named a line two short.
    #[test]
    fn the_lines_after_a_string_of_several_lines_are_counted() {
        let tokens = Lexer::new("s = 'one\ntwo\nthree' + x\nxs = [1]\nlog[xs[5]]\n").tokenize().expect("lexes");
        let at = |name: &str| tokens.iter().find(|t| t.token == Token::Ident(name.to_string())).map(|t| (t.line, t.col));
        assert_eq!(at("x"), Some((3, 10)));
        assert_eq!(at("xs"), Some((4, 1)));
        assert_eq!(at("log"), Some((5, 1)));
        // (an escaped character before the break, and a break escaped)
        let tokens = Lexer::new("s = 'a\\t\nb\\\nc'\ny = 1\n").tokenize().expect("lexes");
        assert_eq!(tokens.iter().find(|t| t.token == Token::Ident("y".into())).map(|t| t.line), Some(4));
    }

    /// A line that holds only a comment opens and closes no block, whatever
    /// its indentation: it lexes to nothing, as a blank line does (spec,
    /// Blocks). A comment continued under a trailing one, aligned with it,
    /// had been a line indented under one that opens no block; one at the
    /// margin inside a block had ended the block; one before a chain's first
    /// step had left the step a line indented under nothing.
    #[test]
    fn a_comment_alone_on_its_line_is_no_line_of_a_block() {
        let tokens = |src: &str| -> Vec<Token> { Lexer::new(src).tokenize().expect("lexes").into_iter().map(|t| t.token).collect() };
        let pairs = [
            (
                "fn f[x]\n    return x + 1      --- a comment that\n                      --- goes on\nlog[f[1]]\n",
                "fn f[x]\n    return x + 1\nlog[f[1]]\n",
            ),
            (
                "fn f[x]\n--- the body\n    y = x + 1\n--- at the margin\n        --- deeper\n  --- at no block's\n    return y\n",
                "fn f[x]\n    y = x + 1\n    return y\n",
            ),
            ("x = s\n--- then the steps\n    then trim\n        --- one more\n    then upper\n", "x = s\n    then trim\n    then upper\n"),
            ("if a\n    b = 1\n\t--- a tab's\n  --- between the branches\nelse\n    b = 2\n", "if a\n    b = 1\nelse\n    b = 2\n"),
            // the last line, with no line break after it
            ("x = 1\n    --- the end", "x = 1\n"),
        ];
        for (with, without) in pairs {
            assert_eq!(tokens(with), tokens(without), "{:?}", with);
        }
        // the lines after the comments are counted on
        let after = Lexer::new("--- one\n    --- two\nx = 1\n").tokenize().expect("lexes");
        assert_eq!((after[0].token.clone(), after[0].line, after[0].col), (Token::Ident("x".into()), 3, 1));
        // a line of code indented under a comment is still under nothing
        assert_eq!(tokens("x = 1\n--- a comment\n    y = 2\n"), tokens("x = 1\n    y = 2\n"));
    }

    #[test]
    fn ordinary_code_still_lexes() {
        assert!(Lexer::new("x = 1 + 2\nlog[x]\n").tokenize().is_ok());
        // an identifier that merely starts with f is fine
        assert!(Lexer::new("factor = 3\n").tokenize().is_ok());
    }

    /// A column's name after a dot with a backtick piece is one name token
    /// holding the raw pieces (spec, Frames): `lag_`k``, `` `name` ``, a
    /// quoted text inside a piece with its own backtick kept; a plain name
    /// after a dot lexes as it did, and a piece left open is refused.
    #[test]
    fn a_column_name_with_a_piece_is_one_token() {
        let names = |src: &str| -> Vec<String> {
            Lexer::new(src).tokenize().expect("lexes").into_iter().filter_map(|t| if let Token::Ident(n) = t.token { Some(n) } else { None }).collect()
        };
        assert_eq!(names("df.lag_`k` = v\n"), vec!["df", "lag_`k`", "v"]);
        assert_eq!(names("log[df.`name`[3]]\n"), vec!["log", "df", "`name`"]);
        assert_eq!(names("df.`'in px'` = v\n"), vec!["df", "`'in px'`", "v"]);
        assert_eq!(names("df.`'a`b'`_x = v\n"), vec!["df", "`'a`b'`_x", "v"]);
        assert_eq!(names("df.qty\n"), vec!["df", "qty"]);
        let e = Lexer::new("df.lag_`k = v\n").tokenize().expect_err("an open piece");
        assert!(e.contains("not closed"), "{}", e);
    }
}
