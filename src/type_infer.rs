/// Type inference engine for the Crow -> Rust transpiler.

use std::collections::BTreeMap;
use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum CrowType {
    Int,
    Float,
    Bool,
    Str,
    Array(Box<CrowType>),
    Map(Box<CrowType>),
    Struct(String),
    /// Recursive/optional struct field: Option<Box<StructName>>
    /// Used when a struct field holds both a sentinel (e.g. 0) and a recursive struct reference.
    OptBoxStruct(String),
    Fn(Vec<CrowType>, Box<CrowType>),
    /// Error-value wrapper: CrowResult<T> with .ok, .reason, .kind, .trace
    Result(Box<CrowType>),
    /// Dynamic JSON value (serde_json::Value) — returned by json[]
    Json,
    /// Numeric array backed by CrowTensor (f64 internally). Inner type tracks original element type.
    NumArray(Box<CrowType>),
    Void,
    Unknown,
}

impl CrowType {
    pub fn to_rust_type(&self) -> String {
        match self {
            CrowType::Int => "i64".into(),
            CrowType::Float => "f64".into(),
            CrowType::Bool => "bool".into(),
            CrowType::Str => "String".into(),
            CrowType::Array(inner) => format!("Vec<{}>", inner.to_rust_type()),
            CrowType::Map(inner) => format!("std::collections::BTreeMap<String, {}>", inner.to_rust_type()),
            CrowType::Struct(name) => name.clone(),
            CrowType::OptBoxStruct(name) => format!("Option<Box<{}>>", name),
            CrowType::Fn(params, ret) => {
                let param_types: Vec<String> = params.iter().map(|p| p.to_rust_type()).collect();
                format!("fn({}) -> {}", param_types.join(", "), ret.to_rust_type())
            }
            CrowType::Result(inner) => format!("CrowResult<{}>", inner.to_rust_type()),
            CrowType::Json => "serde_json::Value".into(),
            CrowType::NumArray(_) => "crow_runtime::tensor::CrowTensor".into(),
            CrowType::Void => "()".into(),
            CrowType::Unknown => "String".into(),
        }
    }

    /// Rust expression for a default/zero value of this type.
    /// Used in auto-propagation to construct CrowResult error variants.
    pub fn default_rust_value(&self) -> String {
        match self {
            CrowType::Int => "0_i64".into(),
            CrowType::Float => "0.0_f64".into(),
            CrowType::Bool => "false".into(),
            CrowType::Str => "String::new()".into(),
            CrowType::Array(inner) => format!("Vec::<{}>::new()", inner.to_rust_type()),
            CrowType::Map(inner) => format!("std::collections::BTreeMap::<String, {}>::new()", inner.to_rust_type()),
            CrowType::Struct(name) => format!("unsafe {{ std::mem::zeroed::<{}>() }}", name),
            CrowType::OptBoxStruct(_) => "None".into(),
            CrowType::Fn(_, _) => "panic!(\"unreachable\")".into(),
            CrowType::Result(inner) => format!("CrowResult {{ ok: false, value: {}, reason: String::new(), kind: String::new(), trace: vec![] }}", inner.default_rust_value()),
            CrowType::Json => "serde_json::Value::Null".into(),
            CrowType::NumArray(_) => "crow_runtime::tensor::CrowTensor::zeros(vec![0])".into(),
            CrowType::Void => "()".into(),
            CrowType::Unknown => "String::new()".into(),
        }
    }

    /// Human-readable Crow type name for error messages.
    pub fn crow_name(&self) -> String {
        match self {
            CrowType::Int => "int".into(),
            CrowType::Float => "float".into(),
            CrowType::Bool => "bool".into(),
            CrowType::Str => "str".into(),
            CrowType::Array(inner) => format!("array[{}]", inner.crow_name()),
            CrowType::NumArray(_) => "array".into(),
            CrowType::Map(inner) => format!("map[{}]", inner.crow_name()),
            CrowType::Struct(name) => name.clone(),
            CrowType::OptBoxStruct(name) => name.clone(),
            CrowType::Fn(params, ret) => {
                let p: Vec<String> = params.iter().map(|p| p.crow_name()).collect();
                format!("fn[{}] -> {}", p.join(", "), ret.crow_name())
            }
            CrowType::Result(inner) => format!("result[{}]", inner.crow_name()),
            CrowType::Json => "json".into(),
            CrowType::Void => "void".into(),
            CrowType::Unknown => "unknown".into(),
        }
    }

    /// Check if two types are compatible for reassignment.
    /// Returns true if the same variable can hold both types.
    pub fn compatible(&self, other: &CrowType) -> bool {
        if self == other { return true; }
        match (self, other) {
            (CrowType::Unknown, _) | (_, CrowType::Unknown) => true,
            (CrowType::Void, _) | (_, CrowType::Void) => true,
            (CrowType::Int, CrowType::Float) | (CrowType::Float, CrowType::Int) => true,
            (CrowType::Array(a), CrowType::Array(b)) => a.compatible(b),
            (CrowType::Map(a), CrowType::Map(b)) => a.compatible(b),
            (CrowType::Result(a), CrowType::Result(b)) => a.compatible(b),
            // NumArray is compatible with Array of numeric types
            (CrowType::NumArray(_), CrowType::Array(inner)) | (CrowType::Array(inner), CrowType::NumArray(_)) => {
                matches!(inner.as_ref(), CrowType::Int | CrowType::Float | CrowType::Unknown)
            }
            (CrowType::NumArray(_), CrowType::NumArray(_)) => true,
            (CrowType::Int, CrowType::Struct(_)) | (CrowType::Struct(_), CrowType::Int) => true, // OptBoxStruct
            (CrowType::OptBoxStruct(_), CrowType::Int) | (CrowType::Int, CrowType::OptBoxStruct(_)) => true,
            (CrowType::OptBoxStruct(_), CrowType::Struct(_)) | (CrowType::Struct(_), CrowType::OptBoxStruct(_)) => true,
            _ => false,
        }
    }

    pub fn unify(&self, other: &CrowType) -> CrowType {
        if self == other { return self.clone(); }
        match (self, other) {
            (CrowType::Unknown, t) | (t, CrowType::Unknown) => t.clone(),
            (CrowType::Int, CrowType::Float) | (CrowType::Float, CrowType::Int) => CrowType::Float,
            (CrowType::NumArray(a), CrowType::NumArray(b)) => CrowType::NumArray(Box::new(a.unify(b))),
            (CrowType::Array(a), CrowType::Array(b)) => CrowType::Array(Box::new(a.unify(b))),
            (CrowType::Map(a), CrowType::Map(b)) => CrowType::Map(Box::new(a.unify(b))),
            (CrowType::Void, t) | (t, CrowType::Void) => t.clone(),
            // Int + Struct => OptBoxStruct (recursive struct with sentinel)
            (CrowType::Int, CrowType::Struct(name)) | (CrowType::Struct(name), CrowType::Int) => {
                CrowType::OptBoxStruct(name.clone())
            }
            // OptBoxStruct stays OptBoxStruct
            (CrowType::OptBoxStruct(name), CrowType::Int) | (CrowType::Int, CrowType::OptBoxStruct(name)) => {
                CrowType::OptBoxStruct(name.clone())
            }
            (CrowType::OptBoxStruct(name), CrowType::Struct(_)) | (CrowType::Struct(_), CrowType::OptBoxStruct(name)) => {
                CrowType::OptBoxStruct(name.clone())
            }
            // Result unification
            (CrowType::Result(a), CrowType::Result(b)) => CrowType::Result(Box::new(a.unify(b))),
            // Result(T) + U => Result(U): the non-error return tells us the actual value type
            (CrowType::Result(_), other) => CrowType::Result(Box::new(other.clone())),
            (other, CrowType::Result(_)) => CrowType::Result(Box::new(other.clone())),
            // Fn unification
            (CrowType::Fn(p1, r1), CrowType::Fn(p2, r2)) if p1.len() == p2.len() => {
                let params: Vec<CrowType> = p1.iter().zip(p2.iter()).map(|(a, b)| a.unify(b)).collect();
                let ret = r1.unify(r2);
                CrowType::Fn(params, Box::new(ret))
            }
            _ => self.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<(String, CrowType)>,
    pub return_type: CrowType,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct StructSig {
    pub fields: Vec<(String, CrowType)>,
}

#[derive(Debug, Clone)]
pub struct TypeContext {
    pub vars: BTreeMap<String, CrowType>,
    pub functions: BTreeMap<String, FnSig>,
    pub structs: BTreeMap<String, StructSig>,
    pub cpu_count: Option<usize>,
    /// Param names of the function currently being body-scanned (to avoid
    /// incorrectly inferring [] indexing as a function call on params).
    current_fn_params: Vec<String>,
}

impl TypeContext {
    pub fn new() -> Self {
        TypeContext {
            vars: BTreeMap::new(),
            functions: BTreeMap::new(),
            structs: BTreeMap::new(),
            cpu_count: None,
            current_fn_params: Vec::new(),
        }
    }

    /// Main entry point: run multi-pass type inference on a program.
    pub fn infer_program(&mut self, stmts: &[Stmt]) {
        // Pass 1: collect struct and function declarations
        for stmt in stmts {
            match stmt {
                Stmt::StructDecl { name, fields } => {
                    self.structs.insert(name.clone(), StructSig {
                        fields: fields.iter().map(|f| (f.clone(), CrowType::Unknown)).collect(),
                    });
                }
                Stmt::FnDecl { name, params, body } => {
                    self.functions.insert(name.clone(), FnSig {
                        params: params.iter().map(|p| (p.clone(), CrowType::Unknown)).collect(),
                        return_type: CrowType::Unknown,
                        body: body.clone(),
                    });
                }
                Stmt::Resource { kind, count } => {
                    if kind == "cpu" {
                        if let Expr::Int(n) = count {
                            self.cpu_count = Some(*n as usize);
                        }
                    }
                }
                _ => {}
            }
        }

        // Pass 2+: iterate call-site scanning and function body inference
        // until types stabilize (handles cross-function type propagation)
        for _round in 0..5 {
            // Scan call sites with function-local variable context
            for stmt in stmts {
                self.collect_call_types_stmt(stmt);
            }

            // Propagate types from indirect calls back to original functions
            self.propagate_indirect_fn_types(stmts);

            // Infer function bodies and return types
            let fn_entries: Vec<(String, Vec<Stmt>)> = self.functions.iter()
                .map(|(n, sig)| (n.clone(), sig.body.clone()))
                .collect();
            for (name, body) in &fn_entries {
                self.infer_function_body(name, body);
            }

        }

        // Pass 4: infer top-level variables
        for stmt in stmts {
            self.infer_stmt(stmt);
        }
    }

    /// Propagate types from indirect function calls back to original function declarations.
    /// When we see `op = double` and `op[5]`, propagate i64 back to double's param.
    fn propagate_indirect_fn_types(&mut self, stmts: &[Stmt]) {
        // Collect var -> function name mappings from assignments
        let mut var_to_fn: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        self.collect_var_fn_assignments(stmts, &mut var_to_fn);

        // For each variable that holds a function, if the variable has a Fn type
        // with concrete params, propagate those to the original function
        for (var_name, fn_names) in &var_to_fn {
            if let Some(var_type) = self.vars.get(var_name).cloned() {
                if let CrowType::Fn(param_types, ret_type) = &var_type {
                    for fn_name in fn_names {
                        if let Some(sig) = self.functions.get(fn_name).cloned() {
                            let mut new_params = sig.params.clone();
                            for (i, pt) in param_types.iter().enumerate() {
                                if i < new_params.len() {
                                    new_params[i].1 = new_params[i].1.unify(pt);
                                }
                            }
                            let new_ret = sig.return_type.unify(ret_type);
                            let target = self.functions.get_mut(fn_name).unwrap();
                            target.params = new_params;
                            target.return_type = new_ret;
                        }
                    }
                }
            }
        }
    }

    /// Collect assignments of function names to variables.
    fn collect_var_fn_assignments(&self, stmts: &[Stmt], map: &mut std::collections::BTreeMap<String, Vec<String>>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, value } => {
                    if let Expr::Ident(fn_name) = value {
                        if self.functions.contains_key(fn_name) {
                            map.entry(name.clone()).or_default().push(fn_name.clone());
                        }
                    }
                }
                Stmt::FnDecl { body, .. } => {
                    self.collect_var_fn_assignments(body, map);
                }
                Stmt::If { then_body, else_body, .. } => {
                    self.collect_var_fn_assignments(then_body, map);
                    if let Some(eb) = else_body {
                        self.collect_var_fn_assignments(eb, map);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    self.collect_var_fn_assignments(body, map);
                }
                _ => {}
            }
        }
    }


    /// Scan a statement for call sites to infer parameter types.
    fn collect_call_types_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                // Track local variable types during call-site scanning
                let ty = self.infer_expr(value);
                let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                self.vars.insert(name.clone(), existing.unify(&ty));
                self.collect_call_types_expr(value);
            }
            Stmt::MultiAssign { names, values, .. } => {
                for (name, value) in names.iter().zip(values.iter()) {
                    let ty = self.infer_expr(value);
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    self.vars.insert(name.clone(), existing.unify(&ty));
                }
                for v in values { self.collect_call_types_expr(v); }
            }
            Stmt::FnDecl { name, body, .. } => {
                // Add function params as known vars before scanning body
                let saved_vars = self.vars.clone();
                let saved_fn_params = self.current_fn_params.clone();
                if let Some(sig) = self.functions.get(name) {
                    self.current_fn_params = sig.params.iter().map(|(n, _)| n.clone()).collect();
                    for (pname, pty) in &sig.params.clone() {
                        self.vars.insert(pname.clone(), pty.clone());
                    }
                }
                for s in body { self.collect_call_types_stmt(s); }

                // After scanning body, propagate any updated Fn-typed param types back.
                // This handles `f[result, array[i]]` where `f` is a known function param.
                if let Some(sig) = self.functions.get(name).cloned() {
                    let mut new_params = sig.params.clone();
                    for (i, (pname, _pty)) in sig.params.iter().enumerate() {
                        if let Some(updated_type) = self.vars.get(pname) {
                            if let CrowType::Fn(_, _) = updated_type {
                                new_params[i].1 = new_params[i].1.unify(updated_type);
                            }
                        }
                    }
                    self.functions.get_mut(name).unwrap().params = new_params;
                }

                self.current_fn_params = saved_fn_params;
                self.vars = saved_vars;
            }
            Stmt::Return(Some(e)) => self.collect_call_types_expr(e),
            Stmt::ExprStmt(e) => self.collect_call_types_expr(e),
            Stmt::If { condition, then_body, else_body } => {
                self.collect_call_types_expr(condition);
                for s in then_body { self.collect_call_types_stmt(s); }
                if let Some(eb) = else_body {
                    for s in eb { self.collect_call_types_stmt(s); }
                }
            }
            Stmt::For { var, iter, body } => {
                self.collect_call_types_expr(iter);
                // Infer loop variable type from iterator
                let iter_type = self.infer_expr(iter);
                match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => {
                        self.vars.insert(var.clone(), *inner.clone());
                    }
                    _ => {}
                }
                for s in body { self.collect_call_types_stmt(s); }
            }
            Stmt::While { condition, body } => {
                self.collect_call_types_expr(condition);
                for s in body { self.collect_call_types_stmt(s); }
            }
            Stmt::Match { value, arms } => {
                self.collect_call_types_expr(value);
                for arm in arms {
                    for s in &arm.body { self.collect_call_types_stmt(s); }
                }
            }
            Stmt::FieldAssign { object, value, .. } => {
                self.collect_call_types_expr(object);
                self.collect_call_types_expr(value);
            }
            Stmt::IndexAssign { object, index, value } => {
                self.collect_call_types_expr(object);
                self.collect_call_types_expr(index);
                self.collect_call_types_expr(value);
            }
            _ => {}
        }
    }

    /// Scan an expression for call sites; update function param types and struct field types.
    fn collect_call_types_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    let arg_types: Vec<CrowType> = args.iter()
                        .map(|a| self.infer_expr(a))
                        .collect();

                    // Is it a function call?
                    if self.functions.contains_key(name) {
                        let sig = self.functions.get(name).unwrap().clone();
                        let mut new_params = sig.params.clone();
                        for (i, at) in arg_types.iter().enumerate() {
                            if i < new_params.len() {
                                // Normalize types for function params:
                                // - OptBoxStruct -> Struct (transpiler unwraps before passing)
                                // - Result<T> -> T (auto-propagation unwraps at call site)
                                let normalized = match at {
                                    CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                    CrowType::Result(inner) => *inner.clone(),
                                    other => other.clone(),
                                };
                                new_params[i].1 = new_params[i].1.unify(&normalized);
                            }
                        }
                        self.functions.get_mut(name).unwrap().params = new_params;

                        // Propagate types to functions passed as arguments:
                        // If param type is Fn(...) and arg is a function name, update that function's signature
                        let updated_sig = self.functions.get(name).unwrap().clone();
                        for (i, arg) in args.iter().enumerate() {
                            if i < updated_sig.params.len() {
                                if let CrowType::Fn(param_types, ret_type) = &updated_sig.params[i].1 {
                                    if let Expr::Ident(fn_name) = arg {
                                        if self.functions.contains_key(fn_name) {
                                            let target_sig = self.functions.get(fn_name).unwrap().clone();
                                            let mut new_target_params = target_sig.params.clone();
                                            for (j, pt) in param_types.iter().enumerate() {
                                                if j < new_target_params.len() {
                                                    new_target_params[j].1 = new_target_params[j].1.unify(pt);
                                                }
                                            }
                                            let new_ret = target_sig.return_type.unify(ret_type);
                                            let target = self.functions.get_mut(fn_name).unwrap();
                                            target.params = new_target_params;
                                            target.return_type = new_ret;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Is it a struct constructor?
                    if self.structs.contains_key(name) {
                        let sig = self.structs.get(name).unwrap().clone();
                        let mut new_fields = sig.fields.clone();
                        for (i, at) in arg_types.iter().enumerate() {
                            if i < new_fields.len() {
                                new_fields[i].1 = new_fields[i].1.unify(at);
                            }
                        }
                        self.structs.get_mut(name).unwrap().fields = new_fields;
                    }

                    // Is it a variable holding a function? (not a declared function or struct)
                    // Only update Fn type — don't override concrete types like Array/Map.
                    // For function params with Unknown type, skip — [] is ambiguous (call vs index)
                    // and the call site will provide the actual type.
                    // For params already known to be Fn, allow refinement of arg types.
                    if !self.functions.contains_key(name) && !self.structs.contains_key(name) {
                        let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                        let is_param = self.current_fn_params.contains(name);
                        match &existing {
                            CrowType::Fn(_, _) => {
                                // Already known to be Fn — refine arg types
                                let fn_type = CrowType::Fn(arg_types.clone(), Box::new(CrowType::Unknown));
                                let unified = existing.unify(&fn_type);
                                self.vars.insert(name.clone(), unified);
                            }
                            CrowType::Unknown if !is_param => {
                                // Non-param Unknown variable called with [] — assume it's a function
                                let fn_type = CrowType::Fn(arg_types, Box::new(CrowType::Unknown));
                                self.vars.insert(name.clone(), fn_type);
                            }
                            _ => {} // Array, Map, param with Unknown — [] means indexing
                        }
                    }
                }
                // Module function/struct call: module.func[args] or module.Struct[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(mod_name) = object.as_ref() {
                        let prefixed = format!("{}_{}", mod_name, field);
                        if self.functions.contains_key(&prefixed) {
                            let arg_types: Vec<CrowType> = args.iter()
                                .map(|a| self.infer_expr(a))
                                .collect();
                            let sig = self.functions.get(&prefixed).unwrap().clone();
                            let mut new_params = sig.params.clone();
                            for (i, at) in arg_types.iter().enumerate() {
                                if i < new_params.len() {
                                    let normalized = match at {
                                        CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                        other => other.clone(),
                                    };
                                    new_params[i].1 = new_params[i].1.unify(&normalized);
                                }
                            }
                            self.functions.get_mut(&prefixed).unwrap().params = new_params;
                        }
                        // Module struct constructor: update field types from args
                        if self.structs.contains_key(&prefixed) {
                            let arg_types: Vec<CrowType> = args.iter()
                                .map(|a| self.infer_expr(a))
                                .collect();
                            let sig = self.structs.get(&prefixed).unwrap().clone();
                            let mut new_fields = sig.fields.clone();
                            for (i, at) in arg_types.iter().enumerate() {
                                if i < new_fields.len() {
                                    new_fields[i].1 = new_fields[i].1.unify(at);
                                }
                            }
                            self.structs.get_mut(&prefixed).unwrap().fields = new_fields;
                        }
                    }
                }
                // Recurse into args
                for a in args { self.collect_call_types_expr(a); }
            }
            Expr::BinOp { left, right, .. } => {
                self.collect_call_types_expr(left);
                self.collect_call_types_expr(right);
            }
            Expr::UnaryOp { expr: e, .. } => self.collect_call_types_expr(e),
            Expr::FieldAccess { object, .. } => self.collect_call_types_expr(object),
            Expr::Array(items) => {
                for i in items { self.collect_call_types_expr(i); }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.collect_call_types_expr(k);
                    self.collect_call_types_expr(v);
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.collect_call_types_expr(iter);
                // Infer loop variable type from iterator
                let iter_type = self.infer_expr(iter);
                match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => {
                        self.vars.insert(var.clone(), *inner.clone());
                    }
                    _ => {}
                }
                for s in body { self.collect_call_types_stmt(s); }
            }
            Expr::Str(s) => {
                // Parse string interpolations to discover embedded call sites
                self.collect_call_types_in_string(s);
            }
            _ => {}
        }
    }

    /// Parse string interpolations and collect call types from embedded expressions.
    fn collect_call_types_in_string(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{' {
                let start = i + 1;
                let mut depth = 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' { depth += 1; }
                    else if chars[i] == '}' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                let expr_str: String = chars[start..i].iter().collect();
                i += 1;

                // Parse and collect call types from the embedded expression
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                if let Ok(tokens) = lexer.tokenize() {
                    let mut parser = crate::parser::Parser::new(tokens);
                    if let Ok(stmts) = parser.parse() {
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, Stmt::Line(_))) {
                            self.collect_call_types_expr(expr);
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Infer a function's return type from its body.
    fn infer_function_body(&mut self, name: &str, body: &[Stmt]) {
        // Temporarily add param types as vars so body inference works
        let saved_vars = self.vars.clone();
        if let Some(sig) = self.functions.get(name) {
            for (pname, pty) in &sig.params {
                self.vars.insert(pname.clone(), pty.clone());
            }
        }

        // Infer body statements
        for stmt in body {
            self.infer_stmt(stmt);
        }

        // Find return type
        let ret = self.find_return_type(body);
        if let Some(sig) = self.functions.get_mut(name) {
            sig.return_type = sig.return_type.unify(&ret);
        }

        self.vars = saved_vars;
    }

    /// Find the return type from a list of statements.
    fn find_return_type(&self, stmts: &[Stmt]) -> CrowType {
        let mut ret = CrowType::Void;
        for stmt in stmts {
            match stmt {
                Stmt::Return(Some(e)) => {
                    ret = ret.unify(&self.infer_expr(e));
                }
                Stmt::Return(None) => {
                    ret = ret.unify(&CrowType::Void);
                }
                Stmt::If { then_body, else_body, .. } => {
                    let t = self.find_return_type(then_body);
                    ret = ret.unify(&t);
                    if let Some(eb) = else_body {
                        let e = self.find_return_type(eb);
                        ret = ret.unify(&e);
                    }
                }
                Stmt::While { body, .. } => {
                    let w = self.find_return_type(body);
                    ret = ret.unify(&w);
                }
                Stmt::For { body, .. } => {
                    let f = self.find_return_type(body);
                    ret = ret.unify(&f);
                }
                _ => {}
            }
        }
        ret
    }

    /// Infer types from a statement (mainly assignments).
    fn infer_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                let ty = self.infer_expr(value);
                let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                self.vars.insert(name.clone(), existing.unify(&ty));
            }
            Stmt::MultiAssign { names, values } => {
                for (name, value) in names.iter().zip(values.iter()) {
                    let ty = self.infer_expr(value);
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    self.vars.insert(name.clone(), existing.unify(&ty));
                }
            }
            Stmt::If { then_body, else_body, .. } => {
                for s in then_body { self.infer_stmt(s); }
                if let Some(eb) = else_body {
                    for s in eb { self.infer_stmt(s); }
                }
            }
            Stmt::While { body, .. } => {
                for s in body { self.infer_stmt(s); }
            }
            Stmt::For { var, iter, body } => {
                let iter_type = self.infer_expr(iter);
                match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => {
                        self.vars.insert(var.clone(), *inner.clone());
                    }
                    _ => {}
                }
                for s in body { self.infer_stmt(s); }
            }
            Stmt::ExprStmt(Expr::Call { callee, args }) => {
                // Propagate value type from set[map_var, key, val] to the map variable
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "set" && args.len() == 3 {
                        if let Expr::Ident(var_name) = &args[0] {
                            let val_type = self.infer_expr(&args[2]);
                            let map_type = CrowType::Map(Box::new(val_type));
                            let existing = self.vars.get(var_name).cloned().unwrap_or(CrowType::Unknown);
                            self.vars.insert(var_name.clone(), existing.unify(&map_type));
                        }
                    }
                }
            }
            Stmt::FieldAssign { value, .. } => {
                // Just infer the value expression for side effects
                self.infer_expr(value);
            }
            Stmt::IndexAssign { object, value, .. } => {
                // Infer value type and update the collection's element type
                let val_type = self.infer_expr(value);
                let obj_type = self.infer_expr(object);
                // If assigning to a map, unify value type
                if let Expr::Ident(var_name) = object {
                    match &obj_type {
                        CrowType::Map(inner) => {
                            let new_inner = inner.unify(&val_type);
                            self.vars.insert(var_name.clone(), CrowType::Map(Box::new(new_inner)));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Infer the base return type of a named call (function or builtin),
    /// without auto-propagation wrapping.
    pub fn infer_call_base(&self, name: &str, args: &[Expr], _callee: &Expr) -> CrowType {
        // Struct constructor
        if self.structs.contains_key(name) {
            return CrowType::Struct(name.to_string());
        }
        // User-defined function call
        if let Some(sig) = self.functions.get(name) {
            return sig.return_type.clone();
        }
        // Builtins (check before vars to prevent builtins from being shadowed by Fn inference)
        match name {
            "log" | "cpu" | "gpu" | "assert" => CrowType::Void,
            "type" => CrowType::Str,
            "error" => CrowType::Result(Box::new(CrowType::Str)),
            "map" => CrowType::Map(Box::new(CrowType::Unknown)),
            "range" => CrowType::Array(Box::new(CrowType::Int)),
            // Math builtins
            "abs" => {
                if let Some(arg) = args.first() {
                    let t = self.infer_expr(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Int
            }
            "sqrt" => CrowType::Float,
            "floor" | "ceil" | "round" => CrowType::Int,
            "pow" | "min" | "max" => {
                if args.len() >= 2 {
                    let t1 = self.infer_expr(&args[0]);
                    let t2 = self.infer_expr(&args[1]);
                    let t1u = if let CrowType::Result(inner) = t1 { *inner } else { t1 };
                    let t2u = if let CrowType::Result(inner) = t2 { *inner } else { t2 };
                    if t1u == CrowType::Float || t2u == CrowType::Float {
                        return CrowType::Float;
                    }
                    return t1u;
                }
                CrowType::Int
            }
            // String builtins
            "trim" | "upper" | "lower" | "replace" | "join" | "repeat" | "pad" => CrowType::Str,
            "split" => CrowType::Array(Box::new(CrowType::Str)),
            "starts" | "ends" | "contains" => CrowType::Bool,
            // Conversion builtins — fallible when parsing from string
            "int" => {
                if let Some(arg) = args.first() {
                    let arg_type = self.infer_expr(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str) {
                        return CrowType::Result(Box::new(CrowType::Int));
                    }
                }
                CrowType::Int
            }
            "float" => {
                if let Some(arg) = args.first() {
                    let arg_type = self.infer_expr(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str) {
                        return CrowType::Result(Box::new(CrowType::Float));
                    }
                }
                CrowType::Float
            }
            "str" => CrowType::Str,
            // Immutable collection builtins (return same type as first arg)
            "push" => {
                // push[array, val] returns Array/NumArray with element type from val
                if let Some(arg) = args.first() {
                    let arr_type = self.infer_expr(arg);
                    if args.len() == 2 {
                        let val_type = self.infer_expr(&args[1]);
                        let existing_inner = match &arr_type {
                            CrowType::Array(inner) => *inner.clone(),
                            CrowType::NumArray(inner) => *inner.clone(),
                            _ => CrowType::Unknown,
                        };
                        let unified = existing_inner.unify(&val_type);
                        // Preserve NumArray if input was NumArray and new value is numeric
                        if matches!(&arr_type, CrowType::NumArray(_)) && matches!(unified, CrowType::Int | CrowType::Float) {
                            return CrowType::NumArray(Box::new(unified));
                        }
                        return CrowType::Array(Box::new(unified));
                    }
                    return arr_type;
                }
                CrowType::Unknown
            }
            "pop" | "sort" | "reverse" | "append" => {
                if let Some(arg) = args.first() {
                    return self.infer_expr(arg);
                }
                CrowType::Unknown
            }
            "set" => {
                // set[map, key, val] returns Map with value type from val arg
                if let Some(arg) = args.first() {
                    let map_type = self.infer_expr(arg);
                    if args.len() == 3 {
                        let val_type = self.infer_expr(&args[2]);
                        let existing_inner = if let CrowType::Map(inner) = &map_type {
                            *inner.clone()
                        } else {
                            CrowType::Unknown
                        };
                        return CrowType::Map(Box::new(existing_inner.unify(&val_type)));
                    }
                    return map_type;
                }
                CrowType::Unknown
            }
            "del" => {
                if let Some(arg) = args.first() {
                    return self.infer_expr(arg);
                }
                CrowType::Unknown
            }
            // Map builtins
            "keys" => CrowType::Array(Box::new(CrowType::Str)),
            "has" => CrowType::Bool,
            "values" => {
                if let Some(arg) = args.first() {
                    let map_type = self.infer_expr(arg);
                    if let CrowType::Map(inner) = map_type {
                        return CrowType::Array(inner);
                    }
                }
                CrowType::Array(Box::new(CrowType::Unknown))
            }
            "slice" => {
                if let Some(arg) = args.first() {
                    let t = self.infer_expr(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Unknown
            }
            "len" => CrowType::Int,
            // CLI / IO
            "args" => CrowType::Array(Box::new(CrowType::Str)),
            "input" => CrowType::Result(Box::new(CrowType::Str)),
            // File I/O
            "read" => CrowType::Result(Box::new(CrowType::Str)),
            "write" => CrowType::Result(Box::new(CrowType::Str)),
            // System builtins
            "rand" => {
                if args.is_empty() { CrowType::Float } else { CrowType::Int }
            }
            "time" => CrowType::Float,
            "sleep" => CrowType::Int,
            "env" => CrowType::Str,
            "exit" => CrowType::Int,
            // JSON builtins
            "json" => CrowType::Result(Box::new(CrowType::Json)),
            "encode" => CrowType::Result(Box::new(CrowType::Str)),
            // Autograd
            "grad" => {
                // grad[f] returns a function with the same arity as f, but all params are Float
                // and return is Float (1 param) or Array<Float> (N params)
                if let Some(Expr::Ident(fn_name)) = args.first() {
                    if let Some(sig) = self.functions.get(fn_name) {
                        let n = sig.params.len();
                        let param_types = vec![CrowType::Float; n];
                        let ret = if n == 1 {
                            CrowType::Float
                        } else {
                            CrowType::Array(Box::new(CrowType::Float))
                        };
                        return CrowType::Fn(param_types, Box::new(ret));
                    }
                }
                CrowType::Fn(vec![CrowType::Float], Box::new(CrowType::Float))
            }
            _ => {
                // Check variable type: could be a function call, array/map indexing, etc.
                if let Some(var_type) = self.vars.get(name) {
                    match var_type {
                        CrowType::Fn(_, ret) => return *ret.clone(),
                        CrowType::Array(inner) => return *inner.clone(),
                        CrowType::NumArray(inner) => return *inner.clone(),
                        CrowType::Map(inner) => return *inner.clone(),
                        CrowType::Str => return CrowType::Str,
                        _ => {}
                    }
                }
                CrowType::Unknown
            },
        }
    }

    /// Infer the type of an expression.
    pub fn infer_expr(&self, expr: &Expr) -> CrowType {
        match expr {
            Expr::Int(_) => CrowType::Int,
            Expr::Float(_) => CrowType::Float,
            Expr::Bool(_) => CrowType::Bool,
            Expr::Str(_) => CrowType::Str,
            Expr::Ident(name) => {
                // Check local vars first
                if let Some(ty) = self.vars.get(name) {
                    return ty.clone();
                }
                // Check if it's a known function name — return as CrowType::Fn
                if let Some(sig) = self.functions.get(name) {
                    let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                    return CrowType::Fn(param_types, Box::new(sig.return_type.clone()));
                }
                CrowType::Unknown
            }
            Expr::BinOp { left, op, right } => {
                let lt = self.infer_expr(left);
                let rt = self.infer_expr(right);
                // Check if either operand is CrowResult (for auto-propagation)
                let has_result = matches!(&lt, CrowType::Result(_)) || matches!(&rt, CrowType::Result(_));
                // Unwrap to get inner types for determining result type
                let lt_inner = if let CrowType::Result(inner) = &lt { *inner.clone() } else { lt };
                let rt_inner = if let CrowType::Result(inner) = &rt { *inner.clone() } else { rt };
                let result_type = match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
                    | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => CrowType::Bool,
                    BinOp::Mod => CrowType::Int,
                    _ => lt_inner.unify(&rt_inner),
                };
                // If either operand was CrowResult, wrap the result type
                if has_result {
                    CrowType::Result(Box::new(result_type))
                } else {
                    result_type
                }
            }
            Expr::UnaryOp { op, expr: e } => {
                match op {
                    UnaryOp::Not => CrowType::Bool,
                    UnaryOp::Neg => self.infer_expr(e),
                }
            }
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    let base = self.infer_call_base(name, args, callee);
                    // Auto-propagation: wrap in CrowResult if any arg is CrowResult
                    let skip = matches!(name.as_str(), "log" | "assert" | "type" | "cpu" | "gpu" | "error");
                    if !skip && !matches!(base, CrowType::Result(_) | CrowType::Void) {
                        let has_result_arg = args.iter().any(|a| matches!(self.infer_expr(a), CrowType::Result(_)));
                        if has_result_arg {
                            return CrowType::Result(Box::new(base));
                        }
                    }
                    return base;
                }
                // Built-in math module: math.func[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(mod_name) = object.as_ref() {
                        if mod_name == "math" {
                            return match field.as_str() {
                                // Tensor creation → NumArray(Float)
                                "zeros" | "ones" | "eye" | "rand" | "randn" | "linspace"
                                | "arange" | "full" | "diag" | "tensor" => CrowType::NumArray(Box::new(CrowType::Float)),
                                // Element-wise → NumArray(Float)
                                "exp" | "log" | "sqrt" | "sin" | "cos" | "abs" => CrowType::NumArray(Box::new(CrowType::Float)),
                                // Shape ops → NumArray(Float)
                                "reshape" | "flatten" | "transpose" => CrowType::NumArray(Box::new(CrowType::Float)),
                                // Linalg returning tensor
                                "matmul" | "solve" | "inv" | "cholesky" => CrowType::NumArray(Box::new(CrowType::Float)),
                                // Linalg returning scalar
                                "det" => CrowType::Float,
                                // Reductions returning scalar
                                "sum" | "mean" | "min" | "max" => CrowType::Float,
                                // Decompositions returning arrays of tensors
                                "svd" | "qr" | "eigen" => CrowType::Array(Box::new(CrowType::NumArray(Box::new(CrowType::Float)))),
                                _ => CrowType::Unknown,
                            };
                        }
                    }
                }
                // Module call: module.func[args] or module.Struct[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(_mod_name) = object.as_ref() {
                        let prefixed = format!("{}_{}", _mod_name, field);
                        // Module struct constructor
                        if self.structs.contains_key(&prefixed) {
                            return CrowType::Struct(prefixed);
                        }
                        // Module function call
                        if let Some(sig) = self.functions.get(&prefixed) {
                            return sig.return_type.clone();
                        }
                    }
                }
                // Array/Map/Json indexing or indirect function call
                let callee_type = self.infer_expr(callee);
                match callee_type {
                    CrowType::Array(inner) => *inner,
                    CrowType::NumArray(inner) => *inner,
                    CrowType::Map(inner) => *inner,
                    CrowType::Json => CrowType::Json,
                    CrowType::Fn(_, ret) => *ret,
                    _ => CrowType::Unknown,
                }
            }
            Expr::FieldAccess { object, field } => {
                let obj_type = self.infer_expr(object);
                // Error-value metadata fields
                match field.as_str() {
                    "ok" => return CrowType::Bool,
                    "reason" | "kind" => return CrowType::Str,
                    "trace" => return CrowType::Array(Box::new(CrowType::Str)),
                    _ => {}
                }
                if let CrowType::Struct(name) = obj_type {
                    if let Some(sig) = self.structs.get(&name) {
                        for (fname, ftype) in &sig.fields {
                            if fname == field {
                                return ftype.clone();
                            }
                        }
                    }
                }
                // Module variable access: module.var → look up module_var
                if let Expr::Ident(mod_name) = object.as_ref() {
                    let prefixed = format!("{}_{}", mod_name, field);
                    if let Some(t) = self.vars.get(&prefixed) {
                        return t.clone();
                    }
                    // Also check if it's a module function reference
                    if let Some(sig) = self.functions.get(&prefixed) {
                        let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                        return CrowType::Fn(param_types, Box::new(sig.return_type.clone()));
                    }
                }
                CrowType::Unknown
            }
            Expr::Array(items) => {
                if items.is_empty() {
                    CrowType::Array(Box::new(CrowType::Unknown))
                } else {
                    let mut inner = CrowType::Unknown;
                    for item in items {
                        inner = inner.unify(&self.infer_expr(item));
                    }
                    // All-numeric array literals become NumArray (backed by CrowTensor)
                    if matches!(inner, CrowType::Int | CrowType::Float) {
                        CrowType::NumArray(Box::new(inner))
                    } else {
                        CrowType::Array(Box::new(inner))
                    }
                }
            }
            Expr::Map(pairs) => {
                if pairs.is_empty() {
                    CrowType::Map(Box::new(CrowType::Unknown))
                } else {
                    let mut vtype = CrowType::Unknown;
                    for (_, v) in pairs {
                        vtype = vtype.unify(&self.infer_expr(v));
                    }
                    CrowType::Map(Box::new(vtype))
                }
            }
            Expr::ForExpr { var: _, iter, body } => {
                // Determine element type from iterator
                let iter_type = self.infer_expr(iter);
                let _elem_type = match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };

                // Try to find the return type from the body's last expression
                let last_type = if let Some(last_stmt) = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
                    match last_stmt {
                        Stmt::ExprStmt(e) => self.infer_expr(e),
                        _ => CrowType::Unknown,
                    }
                } else {
                    CrowType::Unknown
                };

                // If the last expression is still unknown, try element type
                let result_elem = if last_type == CrowType::Unknown {
                    _elem_type
                } else {
                    last_type
                };

                CrowType::Array(Box::new(result_elem))
            }
        }
    }
}

