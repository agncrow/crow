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
    /// Function declaration: fn name[params] body.
    /// `defaults[i]` is the default literal for `params[i]` (named parameters
    /// must come after all positional ones and must have defaults).
    FnDecl {
        name: String,
        params: Vec<String>,
        defaults: Vec<Option<Expr>>,
        body: Vec<Stmt>,
        /// True for `view` declarations — UI-tree builders. Inside a view the
        /// HTML element vocabulary is active; outside it doesn't exist.
        is_view: bool,
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
    /// Struct declaration: struct Name [fields]
    StructDecl {
        name: String,
        fields: Vec<String>,
    },
    /// Field assignment: obj.field = value
    FieldAssign {
        object: Expr,
        field: String,
        value: Expr,
    },
    /// Index assignment: array[idx] = value, map[key] = value
    IndexAssign {
        object: Expr,
        index: Expr,
        value: Expr,
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
    /// Break out of a loop
    Break,
    /// Continue to next loop iteration
    Continue,
    /// Line marker for error reporting (not a real statement)
    Line(usize),
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
    /// Function/array/map call: name[args] or expr[args]
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// Field access: expr.field
    FieldAccess {
        object: Box<Expr>,
        field: String,
    },
    /// Array literal: [1, 2, 3]
    Array(Vec<Expr>),
    /// Map literal: [key = val, ...] (bare-name keys) or ['key' = val, ...]
    Map(Vec<(Expr, Expr)>),
    /// Named argument at a call site: f[x, axis = 0].
    /// Only valid inside call argument lists; the lowering pass resolves every
    /// occurrence to a positional argument before execution or codegen.
    Named {
        name: String,
        value: Box<Expr>,
    },
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
    Pow,      // ^ (scalar power)
    ElemMul,  // .* (element-wise multiply)
    ElemDiv,  // ./ (element-wise divide)
    ElemPow,  // .^ (element-wise power)
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

