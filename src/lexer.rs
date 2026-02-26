use crate::token::{SpannedToken, Token};

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending_tokens: Vec<SpannedToken>,
    at_line_start: bool,
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

            // handle line start indentation
            if self.at_line_start {
                self.handle_indentation(&mut tokens)?;
                self.at_line_start = false;
                continue;
            }

            // skip spaces and tabs (not at line start)
            if ch == ' ' || ch == '\t' {
                self.advance();
                continue;
            }

            // newline
            if ch == '\n' {
                tokens.push(self.make_token(Token::Newline));
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
                tokens.push(tok);
                continue;
            }

            // two-character tokens
            if ch == '.' && self.peek_at(1) == Some('.') {
                let tok = self.make_token(Token::DotDot);
                self.advance();
                self.advance();
                tokens.push(tok);
                continue;
            }

            // single-character tokens
            let tok = match ch {
                '+' => Token::Plus,
                '-' => Token::Minus,
                '*' => Token::Star,
                '/' => Token::Slash,
                '%' => Token::Percent,
                '=' => Token::Assign,
                ',' => Token::Comma,
                '.' => Token::Dot,
                '[' => Token::LBracket,
                ']' => Token::RBracket,
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

    fn skip_comment(&mut self) -> Result<(), String> {
        let start_line = self.line;
        // skip opening ---
        self.pos += 3;
        self.col += 3;

        // find closing ---
        loop {
            if self.pos >= self.source.len() {
                return Err(format!(
                    "unterminated comment starting at line {}",
                    start_line
                ));
            }
            if self.check_str("---") {
                self.pos += 3;
                self.col += 3;
                return Ok(());
            }
            if self.source[self.pos] == '\n' {
                self.line += 1;
                self.col = 0; // will be incremented
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
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "for" => Token::For,
            "in" => Token::In,
            "while" => Token::While,
            "match" => Token::Match,
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
            "_" => Token::Underscore,
            _ => Token::Ident(ident),
        };

        SpannedToken { token, line, col }
    }
}

