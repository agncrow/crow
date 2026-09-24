use crate::token::{SpannedToken, Token};

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
                            "'{}' string prefixes don't exist in Crow (line {}, col {}) — \
                             strings interpolate by default with backticks: 'hello `name`'. \
                             Drop the '{}' prefix.",
                            name, tok.line, tok.col, name
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
                    _ => {} // fall through to single-char '.'
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
                        "unexpected character '{}' at line {}, col {}",
                        ch, self.line, self.col
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
        let at = format!("line {}, col {}", self.line, self.col);
        let msg = match ch {
            '"' => format!(
                "Crow strings use single quotes: 'text', not \"text\" ({at}). \
                 Nested quotes escape as \\'."
            ),
            ':' => format!(
                "Crow has no ':' ({at}) — blocks are defined by indentation, \
                 not colons. Declare functions with fn name[params] and start \
                 the body on the next indented line."
            ),
            '+' | '-' | '*' | '/' | '%' if next == Some('=') => {
                let op = ch;
                format!(
                    "Crow has no augmented assignment ('{op}=') ({at}) — write it \
                     out: x = x {op} 1. (The compiler still optimizes this to an \
                     in-place update.)"
                )
            }
            '=' if next == Some('=') => format!(
                "Crow compares with the word 'eq', not '==' ({at}): a eq b."
            ),
            '!' if next == Some('=') => format!(
                "Crow compares with the word 'ne', not '!=' ({at}): a ne b."
            ),
            '<' if next == Some('=') => format!(
                "Crow uses the word 'lte', not '<=' ({at}): a lte b."
            ),
            '>' if next == Some('=') => format!(
                "Crow uses the word 'gte', not '>=' ({at}): a gte b."
            ),
            '<' => format!("Crow uses the word 'lt', not '<' ({at}): a lt b."),
            '>' => format!("Crow uses the word 'gt', not '>' ({at}): a gt b."),
            '&' if next == Some('&') => {
                format!("Crow uses the word 'and', not '&&' ({at}): a and b.")
            }
            '|' if next == Some('|') => {
                format!("Crow uses the word 'or', not '||' ({at}): a or b.")
            }
            '!' => format!(
                "Crow uses the word 'not', not '!' ({at}): not a. \
                 (And 'ne' for inequality: a ne b.)"
            ),
            _ => return None,
        };
        Some(msg)
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

        // skip comment-only lines
        if self.check_str("---") {
            // this is a comment line, let the main loop handle it
            // but we still need to process indentation for it
        }

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
                        Some('\\') => { s.push('\\'); self.advance(); }
                        Some('\'') => { s.push('\''); self.advance(); }
                        // Escaped backtick: kept as \` in the raw string so
                        // interpolation treats it as a literal character
                        Some('`') => { s.push('\\'); s.push('`'); self.advance(); }
                        Some(ch) => {
                            return Err(format!(
                                "unknown escape sequence '\\{}' at line {}, col {}",
                                ch, self.line, self.col
                            ));
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
                }
            }
        }

        Ok(SpannedToken {
            token: Token::Str(s),
            line,
            col,
        })
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
            Token::Int(num_str.parse::<i64>().map_err(|e| {
                format!("invalid integer '{}' at line {}: {}", num_str, line, e)
            })?)
        };

        Ok(SpannedToken {
            token,
            line,
            col,
        })
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
        assert!(err("g = lambda x: x").contains("no ':'"));
    }

    #[test]
    fn python_string_prefix_teaches_backticks() {
        assert!(err("x = f'hi'").contains("backticks"));
        assert!(err("x = r'raw'").contains("prefix"));
    }

    #[test]
    fn ordinary_code_still_lexes() {
        assert!(Lexer::new("x = 1 + 2\nlog[x]\n").tokenize().is_ok());
        // an identifier that merely starts with f is fine
        assert!(Lexer::new("factor = 3\n").tokenize().is_ok());
    }
}
