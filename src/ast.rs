/// AST node types for the Crow language

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Stmt {
    /// Variable assignment: name = expr
    Assign {
        name: String,
        value: Expr,
    },
    /// Multiple assignment: a, b, c = 1, 2, 3
    MultiAssign {
        names: Vec<String>,
        values: Vec<Expr>,
    },
    /// Function declaration: fn name[params] body
    FnDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    /// Return statement: return expr
    Return(Option<Expr>),
    /// Expression statement (function call, etc.)
    ExprStmt(Expr),
    /// If statement: if cond body else body
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    /// For loop: for var in expr body
    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    /// While loop: while cond body
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    /// Match statement: match expr arms
    Match {
        value: Expr,
        arms: Vec<MatchArm>,
    },
    /// Struct declaration: struct Name [fields]
    StructDecl {
        name: String,
        fields: Vec<String>,
    },
    /// Use statement: use module/path
    Use {
        path: String,
    },
    /// Resource control: cpu[n] or gpu[n]
    Resource {
        kind: String, // "cpu" or "gpu"
        count: Expr,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pattern {
    /// Literal value
    Literal(Expr),
    /// Range: start..end
    Range(Expr, Expr),
    /// Wildcard: _
    Wildcard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    /// Integer literal
    Int(i64),
    /// Float literal
    Float(f64),
    /// String literal (raw, with {expr} for interpolation)
    Str(String),
    /// Boolean literal
    Bool(bool),
    /// Variable reference
    Ident(String),
    /// Binary operation: a + b, a eq b, a and b
    BinOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    /// Unary operation: not x, -x
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    /// Function/list/map call: name[args] or expr[args]
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// Field access: expr.field
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    /// List literal: [1, 2, 3]
    List(Vec<Expr>),
    /// Map literal: ['key' = val, ...]
    Map(Vec<(Expr, Expr)>),
    /// For expression (assigned to variable): for x in iter body
    ForExpr {
        var: String,
        iter: Box<Expr>,
        body: Vec<Stmt>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
}

