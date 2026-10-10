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
    /// A derivation: `derive name[fresh = N]` and a body. A value computed
    /// from facts and sources, read by name, never assigned, recomputed
    /// when an input changed (derive-design.md). `fresh` is seconds; 0 is
    /// "as fresh as the loop".
    Derive {
        name: String,
        fresh: f64,
        body: Vec<Stmt>,
    },
    /// Verification block: `test[params]` (attached to the preceding fn)
    /// or `test name[params]` (standalone). Never executes at runtime —
    /// crow check consumes these; crow build strips them.
    TestBlock {
        name: Option<String>,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    /// Use statement: use module/path
    Use {
        path: String,
        /// `use path as alias` — the module becomes qualified-only under the
        /// alias (Python import-as semantics); None = default import
        alias: Option<String>,
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
        /// The key was written as an int (`map[2 = 'b']`): an int key, not
        /// the text `'2'`.
        int_key: bool,
        value: Box<Expr>,
    },
    /// For expression (assigned to variable): for x in iter body
    ForExpr {
        var: String,
        iter: Box<Expr>,
        body: Vec<Stmt>,
    },
}

/// The collection builtins that take their collection by value in the
/// runtime (`crow_runtime::core`), so `x = f[x, …]` can move `x` in; and
/// the text builtins whose taken form changes the text where it lies
/// (`table::taken_path`, the interpreter's `take` bridges).
pub const IN_PLACE_BUILTINS: &[&str] = &["push", "pop", "sort", "reverse", "concat", "set", "del", "replace", "upper", "lower"];

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

