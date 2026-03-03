/// Validation pass for Crow programs.
/// Runs after type inference, before transpilation.
/// Catches user errors and reports them in Crow terms.

use std::collections::BTreeSet;
use crate::ast::*;
use crate::type_infer::{CrowType, TypeContext};

/// All builtin function names recognized by the Crow language.
const BUILTINS: &[&str] = &[
    "log", "assert", "type",
    // Math
    "abs", "sqrt", "floor", "ceil", "round", "min", "max",
    // String
    "split", "trim", "replace", "upper", "lower", "join", "starts", "ends",
    "repeat", "pad",
    // Conversion
    "int", "float", "str",
    // Collections
    "len", "contains", "slice", "push", "pop", "sort", "reverse", "append",
    "range", "map",
    // Map
    "keys", "values", "has", "set", "del",
    // CLI / IO
    "args", "input",
    // File I/O
    "read", "write",
    // System
    "rand", "time", "sleep", "env", "exit",
    // JSON
    "json", "encode",
    // Error
    "error",
    // Resource
    "cpu", "gpu",
    // Autograd
    "grad",
];

pub struct Validator<'a> {
    ctx: &'a TypeContext,
    errors: Vec<String>,
    /// Variables known to be defined at the current point
    defined_vars: BTreeSet<String>,
    current_line: usize,
}

impl<'a> Validator<'a> {
    pub fn new(ctx: &'a TypeContext) -> Self {
        Validator {
            ctx,
            errors: Vec::new(),
            defined_vars: BTreeSet::new(),
            current_line: 0,
        }
    }

    /// Run validation on a program. Returns Ok(()) or Err with all error messages.
    pub fn validate(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        // Pre-populate with function and struct names (these are always valid identifiers)
        // Do NOT pre-populate from ctx.vars — the type inferrer optimistically adds
        // unknown identifiers to vars (e.g. foo[1,2] -> Fn type), which would hide
        // "undefined variable/function" errors.
        for name in self.ctx.functions.keys() {
            self.defined_vars.insert(name.clone());
        }
        for name in self.ctx.structs.keys() {
            self.defined_vars.insert(name.clone());
        }
        // Pre-scan top-level assignments to populate defined_vars
        // (the validator will also add them as it walks, but we need forward refs)
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, .. } => { self.defined_vars.insert(name.clone()); }
                Stmt::MultiAssign { names, .. } => {
                    for n in names { self.defined_vars.insert(n.clone()); }
                }
                _ => {}
            }
        }

        for stmt in stmts {
            self.validate_stmt(stmt);
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.join("\n"))
        }
    }

    fn validate_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Line(n) => { self.current_line = *n; }
            Stmt::Assign { name, value } => {
                self.validate_expr(value);
                self.defined_vars.insert(name.clone());
                // Check type-changing reassignment
                let new_ty = self.ctx.infer_expr(value);
                if let Some(existing_ty) = self.ctx.vars.get(name) {
                    if !existing_ty.compatible(&new_ty)
                        && !matches!(existing_ty, CrowType::Unknown)
                        && !matches!(new_ty, CrowType::Unknown)
                    {
                        self.error(format!(
                            "cannot reassign '{}' from {} to {} — variable type cannot change",
                            name, existing_ty.crow_name(), new_ty.crow_name()
                        ));
                    }
                }
            }
            Stmt::MultiAssign { names, values } => {
                for v in values { self.validate_expr(v); }
                for n in names { self.defined_vars.insert(n.clone()); }
            }
            Stmt::FnDecl { name, params, body } => {
                self.defined_vars.insert(name.clone());
                let saved = self.defined_vars.clone();
                for p in params { self.defined_vars.insert(p.clone()); }
                for s in body { self.validate_stmt(s); }
                self.defined_vars = saved;
                // Re-add the function name itself
                self.defined_vars.insert(name.clone());
            }
            Stmt::Return(Some(e)) => self.validate_expr(e),
            Stmt::Return(None) => {}
            Stmt::ExprStmt(e) => self.validate_expr(e),
            Stmt::If { condition, then_body, else_body } => {
                self.validate_expr(condition);
                for s in then_body { self.validate_stmt(s); }
                if let Some(eb) = else_body {
                    for s in eb { self.validate_stmt(s); }
                }
            }
            Stmt::For { var, iter, body } => {
                self.validate_expr(iter);
                let saved = self.defined_vars.clone();
                self.defined_vars.insert(var.clone());
                for s in body { self.validate_stmt(s); }
                self.defined_vars = saved;
                self.defined_vars.insert(var.clone());
            }
            Stmt::While { condition, body } => {
                self.validate_expr(condition);
                for s in body { self.validate_stmt(s); }
            }
            Stmt::Match { value, arms } => {
                self.validate_expr(value);
                for arm in arms {
                    for s in &arm.body { self.validate_stmt(s); }
                }
            }
            Stmt::StructDecl { name, .. } => {
                self.defined_vars.insert(name.clone());
            }
            Stmt::FieldAssign { object, value, .. } => {
                self.validate_expr(object);
                self.validate_expr(value);
            }
            Stmt::IndexAssign { object, index, value } => {
                self.validate_expr(object);
                self.validate_expr(index);
                self.validate_expr(value);
            }
            Stmt::Use { path } => {
                // Built-in modules register their name as a defined variable
                if path == "math" {
                    self.defined_vars.insert("math".to_string());
                }
                // Other modules handled by expand_module_imports
            }
            Stmt::Resource { .. } => {}
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn validate_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) => {}
            Expr::Ident(name) => {
                if !self.defined_vars.contains(name)
                    && !BUILTINS.contains(&name.as_str())
                {
                    self.error(format!("undefined variable '{}'", name));
                }
            }
            Expr::BinOp { left, op, right } => {
                self.validate_expr(left);
                self.validate_expr(right);
                self.check_binop_types(left, op, right);
            }
            Expr::UnaryOp { expr: e, .. } => {
                self.validate_expr(e);
            }
            Expr::Call { callee, args } => {
                for a in args { self.validate_expr(a); }
                match callee.as_ref() {
                    Expr::Ident(name) => {
                        self.validate_call(name, args);
                    }
                    Expr::FieldAccess { object, field } => {
                        // Module call: module.func[args]
                        if let Expr::Ident(mod_name) = object.as_ref() {
                            let prefixed = format!("{}_{}", mod_name, field);
                            if !self.ctx.functions.contains_key(&prefixed)
                                && !self.ctx.structs.contains_key(&prefixed)
                                && !self.defined_vars.contains(mod_name)
                            {
                                self.error(format!("undefined module or variable '{}'", mod_name));
                            }
                        } else {
                            self.validate_expr(object);
                        }
                    }
                    other => { self.validate_expr(other); }
                }
            }
            Expr::FieldAccess { object, field } => {
                self.validate_expr(object);
                // Check field access validity
                let obj_type = self.ctx.infer_expr(object);
                match field.as_str() {
                    "ok" | "reason" | "kind" | "trace" => {} // error-value fields, always valid
                    _ => {
                        if let CrowType::Struct(name) = &obj_type {
                            if let Some(sig) = self.ctx.structs.get(name) {
                                if !sig.fields.iter().any(|(f, _)| f == field) {
                                    self.error(format!(
                                        "struct '{}' has no field '{}'",
                                        name, field
                                    ));
                                }
                            }
                        }
                        // Module access: module.var — don't flag these
                        // since the type context handles them via prefixed names
                    }
                }
            }
            Expr::Array(items) => {
                for item in items { self.validate_expr(item); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.validate_expr(k);
                    self.validate_expr(v);
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.validate_expr(iter);
                let saved = self.defined_vars.clone();
                self.defined_vars.insert(var.clone());
                for s in body { self.validate_stmt(s); }
                self.defined_vars = saved;
            }
        }
    }

    fn validate_call(&mut self, name: &str, args: &[Expr]) {
        let argc = args.len();

        // Known builtin? Check arg count
        if BUILTINS.contains(&name) {
            self.check_builtin_args(name, argc);
            return;
        }

        // User-defined function?
        if let Some(sig) = self.ctx.functions.get(name) {
            let expected = sig.params.len();
            if argc != expected {
                self.error(format!(
                    "'{}' expects {} argument{}, got {}",
                    name, expected, if expected == 1 { "" } else { "s" }, argc
                ));
            }
            return;
        }

        // Struct constructor?
        if let Some(sig) = self.ctx.structs.get(name) {
            let expected = sig.fields.len();
            if argc != expected {
                self.error(format!(
                    "struct '{}' has {} field{}, got {} argument{}",
                    name, expected, if expected == 1 { "" } else { "s" },
                    argc, if argc == 1 { "" } else { "s" }
                ));
            }
            return;
        }

        // Variable that holds a function, list, or map?
        // Only trust ctx.vars if the variable was actually defined (assigned) in the source.
        // The type inferrer optimistically adds unknown identifiers to vars.
        if self.defined_vars.contains(name) {
            if let Some(var_type) = self.ctx.vars.get(name) {
                match var_type {
                    CrowType::Fn(params, _) => {
                        if argc != params.len() {
                            self.error(format!(
                                "'{}' expects {} argument{}, got {}",
                                name, params.len(),
                                if params.len() == 1 { "" } else { "s" }, argc
                            ));
                        }
                    }
                    CrowType::Array(_) | CrowType::Map(_) | CrowType::Json
                    | CrowType::Result(_) => {
                        // indexing — valid
                    }
                    _ => {
                        // Unknown variable type used as function — might be dynamic
                    }
                }
            }
            return;
        }

        // Truly unknown
        self.error(format!("undefined function '{}'", name));
    }

    fn check_builtin_args(&mut self, name: &str, argc: usize) {
        let valid = match name {
            "log" | "assert" => argc >= 1,
            "type" => argc == 1,
            "abs" | "sqrt" | "floor" | "ceil" | "round" => argc == 1,
            "min" | "max" => argc == 2,
            "split" => argc == 2,
            "trim" | "upper" | "lower" => argc == 1,
            "replace" => argc == 3,
            "join" => argc == 2,
            "starts" | "ends" => argc == 2,
            "int" | "float" | "str" => argc == 1,
            "len" => argc == 1,
            "contains" => argc == 2,
            "slice" => argc == 3,
            "repeat" | "pad" => argc == 2,
            "push" | "append" => argc == 2,
            "pop" | "sort" | "reverse" => argc == 1,
            "range" => argc >= 1 && argc <= 3,
            "map" => argc == 0,
            "keys" | "values" => argc == 1,
            "has" | "del" => argc == 2,
            "set" => argc == 3,
            "args" => argc == 0,
            "input" => argc <= 1,
            "read" => argc == 1,
            "write" => argc == 2,
            "rand" => argc <= 2,
            "time" => argc == 0,
            "sleep" => argc == 1,
            "env" => argc == 1,
            "exit" => argc <= 1,
            "json" | "encode" => argc == 1,
            "error" => argc >= 1 && argc <= 2,
            "cpu" => argc == 1,
            "gpu" => argc <= 1,
            "grad" => argc == 1,
            _ => true,
        };
        if !valid {
            self.error(format!("wrong number of arguments for '{}'", name));
        }
    }

    fn check_binop_types(&mut self, left: &Expr, op: &BinOp, right: &Expr) {
        let lt = self.ctx.infer_expr(left);
        let rt = self.ctx.infer_expr(right);
        // Unwrap CrowResult
        let lt = if let CrowType::Result(inner) = lt { *inner } else { lt };
        let rt = if let CrowType::Result(inner) = rt { *inner } else { rt };

        // Skip if either side is unknown
        if matches!(lt, CrowType::Unknown) || matches!(rt, CrowType::Unknown) {
            return;
        }

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                // String + is not allowed — use interpolation instead
                if matches!(op, BinOp::Add) && (matches!(lt, CrowType::Str) || matches!(rt, CrowType::Str)) {
                    self.error("cannot use '+' with strings — use string interpolation instead".to_string());
                    return;
                }
                // Numeric ops: both must be numeric (including NumArray and Sparse)
                let lt_num = matches!(lt, CrowType::Int | CrowType::Float | CrowType::NumArray(_) | CrowType::Sparse);
                let rt_num = matches!(rt, CrowType::Int | CrowType::Float | CrowType::NumArray(_) | CrowType::Sparse);
                if !lt_num || !rt_num {
                    let op_str = match op {
                        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                        BinOp::Div => "/", BinOp::Mod => "%", _ => "?"
                    };
                    self.error(format!(
                        "cannot use '{}' with {} and {}",
                        op_str, lt.crow_name(), rt.crow_name()
                    ));
                }
            }
            // Comparison and logical ops — skip validation for now
            _ => {}
        }
    }

    fn error(&mut self, msg: String) {
        self.errors.push(format!("line {}: {}", self.current_line, msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::type_infer::TypeContext;

    fn validate_source(source: &str) -> Result<(), String> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("lexer failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parser failed");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut validator = Validator::new(&ctx);
        validator.validate(&stmts)
    }

    #[test]
    fn test_valid_program() {
        let result = validate_source("x = 10\ny = x + 5\nlog[y]\n");
        assert!(result.is_ok(), "valid program should pass: {:?}", result);
    }

    #[test]
    fn test_undefined_variable() {
        let result = validate_source("log[x]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined variable 'x'"));
    }

    #[test]
    fn test_undefined_function() {
        let result = validate_source("y = foo[1, 2]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("undefined function 'foo'"));
    }

    #[test]
    fn test_wrong_arg_count_builtin() {
        let result = validate_source("x = min[1]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("wrong number of arguments for 'min'"));
    }

    #[test]
    fn test_wrong_arg_count_user_fn() {
        let result = validate_source("fn add[a, b]\n  return a + b\ny = add[1]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expects 2 arguments, got 1"));
    }

    #[test]
    fn test_arithmetic_type_mismatch() {
        let result = validate_source("x = true - 5\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot use '-'"));
    }

    #[test]
    fn test_string_concat_rejected() {
        let result = validate_source("x = 'hello' + ' world'\n");
        assert!(result.is_err(), "string + should be rejected");
        assert!(result.unwrap_err().contains("interpolation"), "should suggest interpolation");
    }

    #[test]
    fn test_struct_bad_field() {
        let result = validate_source("struct Point[x, y]\np = Point[1, 2]\nlog[p.z]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("has no field 'z'"));
    }

    #[test]
    fn test_struct_wrong_arg_count() {
        let result = validate_source("struct Point[x, y]\np = Point[1]\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("has 2 fields, got 1"));
    }

    #[test]
    fn test_for_loop_var_defined() {
        let result = validate_source("for i in range[5]\n  log[i]\n");
        assert!(result.is_ok(), "for loop var should be defined: {:?}", result);
    }

    #[test]
    fn test_grad_valid() {
        let result = validate_source("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
        assert!(result.is_ok(), "grad[f] should be valid: {:?}", result);
    }

    #[test]
    fn test_grad_wrong_arg_count() {
        let result = validate_source("fn f[x]\n  x * x\nfn g[x]\n  x + 1\ndf = grad[f, g]\n");
        assert!(result.is_err(), "grad should require exactly 1 arg");
    }
}
