use crate::ast::*;
use crate::token::{SpannedToken, Token};

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.is_at_end() {
            let line = self.current_line();
            stmts.push(Stmt::Line(line));
            let parsed = self.parse_stmt()?;
            stmts.extend(parsed);
            self.skip_newlines();
        }
        Ok(stmts)
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
            let line = self.current_line();
            Err(format!(
                "expected '{}', got '{}' at line {}",
                expected,
                self.peek(),
                line
            ))
        }
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
        match self.peek().clone() {
            Token::Fn => self.parse_fn_decl().map(|s| vec![s]),
            Token::Return => self.parse_return().map(|s| vec![s]),
            Token::Break => { self.advance(); Ok(vec![Stmt::Break]) }
            Token::Continue => { self.advance(); Ok(vec![Stmt::Continue]) }
            Token::If => self.parse_if().map(|s| vec![s]),
            Token::For => self.parse_for_stmt().map(|s| vec![s]),
            Token::While => self.parse_while().map(|s| vec![s]),
            Token::Struct => self.parse_struct().map(|s| vec![s]),
            Token::Use => self.parse_use(),
            _ => self.parse_assign_or_expr().map(|s| vec![s]),
        }
    }

    fn parse_fn_decl(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'fn'
        let name = self.expect_ident()?;
        self.expect(&Token::LBracket)?;
        let (params, defaults) = self.parse_param_list()?;
        self.expect(&Token::RBracket)?;
        self.skip_newlines();
        let mut body = self.parse_block()?;
        // Implicit return: the last expression statement in a function body is
        // the return value. Desugar here so the interpreter, type inference,
        // and transpiler all see the same explicit AST.
        if let Some(last_expr_pos) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if matches!(body[last_expr_pos], Stmt::ExprStmt(_)) {
                if let Stmt::ExprStmt(e) = body.remove(last_expr_pos) {
                    body.insert(last_expr_pos, Stmt::Return(Some(e)));
                }
            }
        }
        Ok(Stmt::FnDecl { name, params, defaults, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, String> {
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
        let body = self.parse_block()?;
        Ok(Stmt::For { var, iter, body })
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.advance(); // skip 'while'
        let condition = self.parse_expr()?;
        self.skip_newlines();
        let body = self.parse_block()?;
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
        // Parse first module path
        let path = self.parse_use_path()?;
        stmts.push(Stmt::Use { path });
        // Support comma-separated: use web, ui, views.home
        while self.check(&Token::Comma) {
            self.advance();
            let path = self.parse_use_path()?;
            stmts.push(Stmt::Use { path });
        }
        Ok(stmts)
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
        let body = self.parse_block()?;
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
            let parsed = self.parse_stmt()?;
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
            _ => Err(format!(
                "unexpected token '{}' at line {}",
                self.peek(),
                self.current_line()
            )),
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
            _ => Err(format!(
                "expected identifier, got '{}' at line {}",
                self.peek(),
                self.current_line()
            )),
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

