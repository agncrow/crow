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
    /// Sparse matrix backed by CrowSparse (CSR format).
    Sparse,
    /// Column-oriented data frame.
    Frame,
    /// A single typed column from a frame (CrowColumn wrapper).
    FrameCol,
    /// Web application (crow_web::server::CrowApp).
    WebApp,
    /// Web HTTP response (crow_web::core::CrowResponse).
    WebResponse,
    /// Web HTTP request (crow_web::core::CrowRequest).
    WebRequest,
    /// SQLite database handle (crow_web::db::CrowSqlite).
    WebDb,
    /// Database connection handle (crow_db::DbHandle) — `use db` connect[].
    DbHandle,
    /// Database transaction handle (&dyn crow_db::DbTx) — transact[] callback param.
    DbTx,
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
            CrowType::Sparse => "crow_runtime::sparse::CrowSparse".into(),
            CrowType::Frame => "crow_runtime::frame::CrowFrame".into(),
            CrowType::FrameCol => "crow_runtime::frame::CrowColumn".into(),
            CrowType::WebApp => "crow_web::server::CrowApp".into(),
            CrowType::WebResponse => "crow_web::core::CrowResponse".into(),
            CrowType::WebRequest => "crow_web::core::CrowRequest".into(),
            CrowType::WebDb => "crow_web::db::CrowSqlite".into(),
            CrowType::DbHandle => "crow_db::DbHandle".into(),
            CrowType::DbTx => "&dyn crow_db::DbTx".into(),
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
            CrowType::Sparse => "CrowSparse::from_coo(&[], &[], &[], 0, 0)".into(),
            CrowType::Frame => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])".into(),
            CrowType::FrameCol => "crow_runtime::frame::CrowColumn { data: std::sync::Arc::new(crow_runtime::frame::Column::Int(vec![])), valid: std::sync::Arc::new(vec![]) }".into(),
            CrowType::WebApp => "crow_web::server::CrowApp::new()".into(),
            CrowType::WebResponse => "crow_web::core::CrowResponse::default()".into(),
            CrowType::WebRequest => "crow_web::core::CrowRequest::new()".into(),
            CrowType::WebDb => "panic!(\"no default db\")".into(),
            CrowType::DbHandle => "panic!(\"no default db connection\")".into(),
            CrowType::DbTx => "panic!(\"no default db transaction\")".into(),
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
            CrowType::Sparse => "sparse".into(),
            CrowType::Frame => "frame".into(),
            CrowType::FrameCol => "column".into(),
            CrowType::Map(inner) => format!("map[{}]", inner.crow_name()),
            CrowType::Struct(name) => name.clone(),
            CrowType::OptBoxStruct(name) => name.clone(),
            CrowType::Fn(params, ret) => {
                let p: Vec<String> = params.iter().map(|p| p.crow_name()).collect();
                format!("fn[{}] -> {}", p.join(", "), ret.crow_name())
            }
            CrowType::Result(inner) => format!("result[{}]", inner.crow_name()),
            CrowType::Json => "json".into(),
            CrowType::WebApp => "web.app".into(),
            CrowType::WebResponse => "web.response".into(),
            CrowType::WebRequest => "web.request".into(),
            CrowType::WebDb => "web.db".into(),
            CrowType::DbHandle => "db.conn".into(),
            CrowType::DbTx => "db.tx".into(),
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
            // Fn types: compatible when arity matches and every component is
            // (Unknown params from a not-yet-typed fn are wildcards)
            (CrowType::Fn(p1, r1), CrowType::Fn(p2, r2)) if p1.len() == p2.len() => {
                p1.iter().zip(p2.iter()).all(|(a, b)| a.compatible(b)) && r1.compatible(r2)
            }
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
    /// F2 monomorphization: distinct concrete call signatures per user fn
    pub call_signatures: BTreeMap<String, Vec<Vec<CrowType>>>,
    /// Names declared with `view` — UI-tree builders; the element vocabulary
    /// is active only inside their bodies
    pub views: std::collections::BTreeSet<String>,
    /// Fns referenced as values inside view bodies or page[] registrations —
    /// event handlers, which share the reactive-state exemption with views
    pub view_handlers: std::collections::BTreeSet<String>,
    /// fn name → [(signature, mangled clone name)] for specialized functions
    pub specializations: BTreeMap<String, Vec<(Vec<CrowType>, String)>>,
    pub structs: BTreeMap<String, StructSig>,
    pub cpu_count: Option<usize>,
    /// Param names of the function currently being body-scanned (to avoid
    /// incorrectly inferring [] indexing as a function call on params).
    current_fn_params: Vec<String>,
    /// Module registry for generic type dispatch
    pub registry: crate::modules::ModuleRegistry,
}

impl TypeContext {
    pub fn new() -> Self {
        let mut structs = BTreeMap::new();
        // Synthetic struct for the regex find[] builtin result — gives m.ok
        // and m.groups concrete types in both modes (maps are homogeneous,
        // so a bool + string-array result can't be a map in compiled code)
        structs.insert(
            "CrowShell".to_string(),
            StructSig {
                fields: vec![
                    ("status".to_string(), CrowType::Int),
                    ("stdout".to_string(), CrowType::Str),
                    ("stderr".to_string(), CrowType::Str),
                ],
            },
        );
        structs.insert(
            "CrowElapsed".to_string(),
            StructSig {
                fields: vec![
                    ("seconds".to_string(), CrowType::Float),
                    ("minutes".to_string(), CrowType::Float),
                    ("hours".to_string(), CrowType::Float),
                    ("days".to_string(), CrowType::Float),
                ],
            },
        );
        structs.insert(
            "CrowFindResult".to_string(),
            StructSig {
                fields: vec![
                    ("ok".to_string(), CrowType::Bool),
                    ("groups".to_string(), CrowType::Array(Box::new(CrowType::Str))),
                ],
            },
        );
        TypeContext {
            vars: BTreeMap::new(),
            functions: BTreeMap::new(),
            call_signatures: BTreeMap::new(),
            views: std::collections::BTreeSet::new(),
            view_handlers: std::collections::BTreeSet::new(),
            specializations: BTreeMap::new(),
            structs,
            cpu_count: None,
            current_fn_params: Vec::new(),
            registry: crate::modules::ModuleRegistry::new(),
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
                Stmt::FnDecl { name, params, body, is_view, .. } => {
                    self.functions.insert(name.clone(), FnSig {
                        params: params.iter().map(|p| (p.clone(), CrowType::Unknown)).collect(),
                        return_type: CrowType::Unknown,
                        body: body.clone(),
                    });
                    if *is_view {
                        self.views.insert(name.clone());
                    }
                }
                Stmt::Use { path } => {
                    if path == "math" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                    }
                    if path == "math.stats" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                        if !self.registry.is_loaded("math.stats") {
                            self.registry.load(std::sync::Arc::new(crate::modules::stats::StatsModule));
                        }
                    }
                    if path == "math.nn" {
                        if !self.registry.is_loaded("math") {
                            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
                        }
                        if !self.registry.is_loaded("math.nn") {
                            self.registry.load(std::sync::Arc::new(crate::modules::nn::NnModule));
                        }
                    }
                    if path == "io" {
                        if !self.registry.is_loaded("io") {
                            self.registry.load(std::sync::Arc::new(crate::modules::io::IoModule));
                        }
                    }
                    if path == "frame" {
                        if !self.registry.is_loaded("frame") {
                            self.registry.load(std::sync::Arc::new(crate::modules::frame::FrameModule));
                        }
                    }
                    if path == "db" {
                        if !self.registry.is_loaded("db") {
                            self.registry.load(std::sync::Arc::new(crate::modules::db::DbModule));
                        }
                    }
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

        // Event handlers: fns referenced as bare values inside view bodies
        // (click = inc — lowered into attr maps) or in page[] registration
        // args. Generous by design: worst case a fn gains reactive-state
        // visibility it didn't need.
        {
            let mut handlers = std::collections::BTreeSet::new();
            for stmt in stmts {
                if let Stmt::FnDecl { body, is_view: true, .. } = stmt {
                    for st in body {
                        Self::collect_fn_value_refs_stmt(st, &self.functions, &mut handlers);
                    }
                }
                if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
                    if matches!(callee.as_ref(), Expr::Ident(n) if n == "page") {
                        for a in args.iter().skip(1) {
                            if let Expr::Ident(n) = a {
                                if self.functions.contains_key(n) {
                                    handlers.insert(n.clone());
                                }
                            }
                        }
                    }
                }
            }
            self.view_handlers = handlers;
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

        // F2: monomorphize functions used at multiple type signatures
        self.specialize_functions();

        // Pass 4: infer top-level variables
        for stmt in stmts {
            self.infer_stmt(stmt);
        }

        if std::env::var("CROW_DEBUG_TYPES").is_ok() {
            for (n, t) in &self.vars {
                eprintln!("VAR {} : {:?}", n, t);
            }
            for (n, sig) in &self.functions {
                eprintln!("FN {} params={:?} ret={:?}", n, sig.params, sig.return_type);
            }
        }
    }

    /// Propagate types from indirect function calls back to original function declarations.
    /// When we see `op = double` and `op[5]`, propagate i64 back to double's param.
    fn propagate_indirect_fn_types(&mut self, stmts: &[Stmt]) {
        // Collect var -> function name mappings from assignments
        let mut var_to_fn: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        self.collect_var_fn_assignments(stmts, &mut var_to_fn);

        // See through maps of fns: ops = map[add = add] (lowered to a set[]
        // chain) followed by r = ops.add makes r an alias of add
        let mut map_fn_fields: std::collections::BTreeMap<(String, String), Vec<String>> =
            std::collections::BTreeMap::new();
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                let mut pairs = Vec::new();
                Self::extract_set_chain_fns(value, self, &mut pairs);
                for (key, fn_name) in pairs {
                    map_fn_fields.entry((name.clone(), key)).or_default().push(fn_name);
                }
            }
        }
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                if let Expr::FieldAccess { object, field } = value {
                    if let Expr::Ident(map_var) = object.as_ref() {
                        if let Some(fns) = map_fn_fields.get(&(map_var.clone(), field.clone())) {
                            var_to_fn.entry(name.clone()).or_default().extend(fns.clone());
                        }
                    }
                }
            }
        }

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

    /// Walk a set[] chain (the lowered map constructor) collecting
    /// (key, fn-name) pairs for values that are named functions.
    fn extract_set_chain_fns(e: &Expr, ctx: &TypeContext, out: &mut Vec<(String, String)>) {
        if let Expr::Call { callee, args } = e {
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "set" && args.len() == 3 {
                    Self::extract_set_chain_fns(&args[0], ctx, out);
                    if let (Expr::Str(key), Expr::Ident(fn_name)) = (&args[1], &args[2]) {
                        if ctx.functions.contains_key(fn_name) {
                            out.push((key.clone(), fn_name.clone()));
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

                    // Search builtins: the predicate's parameter is the
                    // array's element type (any[arr, pred] etc.)
                    if matches!(name.as_str(), "any" | "all" | "find") && args.len() == 2 {
                        if let Expr::Ident(fn_name) = &args[1] {
                            let elem = match arg_types.first() {
                                Some(CrowType::Array(inner)) => Some(inner.as_ref().clone()),
                                Some(CrowType::NumArray(inner)) => Some(inner.as_ref().clone()),
                                _ => None,
                            };
                            if let Some(elem) = elem {
                                if let Some(sig) = self.functions.get_mut(fn_name) {
                                    if let Some(p) = sig.params.first_mut() {
                                        p.1 = p.1.unify(&elem);
                                    }
                                }
                            }
                        }
                    }

                    // db transact[conn, fn]: the callback's parameter is a
                    // db connection handle
                    if name == "transact" && args.len() == 2 {
                        if let Expr::Ident(fn_name) = &args[1] {
                            if let Some(sig) = self.functions.get_mut(fn_name) {
                                if let Some(p) = sig.params.first_mut() {
                                    p.1 = CrowType::DbTx;
                                }
                            }
                        }
                    }

                    // Is it a function call?
                    if self.functions.contains_key(name) {
                        let sig = self.functions.get(name).unwrap().clone();
                        let mut new_params = sig.params.clone();
                        let mut normalized_sig: Vec<CrowType> = Vec::with_capacity(arg_types.len());
                        for (i, at) in arg_types.iter().enumerate() {
                            // Normalize types for function params:
                            // - OptBoxStruct -> Struct (transpiler unwraps before passing)
                            // - Result<T> -> T (auto-propagation unwraps at call site)
                            let normalized = match at {
                                CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                CrowType::Result(inner) => *inner.clone(),
                                other => other.clone(),
                            };
                            normalized_sig.push(normalized.clone());
                            if i < new_params.len() {
                                new_params[i].1 = new_params[i].1.unify(&normalized);
                            }
                        }
                        // F2: record this call's concrete signature for the
                        // monomorphization pass (skip if anything is unknown)
                        if normalized_sig.len() == sig.params.len()
                            && normalized_sig.iter().all(Self::type_is_concrete)
                        {
                            let entry = self.call_signatures.entry(name.clone()).or_default();
                            let dup = entry.iter().any(|existing| {
                                existing.iter().zip(&normalized_sig).all(|(a, b)| a.compatible(b))
                            });
                            if !dup {
                                entry.push(normalized_sig);
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
                // Iterating an array literal of named fns: calls through the
                // loop variable type each fn's params (an uncalled fn would
                // otherwise keep Unknown params and be skipped at emission)
                if let Expr::Array(elems) = iter.as_ref() {
                    let fn_names: Vec<String> = elems.iter()
                        .filter_map(|e| match e {
                            Expr::Ident(n) if self.functions.contains_key(n) => Some(n.clone()),
                            _ => None,
                        })
                        .collect();
                    if fn_names.len() == elems.len() && !fn_names.is_empty() {
                        // Infer arg types outside the closure (borrow rules)
                        let mut typed_calls: Vec<Vec<CrowType>> = Vec::new();
                        let mut raw_calls: Vec<Vec<Expr>> = Vec::new();
                        Self::find_calls_to_var(body, var, &mut |args| {
                            raw_calls.push(args.to_vec());
                        });
                        for args in &raw_calls {
                            typed_calls.push(args.iter().map(|a| self.infer_expr(a)).collect());
                        }
                        for fname in &fn_names {
                            if let Some(sig) = self.functions.get_mut(fname) {
                                for tys in &typed_calls {
                                    if tys.len() == sig.params.len() {
                                        for (p, t) in sig.params.iter_mut().zip(tys.iter()) {
                                            p.1 = p.1.unify(t);
                                        }
                                    }
                                }
                            }
                        }
                    }
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

    /// Collect bare references to known fn names (Ident in value position —
    /// not as a call's callee) inside view bodies: these are event handlers.
    fn collect_fn_value_refs_stmt(
        stmt: &Stmt,
        fns: &BTreeMap<String, FnSig>,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        fn walk_expr(e: &Expr, fns: &BTreeMap<String, FnSig>, out: &mut std::collections::BTreeSet<String>) {
            match e {
                Expr::Ident(n) => {
                    if fns.contains_key(n) {
                        out.insert(n.clone());
                    }
                }
                Expr::Call { callee, args } => {
                    // The callee name itself is a call, not a value ref —
                    // but nested exprs inside it still count
                    if !matches!(callee.as_ref(), Expr::Ident(_)) {
                        walk_expr(callee, fns, out);
                    }
                    for a in args {
                        walk_expr(a, fns, out);
                    }
                }
                Expr::Named { value, .. } => walk_expr(value, fns, out),
                Expr::BinOp { left, right, .. } => {
                    walk_expr(left, fns, out);
                    walk_expr(right, fns, out);
                }
                Expr::UnaryOp { expr, .. } => walk_expr(expr, fns, out),
                Expr::Array(items) => {
                    for i in items {
                        walk_expr(i, fns, out);
                    }
                }
                Expr::Map(pairs) => {
                    for (_, v) in pairs {
                        walk_expr(v, fns, out);
                    }
                }
                Expr::FieldAccess { object, .. } => walk_expr(object, fns, out),
                Expr::ForExpr { iter, body, .. } => {
                    walk_expr(iter, fns, out);
                    for st in body {
                        TypeContext::collect_fn_value_refs_stmt(st, fns, out);
                    }
                }
                _ => {}
            }
        }
        match stmt {
            Stmt::Assign { value, .. } => walk_expr(value, fns, out),
            Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => walk_expr(e, fns, out),
            Stmt::If { condition, then_body, else_body } => {
                walk_expr(condition, fns, out);
                for st in then_body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
                if let Some(eb) = else_body {
                    for st in eb {
                        Self::collect_fn_value_refs_stmt(st, fns, out);
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                walk_expr(iter, fns, out);
                for st in body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
            }
            Stmt::While { condition, body } => {
                walk_expr(condition, fns, out);
                for st in body {
                    Self::collect_fn_value_refs_stmt(st, fns, out);
                }
            }
            _ => {}
        }
    }

    /// Walk statements and invoke f with the args of every call whose callee
    /// is the given variable name.
    fn find_calls_to_var(stmts: &[Stmt], var: &str, f: &mut dyn FnMut(&[Expr])) {
        fn walk_expr(e: &Expr, var: &str, f: &mut dyn FnMut(&[Expr])) {
            match e {
                Expr::Call { callee, args } => {
                    if let Expr::Ident(n) = callee.as_ref() {
                        if n == var {
                            f(args);
                        }
                    }
                    walk_expr(callee, var, f);
                    for a in args { walk_expr(a, var, f); }
                }
                Expr::BinOp { left, right, .. } => {
                    walk_expr(left, var, f);
                    walk_expr(right, var, f);
                }
                Expr::UnaryOp { expr, .. } => walk_expr(expr, var, f),
                Expr::Array(items) => for i in items { walk_expr(i, var, f); },
                Expr::FieldAccess { object, .. } => walk_expr(object, var, f),
                _ => {}
            }
        }
        for s in stmts {
            match s {
                Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Assign { value: e, .. } => {
                    walk_expr(e, var, f)
                }
                Stmt::If { condition, then_body, else_body } => {
                    walk_expr(condition, var, f);
                    Self::find_calls_to_var(then_body, var, f);
                    if let Some(eb) = else_body {
                        Self::find_calls_to_var(eb, var, f);
                    }
                }
                Stmt::While { condition, body } => {
                    walk_expr(condition, var, f);
                    Self::find_calls_to_var(body, var, f);
                }
                Stmt::For { iter, body, .. } => {
                    walk_expr(iter, var, f);
                    Self::find_calls_to_var(body, var, f);
                }
                _ => {}
            }
        }
    }

    /// Parse string interpolations and collect call types from embedded expressions.
    fn collect_call_types_in_string(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Escaped backtick: literal, not interpolation
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                i += 2;
                continue;
            }
            if chars[i] == '`' {
                // Interpolation span: scan to closing backtick, quote-aware
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() { i += 2; continue; }
                        if c == '\'' { in_str = false; }
                    } else if c == '\'' {
                        in_str = true;
                    } else if c == '`' {
                        break;
                    }
                    i += 1;
                }
                let expr_str: String = chars[start..i.min(chars.len())].iter().collect();
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

    /// A type is concrete if it contains no Unknown anywhere (used to decide
    /// whether a call signature is usable for monomorphization).
    fn type_is_concrete(t: &CrowType) -> bool {
        !format!("{:?}", t).contains("Unknown")
    }

    /// The F2 monomorphization pass: functions called with multiple
    /// incompatible signatures get one clone per signature group. Single-
    /// signature functions are untouched. Runs to fixpoint so specialization
    /// propagates through call chains.
    fn specialize_functions(&mut self) {
        for _round in 0..5 {
            let mut new_clones: Vec<(String, Vec<CrowType>, String)> = Vec::new();
            for (name, sigs) in self.call_signatures.clone() {
                if sigs.len() < 2 || self.specializations.contains_key(&name) {
                    continue;
                }
                // Don't specialize functions that are themselves clones
                if name.contains("__") {
                    continue;
                }
                let Some(base) = self.functions.get(&name).cloned() else { continue };
                // v1 limitation: generic higher-order functions (fn-typed
                // params) used at multiple types are not specialized — the
                // predicate's types would need per-clone propagation
                if sigs.iter().any(|sig| sig.iter().any(|t| matches!(t, CrowType::Fn(_, _)))) {
                    continue;
                }
                for (idx, sig) in sigs.iter().enumerate() {
                    let mangled = format!("{}__{}", name, idx + 1);
                    let params: Vec<(String, CrowType)> = base
                        .params
                        .iter()
                        .zip(sig.iter())
                        .map(|((pname, _), ty)| (pname.clone(), ty.clone()))
                        .collect();
                    self.functions.insert(mangled.clone(), FnSig {
                        params,
                        return_type: CrowType::Unknown,
                        body: base.body.clone(),
                    });
                    self.specializations.entry(name.clone()).or_default().push((sig.clone(), mangled.clone()));
                    new_clones.push((name.clone(), sig.clone(), mangled));
                }
            }
            if new_clones.is_empty() {
                break;
            }
            // Infer each clone's body under its own parameter types; this also
            // records the clone's callees' signatures (transitive propagation)
            for (_, _, mangled) in &new_clones {
                let body = self.functions.get(mangled).unwrap().body.clone();
                // record callee signatures from the clone's body with params bound
                let saved_vars = self.vars.clone();
                if let Some(sig) = self.functions.get(mangled) {
                    let bind: Vec<(String, CrowType)> = sig.params.clone();
                    for (p, t) in bind {
                        self.vars.insert(p, t);
                    }
                }
                for stmt in &body {
                    self.collect_call_types_stmt(stmt);
                }
                self.vars = saved_vars;
                self.infer_function_body(mangled, &body);
            }
        }
        // Settle: re-infer all clone bodies so transitive return types
        // stabilize regardless of clone creation order
        for _ in 0..2 {
            let clone_names: Vec<String> = self
                .specializations
                .values()
                .flat_map(|v| v.iter().map(|(_, m)| m.clone()))
                .collect();
            for mangled in clone_names {
                if let Some(sig) = self.functions.get(&mangled) {
                    let body = sig.body.clone();
                    // reset the return type so stale unifications don't stick
                    self.functions.get_mut(&mangled).unwrap().return_type = CrowType::Unknown;
                    self.infer_function_body(&mangled, &body);
                }
            }
        }
    }

    /// Does any if/while condition in this body infer to a Result type?
    /// (Such conditions can propagate errors out of the function.)
    fn body_has_result_condition(&self, stmts: &[Stmt]) -> bool {
        for s in stmts {
            match s {
                Stmt::If { condition, then_body, else_body } => {
                    if matches!(self.infer_expr(condition), CrowType::Result(_)) {
                        return true;
                    }
                    if self.body_has_result_condition(then_body) {
                        return true;
                    }
                    if let Some(eb) = else_body {
                        if self.body_has_result_condition(eb) {
                            return true;
                        }
                    }
                }
                Stmt::While { condition, body } => {
                    if matches!(self.infer_expr(condition), CrowType::Result(_)) {
                        return true;
                    }
                    if self.body_has_result_condition(body) {
                        return true;
                    }
                }
                Stmt::For { body, .. } => {
                    if self.body_has_result_condition(body) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
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
        // A fallible condition can propagate an error out of the function
        // (spec §11: Errors in Conditions) — the return type widens to Result
        let has_fallible_condition = self.body_has_result_condition(body);
        if let Some(sig) = self.functions.get_mut(name) {
            sig.return_type = sig.return_type.unify(&ret);
            if has_fallible_condition && !matches!(sig.return_type, CrowType::Result(_)) {
                let inner = std::mem::replace(&mut sig.return_type, CrowType::Unknown);
                sig.return_type = CrowType::Result(Box::new(inner));
            }
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
        // User-defined function call (specialization-aware: a monomorphized
        // callee's return type depends on this call's argument types)
        if let Some(sig) = self.functions.get(name) {
            if let Some(clones) = self.specializations.get(name) {
                let call_sig: Vec<CrowType> = args.iter()
                    .map(|a| match self.infer_expr(a) {
                        CrowType::Result(inner) => *inner,
                        CrowType::OptBoxStruct(n) => CrowType::Struct(n),
                        other => other,
                    })
                    .collect();
                if let Some((_, mangled)) = clones.iter().find(|(csig, _)| {
                    csig.len() == call_sig.len()
                        && csig.iter().zip(&call_sig).all(|(a, b)| a.compatible(b))
                }) {
                    if let Some(clone_sig) = self.functions.get(mangled) {
                        return clone_sig.return_type.clone();
                    }
                }
            }
            return sig.return_type.clone();
        }
        // Module function dispatch (overlapping builtins: check arg type)
        let is_overlapping = matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
        if is_overlapping && self.registry.has_function(name) {
            let arg_types: Vec<CrowType> = args.iter().map(|a| self.infer_expr(a)).collect();
            let has_numarray = arg_types.iter().any(|t| matches!(t, CrowType::NumArray(_)));
            if has_numarray {
                if let Some(t) = self.registry.type_of(name, args.len()) {
                    return t;
                }
            }
        }
        // Non-overlapping module functions
        if !is_overlapping && self.registry.has_function(name) {
            if let Some(t) = self.registry.type_of(name, args.len()) {
                return t;
            }
        }

        // Builtins (check before vars to prevent builtins from being shadowed by Fn inference)
        match name {
            "log" | "cpu" | "gpu" | "assert" => CrowType::Void,
            "type" => CrowType::Str,
            "error" => CrowType::Result(Box::new(CrowType::Str)),
            "map" => CrowType::Map(Box::new(CrowType::Unknown)),
            "range" => CrowType::Array(Box::new(CrowType::Int)),
            // Math builtins (scalar)
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
            "min" | "max" => {
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
            "trim" | "upper" | "lower" | "replace" | "join" | "repeat" | "fmt" => CrowType::Str,
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
                        // push[m, row] on a rank-2 tensor keeps the tensor type
                        if matches!(&arr_type, CrowType::NumArray(_)) && matches!(val_type, CrowType::NumArray(_)) {
                            return arr_type;
                        }
                        return CrowType::Array(Box::new(unified));
                    }
                    return arr_type;
                }
                CrowType::Unknown
            }
            "pop" | "sort" | "reverse" | "concat" => {
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
            "shell" => CrowType::Struct("CrowShell".to_string()),
            "now" | "date" => CrowType::Map(Box::new(CrowType::Int)),
            "elapsed" => CrowType::Struct("CrowElapsed".to_string()),
            // Regex + search builtins
            "matches" => CrowType::Bool,
            "any" | "all" => CrowType::Bool,
            "find" => {
                // Dispatch: find[str, pattern] → regex result struct;
                // find[array, predicate] → Result(element)
                let arg0 = args.first().map(|a| self.infer_expr(a)).unwrap_or(CrowType::Unknown);
                match arg0 {
                    CrowType::Array(inner) => CrowType::Result(inner),
                    CrowType::NumArray(inner) => match inner.as_ref() {
                        CrowType::NumArray(_) => CrowType::Result(inner),
                        _ => CrowType::Result(inner),
                    },
                    _ => CrowType::Struct("CrowFindResult".to_string()),
                }
            }
            "sub" => CrowType::Str,
            // CLI / IO
            "args" => CrowType::Array(Box::new(CrowType::Str)),
            "input" => CrowType::Result(Box::new(CrowType::Str)),
            // File I/O
            "read" => CrowType::Result(Box::new(CrowType::Str)),
            "write" => CrowType::Result(Box::new(CrowType::Str)),
            // File system builtins
            "exists" => CrowType::Bool,
            "mkdir" => CrowType::Result(Box::new(CrowType::Str)),
            "ls" => CrowType::Result(Box::new(CrowType::Array(Box::new(CrowType::Str)))),
            "rm" => CrowType::Result(Box::new(CrowType::Str)),
            "mv" => CrowType::Result(Box::new(CrowType::Str)),
            "cp" => CrowType::Result(Box::new(CrowType::Str)),
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
            // Web builtins (available when `use web` is active)
            "route" => CrowType::WebApp,
            "serve" => CrowType::Void,
            "respond" => CrowType::WebResponse,
            "redirect" => CrowType::WebResponse,
            "html" => CrowType::WebResponse,
            "text" => CrowType::WebResponse,
            // Web.db builtins
            "sqlite" => CrowType::WebDb,
            "exec" => CrowType::Void,
            "query" => CrowType::Array(Box::new(CrowType::Json)),
            // Web.auth builtins
            "sign" => CrowType::Str,
            "verify" => CrowType::Json,
            // Reactive data sources (UI)
            "poll" | "fetch" => CrowType::Frame,
            "today" => CrowType::Str,
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
                    // Unwrap Result for indexing purposes
                    let unwrapped = match var_type {
                        CrowType::Result(inner) => inner.as_ref(),
                        other => other,
                    };
                    match unwrapped {
                        CrowType::Fn(_, ret) => return *ret.clone(),
                        CrowType::Array(inner) => return *inner.clone(),
                        CrowType::NumArray(inner) => return *inner.clone(),
                        CrowType::Map(inner) => return *inner.clone(),
                        CrowType::Str => return CrowType::Str,
                        CrowType::FrameCol => return CrowType::Str, // col[i] → string
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
            // Lowering resolves named args before inference; type is the value's
            Expr::Named { value, .. } => self.infer_expr(value),
            Expr::Ident(name) => {
                // User variables shadow module constants (matches codegen +
                // interpreter scoping: environment first, then constants)
                if let Some(ty) = self.vars.get(name) {
                    return ty.clone();
                }
                // Module constants: pi, e, inf, nan
                if self.registry.has_constant(name) {
                    return CrowType::Float;
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
                // Check if either side is a frame column (FrameCol or legacy Array(Str))
                let is_frame_col = |t: &CrowType| matches!(t, CrowType::FrameCol) || matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str));
                let result_type = match op {
                    BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
                    | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => {
                        // Boolean-mask semantics: comparisons on frame columns and
                        // logical ops on boolean arrays are element-wise → Array(Bool)
                        let is_bool_mask = |t: &CrowType| matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Bool));
                        // Tensor vs scalar comparisons broadcast to masks too
                        let tensor_vs_scalar = (matches!(lt_inner, CrowType::NumArray(_))
                            && matches!(rt_inner, CrowType::Int | CrowType::Float))
                            || (matches!(rt_inner, CrowType::NumArray(_))
                                && matches!(lt_inner, CrowType::Int | CrowType::Float));
                        if is_frame_col(&lt_inner) || is_frame_col(&rt_inner)
                            || is_bool_mask(&lt_inner) || is_bool_mask(&rt_inner)
                            || tensor_vs_scalar
                        {
                            CrowType::Array(Box::new(CrowType::Bool))
                        } else {
                            CrowType::Bool
                        }
                    }
                    BinOp::Mod => CrowType::Int,
                    BinOp::Pow => {
                        // ^ returns Float if either operand is Float, else Int
                        if lt_inner == CrowType::Float || rt_inner == CrowType::Float {
                            CrowType::Float
                        } else {
                            CrowType::Int
                        }
                    }
                    BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow => {
                        // Element-wise tensor ops return NumArray
                        if matches!(&lt_inner, CrowType::NumArray(_)) {
                            lt_inner.clone()
                        } else if matches!(&rt_inner, CrowType::NumArray(_)) {
                            rt_inner.clone()
                        } else {
                            lt_inner.unify(&rt_inner)
                        }
                    }
                    BinOp::Mul => {
                        // Frame column arithmetic with scalar → FrameCol
                        if is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float) {
                            CrowType::FrameCol
                        } else if is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float) {
                            CrowType::FrameCol
                        // sparse * dense → NumArray (matmul result)
                        } else if matches!(&lt_inner, CrowType::Sparse) && matches!(&rt_inner, CrowType::NumArray(_)) {
                            CrowType::NumArray(Box::new(CrowType::Float))
                        // * on tensor-tensor is matmul (returns NumArray)
                        } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                            CrowType::NumArray(Box::new(CrowType::Float))
                        } else {
                            lt_inner.unify(&rt_inner)
                        }
                    }
                    BinOp::Add => {
                        // Frame column arithmetic → FrameCol
                        if (is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float))
                            || (is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float)) {
                            CrowType::FrameCol
                        // sparse + sparse → Sparse
                        } else if matches!(&lt_inner, CrowType::Sparse) && matches!(&rt_inner, CrowType::Sparse) {
                            CrowType::Sparse
                        } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                            CrowType::NumArray(Box::new(CrowType::Float))
                        } else {
                            lt_inner.unify(&rt_inner)
                        }
                    }
                    BinOp::Sub | BinOp::Div => {
                        // Frame column arithmetic → FrameCol
                        if (is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float))
                            || (is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float)) {
                            CrowType::FrameCol
                        // scalar - tensor, tensor - scalar, etc. → NumArray
                        } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                            CrowType::NumArray(Box::new(CrowType::Float))
                        } else {
                            lt_inner.unify(&rt_inner)
                        }
                    }
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
                    UnaryOp::Not => {
                        // not on a boolean mask negates element-wise → Array(Bool)
                        let inner = self.infer_expr(e);
                        let inner = if let CrowType::Result(i) = inner { *i } else { inner };
                        if matches!(&inner, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                            CrowType::Array(Box::new(CrowType::Bool))
                        } else {
                            CrowType::Bool
                        }
                    }
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
                // Module call: module.func[args] or module.Struct[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(_mod_name) = object.as_ref() {
                        // web.new[] → WebApp
                        if _mod_name == "web" && field == "new" {
                            return CrowType::WebApp;
                        }
                        // Registry-module prefixed call (math.zeros[...]):
                        // same type as the bare call
                        if matches!(_mod_name.as_str(), "math" | "io" | "frame" | "db")
                            && self.registry.is_loaded(_mod_name)
                            && self.registry.has_function(field)
                        {
                            if let Some(t) = self.registry.type_of(field, args.len()) {
                                return t;
                            }
                        }
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
                    CrowType::NumArray(inner) => {
                        // Advanced indexing: tensor[tensor] → tensor (gather)
                        if !args.is_empty() {
                            let idx_type = self.infer_expr(&args[0]);
                            if matches!(idx_type, CrowType::NumArray(_)) {
                                return CrowType::NumArray(inner);
                            }
                        }
                        *inner
                    }
                    CrowType::FrameCol => CrowType::Str, // col[i] → string
                    CrowType::Map(inner) => *inner,
                    CrowType::Json => CrowType::Json,
                    CrowType::Fn(_, ret) => *ret,
                    _ => CrowType::Unknown,
                }
            }
            Expr::FieldAccess { object, field } => {
                let obj_type = self.infer_expr(object);
                // Json object field access stays Json (dynamic values)
                {
                    let inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                    if matches!(inner, CrowType::Json)
                        && !matches!(field.as_str(), "ok" | "reason" | "kind" | "trace")
                    {
                        return CrowType::Json;
                    }
                }
                // .ok on FrameCol or Array(Str) → Array(Bool) (validity bitmap)
                if field == "ok" {
                    if matches!(&obj_type, CrowType::FrameCol)
                        || matches!(&obj_type, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str)) {
                        return CrowType::Array(Box::new(CrowType::Bool));
                    }
                }
                // DateTime properties on strings → Int
                if matches!(&obj_type, CrowType::Str) {
                    match field.as_str() {
                        "year" | "month" | "day" | "hour" | "minute" | "second" | "offset" => return CrowType::Int,
                        _ => {}
                    }
                }
                // Error-value metadata fields
                match field.as_str() {
                    "ok" => return CrowType::Bool,
                    "reason" | "kind" => return CrowType::Str,
                    "trace" => return CrowType::Array(Box::new(CrowType::Str)),
                    _ => {}
                }
                // Map dot-access: m.key reads a value of the map's value type
                {
                    let inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
                    if let CrowType::Map(v) = inner {
                        return *v;
                    }
                }
                // NumArray properties (also handle Result(NumArray))
                let is_numarray = matches!(obj_type, CrowType::NumArray(_))
                    || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::NumArray(_)));
                if is_numarray {
                    match field.as_str() {
                        "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                        "dtype" | "device" | "grad" => return CrowType::Str,
                        "len" => return CrowType::Int,
                        _ => {}
                    }
                }
                // Frame properties
                let is_frame = matches!(obj_type, CrowType::Frame)
                    || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame));
                if is_frame {
                    match field.as_str() {
                        "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                        "columns" | "dtypes" => return CrowType::Array(Box::new(CrowType::Str)),
                        _ => return CrowType::FrameCol, // column access by name → typed column
                    }
                }
                // FrameCol properties
                if matches!(obj_type, CrowType::FrameCol) {
                    match field.as_str() {
                        "ok" => return CrowType::Array(Box::new(CrowType::Bool)),
                        _ => return CrowType::Unknown,
                    }
                }
                // Auto-unwrap: field access on Result(Struct) reads the value's
                // field; OptBoxStruct (recursive struct link) unwraps the box
                let struct_obj = match &obj_type {
                    CrowType::Struct(n) | CrowType::OptBoxStruct(n) => Some(n.clone()),
                    CrowType::Result(inner) => match inner.as_ref() {
                        CrowType::Struct(n) => Some(n.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = struct_obj {
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
                    // All-numeric array literals become NumArray (backed by CrowTensor).
                    // Nested numeric literals build higher-rank tensors: the inner
                    // type of a rank-2 tensor is the rank-1 row tensor.
                    if matches!(inner, CrowType::Int | CrowType::Float) {
                        CrowType::NumArray(Box::new(inner))
                    } else if matches!(inner, CrowType::NumArray(_)) {
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
            Expr::ForExpr { var, iter, body } => {
                // Determine element type from iterator
                let iter_type = self.infer_expr(iter);
                let _elem_type = match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };

                // Infer the yield type from the body's last expression, with
                // the loop variable bound — otherwise a body like f[10]
                // (f = loop var holding a fn) infers Unknown. A trailing if
                // (the filter pattern) yields from inside its branches.
                fn body_yield_type(ctx: &TypeContext, body: &[Stmt]) -> CrowType {
                    match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
                        Some(Stmt::ExprStmt(e)) => ctx.infer_expr(e),
                        Some(Stmt::If { then_body, else_body, .. }) => {
                            let t = body_yield_type(ctx, then_body);
                            match else_body {
                                Some(eb) => t.unify(&body_yield_type(ctx, eb)),
                                None => t,
                            }
                        }
                        _ => CrowType::Unknown,
                    }
                }
                let last_type = if _elem_type != CrowType::Unknown {
                    let mut sub = self.clone();
                    sub.vars.insert(var.clone(), _elem_type.clone());
                    body_yield_type(&sub, body)
                } else {
                    body_yield_type(self, body)
                };

                // If the body type is still unknown, leave it unknown — a
                // later inference pass settles it. (Falling back to the
                // element type is wrong whenever the body maps elements to a
                // different type, e.g. calling fns held in the array.)
                CrowType::Array(Box::new(last_type))
            }
        }
    }
}

