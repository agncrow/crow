/// Type inference engine for the Crow -> Rust transpiler.

use std::collections::BTreeMap;
use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum CrowType {
    Int,
    Float,
    Bool,
    Str,
    List(Box<CrowType>),
    Map(Box<CrowType>),
    Struct(String),
    /// Recursive/optional struct field: Option<Box<StructName>>
    /// Used when a struct field holds both a sentinel (e.g. 0) and a recursive struct reference.
    OptBoxStruct(String),
    Fn(Vec<CrowType>, Box<CrowType>),
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
            CrowType::List(inner) => format!("Vec<{}>", inner.to_rust_type()),
            CrowType::Map(inner) => format!("std::collections::BTreeMap<String, {}>", inner.to_rust_type()),
            CrowType::Struct(name) => name.clone(),
            CrowType::OptBoxStruct(name) => format!("Option<Box<{}>>", name),
            CrowType::Fn(_, _) => "fn".into(),
            CrowType::Void => "()".into(),
            CrowType::Unknown => "CrowVal".into(),
        }
    }

    pub fn unify(&self, other: &CrowType) -> CrowType {
        if self == other { return self.clone(); }
        match (self, other) {
            (CrowType::Unknown, t) | (t, CrowType::Unknown) => t.clone(),
            (CrowType::Int, CrowType::Float) | (CrowType::Float, CrowType::Int) => CrowType::Float,
            (CrowType::List(a), CrowType::List(b)) => CrowType::List(Box::new(a.unify(b))),
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
}

impl TypeContext {
    pub fn new() -> Self {
        TypeContext {
            vars: BTreeMap::new(),
            functions: BTreeMap::new(),
            structs: BTreeMap::new(),
            cpu_count: None,
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

        // Pass 2+3: iterate call-site scanning and function body inference
        // until types stabilize (handles cross-function type propagation)
        for _round in 0..3 {
            // Scan call sites with function-local variable context
            for stmt in stmts {
                self.collect_call_types_stmt(stmt);
            }

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
                if let Some(sig) = self.functions.get(name) {
                    for (pname, pty) in &sig.params.clone() {
                        self.vars.insert(pname.clone(), pty.clone());
                    }
                }
                for s in body { self.collect_call_types_stmt(s); }
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
                if let CrowType::List(inner) = &iter_type {
                    self.vars.insert(var.clone(), *inner.clone());
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
                                // Normalize OptBoxStruct -> Struct for function params,
                                // because the transpiler unwraps OptBoxStruct args before passing.
                                let normalized = match at {
                                    CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                    other => other.clone(),
                                };
                                new_params[i].1 = new_params[i].1.unify(&normalized);
                            }
                        }
                        self.functions.get_mut(name).unwrap().params = new_params;
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
            Expr::List(items) => {
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
                if let CrowType::List(inner) = &iter_type {
                    self.vars.insert(var.clone(), *inner.clone());
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
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.first() {
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
                if let CrowType::List(inner) = &iter_type {
                    self.vars.insert(var.clone(), *inner.clone());
                }
                for s in body { self.infer_stmt(s); }
            }
            _ => {}
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
                self.vars.get(name).cloned().unwrap_or(CrowType::Unknown)
            }
            Expr::BinOp { left, op, right } => {
                let lt = self.infer_expr(left);
                let rt = self.infer_expr(right);
                match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
                    | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => CrowType::Bool,
                    BinOp::Mod => CrowType::Int,
                    _ => lt.unify(&rt),
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
                    // Struct constructor
                    if self.structs.contains_key(name) {
                        return CrowType::Struct(name.clone());
                    }
                    // Function call
                    if let Some(sig) = self.functions.get(name) {
                        return sig.return_type.clone();
                    }
                    // Builtins
                    match name.as_str() {
                        "log" | "cpu" | "gpu" => return CrowType::Void,
                        "map" => return CrowType::List(Box::new(CrowType::Unknown)),
                        _ => {}
                    }
                }
                // List/Map indexing
                let callee_type = self.infer_expr(callee);
                match callee_type {
                    CrowType::List(inner) => *inner,
                    CrowType::Map(inner) => *inner,
                    _ => CrowType::Unknown,
                }
            }
            Expr::FieldAccess { object, field } => {
                let obj_type = self.infer_expr(object);
                if let CrowType::Struct(name) = obj_type {
                    if let Some(sig) = self.structs.get(&name) {
                        for (fname, ftype) in &sig.fields {
                            if fname == field {
                                return ftype.clone();
                            }
                        }
                    }
                }
                CrowType::Unknown
            }
            Expr::List(items) => {
                if items.is_empty() {
                    CrowType::List(Box::new(CrowType::Unknown))
                } else {
                    let mut inner = CrowType::Unknown;
                    for item in items {
                        inner = inner.unify(&self.infer_expr(item));
                    }
                    CrowType::List(Box::new(inner))
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
            Expr::ForExpr { var, iter, body } => {
                // Determine element type from iterator
                let iter_type = self.infer_expr(iter);
                let _elem_type = match &iter_type {
                    CrowType::List(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };

                // Try to find the return type from the body's last expression
                let last_type = if let Some(last_stmt) = body.last() {
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

                CrowType::List(Box::new(result_elem))
            }
        }
    }
}

