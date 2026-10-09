use crate::ast::*;
use crate::token::{SpannedToken, Token};

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
    /// The statement before this one, when it was a name alone on its line
    /// (`esle`), with its place: a line indented under it, or a second
    /// statement after it, may be a keyword's slip.
    lone: Option<(String, usize, usize)>,
    /// The loops the statement is inside, innermost last: `true` for a
    /// `while`. `break` and `continue` are a while loop's alone.
    loops: Vec<bool>,
    /// How many bodies a `return` may leave the statement is inside (a
    /// function's, a view's, a derivation's).
    bodies: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0, lone: None, loops: Vec::new(), bodies: 0 }
    }

    pub fn parse(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.is_at_end() {
            let line = self.current_line();
            stmts.push(Stmt::Line(line));
            let start = self.pos;
            let parsed = self.parse_stmt()?;
            self.end_statement(start, &parsed)?;
            stmts.extend(parsed);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    /// Error-tolerant parse for the language server: a statement that fails
    /// to parse is recorded as (line, message) and skipped, and parsing
    /// continues with the next statement. The compiler paths never use this —
    /// `parse()` stays strict, so build/run semantics are untouched.
    pub fn parse_lenient(&mut self) -> (Vec<Stmt>, Vec<(usize, String)>) {
        let mut stmts = Vec::new();
        let mut errors = Vec::new();
        self.skip_newlines();
        while !self.is_at_end() {
            let line = self.current_line();
            let checkpoint = self.pos;
            match self.parse_stmt().and_then(|parsed| self.end_statement(checkpoint, &parsed).map(|_| parsed)) {
                Ok(parsed) => {
                    stmts.push(Stmt::Line(line));
                    stmts.extend(parsed);
                }
                Err(e) => {
                    // a bracket never closed: the lexer read the rest of the
                    // file as part of it, with no lines or blocks — nothing
                    // after it parses as what the program wrote
                    let unclosed = e.contains("is never closed");
                    errors.push((line, e));
                    if unclosed {
                        break;
                    }
                    // If parse_stmt consumed nothing, force progress.
                    if self.pos == checkpoint {
                        self.pos += 1;
                    }
                    // an error inside one of the statement's blocks: the
                    // rest of the statement goes with it, its blocks too,
                    // so their lines are not read as top-level statements
                    // (a function's `return` had been "outside a function")
                    let mut depth: i64 = self.tokens[checkpoint..self.pos.min(self.tokens.len())]
                        .iter()
                        .map(|t| match t.token {
                            Token::Indent => 1,
                            Token::Dedent => -1,
                            _ => 0,
                        })
                        .sum();
                    if depth > 0 {
                        while depth > 0 && !self.is_at_end() {
                            match self.peek() {
                                Token::Indent => depth += 1,
                                Token::Dedent => depth -= 1,
                                _ => {}
                            }
                            self.pos += 1;
                        }
                    } else {
                        self.sync_to_statement_end();
                    }
                    // (a block the broken statement left ends here: its end
                    // is no statement of its own)
                    while matches!(self.peek(), Token::Dedent | Token::Newline) {
                        self.pos += 1;
                    }
                }
            }
            self.skip_newlines();
        }
        (stmts, errors)
    }

    /// Skip past the broken statement. Newline tokens are unreliable here —
    /// an unclosed `[` suppresses them (bracket continuation), turning the
    /// rest of the file into one logical line — so the sync walks by
    /// *physical* token lines instead: consume everything on the error's
    /// line, then any indented block that belonged to the broken header.
    fn sync_to_statement_end(&mut self) {
        let start_line = self
            .tokens
            .get(self.pos.saturating_sub(1))
            .map(|t| t.line)
            .unwrap_or(0);
        while !self.is_at_end() {
            let t = &self.tokens[self.pos];
            let is_layout = matches!(
                t.token,
                crate::token::Token::Newline
                    | crate::token::Token::Indent
                    | crate::token::Token::Dedent
            );
            if t.line > start_line && !is_layout {
                break;
            }
            if t.line > start_line && matches!(t.token, crate::token::Token::Indent) {
                break; // indented body of the broken header — handled below
            }
            self.pos += 1;
        }
        // A broken block header owns the indented body that follows; consume
        // balanced Indent..Dedent groups so body statements don't reparse at
        // the wrong nesting level.
        while matches!(self.peek_token(), Some(crate::token::Token::Indent)) {
            let mut depth = 0i32;
            while !self.is_at_end() {
                match self.peek_token() {
                    Some(crate::token::Token::Indent) => {
                        depth += 1;
                        self.pos += 1;
                    }
                    Some(crate::token::Token::Dedent) => {
                        depth -= 1;
                        self.pos += 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => self.pos += 1,
                }
            }
        }
    }

    fn peek_token(&self) -> Option<&crate::token::Token> {
        self.tokens.get(self.pos).map(|t| &t.token)
    }

    // --- Helpers ---

    fn peek(&self) -> &Token {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].token
        } else {
            &Token::Eof
        }
    }

    fn peek_ahead(&self, n: usize) -> &Token {
        if self.pos + n < self.tokens.len() {
            &self.tokens[self.pos + n].token
        } else {
            &Token::Eof
        }
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos].token;
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.unexpected(Some(expected)))
        }
    }

    /// Where the parser is, as a refusal begins: `line N, col M: `.
    fn at(&self) -> String {
        let t = self.tokens.get(self.pos).or_else(|| self.tokens.last());
        match t {
            Some(t) => format!("line {}, col {}: ", t.line, t.col),
            None => String::new(),
        }
    }

    /// The bracket a closing one would close, where it opened.
    fn opener(&self, close: &Token) -> Option<&SpannedToken> {
        let open = match close {
            Token::RBracket => Token::LBracket,
            Token::RParen => Token::LParen,
            _ => return None,
        };
        let mut depth = 0usize;
        for t in self.tokens[..self.pos.min(self.tokens.len())].iter().rev() {
            if t.token == *close {
                depth += 1;
            } else if t.token == open {
                if depth == 0 {
                    return Some(t);
                }
                depth -= 1;
            }
        }
        None
    }

    /// What the parser met where it could not go on, and the form the
    /// program meant, when the place says it: a call written with `(`, a
    /// bracket never closed, a body not indented, a comprehension, Python's
    /// `**` and `++` (spec, Error Messages). `expected` is the token the
    /// grammar wanted, when it wanted one.
    fn unexpected(&self, expected: Option<&Token>) -> String {
        let got = self.peek();
        let back = |n: usize| self.pos.checked_sub(n).and_then(|i| self.tokens.get(i)).map(|t| &t.token);
        let before = back(1);
        if let (Token::LParen, Some(Token::Ident(name))) = (got, before) {
            // a value's method, as Python writes it: the value goes first
            if let (Some(Token::Dot), Some(Token::Ident(of))) = (back(2), back(3)) {
                if name == "append" {
                    return format!("{}a list grows with push, which answers the longer list: {} = push[{}, …]", self.at(), of, of);
                }
                return format!("{}{}.{}(…) is {}[{}, …]: a function takes its value first, in square brackets", self.at(), of, name, name, of);
            }
            return format!("{}calls use square brackets: {}[…], not {}(…)", self.at(), name, name);
        }
        if matches!(got, Token::RBracket | Token::RParen) && self.opener(got).is_none() {
            return format!("{}this {} closes nothing — no bracket is open before it", self.at(), got.said());
        }
        if let Some(close @ (Token::RBracket | Token::RParen)) = expected {
            let open = if matches!(close, Token::RBracket) { '[' } else { '(' };
            let shut = if matches!(close, Token::RBracket) { ']' } else { ')' };
            if let Some(o) = self.opener(close)
                && (matches!(got, Token::Newline | Token::Eof | Token::Dedent | Token::Indent) || self.current_line() > o.line)
            {
                return format!("line {}, col {}: this '{}' is never closed — close it with '{}'; the lines after it are read as part of it until it is", o.line, o.col, open, shut);
            }
            if matches!(got, Token::For) {
                return format!("{}Crow collects with a for block, not inside brackets: ys = for x in xs, its item's expression indented under it", self.at());
            }
        }
        if let (Some(Token::Indent), _) = (expected, got) {
            let opened = before.map(|_| self.tokens[self.pos.saturating_sub(1)].line).unwrap_or(0);
            return format!("{}the block opened on line {} has no indented body — indent its lines under it", self.at(), opened.max(1));
        }
        if let (Some(Token::LBracket), Token::Newline | Token::Eof, Some(Token::Ident(name))) = (expected, got, before) {
            let head = match back(2) {
                Some(k @ (Token::Fn | Token::View | Token::Derive)) => format!("{} {}", k, name),
                _ => name.clone(),
            };
            return format!("{}{} takes its parameters in brackets: {}[] or {}[x]", self.at(), head, head, head);
        }
        match (got, before) {
            (Token::Star, Some(Token::Star)) => return format!("{}Crow's power is ^: x ^ 2, not x ** 2", self.at()),
            (Token::Plus, Some(Token::Plus)) => return format!("{}Crow has no ++: write x = x + 1", self.at()),
            (Token::RBracket | Token::RParen, Some(Token::Comma)) => return format!("{}a value is missing after the ','", self.at()),
            (Token::Else, _) if expected.is_none() => return format!("{}else follows an if's block, at the if's indentation", self.at()),
            (Token::Indent, _) => return format!("{}this line is indented, but the line above it opens no block", self.at()),
            _ => {}
        }
        match expected {
            Some(e) => format!("{}expected {}, got {}", self.at(), e.said(), got.said()),
            None if matches!(got, Token::Newline | Token::Eof | Token::Dedent) => format!("{}a value is missing before {}", self.at(), got.said()),
            None => format!("{}{} cannot start a value here", self.at(), got.said()),
        }
    }

    /// A statement ends with its line. A second one on the line is refused
    /// (`x = 1 y = 2` had run as two), and a name alone on its line one slip
    /// from a keyword says which (`retrun x`, `esle`).
    fn end_statement(&mut self, start: usize, parsed: &[Stmt]) -> Result<(), String> {
        let layout = |t: &Token| matches!(t, Token::Newline | Token::Indent | Token::Dedent | Token::Eof);
        let last = self.tokens[start.min(self.pos)..self.pos].iter().rev().find(|t| !layout(&t.token));
        let lone = match parsed.last() {
            Some(Stmt::ExprStmt(Expr::Ident(n))) => last.map(|t| (n.clone(), t.line, t.col)),
            _ => None,
        };
        if let (Some(last), Some(next)) = (last, self.tokens.get(self.pos)) {
            if next.line == last.line && !layout(&next.token) {
                if let Some((n, line, col)) = &lone
                    && let Some(k) = keyword_near(n)
                {
                    return Err(format!("line {}, col {}: '{}' is not a word Crow knows — did you mean '{}'?", line, col, n, k));
                }
                if matches!(next.token, Token::LParen | Token::RBracket | Token::RParen) {
                    return Err(self.unexpected(None));
                }
                return Err(format!(
                    "line {}, col {}: a line holds one statement — {} starts a second one: put it on a line of its own",
                    next.line,
                    next.col,
                    next.token.said()
                ));
            }
        }
        self.lone = lone;
        Ok(())
    }

    fn current_line(&self) -> usize {
        if self.pos < self.tokens.len() {
            self.tokens[self.pos].line
        } else {
            0
        }
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }

    fn check(&self, token: &Token) -> bool {
        self.peek() == token
    }

    // --- Statement parsing ---

    fn parse_stmt(&mut self) -> Result<Vec<Stmt>, String> {
        // a line indented under one that opens no block: a keyword's slip on
        // the line above (`esle`) says which keyword
        if matches!(self.peek(), Token::Indent) {
            if let Some((n, line, col)) = &self.lone
                && let Some(k) = keyword_near(n)
            {
                return Err(format!("line {}, col {}: '{}' is not a word Crow knows — did you mean '{}'?", line, col, n, k));
            }
            return Err(self.unexpected(None));
        }
        // Python/JS keyword reflexes in statement position — teach the Crow
        // form rather than letting them fail as stray identifiers. Skip when the
        // name is being assigned to (`from = x`, `print = 5`) — then it's just a
        // variable that happens to share a Python keyword's spelling.
        if let Token::Ident(name) = self.peek() {
            if !matches!(self.peek_ahead(1), Token::Assign) {
                if let Some(hint) = python_keyword_hint(name) {
                    return Err(format!("{}{}", self.at(), hint));
                }
            }
        }
        // `test` blocks: reserved word, recognized in statement position.
        if let Token::Ident(name) = self.peek() {
            if name == "test"
                && matches!(
                    self.peek_ahead(1),
                    Token::Ident(_) | Token::LBracket | Token::Newline
                )
            {
                return self.parse_test_block().map(|s| vec![s]);
            }
        }
        // `then` opens a step of a chain: it continues an assignment's value
        // from the line above (spec, Then Chaining). Alone it is a stray line.
        if matches!(self.peek(), Token::Then) {
            return Err(format!(
                "line {}: `then` continues an assignment — write `x = …` on the line above, with each `then` step indented under it",
                self.current_line()
            ));
        }
        match self.peek().clone() {
            Token::Fn => self.parse_fn_decl(false).map(|s| vec![s]),
            Token::View => self.parse_fn_decl(true).map(|s| vec![s]),
            Token::Derive => self.parse_derive().map(|s| vec![s]),
            Token::Return => self.parse_return().map(|s| vec![s]),
            Token::Break | Token::Continue => {
                // a while loop's: a for loop's rounds run at once, in no
                // order, so there is no round after this one to skip to and
                // none before it that stopping would keep (spec, Design
                // Note: break and continue — Vijay, 2026-10-08: "for is
                // parallel, while is for sequential"; it had run them, the
                // loop quietly made sequential)
                let word = if matches!(self.peek(), Token::Break) { "break" } else { "continue" };
                match self.loops.last() {
                    None => return Err(format!("{}{} is a while loop's, and this line is in no loop", self.at(), word)),
                    Some(false) if word == "break" => {
                        return Err(format!(
                            "{}break stops a while loop — a for loop's rounds run at once, in no order: loop with while to stop early, or search with find[…], any[…] or all[…]",
                            self.at()
                        ))
                    }
                    Some(false) => {
                        return Err(format!(
                            "{}continue skips to a while loop's next round — a for loop's rounds run at once, in no order: skip an item with if, its lines indented under it",
                            self.at()
                        ))
                    }
                    Some(true) => {}
                }
                let stmt = if word == "break" { Stmt::Break } else { Stmt::Continue };
                self.advance();
                Ok(vec![stmt])
            }
            Token::If => self.parse_if().map(|s| vec![s]),
            Token::For => self.parse_for_stmt().map(|s| vec![s]),
            Token::While => self.parse_while().map(|s| vec![s]),
            Token::Struct => self.parse_struct().map(|s| vec![s]),
            Token::Use => self.parse_use(),
            _ => {
                let s = self.parse_assign_or_expr()?;
                self.then_lines(s)
            }
        }
    }

    fn parse_fn_decl(&mut self, is_view: bool) -> Result<Stmt, String> {
        self.advance(); // skip 'fn' or 'view'
        let name = self.expect_ident()?;
        self.expect(&Token::LBracket)?;
        let (params, defaults) = self.parse_param_list()?;
        self.expect(&Token::RBracket)?;
        self.skip_newlines();
        let mut body = self.body(|p| p.parse_block())?;
        // Implicit return: the last expression statement in a function body is
        // the return value. Desugar here so type inference and the lowerings
        // all see the same explicit AST.
        if let Some(last_expr_pos) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if matches!(body[last_expr_pos], Stmt::ExprStmt(_)) {
                if let Stmt::ExprStmt(e) = body.remove(last_expr_pos) {
                    body.insert(last_expr_pos, Stmt::Return(Some(e)));
                }
            }
        }
        Ok(Stmt::FnDecl { name, params, defaults, body, is_view })
    }

    /// `derive name` or `derive name[fresh = N]`, then a body. No parameters,
    /// ever: a derivation reads facts and sources, it is not called.
    fn parse_derive(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'derive'
        let name = self.expect_ident()?;
        let mut fresh = 0.0;
        if self.check(&Token::LBracket) {
            self.advance();
            while !self.check(&Token::RBracket) {
                let key = self.expect_ident()?;
                if key != "fresh" {
                    return Err(format!("derive {}: unknown option '{}' — the one option is fresh = seconds", name, key));
                }
                self.expect(&Token::Assign)?;
                fresh = match self.peek().clone() {
                    Token::Int(n) => { self.advance(); n as f64 }
                    Token::Float(x) => { self.advance(); x }
                    other => return Err(format!("derive {}: fresh = takes seconds, got {}", name, other)),
                };
                if self.check(&Token::Comma) { self.advance(); }
            }
            self.expect(&Token::RBracket)?;
        }
        self.skip_newlines();
        let mut body = self.body(|p| p.parse_block())?;
        if let Some(last) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if matches!(body[last], Stmt::ExprStmt(_)) {
                if let Stmt::ExprStmt(e) = body.remove(last) {
                    body.insert(last, Stmt::Return(Some(e)));
                }
            }
        }
        Ok(Stmt::Derive { name, fresh, body })
    }

    /// `test` (attached, applies to the fn above) or `test name` (standalone),
    /// optionally with [params] — params are generated inputs. Body is an
    /// ordinary statement block; bare boolean lines are the truths.
    fn parse_test_block(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'test'
        let name = if let Token::Ident(_) = self.peek() {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let mut params = Vec::new();
        if self.check(&Token::LBracket) {
            self.advance();
            while !self.check(&Token::RBracket) {
                params.push(self.expect_ident()?);
                if self.check(&Token::Comma) {
                    self.advance();
                }
            }
            self.expect(&Token::RBracket)?;
        }
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Stmt::TestBlock { name, params, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, String> {
        if self.bodies == 0 {
            return Err(format!("{}return leaves a function, and this line is in none — log the value, or put these lines in a fn", self.at()));
        }
        self.advance(); // skip 'return'
        if matches!(self.peek(), Token::Newline | Token::Eof | Token::Dedent) {
            Ok(Stmt::Return(None))
        } else {
            let expr = self.parse_expr()?;
            Ok(Stmt::Return(Some(expr)))
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'if'
        let condition = self.parse_expr()?;
        self.skip_newlines();
        let then_body = self.parse_block()?;
        self.skip_newlines();
        let else_body = if self.check(&Token::Else) {
            self.advance();
            self.skip_newlines();
            if self.check(&Token::If) {
                // else if
                let elif = self.parse_if()?;
                Some(vec![elif])
            } else {
                // else
                let body = self.parse_block()?;
                Some(body)
            }
        } else {
            None
        };
        Ok(Stmt::If {
            condition,
            then_body,
            else_body,
        })
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'for'
        let var = self.expect_ident()?;
        self.expect(&Token::In)?;
        let iter = self.parse_expr()?;
        self.skip_newlines();
        let body = self.in_loop(false, |p| p.parse_block())?;
        Ok(Stmt::For { var, iter, body })
    }

    /// A loop's body, parsed knowing which loop it is in.
    fn in_loop<T>(&mut self, is_while: bool, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        self.loops.push(is_while);
        let out = f(self);
        self.loops.pop();
        out
    }

    /// A body a `return` may leave: no loop of the lines around it is its.
    fn body<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let loops = std::mem::take(&mut self.loops);
        self.bodies += 1;
        let out = f(self);
        self.bodies -= 1;
        self.loops = loops;
        out
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'while'
        let condition = self.parse_expr()?;
        self.skip_newlines();
        let body = self.in_loop(true, |p| p.parse_block())?;
        Ok(Stmt::While { condition, body })
    }

    fn parse_struct(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'struct'
        let name = self.expect_ident()?;
        self.expect(&Token::LBracket)?;
        let (fields, defaults) = self.parse_param_list()?;
        if defaults.iter().any(|d| d.is_some()) {
            return Err(format!(
                "struct fields cannot have defaults at line {}",
                self.current_line()
            ));
        }
        self.expect(&Token::RBracket)?;
        Ok(Stmt::StructDecl { name, fields })
    }

    fn parse_use(&mut self) -> Result<Vec<Stmt>, String> {
        self.advance(); // skip 'use'
        let mut stmts = Vec::new();
        // Parse first module path (with optional `as alias`)
        let (path, alias) = self.parse_use_path_aliased()?;
        stmts.push(Stmt::Use { path, alias });
        // Support comma-separated: use web, ui, views.home
        while self.check(&Token::Comma) {
            self.advance();
            let (path, alias) = self.parse_use_path_aliased()?;
            stmts.push(Stmt::Use { path, alias });
        }
        Ok(stmts)
    }

    /// Parse a use path with an optional trailing `as alias`. `as` is a
    /// contextual keyword — it only has meaning here, so a variable named
    /// `as` stays legal everywhere else.
    fn parse_use_path_aliased(&mut self) -> Result<(String, Option<String>), String> {
        let path = self.parse_use_path()?;
        if matches!(self.peek(), Token::Ident(n) if n == "as") {
            self.advance(); // skip 'as'
            let alias = self.expect_ident()?;
            return Ok((path, Some(alias)));
        }
        Ok((path, None))
    }

    fn parse_use_path(&mut self) -> Result<String, String> {
        let mut path = self.expect_ident()?;
        // Support both slash paths (use myapp/utils) and dot paths (use math.stats)
        while self.check(&Token::Slash) || self.check(&Token::Dot) {
            let sep = if self.check(&Token::Dot) { '.' } else { '/' };
            self.advance();
            let next = self.expect_ident()?;
            path.push(sep);
            path.push_str(&next);
        }
        Ok(path)
    }

    /// A chain written as lines: `x = e` and, indented under it, lines that
    /// each start with `then` (spec, Then Chaining). Every step is a
    /// statement of its own, assigned to a hidden name so the value's kind
    /// may change from step to step, as a variable's may not: `x__then1 = e`,
    /// each step reads the value so far, and the last step assigns `x`. A
    /// step that names `x` reads the value where it says; a step that does
    /// not is a call, and the value is its first argument, as `x then f[a]`
    /// is `f[x, a]`.
    fn then_lines(&mut self, s: Stmt) -> Result<Vec<Stmt>, String> {
        let Stmt::Assign { name, value } = s else { return Ok(vec![s]) };
        if !(matches!(self.peek(), Token::Newline)
            && matches!(self.peek_ahead(1), Token::Indent)
            && matches!(self.peek_ahead(2), Token::Then))
        {
            return Ok(vec![Stmt::Assign { name, value }]);
        }
        self.advance(); // the line's end
        self.advance(); // the indent
        let mut steps: Vec<(usize, Expr)> = Vec::new();
        while self.check(&Token::Then) {
            let line = self.current_line();
            self.advance();
            let step = self.parse_expr()?;
            steps.push((line, step));
            // (a step's line ends with it: what follows on the line is the
            // mistake, said as it is — `then len(x)` a call with `( )`)
            if !matches!(self.peek(), Token::Newline | Token::Dedent | Token::Eof) {
                return Err(self.unexpected(None));
            }
            self.skip_newlines();
        }
        if self.check(&Token::Dedent) {
            self.advance();
        } else if !self.is_at_end() {
            return Err(format!("line {}: a chain's lines each start with `then`", self.current_line()));
        }
        let hidden = |k: usize| format!("{}__then{}", name, k);
        let n = steps.len();
        let mut out = vec![Stmt::Assign { name: hidden(1), value }];
        for (k, (line, mut step)) in steps.into_iter().enumerate() {
            let prev = hidden(k + 1);
            if mentions(&step, &name) {
                let renames = std::collections::BTreeMap::from([(name.clone(), prev)]);
                let visible = std::collections::BTreeSet::from([name.clone()]);
                crate::rust_names::rename_expr(&mut step, &renames, &visible);
            } else {
                step = match step {
                    Expr::Call { callee, mut args } => {
                        args.insert(0, Expr::Ident(prev));
                        Expr::Call { callee, args }
                    }
                    Expr::Ident(f) => Expr::Call { callee: Box::new(Expr::Ident(f)), args: vec![Expr::Ident(prev)] },
                    _ => {
                        return Err(format!(
                            "line {}: a `then` step is a call — `then f[…]`, the value so far its first argument — or an expression that names `{}`",
                            line, name
                        ))
                    }
                };
            }
            let target = if k + 1 == n { name.clone() } else { hidden(k + 2) };
            out.push(Stmt::Line(line));
            out.push(Stmt::Assign { name: target, value: step });
        }
        Ok(out)
    }

    fn parse_assign_or_expr(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_expr()?;

        // check for assignment: ident = expr
        if self.check(&Token::Assign) {
            match expr {
                Expr::Ident(name) => {
                    self.advance(); // skip '='
                    // check for 'for' expression
                    if self.check(&Token::For) {
                        let for_expr = self.parse_for_expr()?;
                        return Ok(Stmt::Assign {
                            name,
                            value: for_expr,
                        });
                    }
                    let value = self.parse_expr()?;
                    // check for multiple assignment: a, b = 1, 2
                    // already parsed as single, handle comma case
                    Ok(Stmt::Assign { name, value })
                }
                // field assignment: obj.field = value
                Expr::FieldAccess { object, field } => {
                    self.advance(); // skip '='
                    let value = self.parse_expr()?;
                    Ok(Stmt::FieldAssign { object: *object, field, value })
                }
                // index assignment: list[idx] = value, map[key] = value
                Expr::Call { callee, args } if args.len() == 1 => {
                    self.advance(); // skip '='
                    let value = self.parse_expr()?;
                    Ok(Stmt::IndexAssign { object: *callee, index: args.into_iter().next().unwrap(), value })
                }
                _ => {
                    Err(format!(
                        "invalid assignment target at line {}",
                        self.current_line()
                    ))
                }
            }
        } else if self.check(&Token::Comma) {
            // multiple assignment: first name already parsed
            match expr {
                Expr::Ident(first_name) => {
                    let mut names = vec![first_name];
                    while self.check(&Token::Comma) {
                        self.advance();
                        names.push(self.expect_ident()?);
                    }
                    self.expect(&Token::Assign)?;
                    let mut values = vec![self.parse_expr()?];
                    while self.check(&Token::Comma) {
                        self.advance();
                        values.push(self.parse_expr()?);
                    }
                    Ok(Stmt::MultiAssign { names, values })
                }
                _ => {
                    Err(format!(
                        "invalid multiple assignment at line {}",
                        self.current_line()
                    ))
                }
            }
        } else {
            Ok(Stmt::ExprStmt(expr))
        }
    }

    fn parse_for_expr(&mut self) -> Result<Expr, String> {
        self.advance(); // skip 'for'
        let var = self.expect_ident()?;
        self.expect(&Token::In)?;
        let iter = self.parse_expr()?;
        self.skip_newlines();
        let body = self.in_loop(false, |p| p.parse_block())?;
        Ok(Expr::ForExpr {
            var,
            iter: Box::new(iter),
            body,
        })
    }

    // --- Block parsing ---

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(&Token::Indent)?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Token::Dedent) || self.is_at_end() {
                break;
            }
            let line = self.current_line();
            stmts.push(Stmt::Line(line));
            let start = self.pos;
            let parsed = self.parse_stmt()?;
            self.end_statement(start, &parsed)?;
            stmts.extend(parsed);
            self.skip_newlines();
        }
        if self.check(&Token::Dedent) {
            self.advance();
        }
        Ok(stmts)
    }

    // --- Expression parsing (precedence climbing) ---

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.check(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        while self.check(&Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_then()?;
        loop {
            // `x in xs` and `x not in xs` (spec, Membership): the row
            // `contains`, reached with the collection first — the operator
            // is the one way to write it
            if self.check(&Token::In) || (self.check(&Token::Not) && matches!(self.peek_ahead(1), Token::In)) {
                let negated = self.check(&Token::Not);
                self.advance();
                if negated {
                    self.advance();
                }
                let right = self.parse_then()?;
                let call = Expr::Call { callee: Box::new(Expr::Ident("contains".to_string())), args: vec![right, left] };
                left = if negated { Expr::UnaryOp { op: UnaryOp::Not, expr: Box::new(call) } } else { call };
                continue;
            }
            let op = match self.peek() {
                Token::Eq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                Token::Gt => BinOp::Gt,
                Token::Lt => BinOp::Lt,
                Token::Gte => BinOp::Gte,
                Token::Lte => BinOp::Lte,
                _ => break,
            };
            self.advance();
            let right = self.parse_then()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// Parse `then` chaining: `x then f` → `f[x]`, `x then f[a, b]` → `f[x, a, b]`
    fn parse_then(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_addition()?;
        while self.check(&Token::Then) {
            self.advance();
            // Right side must be an identifier (function name)
            let func_name = self.expect_ident()?;
            // Check if followed by [ for additional args
            let mut args = vec![left];
            if self.check(&Token::LBracket) {
                self.advance();
                let extra_args = self.parse_arg_list()?;
                self.expect(&Token::RBracket)?;
                args.extend(extra_args);
            }
            left = Expr::Call {
                callee: Box::new(Expr::Ident(func_name)),
                args,
            };
        }
        Ok(left)
    }

    fn parse_addition(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplication(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_power()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                Token::DotStar => BinOp::ElemMul,
                Token::DotSlash => BinOp::ElemDiv,
                _ => break,
            };
            self.advance();
            let right = self.parse_power()?;
            left = Expr::BinOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let base = self.parse_unary()?;
        let op = match self.peek() {
            Token::Caret => BinOp::Pow,
            Token::DotCaret => BinOp::ElemPow,
            _ => return Ok(base),
        };
        self.advance();
        // Right-associative: parse_power recurses
        let exp = self.parse_power()?;
        Ok(Expr::BinOp {
            left: Box::new(base),
            op,
            right: Box::new(exp),
        })
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.check(&Token::Not) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            });
        }
        if self.check(&Token::Minus) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check(&Token::LBracket) {
                self.advance();
                let args = self.parse_arg_list()?;
                self.expect(&Token::RBracket)?;
                // `contains[xs, x]` is written `x in xs` (spec, Membership)
                let named = match &expr {
                    Expr::Ident(n) => Some(n.as_str()),
                    Expr::FieldAccess { field, .. } => Some(field.as_str()),
                    _ => None,
                };
                if named == Some("contains") {
                    return Err(format!(
                        "line {}: `contains[xs, x]` is written `x in xs` — and `x not in xs` for its absence",
                        self.current_line()
                    ));
                }
                if named == Some("has") {
                    return Err(format!("line {}: `has[m, key]` is written `key in m`", self.current_line()));
                }
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else if self.check(&Token::Dot) {
                self.advance();
                let field = self.expect_ident()?;
                expr = Expr::FieldAccess {
                    object: Box::new(expr),
                    field,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::Int(n) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::Float(n))
            }
            Token::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Token::Ident(name) => {
                // Python/JS value keywords in expression position (x = None,
                // flag = True) — teach the Crow form.
                if let Some(hint) = python_value_hint(&name) {
                    return Err(format!("{}{}", self.at(), hint));
                }
                self.advance();
                Ok(Expr::Ident(name))
            }
            Token::LBracket => self.parse_array_or_map(),
            Token::LParen => {
                self.advance(); // skip '('
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            _ => Err(self.unexpected(None)),
        }
    }


    fn parse_array_or_map(&mut self) -> Result<Expr, String> {
        self.advance(); // skip '['

        // empty list
        if self.check(&Token::RBracket) {
            self.advance();
            return Ok(Expr::Array(vec![]));
        }

        // parse first element
        let first = self.parse_expr()?;

        // Bare map literals were removed — bare [...] is always an array.
        // Maps use the map[...] constructor (Design Note in language-spec.md).
        if self.check(&Token::Assign) {
            return Err(format!(
                "map literals were removed — write map[key = value, ...] instead of [key = value, ...] at line {}",
                self.current_line()
            ));
        }

        let mut items = vec![first];
        while self.check(&Token::Comma) {
            self.advance();
            items.push(self.parse_expr()?);
        }
        self.expect(&Token::RBracket)?;
        Ok(Expr::Array(items))
    }

    // --- Helper parsers ---

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(name)
            }
            _ => Err(format!("{}a name is wanted here, not {}", self.at(), self.peek().said())),
        }
    }

    fn parse_param_list(&mut self) -> Result<(Vec<String>, Vec<Option<Expr>>), String> {
        let mut params = Vec::new();
        let mut defaults = Vec::new();
        if !self.check(&Token::RBracket) {
            loop {
                let name = self.expect_ident()?;
                let default = if self.check(&Token::Assign) {
                    self.advance();
                    Some(self.parse_default_literal()?)
                } else {
                    if defaults.last().map_or(false, |d: &Option<Expr>| d.is_some()) {
                        return Err(format!(
                            "parameter '{}' without a default cannot follow parameters with defaults at line {}",
                            name, self.current_line()
                        ));
                    }
                    None
                };
                params.push(name);
                defaults.push(default);
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
        }
        Ok((params, defaults))
    }

    /// Default parameter values are restricted to literals (int, float, str,
    /// bool, optionally negated numbers) so evaluation order and scope never
    /// come into question.
    fn parse_default_literal(&mut self) -> Result<Expr, String> {
        let line = self.current_line();
        let expr = self.parse_expr()?;
        let is_literal = |e: &Expr| matches!(e, Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_));
        let ok = match &expr {
            e if is_literal(e) => true,
            Expr::UnaryOp { expr: inner, .. } => matches!(inner.as_ref(), Expr::Int(_) | Expr::Float(_)),
            _ => false,
        };
        if !ok {
            return Err(format!(
                "default values must be literals (int, float, str, bool) at line {}",
                line
            ));
        }
        Ok(expr)
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        let mut seen_named = false;
        if !self.check(&Token::RBracket) {
            loop {
                // Named argument: ident = expr, or 'quoted key' = expr for
                // map[...] keys with special characters (lookahead for the '=')
                let named_key = match (self.peek(), self.peek_ahead(1)) {
                    (Token::Ident(name), Token::Assign) => Some(name.clone()),
                    (Token::Str(s), Token::Assign) => Some(s.clone()),
                    _ => None,
                };
                if let Some(name) = named_key {
                    self.advance(); // key
                    self.advance(); // =
                    let value = self.parse_expr()?;
                    args.push(Expr::Named { name, value: Box::new(value) });
                    seen_named = true;
                } else {
                    if seen_named {
                        return Err(format!(
                            "positional argument cannot follow a named argument at line {}",
                            self.current_line()
                        ));
                    }
                    args.push(self.parse_expr()?);
                }
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
        }
        Ok(args)
    }
}

/// Python/JS keywords that lead a statement and have a direct Crow form. These
/// are plain identifiers in Crow's grammar (not keywords), so without this they
/// fail as stray names or confusing indent errors. Returning a message teaches
/// the Crow equivalent instead.
fn python_keyword_hint(name: &str) -> Option<String> {
    let hint = match name {
        "def" => "Crow declares functions with 'fn name[params]', not 'def' — \
                  and blocks use indentation, no ':' or '()'.",
        "lambda" => "Crow has no lambdas — define a named function with \
                     'fn name[params]' and pass its name (see Design Note: No Lambdas).",
        "class" => "Crow has no classes — use 'struct Name [field1, field2]' for \
                    records, and free functions that take the struct as a parameter.",
        "match" => "Crow removed 'match' — use if / else. There are no sum types or \
                    pattern matching (see Design Note: No Pattern Matching).",
        "elif" => "Crow has no 'elif' — write 'else' on its own line, then an \
                   indented 'if' beneath it.",
        "pass" => "Crow has no 'pass' — a block is just its indented lines; there's \
                   nothing to fill an empty one with.",
        "import" | "from" => "Crow imports with 'use module' (e.g. 'use math', \
                              'use myapp/utils'), not 'import'.",
        "print" => "Crow prints with 'log[value]', not 'print(...)'. Brackets are \
                    square: log['hello'].",
        "async" | "await" => "Crow has no async/await — concurrency is automatic: \
                              a 'for' loop runs its iterations in parallel.",
        "try" | "except" | "catch" | "raise" | "throw" => "Crow has no try/except — \
                    errors are values. A fallible call returns a value you can check \
                    with '.ok'; branching on an error propagates it (see §11 Error Handling).",
        _ => return None,
    };
    Some(hint.to_string())
}

/// The keyword a name is one slip from (`esle` → `else`, `retrun` →
/// `return`), by the rule a refusal's suggestion keeps (lower_ir::slips).
fn keyword_near(name: &str) -> Option<&'static str> {
    let limit = match name.chars().count() {
        0..=2 => return None,
        3..=5 => 1,
        _ => 2,
    };
    crate::lexer::KEYWORDS
        .iter()
        .filter(|k| **k != name)
        .map(|k| (crate::lower_ir::slips(name, k), *k))
        .filter(|(d, _)| *d <= limit)
        .min()
        .map(|(_, k)| k)
}

/// Python/JS literal keywords that appear in expression position. Called on
/// every identifier reference, so it must stay a cheap match that returns None
/// for ordinary names.
fn python_value_hint(name: &str) -> Option<String> {
    let hint = match name {
        "True" => "Crow booleans are lowercase: use 'true', not 'True'.",
        "False" => "Crow booleans are lowercase: use 'false', not 'False'.",
        "None" | "null" | "nil" | "undefined" => "Crow has no null value — a missing \
                    or failed value is an error-value (checked with '.ok'), and there \
                    is no 'none' (see Design Note: No none).",
        "lambda" => "Crow has no lambdas — define a named function with 'fn' and pass \
                     its name (see Design Note: No Lambdas).",
        _ => return None,
    };
    Some(hint.to_string())
}


/// Whether an expression names the variable: as an identifier, a field's
/// object, in a call, a list, a map, a named argument, or a span of text
/// (a word there that no `.` precedes, as a field's name is).
fn mentions(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Ident(n) => n == name,
        Expr::Str(s) => s.contains('`') && s.split('`').skip(1).step_by(2).any(|span| span_names_word(span, name)),
        Expr::BinOp { left, right, .. } => mentions(left, name) || mentions(right, name),
        Expr::UnaryOp { expr, .. } => mentions(expr, name),
        Expr::Call { callee, args } => mentions(callee, name) || args.iter().any(|a| mentions(a, name)),
        Expr::FieldAccess { object, .. } => mentions(object, name),
        Expr::Array(items) => items.iter().any(|i| mentions(i, name)),
        Expr::Map(pairs) => pairs.iter().any(|(k, v)| mentions(k, name) || mentions(v, name)),
        Expr::Named { value, .. } => mentions(value, name),
        Expr::ForExpr { var, iter, .. } => var != name && mentions(iter, name),
        _ => false,
    }
}

/// Whether a span of interpolated text holds the word as a name: whole, and
/// not a field (`t.name`).
fn span_names_word(span: &str, name: &str) -> bool {
    let chars: Vec<char> = span.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < chars.len() {
        if is_word(chars[i]) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let after_dot = start > 0 && chars[start - 1] == '.';
            if word == name && !after_dot {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod membership_tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parsed(src: &str) -> String {
        let tokens = Lexer::new(src).tokenize().expect("lexes");
        format!("{:?}", Parser::new(tokens).parse().expect("parses"))
    }

    fn err(src: &str) -> String {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(t) => t,
            Err(e) => return e,
        };
        Parser::new(tokens).parse().err().unwrap_or_default()
    }

    #[test]
    fn in_is_the_contains_row_with_the_collection_first() {
        let p = parsed("log[1 in [1, 2]]\n");
        assert!(p.contains("callee: Ident(\"contains\"), args: [Array([Int(1), Int(2)]), Int(1)]"), "{}", p);
        let p = parsed("log['z' not in xs]\n");
        assert!(p.contains("UnaryOp { op: Not, expr: Call { callee: Ident(\"contains\"), args: [Ident(\"xs\"), Str(\"z\")]"), "{}", p);
        // a comparison's place: tighter than `and`, looser than arithmetic
        let p = parsed("log[n + 1 in [3] and ok]\n");
        assert!(p.contains("op: And") && p.contains("args: [Array([Int(3)]), BinOp { left: Ident(\"n\"), op: Add"), "{}", p);
        // a loop's `in` is the loop's
        let p = parsed("for v in xs\n    log[v in ys]\n");
        assert!(p.contains("For {") && p.contains("callee: Ident(\"contains\"), args: [Ident(\"ys\"), Ident(\"v\")]"), "{}", p);
    }

    #[test]
    fn the_functions_behind_the_operator_are_not_written() {
        assert!(err("log[contains[[1], 1]]\n").contains("`contains[xs, x]` is written `x in xs`"));
        assert!(err("log[frame.contains[t.sym, 'a']]\n").contains("written `x in xs`"));
        assert!(err("log[has[m, 'a']]\n").contains("`has[m, key]` is written `key in m`"));
    }
}

#[cfg(test)]
mod then_lines_tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parsed(src: &str) -> String {
        let tokens = Lexer::new(src).tokenize().expect("lexes");
        format!("{:?}", Parser::new(tokens).parse().expect("parses"))
    }

    fn err(src: &str) -> String {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(t) => t,
            Err(e) => return e,
        };
        Parser::new(tokens).parse().err().unwrap_or_default()
    }

    #[test]
    fn a_chains_steps_are_statements_over_hidden_names() {
        // the value first where the step is a call; where the step names
        // the variable, the value is where it says, and the last assigns it
        let p = parsed("x = [3, 1, 2]\n    then sort\n    then reverse[x]\n    then len[x] + 1\nlog[x]\n");
        assert!(p.contains("name: \"x__then1\""), "{}", p);
        assert!(p.contains("Ident(\"x__then1\")"), "{}", p);
        assert!(p.contains("name: \"x__then2\""), "{}", p);
        assert!(p.contains("name: \"x__then3\""), "{}", p);
        assert!(!p.contains("name: \"x__then4\""), "{}", p);
        assert!(p.matches("name: \"x\"").count() == 1, "{}", p);
        // the inserted call: sort[x__then1]; the named step: reverse[x__then2]
        assert!(p.contains("callee: Ident(\"sort\"), args: [Ident(\"x__then1\")]"), "{}", p);
        assert!(p.contains("callee: Ident(\"reverse\"), args: [Ident(\"x__then2\")]"), "{}", p);
        // each step keeps its line
        assert!(p.contains("Line(2)") && p.contains("Line(3)") && p.contains("Line(4)"), "{}", p);
    }

    #[test]
    fn a_step_names_the_variable_in_a_field_a_span_or_a_nested_call() {
        let p = parsed("pr = frame.new[a = [1]]\n    then max[abs[pr.a - 1.0]]\n");
        assert!(p.contains("object: Ident(\"pr__then1\")"), "{}", p);
        let p = parsed("t = 'a'\n    then 'got `t`'\n");
        assert!(p.contains("got `t__then1`"), "{}", p);
        assert!(mentions(&Expr::Str("`t.x`".into()), "x") == false);
        assert!(mentions(&Expr::Str("`x.t`".into()), "x"));
    }

    #[test]
    fn a_step_that_is_not_a_call_and_names_nothing_is_refused() {
        let e = err("x = 1\n    then 2 + 3\n");
        assert!(e.contains("line 2: a `then` step is a call"), "{}", e);
    }

    #[test]
    fn a_then_line_without_an_assignment_is_refused() {
        let e = err("log[1]\nthen upper\n");
        assert!(e.contains("`then` continues an assignment"), "{}", e);
        let e = err("xs = for v in [1, 2]\n    v\n    then sort\n");
        assert!(e.contains("`then` continues an assignment"), "{}", e);
    }

    #[test]
    fn a_chain_inside_a_body_ends_with_the_body() {
        let p = parsed("fn f[s]\n    r = s\n        then upper\n    return r\nlog[f['a']]\n");
        assert!(p.contains("name: \"r__then1\"") && p.contains("Return"), "{}", p);
    }
}

#[cfg(test)]
mod pyism_tests {
    use super::*;
    use crate::lexer::Lexer;

    fn err(src: &str) -> String {
        let tokens = match Lexer::new(src).tokenize() {
            Ok(t) => t,
            Err(e) => return e,
        };
        Parser::new(tokens).parse().err().unwrap_or_default()
    }

    #[test]
    fn keyword_statements_teach_crow_form() {
        assert!(err("def f[x]\n  return x\n").contains("fn name[params]"));
        assert!(err("class Foo\n  x\n").contains("struct"));
        assert!(err("match v\n  1\n    log['a']\n").contains("if / else"));
        assert!(err("import math\n").contains("use module"));
        assert!(err("print(5)\n").contains("log["));
        assert!(err("try\n  x = 1\n").contains("errors are values"));
    }

    #[test]
    fn value_keywords_teach_crow_form() {
        assert!(err("flag = True\n").contains("lowercase"));
        assert!(err("x = None\n").contains("no null"));
    }

    #[test]
    fn keyword_names_are_still_usable_as_variables() {
        // A variable that shares a Python keyword's spelling, when assigned to,
        // must not trip the hint.
        assert!(err("from = 'alice'\nlog[from]\n").is_empty());
        assert!(err("print = 5\nlog[print]\n").is_empty());
    }

    #[test]
    fn ordinary_program_parses() {
        assert!(err("fn double[x]\n  return x * 2\nlog[double[5]]\n").is_empty());
    }
}
