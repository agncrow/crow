/// Token types for the Crow language
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Int(i64),
    Float(f64),
    Str(String), // raw string content including {expr} for interpolation
    True,
    False,

    // Identifier
    Ident(String),

    // Keywords
    Fn,
    Return,
    If,
    Else,
    For,
    In,
    While,
    Match,
    Struct,
    Use,
    Break,
    Continue,

    // Comparison operators (word-based)
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,

    // Logical operators
    And,
    Or,
    Not,

    // Arithmetic operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // Symbols
    Assign,    // =
    Comma,     // ,
    Dot,       // .
    DotDot,    // ..
    LBracket,  // [
    RBracket,  // ]
    LParen,    // (
    RParen,    // )
    Underscore, // _

    // Indentation
    Indent,
    Dedent,
    Newline,

    // End of file
    Eof,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Int(n) => write!(f, "{}", n),
            Token::Float(n) => write!(f, "{}", n),
            Token::Str(s) => write!(f, "'{}'", s),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Ident(s) => write!(f, "{}", s),
            Token::Fn => write!(f, "fn"),
            Token::Return => write!(f, "return"),
            Token::If => write!(f, "if"),
            Token::Else => write!(f, "else"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::While => write!(f, "while"),
            Token::Match => write!(f, "match"),
            Token::Struct => write!(f, "struct"),
            Token::Use => write!(f, "use"),
            Token::Break => write!(f, "break"),
            Token::Continue => write!(f, "continue"),
            Token::Eq => write!(f, "eq"),
            Token::Ne => write!(f, "ne"),
            Token::Gt => write!(f, "gt"),
            Token::Lt => write!(f, "lt"),
            Token::Gte => write!(f, "gte"),
            Token::Lte => write!(f, "lte"),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::Assign => write!(f, "="),
            Token::Comma => write!(f, ","),
            Token::Dot => write!(f, "."),
            Token::DotDot => write!(f, ".."),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::Underscore => write!(f, "_"),
            Token::Indent => write!(f, "INDENT"),
            Token::Dedent => write!(f, "DEDENT"),
            Token::Newline => write!(f, "NEWLINE"),
            Token::Eof => write!(f, "EOF"),
        }
    }
}

/// A token with its position in the source
#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub line: usize,
    #[allow(dead_code)]
    pub col: usize,
}

