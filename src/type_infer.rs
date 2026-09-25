/// Type inference: the front end both modes share, before the IR.

use std::collections::BTreeMap;
use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum CrowType {
    Int,
    Float,
    Bool,
    Str,
    /// Markup a view, an element or `doc` made: a string everywhere a
    /// string is read, and the one kind of string a view places as markup.
    /// Every other string is text (`infer_expr` reads it as `Str`; only
    /// `infer_expr_full` and the placing of children tell them apart).
    Markup,
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
    /// An SMTP mailer (crow_web::mail::CrowMailer) — `use web.mail` smtp[].
    Mailer,
    Void,
    Unknown,
}

/// The effects a session keeps or holds for its handler's commit: a
/// function that makes one acts for the session.
const SESSION_EFFECTS: &[&str] = &["notice", "sign_in", "sign_out", "download", "mail"];

impl CrowType {
    pub fn to_rust_type(&self) -> String {
        match self {
            CrowType::Int => "i64".into(),
            CrowType::Float => "f64".into(),
            CrowType::Bool => "bool".into(),
            CrowType::Str | CrowType::Markup => "String".into(),
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
            CrowType::Mailer => "crow_web::mail::CrowMailer".into(),
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
            CrowType::Str | CrowType::Markup => "String::new()".into(),
            CrowType::Array(inner) => format!("Vec::<{}>::new()", inner.to_rust_type()),
            CrowType::Map(inner) => format!("std::collections::BTreeMap::<String, {}>::new()", inner.to_rust_type()),
            // each generated struct has its own default, field by field (a
            // zeroed struct holding a list is undefined, and Rust refuses it)
            CrowType::Struct(name) => format!("{}::crow_default()", name),
            CrowType::OptBoxStruct(_) => "None".into(),
            CrowType::Fn(_, _) => "panic!(\"unreachable\")".into(),
            CrowType::Result(inner) => format!("CrowResult {{ ok: false, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", inner.default_rust_value()),
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
            CrowType::Mailer => "panic!(\"no default mailer\")".into(),
            CrowType::Void => "()".into(),
            CrowType::Unknown => "String::new()".into(),
        }
    }

    /// The type as every string reader sees it: markup is a string.
    pub fn text(&self) -> CrowType {
        match self {
            CrowType::Markup => CrowType::Str,
            CrowType::Array(t) => CrowType::Array(Box::new(t.text())),
            CrowType::Map(t) => CrowType::Map(Box::new(t.text())),
            CrowType::Result(t) => CrowType::Result(Box::new(t.text())),
            CrowType::Fn(p, r) => CrowType::Fn(p.iter().map(|t| t.text()).collect(), Box::new(r.text())),
            other => other.clone(),
        }
    }

    /// Whether markup is anywhere in the type: two call signatures that
    /// differ only here are two specializations (a view given markup, and
    /// the same view given text).
    pub fn has_markup(&self) -> bool {
        match self {
            CrowType::Markup => true,
            CrowType::Array(t) | CrowType::Map(t) | CrowType::Result(t) => t.has_markup(),
            CrowType::Fn(p, r) => p.iter().any(|t| t.has_markup()) || r.has_markup(),
            _ => false,
        }
    }

    /// Human-readable Crow type name for error messages.
    pub fn crow_name(&self) -> String {
        match self {
            CrowType::Int => "int".into(),
            CrowType::Float => "float".into(),
            CrowType::Bool => "bool".into(),
            CrowType::Str => "str".into(),
            CrowType::Markup => "markup".into(),
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
            CrowType::Mailer => "mailer".into(),
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
            // markup is a string wherever a string is read
            (CrowType::Markup, CrowType::Str) | (CrowType::Str, CrowType::Markup) => true,
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
            // markup mixed with text is text: nothing a string holds makes it markup
            (CrowType::Markup, CrowType::Str) | (CrowType::Str, CrowType::Markup) => CrowType::Str,
            (CrowType::NumArray(a), CrowType::NumArray(b)) => CrowType::NumArray(Box::new(a.unify(b))),
            (CrowType::Array(a), CrowType::Array(b)) => CrowType::Array(Box::new(a.unify(b))),
            // numbers held as a list meet numbers held as a tensor: the
            // first form stays (the lowering converts the other where they
            // meet), and what either knows of the items is kept (an empty
            // list and a tensor of floats are floats)
            (CrowType::Array(a), CrowType::NumArray(b)) => CrowType::Array(Box::new(a.unify(b))),
            (CrowType::NumArray(a), CrowType::Array(b)) => CrowType::NumArray(Box::new(a.unify(b))),
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
            // Result(T) + U => Result(U): the non-error return tells us the
            // actual value type — unified with T, so what either side knows
            // stays (a list of unknown items and a result of floats is a
            // result of floats; an error with no value adds nothing)
            (CrowType::Result(inner), other) | (other, CrowType::Result(inner)) => CrowType::Result(Box::new(other.unify(inner))),
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

/// What `enter_body` saved: restored by `leave_body`.
pub(crate) struct BodyScope {
    saved_vars: BTreeMap<String, CrowType>,
    outer_assigned: Option<std::collections::HashSet<String>>,
    outer_in_view: bool,
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
    /// While a function body is inferred: the names it has assigned so far
    /// (its parameters to begin with). The first assignment of another
    /// name binds a new local (spec §7), whatever the module calls it.
    pub(crate) body_assigned: Option<std::collections::HashSet<String>>,
    /// Top-level names assigned a value that is not a function's name: an
    /// index into one (`got[0]`) is indexing, however early in the
    /// fixpoint its value's type is still unknown
    top_values: std::collections::HashSet<String>,
    /// While a view's body is inferred: an element's name is the element,
    /// whose value is markup
    in_view: bool,
    /// A `use ui` program: its top-level variables are reactive state,
    /// which views and handlers see under their module names.
    is_ui: bool,
    /// Fns referenced as values inside view bodies or page[] registrations —
    /// event handlers, which share the reactive-state exemption with views
    pub view_handlers: std::collections::BTreeSet<String>,
    /// Names declared with `derive` — values computed from facts, read by
    /// name, never assigned (derive-design.md)
    pub derives: std::collections::BTreeSet<String>,
    /// The facts: every top-level assignment. A view or handler assigning
    /// one a float makes it a float for its whole scope, the program
    pub facts: std::collections::BTreeSet<String>,
    /// Modules imported bare (no alias) — their exports are reserved and
    /// callable bare in this file
    pub bare_modules: std::collections::BTreeSet<String>,
    /// alias → module path for `use X as y` imports (qualified-only)
    pub module_aliases: BTreeMap<String, String>,
    /// fn name → [(signature, mangled clone name)] for specialized functions
    pub specializations: BTreeMap<String, Vec<(Vec<CrowType>, String)>>,
    pub structs: BTreeMap<String, StructSig>,
    pub cpu_count: Option<usize>,
    /// Param names of the function currently being body-scanned (to avoid
    /// incorrectly inferring [] indexing as a function call on params).
    current_fn_params: Vec<String>,
    /// Module registry for generic type dispatch
    pub registry: crate::modules::ModuleRegistry,
    /// Connection variables bound to a literal URL (`pg = db.connect['...']`),
    /// in declaration order — the compiler asks these databases what a
    /// query returns.
    pub conn_urls: Vec<(String, String)>,
    /// The tables the program makes itself (`exec[conn, 'CREATE TABLE …']`):
    /// a SQLite database that does not exist yet, or lacks one of them, is
    /// described from these
    pub ddl: Vec<String>,
    /// Frame variables whose columns are known: from a described query,
    /// or derived through select/filter/sort/join from one.
    pub frame_schemas: BTreeMap<String, BTreeMap<String, CrowType>>,
    /// Functions whose returned frame has known columns.
    pub fn_frame_schemas: BTreeMap<String, BTreeMap<String, CrowType>>,
    /// Ask the program's databases what its query literals return. Every
    /// pipeline's gate asks the same way (`CROW_NO_DESCRIBE` turns it off),
    /// so a program refused for a query's column types is refused by run,
    /// run -i, test and check alike.
    pub describe_queries: bool,
    describe_cache: BTreeMap<String, Option<BTreeMap<String, CrowType>>>,
    /// Queries no database could type, as notes: the compiled program reads
    /// their cells as text, so the build prints them; the interpreter types
    /// cells from the rows and has nothing to say.
    pub describe_notes: Vec<String>,
    current_fn_name: Option<String>,
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
            body_assigned: None,
            top_values: std::collections::HashSet::new(),
            in_view: false,
            is_ui: false,
            view_handlers: std::collections::BTreeSet::new(),
            derives: std::collections::BTreeSet::new(),
            facts: std::collections::BTreeSet::new(),
            bare_modules: std::collections::BTreeSet::new(),
            module_aliases: BTreeMap::new(),
            specializations: BTreeMap::new(),
            structs,
            cpu_count: None,
            current_fn_params: Vec::new(),
            registry: crate::modules::ModuleRegistry::new(),
            conn_urls: Vec::new(),
            ddl: Vec::new(),
            frame_schemas: BTreeMap::new(),
            fn_frame_schemas: BTreeMap::new(),
            describe_queries: false,
            describe_cache: BTreeMap::new(),
            describe_notes: Vec::new(),
            current_fn_name: None,
        }
    }

    /// Main entry point: run multi-pass type inference on a program.
    pub fn infer_program(&mut self, stmts: &[Stmt]) {
        self.is_ui = self.is_ui || stmts.iter().any(|st| matches!(st, Stmt::Use { path, .. } if path == "ui" || path == "web.ui"));
        // the tables the program makes, wherever it makes them
        {
            fn in_expr(e: &Expr, out: &mut Vec<String>) {
                if let Expr::Call { callee, args } = e {
                    let is_exec = match callee.as_ref() {
                        Expr::Ident(n) => n == "exec",
                        Expr::FieldAccess { field, .. } => field == "exec",
                        _ => false,
                    };
                    if is_exec {
                        if let Some(Expr::Str(sql)) = args.get(1) {
                            if sql.trim_start().to_ascii_uppercase().starts_with("CREATE") && !out.contains(sql) {
                                out.push(sql.clone());
                            }
                        }
                    }
                    for a in args {
                        in_expr(a, out);
                    }
                }
            }
            fn in_stmts(stmts: &[Stmt], out: &mut Vec<String>) {
                for st in stmts {
                    match st {
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Assign { value: e, .. } => in_expr(e, out),
                        Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } | Stmt::For { body, .. } | Stmt::While { body, .. } => in_stmts(body, out),
                        Stmt::If { then_body, else_body, .. } => {
                            in_stmts(then_body, out);
                            if let Some(eb) = else_body {
                                in_stmts(eb, out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut ddl = std::mem::take(&mut self.ddl);
            in_stmts(stmts, &mut ddl);
            self.ddl = ddl;
        }
        // Pass 1: collect struct and function declarations — and every
        // connection, so a helper scanned before the part that declares its
        // handle still describes its query against the right database
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                self.note_connection(name, value);
            }
        }
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
                // A derivation is typed like a parameterless function and read
                // like a variable of its return type
                Stmt::Derive { name, body, .. } => {
                    self.functions.insert(name.clone(), FnSig {
                        params: vec![],
                        return_type: CrowType::Unknown,
                        body: body.clone(),
                    });
                    self.derives.insert(name.clone());
                }
                Stmt::Use { path, alias } => {
                    match alias {
                        Some(a) => {
                            self.module_aliases.insert(a.clone(), path.clone());
                        }
                        None => {
                            self.bare_modules.insert(path.clone());
                        }
                    }
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
                    if path == "doc" {
                        if !self.registry.is_loaded("doc") {
                            self.registry.load(std::sync::Arc::new(crate::modules::doc::DocModule));
                        }
                    }
                    if path == "chart" {
                        if !self.registry.is_loaded("chart") {
                            self.registry.load(std::sync::Arc::new(crate::modules::chart::ChartModule));
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
            // F-11: click = set_speed[10.0] — the callee of a call-shape in
            // event-attribute position is a handler too.
            fn collect_event_call_handlers(
                e: &Expr,
                fns: &std::collections::BTreeMap<String, crate::type_infer::FnSig>,
                out: &mut std::collections::BTreeSet<String>,
            ) {
                match e {
                    Expr::Map(pairs) => {
                        for (k, v) in pairs {
                            let is_event = matches!(k, Expr::Ident(n) | Expr::Str(n)
                                if matches!(n.as_str(), "click" | "input" | "submit" | "change" | "seek"));
                            if is_event {
                                if let Expr::Call { callee, .. } = v {
                                    if let Expr::Ident(f) = callee.as_ref() {
                                        if fns.contains_key(f) {
                                            out.insert(f.clone());
                                        }
                                    }
                                }
                            }
                            collect_event_call_handlers(v, fns, out);
                        }
                    }
                    Expr::Call { callee, args } => {
                        // chart[..., seek = jump]: the seek target is a handler
                        if let Some(f) = crate::lower::chart_seek_handler(callee, args) {
                            if fns.contains_key(f) {
                                out.insert(f.to_string());
                            }
                        }
                        for a in args { collect_event_call_handlers(a, fns, out); }
                    }
                    Expr::Array(items) => for i in items { collect_event_call_handlers(i, fns, out); },
                    Expr::Named { name, value } => {
                        if matches!(name.as_str(), "click" | "input" | "submit" | "change" | "seek") {
                            if let Expr::Call { callee, .. } = value.as_ref() {
                                if let Expr::Ident(f) = callee.as_ref() {
                                    if fns.contains_key(f) {
                                        out.insert(f.clone());
                                    }
                                }
                            }
                        }
                        collect_event_call_handlers(value, fns, out);
                    }
                    Expr::BinOp { left, right, .. } => {
                        collect_event_call_handlers(left, fns, out);
                        collect_event_call_handlers(right, fns, out);
                    }
                    _ => {}
                }
            }
            fn collect_event_call_stmts(
                stmts: &[Stmt],
                fns: &std::collections::BTreeMap<String, crate::type_infer::FnSig>,
                out: &mut std::collections::BTreeSet<String>,
            ) {
                for st in stmts {
                    match st {
                        Stmt::Assign { value, .. } => collect_event_call_handlers(value, fns, out),
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => collect_event_call_handlers(e, fns, out),
                        Stmt::If { condition, then_body, else_body } => {
                            collect_event_call_handlers(condition, fns, out);
                            collect_event_call_stmts(then_body, fns, out);
                            if let Some(eb) = else_body {
                                collect_event_call_stmts(eb, fns, out);
                            }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => {
                            collect_event_call_stmts(body, fns, out)
                        }
                        _ => {}
                    }
                }
            }
            for stmt in stmts {
                if let Stmt::FnDecl { body, is_view: true, .. } = stmt {
                    for st in body {
                        Self::collect_fn_value_refs_stmt(st, &self.functions, &mut handlers);
                    }
                    collect_event_call_stmts(body, &self.functions, &mut handlers);
                }
            }
            for stmt in stmts {
                if let Stmt::FnDecl { .. } = stmt {}
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
            // click[a, f] inside a test block names f as a handler, and
            // click[a, f[x]] too, with the arguments it is given: the harness
            // drives handlers, so they get the reactive exemption, and a
            // compiled program dispatches them.
            fn collect_click_handlers(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
                fn walk(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
                    if let Expr::Call { callee, args } = e {
                        if matches!(callee.as_ref(), Expr::Ident(n) if n == "click") && args.len() == 2 {
                            match &args[1] {
                                Expr::Ident(h) => {
                                    out.insert(h.clone());
                                }
                                Expr::Call { callee: h, .. } => {
                                    if let Expr::Ident(h) = h.as_ref() {
                                        out.insert(h.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                        for a in args {
                            walk(a, out);
                        }
                    }
                }
                for st in stmts {
                    match st {
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, out),
                        Stmt::Assign { value, .. } => walk(value, out),
                        Stmt::TestBlock { body, .. } => collect_click_handlers(body, out),
                        Stmt::If { then_body, else_body, .. } => {
                            collect_click_handlers(then_body, out);
                            if let Some(eb) = else_body {
                                collect_click_handlers(eb, out);
                            }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => {
                            collect_click_handlers(body, out)
                        }
                        _ => {}
                    }
                }
            }
            {
                let mut clicked = std::collections::BTreeSet::new();
                collect_click_handlers(stmts, &mut clicked);
                for h in clicked {
                    if self.functions.contains_key(&h) {
                        handlers.insert(h);
                    }
                }
            }
            // the module's reactive state: every top-level assignment
            let module_vars: std::collections::BTreeSet<String> = stmts.iter().filter_map(|s| if let Stmt::Assign { name, .. } = s { Some(name.clone()) } else { None }).filter(|n| !crate::plain_entry_name(n)).collect();
            self.facts = module_vars.clone();
            // a function that assigns reactive state — or tells the reader
            // something with notice[], which the session keeps, or sends
            // mail, which waits for the session's commit — acts for the
            // session and joins the handler class. A function's last line
            // is written as its return.
            fn assigns_any(stmts: &[Stmt], names: &std::collections::BTreeSet<String>) -> bool {
                stmts.iter().any(|st| match st {
                    Stmt::ExprStmt(Expr::Call { callee, .. }) | Stmt::Return(Some(Expr::Call { callee, .. })) if matches!(callee.as_ref(), Expr::Ident(n) if SESSION_EFFECTS.contains(&n.as_str())) => true,
                    Stmt::Assign { name, .. } => names.contains(name),
                    Stmt::MultiAssign { names: ns, .. } => ns.iter().any(|n| names.contains(n)),
                    Stmt::If { then_body, else_body, .. } => assigns_any(then_body, names) || else_body.as_ref().map_or(false, |eb| assigns_any(eb, names)),
                    Stmt::For { body, .. } | Stmt::While { body, .. } => assigns_any(body, names),
                    _ => false,
                })
            }

            // the reactive state a function can write: a part's function its
            // own part's, which carries the part's prefix (a local `ch` in a
            // part is not the entry's connection)
            fn own_state(f: &str, module_vars: &std::collections::BTreeSet<String>) -> std::collections::BTreeSet<String> {
                match crate::part_prefix_of_declaration(f) {
                    Some(p) => module_vars.iter().filter(|m| m.starts_with(p.as_str())).cloned().collect(),
                    None => module_vars.clone(),
                }
            }

            // a function that writes its part's state or reaches a session's
            // effect — itself, or through the functions it calls — acts for
            // the handler that calls it, so it joins with them (`clock.jump`
            // moves the head through `moved_to`, which writes it)
            fn effects_any(stmts: &[Stmt]) -> bool {
                assigns_any(stmts, &std::collections::BTreeSet::new())
            }
            let mut reaches: std::collections::BTreeSet<String> = self.functions.iter().filter(|(n, f)| effects_any(&f.body) || assigns_any(&f.body, &own_state(n, &module_vars))).map(|(n, _)| n.clone()).collect();
            loop {
                let before = reaches.len();
                for (n, f) in &self.functions {
                    if !reaches.contains(n) {
                        let mut called = std::collections::BTreeSet::new();
                        Self::collect_called_fns_stmts(&f.body, &mut called);
                        if called.iter().any(|c| reaches.contains(c)) {
                            reaches.insert(n.clone());
                        }
                    }
                }
                if reaches.len() == before {
                    break;
                }
            }

            // later[f] targets scheduled from views/handlers are deferred
            // handler halves: they read/write reactive state off the loop,
            // so they join the handler class (validator exemption, interp
            // exempt frames, the Rust backend's state signature). Fixpoint covers
            // later-chains (handler → later f → later g).
            loop {
                let mut added = false;
                let roots: Vec<String> = handlers.iter().cloned().collect();
                let view_fns: Vec<String> = self
                    .functions
                    .iter()
                    .filter(|(n, _)| self.views.contains(*n))
                    .map(|(n, _)| n.clone())
                    .collect();
                for name in roots.iter().chain(view_fns.iter()) {
                    if let Some(sig) = self.functions.get(name) {
                        let mut targets = std::collections::BTreeSet::new();
                        Self::collect_later_targets_stmts(&sig.body, &mut targets);
                        // and, for a handler, every function it calls that writes
                        // module state: a helper moving state on the handler's
                        // behalf. A pure helper keeps its own scope.
                        if !self.views.contains(name) {
                            let mut called = std::collections::BTreeSet::new();
                            Self::collect_called_fns_stmts(&sig.body, &mut called);
                            for c in called {
                                if reaches.contains(&c) {
                                    targets.insert(c);
                                }
                            }
                        }
                        for t in targets {
                            if self.functions.contains_key(&t) && !self.views.contains(&t) && handlers.insert(t) {
                                added = true;
                            }
                        }
                    }
                }
                if !added {
                    break;
                }
            }
            // a derivation referenced in a view is read, not wired
            let handlers: std::collections::BTreeSet<String> = handlers.into_iter().filter(|h| !self.derives.contains(h)).collect();
            self.view_handlers = handlers;

            // A seek handler receives the clicked bucket's moment: its
            // first parameter is a float whether or not a call site says so.
            // A widget's handler receives what its module sends, as a field's
            // handler receives its text: a parameter nothing else types is a
            // string.
            fn seek_expr(e: &Expr, out: &mut Vec<String>, widgets: &mut Vec<String>) {
                match e {
                    Expr::Call { callee, args } => {
                        if let Some(f) = crate::lower::chart_seek_handler(callee, args) {
                            out.push(f.to_string());
                        }
                        if matches!(callee.as_ref(), Expr::Ident(n) if n == "widget") {
                            for a in args {
                                let Expr::Map(pairs) = a else { continue };
                                for (k, v) in pairs {
                                    if matches!(k, Expr::Ident(s) | Expr::Str(s) if s == "on") {
                                        for (_, h) in crate::lower::map_literal_pairs(v).unwrap_or_default() {
                                            if let Expr::Ident(f) = h {
                                                widgets.push(f);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        seek_expr(callee, out, widgets);
                        for a in args { seek_expr(a, out, widgets); }
                    }
                    Expr::BinOp { left, right, .. } => { seek_expr(left, out, widgets); seek_expr(right, out, widgets); }
                    Expr::UnaryOp { expr, .. } => seek_expr(expr, out, widgets),
                    Expr::FieldAccess { object, .. } => seek_expr(object, out, widgets),
                    Expr::Array(items) => for i in items { seek_expr(i, out, widgets); },
                    Expr::Map(pairs) => for (k, v) in pairs { seek_expr(k, out, widgets); seek_expr(v, out, widgets); },
                    Expr::Named { value, .. } => seek_expr(value, out, widgets),
                    Expr::ForExpr { iter, body, .. } => { seek_expr(iter, out, widgets); seek_stmts(body, out, widgets); }
                    _ => {}
                }
            }
            fn seek_stmts(stmts: &[Stmt], out: &mut Vec<String>, widgets: &mut Vec<String>) {
                for st in stmts {
                    match st {
                        Stmt::Assign { value, .. } => seek_expr(value, out, widgets),
                        Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => seek_expr(e, out, widgets),
                        Stmt::If { condition, then_body, else_body } => {
                            seek_expr(condition, out, widgets);
                            seek_stmts(then_body, out, widgets);
                            if let Some(eb) = else_body { seek_stmts(eb, out, widgets); }
                        }
                        Stmt::For { body, .. } | Stmt::While { body, .. } => seek_stmts(body, out, widgets),
                        Stmt::FnDecl { body, .. } => seek_stmts(body, out, widgets),
                        _ => {}
                    }
                }
            }
            let mut seeks = Vec::new();
            let mut widgets = Vec::new();
            seek_stmts(stmts, &mut seeks, &mut widgets);
            for f in seeks {
                if let Some(sig) = self.functions.get_mut(&f) {
                    if let Some(p) = sig.params.first_mut() {
                        p.1 = CrowType::Float;
                    }
                }
            }
            for f in widgets {
                if let Some(sig) = self.functions.get_mut(&f) {
                    for p in sig.params.iter_mut().filter(|p| matches!(p.1, CrowType::Unknown)) {
                        p.1 = CrowType::Str;
                    }
                }
            }
        }

        // Pass 2+: iterate call-site scanning and function body inference
        // until types stabilize (handles cross-function type propagation)
        // A derivation's name reads as a variable of the type its body returns
        self.sync_derive_types();
        // five rounds at least, as ever; then more while a signature still
        // moves — a chain of derivations learns one link a round
        for round in 0..16 {
            let before: Vec<(String, CrowType, Vec<(String, CrowType)>)> = self.functions.iter().map(|(n, s)| (n.clone(), s.return_type.clone(), s.params.clone())).collect();
            self.sync_derive_types();
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
            let after: Vec<(String, CrowType, Vec<(String, CrowType)>)> = self.functions.iter().map(|(n, s)| (n.clone(), s.return_type.clone(), s.params.clone())).collect();
            if round >= 4 && before == after {
                break;
            }
        }
        // what the last round learned, where the top level reads it
        self.sync_derive_types();

        // what only a test's click types, last, and the bodies again
        if self.type_clicked_handlers(stmts) {
            let fn_entries: Vec<(String, Vec<Stmt>)> = self.functions.iter().map(|(n, sig)| (n.clone(), sig.body.clone())).collect();
            for (name, body) in &fn_entries {
                self.infer_function_body(name, body);
            }
        }

        // F2: monomorphize functions used at multiple type signatures
        self.specialize_functions();
        self.sync_derive_types();

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

    /// A handler a test clicks with arguments (`click[a, h[4]]`) takes
    /// their types for the parameters nothing in the program types: the
    /// click may be the only call it has, and a compiled program dispatches
    /// it by them. The program's own evidence (a view's call, a table's
    /// action, a widget's text) comes first. An argument may be one of the
    /// test's own variables (`eps = …` then `click[a, h[eps]]`): it is read
    /// as the test assigned it before the click. Whether any parameter moved.
    fn type_clicked_handlers(&mut self, stmts: &[Stmt]) -> bool {
        type Click = (String, Vec<Expr>, Vec<(String, Expr)>);
        fn walk(e: &Expr, locals: &[(String, Expr)], out: &mut Vec<Click>) {
            if let Expr::Call { callee, args } = e {
                if matches!(callee.as_ref(), Expr::Ident(n) if n == "click") && args.len() == 2 {
                    if let Expr::Call { callee: h, args: hargs } = &args[1] {
                        if let Expr::Ident(h) = h.as_ref() {
                            out.push((h.clone(), hargs.clone(), locals.to_vec()));
                        }
                    }
                }
                for a in args {
                    walk(a, locals, out);
                }
            }
        }
        fn clicked_calls(stmts: &[Stmt], in_test: bool, locals: &mut Vec<(String, Expr)>, out: &mut Vec<Click>) {
            for st in stmts {
                match st {
                    Stmt::TestBlock { body, .. } => clicked_calls(body, true, &mut Vec::new(), out),
                    Stmt::Assign { name, value } if in_test => {
                        walk(value, locals, out);
                        locals.push((name.clone(), value.clone()));
                    }
                    Stmt::ExprStmt(e) | Stmt::Return(Some(e)) if in_test => walk(e, locals, out),
                    Stmt::If { then_body, else_body, .. } if in_test => {
                        clicked_calls(then_body, true, locals, out);
                        if let Some(eb) = else_body {
                            clicked_calls(eb, true, locals, out);
                        }
                    }
                    Stmt::For { body, .. } | Stmt::While { body, .. } if in_test => clicked_calls(body, true, locals, out),
                    _ => {}
                }
            }
        }
        let mut calls = Vec::new();
        clicked_calls(stmts, false, &mut Vec::new(), &mut calls);
        let mut moved = false;
        for (h, args, locals) in calls {
            // the test's variables as it assigned them, in view while its
            // click's arguments are read, then out of view again
            let mut saved: Vec<(String, Option<CrowType>)> = Vec::new();
            for (n, v) in &locals {
                let t = self.infer_expr(v);
                saved.push((n.clone(), self.vars.insert(n.clone(), t)));
            }
            let types: Vec<CrowType> = args.iter().map(|a| self.infer_expr(a)).collect();
            for (n, old) in saved.into_iter().rev() {
                match old {
                    Some(t) => {
                        self.vars.insert(n, t);
                    }
                    None => {
                        self.vars.remove(&n);
                    }
                }
            }
            if let Some(sig) = self.functions.get_mut(&h) {
                if sig.params.len() == types.len() {
                    for (p, t) in sig.params.iter_mut().zip(types) {
                        if matches!(p.1, CrowType::Unknown) && !matches!(t, CrowType::Unknown) {
                            p.1 = t;
                            moved = true;
                        }
                    }
                }
            }
        }
        moved
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
    /// Fn names appearing as later[f]/running[f] targets.
    fn collect_later_targets_stmts(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        fn walk(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
            if let Expr::Call { callee, args } = e {
                if let Expr::Ident(name) = callee.as_ref() {
                    if (name == "later" || name == "running") && !args.is_empty() {
                        if let Expr::Ident(target) = &args[0] {
                            out.insert(target.clone());
                        }
                    }
                }
                walk(callee, out);
                for a in args {
                    walk(a, out);
                }
            } else if let Expr::BinOp { left, right, .. } = e {
                walk(left, out);
                walk(right, out);
            } else if let Expr::UnaryOp { expr, .. } = e {
                walk(expr, out);
            } else if let Expr::FieldAccess { object, .. } = e {
                walk(object, out);
            } else if let Expr::Array(items) = e {
                for i in items {
                    walk(i, out);
                }
            } else if let Expr::Named { value, .. } = e {
                walk(value, out);
            }
        }
        for stmt in stmts {
            match stmt {
                Stmt::Assign { value, .. } => walk(value, out),
                Stmt::ExprStmt(e) => walk(e, out),
                Stmt::Return(Some(e)) => walk(e, out),
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    walk(condition, out);
                    Self::collect_later_targets_stmts(then_body, out);
                    if let Some(eb) = else_body {
                        Self::collect_later_targets_stmts(eb, out);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    Self::collect_later_targets_stmts(body, out);
                }
                _ => {}
            }
        }
    }

    fn collect_call_types_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                // Track local variable types during call-site scanning (a
                // local holding a view's markup is markup); a function's own
                // local is not the module variable of its name
                let ty = self.infer_expr_full(value);
                if self.fresh_binding(name) {
                    self.vars.insert(name.clone(), ty);
                } else {
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    let ty = self.consumed(&existing, ty);
                    self.vars.insert(name.clone(), existing.unify(&ty));
                }
                if self.body_assigned.is_none() && !matches!(value, Expr::Ident(f) if self.functions.contains_key(f)) {
                    self.top_values.insert(name.clone());
                }
                self.collect_call_types_expr(value);
                self.note_connection(name, value);
                if let Some(schema) = self.frame_schema_of(value) {
                    self.frame_schemas.insert(name.clone(), schema);
                }
            }
            Stmt::MultiAssign { names, values, .. } => {
                for (name, value) in names.iter().zip(values.iter()) {
                    let ty = self.infer_expr(value);
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    let ty = self.consumed(&existing, ty);
                    self.vars.insert(name.clone(), existing.unify(&ty));
                }
                for v in values { self.collect_call_types_expr(v); }
            }
            // A derivation's body is scanned like a parameterless function's:
            // the views and helpers it calls learn their parameter types from
            // its call sites, exactly as they would from a handler's
            Stmt::Derive { name, body, .. } => {
                let saved_vars = self.vars.clone();
                let saved_fn_params = self.current_fn_params.clone();
                let saved_fn_name = self.current_fn_name.replace(name.clone());
                let saved_assigned = self.body_assigned.replace(std::collections::HashSet::new());
                self.current_fn_params = Vec::new();
                for s in body { self.collect_call_types_stmt(s); }
                self.body_assigned = saved_assigned;
                // the frame a derivation returns keeps its columns' types
                // under the derivation's name, as a variable's would
                let returned: Vec<Expr> = body.iter().filter_map(|s| if let Stmt::Return(Some(e)) = s { Some(e.clone()) } else { None }).collect();
                for e in returned {
                    if let Some(schema) = self.frame_schema_of(&e) {
                        if !schema.is_empty() {
                            self.frame_schemas.insert(name.clone(), schema);
                            break;
                        }
                    }
                }
                self.current_fn_params = saved_fn_params;
                self.current_fn_name = saved_fn_name;
                self.vars = saved_vars;
            }
            Stmt::FnDecl { name, body, .. } => {
                // Add function params as known vars before scanning body
                let saved_vars = self.vars.clone();
                let saved_fn_params = self.current_fn_params.clone();
                let saved_fn_name = self.current_fn_name.replace(name.clone());
                // inside a view an element is markup, at the calls it makes too
                let saved_in_view = std::mem::replace(&mut self.in_view, self.views.contains(name));
                let mut assigned = std::collections::HashSet::new();
                if let Some(sig) = self.functions.get(name) {
                    self.current_fn_params = sig.params.iter().map(|(n, _)| n.clone()).collect();
                    for (pname, pty) in &sig.params.clone() {
                        self.vars.insert(pname.clone(), pty.clone());
                        assigned.insert(pname.clone());
                    }
                }
                let saved_assigned = self.body_assigned.replace(assigned);
                for s in body { self.collect_call_types_stmt(s); }
                self.body_assigned = saved_assigned;
                // A module-level variable assigned in this body (reactive
                // state written by a handler) keeps the refined type: an
                // empty default learns its element type from the handler.
                // Only a ui program's facts: anywhere else a body's first
                // assignment is a local of its own (spec §7), and a local
                // named like a module variable says nothing about it.
                let module_vars: Vec<String> = saved_vars.keys().filter(|m| self.is_ui && self.facts.contains(*m)).cloned().collect();
                let mut refined: Vec<(String, CrowType)> = Vec::new();
                for m in &module_vars {
                    if let (Some(before), Some(after)) = (saved_vars.get(m), self.vars.get(m)) {
                        let open = matches!(before, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Unknown))
                            || matches!(before, CrowType::Map(inner) if matches!(inner.as_ref(), CrowType::Unknown))
                            || matches!(before, CrowType::Unknown);
                        if open && before != after { refined.push((m.clone(), before.unify(after))); }
                    }
                }

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
                self.current_fn_name = saved_fn_name;
                self.in_view = saved_in_view;
                self.vars = saved_vars;
                for (m, t) in refined { self.vars.insert(m, t); }
            }
            Stmt::Return(Some(e)) => {
                self.collect_call_types_expr(e);
                if let Some(f) = self.current_fn_name.clone() {
                    if let Some(schema) = self.frame_schema_of(e) {
                        self.fn_frame_schemas.insert(f, schema);
                    }
                }
            }
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
                let iter_type = Self::walked(self.infer_expr(iter));
                match &iter_type {
                    CrowType::Array(inner) | CrowType::NumArray(inner) => {
                        self.vars.insert(var.clone(), *inner.clone());
                    }
                    _ => {}
                }
                self.collect_loop_call_types(body);
            }
            Stmt::While { condition, body } => {
                self.collect_call_types_expr(condition);
                self.collect_loop_call_types(body);
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

    /// A loop's body for call sites: it runs again with what it assigned,
    /// so when a variable from before the loop took a type in it (`made =
    /// []`, a call on `made`, then a push of records), the body is read a
    /// second time and its calls see that type.
    fn collect_loop_call_types(&mut self, body: &[Stmt]) {
        let outer = self.vars.clone();
        for s in body {
            self.collect_call_types_stmt(s);
        }
        let moved = outer.iter().any(|(n, t)| self.vars.get(n).map_or(false, |now| now != t));
        if moved {
            for s in body {
                self.collect_call_types_stmt(s);
            }
        }
    }

    /// Scan an expression for call sites; update function param types and struct field types.
    fn collect_call_types_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { callee, args } => {
                if let Expr::Ident(name) = callee.as_ref() {
                    // markup told apart from text: a view given markup and the
                    // same view given text are two specializations
                    let arg_types: Vec<CrowType> = args.iter()
                        .map(|a| self.infer_expr_full(a))
                        .collect();

                    // table[frame, click = h, key = 'id', actions = map[…]]: the
                    // row's key cell is what every one of those handlers
                    // receives, so their first parameter is the key column's type
                    if name == "table" && args.len() == 2 {
                        if let Expr::Map(pairs) = &args[0] {
                            let key_name = pairs.iter().find_map(|(k, v)| match (k, v) {
                                (Expr::Ident(s), Expr::Str(col)) | (Expr::Str(s), Expr::Str(col)) if s == "key" => Some(col.clone()),
                                _ => None,
                            });
                            let schema = self.frame_schema_of(&args[1]);
                            let key_type = match (&key_name, &schema) {
                                (Some(col), Some(sc)) => sc.get(col).cloned().unwrap_or(CrowType::Str),
                                (None, Some(sc)) => sc.values().next().cloned().unwrap_or(CrowType::Str),
                                _ => CrowType::Str,
                            };
                            let mut handlers: Vec<String> = Vec::new();
                            for (k, v) in pairs {
                                let kname = match k { Expr::Ident(s) | Expr::Str(s) => s.as_str(), _ => continue };
                                match (kname, v) {
                                    ("click", Expr::Ident(h)) => handlers.push(h.clone()),
                                    ("actions", v) => if let Some(acts) = crate::lower::map_literal_pairs(v) {
                                        for (_, h) in acts { if let Expr::Ident(f) = &h { handlers.push(f.clone()); } }
                                    },
                                    _ => {}
                                }
                            }
                            for h in handlers {
                                if let Some(sig) = self.functions.get_mut(&h) {
                                    if let Some(p) = sig.params.first_mut() {
                                        if matches!(p.1, CrowType::Unknown) { p.1 = key_type.clone(); }
                                    }
                                }
                            }
                        }
                    }

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

                    // later[f, args…] IS a call of f with those args — infer
                    // it as one so f's params unify and monomorphization
                    // emits it even when later[] is its only call site.
                    if name == "later" && !args.is_empty() {
                        if let Expr::Ident(target) = &args[0] {
                            if self.functions.contains_key(target) {
                                let synthetic = Expr::Call {
                                    callee: Box::new(Expr::Ident(target.clone())),
                                    args: args[1..].to_vec(),
                                };
                                self.collect_call_types_expr(&synthetic);
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
                            // - OptBoxStruct -> Struct (the Rust backend unwraps before passing)
                            // - Result<T> -> T (auto-propagation unwraps at call site)
                            // - an error with no value (error[…]) says nothing of the
                            //   parameter: that call never reaches the body
                            let normalized = match at {
                                CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Void) => CrowType::Unknown,
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
                            let dup = entry.iter().any(|existing| Self::same_signature(existing, &normalized_sig));
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
                    // A local the body assigned is untyped only until its value
                    // is (a slice of a parameter, early in the fixpoint): `hs[0]`
                    // says nothing, and a function guessed from it stuck to
                    // every function the local was passed to.
                    if !self.functions.contains_key(name) && !self.structs.contains_key(name) {
                        let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                        let is_param = self.current_fn_params.contains(name);
                        let is_local = self.body_assigned.as_ref().map_or(self.top_values.contains(name), |a| a.contains(name));
                        match &existing {
                            CrowType::Fn(_, _) => {
                                // Already known to be Fn — refine arg types
                                let fn_type = CrowType::Fn(arg_types.clone(), Box::new(CrowType::Unknown));
                                let unified = existing.unify(&fn_type);
                                self.vars.insert(name.clone(), unified);
                            }
                            CrowType::Unknown if !is_param && !is_local => {
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
                let iter_type = Self::walked(self.infer_expr(iter));
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
    /// True when any backtick span of the string evaluates to a fallible
    /// value — the string is then fallible itself (errors propagate through
    /// interpolation).
    fn string_has_fallible_span(&self, s: &str) -> bool {
        if !s.contains('`') { return false; }
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' { i += 2; continue; }
            if chars[i] == '`' {
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() { i += 2; continue; }
                        if c == '\'' { in_str = false; }
                    } else if c == '\'' { in_str = true; } else if c == '`' { break; }
                    i += 1;
                }
                let expr_str: String = chars[start..i.min(chars.len())].iter().collect();
                i += 1;
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                if let Ok(tokens) = lexer.tokenize() {
                    let mut parser = crate::parser::Parser::new(tokens);
                    if let Ok(stmts) = parser.parse() {
                        let stmts = crate::lower::lower_fragment(&stmts).unwrap_or(stmts);
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, Stmt::Line(_))) {
                            if matches!(self.infer_expr(expr), CrowType::Result(_)) { return true; }
                        }
                    }
                }
            } else { i += 1; }
        }
        false
    }

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
                        let stmts = crate::lower::lower_fragment(&stmts).unwrap_or(stmts);
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
                    // Clones of a view are views (component monomorphization)
                    if self.views.contains(name.as_str()) {
                        self.views.insert(mangled.clone());
                    }
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

    /// Does any if/while condition in this body, or any loop's list, infer
    /// to a Result type? (Such decisions can propagate errors out of the
    /// function.) A collecting loop counts, its body's conditions too.
    fn body_has_result_condition(&mut self, stmts: &[Stmt]) -> bool {
        for s in stmts {
            if self.loops_have_result_condition(s) {
                return true;
            }
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
                Stmt::For { var, iter, body } => {
                    // a list that can fail is refused like a condition
                    if self.loop_list_can_fail(var, iter, body) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Does a variable whose first value could not fail meet a value that
    /// can? It holds no error, so the assignment refuses a failed one (spec,
    /// Reassignment) and hands it out of the function, as a condition does.
    /// The body is walked as the lowering binds it: a block's first
    /// assignments are its own, and a local keeps its first value's type.
    fn body_refuses_assignment(&mut self, name: &str, body: &[Stmt]) -> bool {
        let saved = self.vars.clone();
        let mut own = std::collections::HashSet::new();
        if let Some(sig) = self.functions.get(name).cloned() {
            for (p, t) in sig.params {
                self.vars.insert(p.clone(), t);
                own.insert(p);
            }
        }
        let mut scopes = vec![own];
        let found = self.refuses_in(body, &mut scopes);
        self.vars = saved;
        found
    }

    fn refuses_in(&mut self, stmts: &[Stmt], scopes: &mut Vec<std::collections::HashSet<String>>) -> bool {
        let mut found = false;
        for s in stmts {
            match s {
                Stmt::Assign { name, value } => found |= self.refused_at(name, value, scopes),
                Stmt::MultiAssign { names, values } if names.len() == values.len() => {
                    for (n, v) in names.iter().zip(values) {
                        found |= self.refused_at(n, v, scopes);
                    }
                }
                Stmt::If { then_body, else_body, .. } => {
                    scopes.push(Default::default());
                    found |= self.refuses_in(then_body, scopes);
                    scopes.pop();
                    if let Some(eb) = else_body {
                        scopes.push(Default::default());
                        found |= self.refuses_in(eb, scopes);
                        scopes.pop();
                    }
                }
                Stmt::While { body, .. } => {
                    scopes.push(Default::default());
                    found |= self.refuses_in(body, scopes);
                    scopes.pop();
                }
                Stmt::For { var, iter, body } => {
                    if let CrowType::Array(t) | CrowType::NumArray(t) = Self::walked(self.infer_expr(iter)) {
                        self.vars.insert(var.clone(), *t);
                    }
                    scopes.push([var.clone()].into_iter().collect());
                    found |= self.refuses_in(body, scopes);
                    scopes.pop();
                }
                _ => {}
            }
        }
        found
    }

    /// One assignment of `refuses_in`: a new name is bound with its value's
    /// type (a fact is written, not bound); a bound one refuses a value
    /// that can fail when it holds a plain value.
    fn refused_at(&mut self, name: &str, value: &Expr, scopes: &mut [std::collections::HashSet<String>]) -> bool {
        let ty = self.infer_expr_full(value);
        if !scopes.iter().any(|s| s.contains(name)) {
            if self.is_ui && self.facts.contains(name) {
                return false;
            }
            if let Some(last) = scopes.last_mut() {
                last.insert(name.to_string());
            }
            if !format!("{:?}", ty).contains("Unknown") {
                self.vars.insert(name.to_string(), ty);
            }
            return false;
        }
        let held = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
        match &ty {
            CrowType::Result(inner) => !matches!(held, CrowType::Result(_)) && !format!("{:?}", held).contains("Unknown") && held.compatible(inner),
            _ => false,
        }
    }

    /// Whether a loop can hand an error out of its function: its list can
    /// fail (the loop refuses it, as a condition is refused), or a decision
    /// in its body can, read with the loop's variable typed as an item.
    fn loop_list_can_fail(&mut self, var: &str, iter: &Expr, body: &[Stmt]) -> bool {
        let list = self.infer_expr(iter);
        if matches!(list, CrowType::Result(_)) {
            return true;
        }
        let elem = match Self::walked(list) {
            CrowType::Array(t) | CrowType::NumArray(t) => *t,
            _ => CrowType::Unknown,
        };
        let saved = self.vars.insert(var.to_string(), elem);
        let found = self.body_has_result_condition(body);
        match saved {
            Some(t) => {
                self.vars.insert(var.to_string(), t);
            }
            None => {
                self.vars.remove(var);
            }
        }
        found
    }

    /// What a loop walks: a list that can fail is refused before the loop
    /// runs (spec, Errors in Conditions), so its items are the value's.
    fn walked(t: CrowType) -> CrowType {
        match t {
            CrowType::Result(inner) => *inner,
            other => other,
        }
    }

    /// The collecting loops in a statement's own expressions, each body
    /// read with its variable typed as an element of what it walks.
    fn loops_have_result_condition(&mut self, s: &Stmt) -> bool {
        let exprs: Vec<&Expr> = match s {
            Stmt::Assign { value, .. } | Stmt::FieldAssign { value, .. } => vec![value],
            Stmt::MultiAssign { values, .. } => values.iter().collect(),
            Stmt::IndexAssign { index, value, .. } => vec![index, value],
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => vec![e],
            Stmt::If { condition, .. } | Stmt::While { condition, .. } => vec![condition],
            Stmt::For { iter, .. } => vec![iter],
            _ => vec![],
        };
        exprs.into_iter().any(|e| self.expr_loops_have_result_condition(e))
    }

    fn expr_loops_have_result_condition(&mut self, e: &Expr) -> bool {
        match e {
            Expr::ForExpr { var, iter, body } => self.expr_loops_have_result_condition(iter) || self.loop_list_can_fail(var, iter, body),
            Expr::Call { callee, args } => self.expr_loops_have_result_condition(callee) || args.iter().any(|a| self.expr_loops_have_result_condition(a)),
            Expr::BinOp { left, right, .. } => self.expr_loops_have_result_condition(left) || self.expr_loops_have_result_condition(right),
            Expr::UnaryOp { expr, .. } => self.expr_loops_have_result_condition(expr),
            Expr::FieldAccess { object, .. } => self.expr_loops_have_result_condition(object),
            Expr::Array(items) => items.iter().any(|i| self.expr_loops_have_result_condition(i)),
            Expr::Map(pairs) => pairs.iter().any(|(k, v)| self.expr_loops_have_result_condition(k) || self.expr_loops_have_result_condition(v)),
            Expr::Named { value, .. } => self.expr_loops_have_result_condition(value),
            _ => false,
        }
    }

    /// Infer a function's return type from its body.
    /// Into a function's body: its parameters typed from its signature,
    /// its first assignments new bindings (spec §7). The scope returned
    /// goes back to `leave_body`; the lowerings type a body between the
    /// two, as inference does.
    pub(crate) fn enter_body(&mut self, name: &str) -> BodyScope {
        let saved_vars = self.vars.clone();
        let mut assigned = std::collections::HashSet::new();
        if let Some(sig) = self.functions.get(name) {
            for (pname, pty) in &sig.params {
                self.vars.insert(pname.clone(), pty.clone());
                assigned.insert(pname.clone());
            }
        }
        let outer_assigned = self.body_assigned.replace(assigned);
        let outer_in_view = std::mem::replace(&mut self.in_view, self.views.contains(name));
        BodyScope { saved_vars, outer_assigned, outer_in_view }
    }

    pub(crate) fn leave_body(&mut self, scope: BodyScope) {
        self.vars = scope.saved_vars;
        self.body_assigned = scope.outer_assigned;
        self.in_view = scope.outer_in_view;
    }

    pub(crate) fn infer_function_body(&mut self, name: &str, body: &[Stmt]) {
        let scope = self.enter_body(name);
        let saved_vars = scope.saved_vars.clone();

        // Infer body statements
        for stmt in body {
            self.infer_stmt(stmt);
        }
        self.body_assigned = scope.outer_assigned;
        self.in_view = scope.outer_in_view;

        // Find return type
        let ret = self.find_return_type(body);
        // A fallible condition can propagate an error out of the function
        // (spec §11: Errors in Conditions) — the return type widens to Result;
        // so can a failed value assigned to a variable that holds no error
        let has_fallible_condition = self.body_has_result_condition(body) || self.body_refuses_assignment(name, body);
        if let Some(sig) = self.functions.get_mut(name) {
            sig.return_type = sig.return_type.unify(&ret);
            if has_fallible_condition && !matches!(sig.return_type, CrowType::Result(_)) {
                let inner = std::mem::replace(&mut sig.return_type, CrowType::Unknown);
                sig.return_type = CrowType::Result(Box::new(inner));
            }
        }

        // A variable ever assigned a float is a float for its whole scope,
        // and a fact's scope is the program: a view or handler assigning a
        // fact a float widens it from its declaration. A plain function's
        // local of the same name is a new binding and says nothing.
        let mut widened: Vec<String> = Vec::new();
        if self.views.contains(name) || self.view_handlers.contains(name) {
            for f in &self.facts {
                let before = saved_vars.get(f).cloned().unwrap_or(CrowType::Unknown);
                if matches!(before, CrowType::Int | CrowType::Unknown)
                    && matches!(self.vars.get(f), Some(CrowType::Float))
                {
                    widened.push(f.clone());
                }
            }
        }
        self.vars = saved_vars;
        for f in widened {
            self.vars.insert(f, CrowType::Float);
        }
    }

    /// Find the return type from a list of statements.
    fn find_return_type(&self, stmts: &[Stmt]) -> CrowType {
        let mut ret = CrowType::Void;
        for stmt in stmts {
            match stmt {
                Stmt::Return(Some(e)) => {
                    ret = ret.unify(&self.infer_expr_full(e));
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

    /// Inside a function, the first assignment of a name that is neither a
    /// parameter nor a fact is a new binding: its type is its own, not the
    /// module variable's of the same name. Reactive state keeps its type
    /// through a handler's assignment; every other module name is shadowed.
    fn fresh_binding(&mut self, name: &str) -> bool {
        let is_ui = self.is_ui;
        let facts = &self.facts;
        match &mut self.body_assigned {
            Some(assigned) => assigned.insert(name.to_string()) && !(is_ui && facts.contains(name)),
            None => false,
        }
    }

    /// Infer types from a statement (mainly assignments).
    /// The type a body's variable takes from a later value: a variable
    /// whose first value could not fail holds no error (spec,
    /// Reassignment), so a value that can fail lands as its payload — the
    /// lowering propagates the failure where it is assigned (`ms = push[ms,
    /// m]` inside `if m.ok` kept `ms` a list of numbers there, and a record
    /// built from it typed its field as a list that could fail). Only in a
    /// body, where the first assignment binds afresh: at the top level the
    /// fixpoint meets a declaration again, its own last type in hand.
    fn consumed(&self, existing: &CrowType, ty: CrowType) -> CrowType {
        match ty {
            CrowType::Result(inner) if self.body_assigned.is_some() && !matches!(existing, CrowType::Unknown | CrowType::Result(_)) => *inner,
            other => other,
        }
    }

    pub(crate) fn infer_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                let ty = self.infer_expr_full(value);
                if self.fresh_binding(name) {
                    self.vars.insert(name.clone(), ty);
                } else {
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    let ty = self.consumed(&existing, ty);
                    self.vars.insert(name.clone(), existing.unify(&ty));
                }
                // the columns a local reads are those of the value assigned
                // here, as the call-site scan and the lowering have them (a
                // cell read later in the body was typed by another frame of
                // the same name)
                if let Some(schema) = self.frame_schema_of(value) {
                    self.frame_schemas.insert(name.clone(), schema);
                }
            }
            Stmt::MultiAssign { names, values } => {
                for (name, value) in names.iter().zip(values.iter()) {
                    let ty = self.infer_expr(value);
                    let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
                    let ty = self.consumed(&existing, ty);
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
                let iter_type = Self::walked(self.infer_expr(iter));
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
    /// `derive x` reads as a variable typed by its body's return type.
    pub fn sync_derive_types(&mut self) {
        for d in self.derives.clone() {
            if let Some(sig) = self.functions.get(&d) {
                if !matches!(sig.return_type, CrowType::Unknown) {
                    self.vars.insert(d, sig.return_type.clone());
                }
            }
        }
    }

    /// Whether a call at `b` reaches the specialization made for `a`: every
    /// argument fits, and markup meets markup — a view given markup is not
    /// the same view given text.
    pub fn same_signature(a: &[CrowType], b: &[CrowType]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.compatible(y) && x.has_markup() == y.has_markup())
    }

    /// The specialization of `name` a call with these arguments reaches,
    /// the one rule the inference, the compiler and the interpreter share.
    pub fn specialization_for(&self, name: &str, args: &[Expr]) -> Option<String> {
        let clones = self.specializations.get(name)?;
        let call_sig: Vec<CrowType> = args
            .iter()
            .map(|a| match self.infer_expr_full(a) {
                CrowType::Result(inner) => *inner,
                CrowType::OptBoxStruct(n) => CrowType::Struct(n),
                other => other,
            })
            .collect();
        clones
            .iter()
            .find(|(csig, _)| Self::same_signature(csig, &call_sig))
            .or_else(|| clones.first())
            .map(|(_, m)| m.clone())
    }

    /// The call's type given its arguments' types, inferred once by the
    /// caller: a nested chain (a map literal is set[set[set[map[], …]]]
    /// after lowering) is walked once per level, not twice — twice per
    /// level doubled the work for every entry, and a 30-entry literal
    /// never finished.
    pub fn infer_call_base_typed(&self, name: &str, args: &[Expr], _callee: &Expr, arg_types: &[CrowType]) -> CrowType {
        let arg_ty = |a: &Expr| -> CrowType {
            match args.iter().position(|x| std::ptr::eq(x, a)) {
                Some(i) => arg_types[i].clone(),
                None => self.infer_expr(a),
            }
        };
        // a body's own list, map or text is indexed, whatever a module
        // another file imports names the same — the environment first, as
        // both backends look (a part's `rsi[jg]` where the program imports
        // chart)
        if self.body_assigned.as_ref().is_some_and(|a| a.contains(name)) {
            let var_type = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
            let unwrapped = match var_type {
                CrowType::Result(inner) => *inner,
                other => other,
            };
            return match unwrapped {
                CrowType::Array(inner) | CrowType::NumArray(inner) | CrowType::Map(inner) => *inner,
                CrowType::Str | CrowType::FrameCol => CrowType::Str,
                CrowType::Fn(_, ret) => *ret,
                // a local not typed yet is still not a module's function
                _ => CrowType::Unknown,
            };
        }
        // Struct constructor
        if self.structs.contains_key(name) {
            return CrowType::Struct(name.to_string());
        }
        // Inside a view an element, a chart, an icon or a bound input is
        // markup (a `select` of two arguments is the bound input there, as
        // both backends read it)
        if self.in_view
            && (crate::lower::UI_ELEMENTS.contains(&name)
                || matches!(name, "chart" | "icon" | "progress" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                || (name == "select" && args.len() == 2))
        {
            return CrowType::Markup;
        }
        // A view's tree is markup at every boundary a type can see — the
        // interpreter's views return strings, and compiled ones render to
        // one wherever a string is expected (a list, a module argument) —
        // and markup, never text, where it is placed as a child.
        if self.views.contains(name) {
            return CrowType::Markup;
        }
        // User-defined function call (specialization-aware: a monomorphized
        // callee's return type depends on this call's argument types)
        if let Some(sig) = self.functions.get(name) {
            if let Some(mangled) = self.specialization_for(name, args) {
                if let Some(clone_sig) = self.functions.get(&mangled) {
                    return clone_sig.return_type.clone();
                }
            }
            return sig.return_type.clone();
        }
        // Module function dispatch (overlapping builtins: check arg type)
        let is_overlapping = matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
        if is_overlapping && self.registry.has_function(name) {
            let arg_types: Vec<CrowType> = args.iter().map(|a| arg_ty(a)).collect();
            let has_numarray = arg_types.iter().any(|t| matches!(t, CrowType::NumArray(_)));
            if has_numarray {
                if let Some(t) = self.registry.type_of(name, args.len()) {
                    return t;
                }
            }
        }
        // An element-wise math function over a plain number is a number
        if crate::modules::math::SCALAR_UNARY.contains(&name) && args.len() == 1 && self.registry.has_function(name) {
            let t = arg_ty(&args[0]);
            let t = if let CrowType::Result(i) = t { *i } else { t };
            if matches!(t, CrowType::Int | CrowType::Float) { return CrowType::Float; }
        }
        // A frame function whose first argument is not a frame is the core
        // builtin of the same name (sort[xs], join[parts, sep] with frame
        // imported). While that argument's type is not known yet (a
        // derivation or a function read before its body was typed: unknown,
        // or nothing until inference knows), neither is the call's: a later
        // round decides, where a guess would stick in what returns it
        let arg0 = args.first().map(|a| Self::walked(arg_ty(a)));
        let frame_core_fallback = self.registry.module_name_of(name) == Some("frame")
            && crate::checker::BUILTINS.contains(&name)
            && !matches!(arg0, Some(CrowType::Frame | CrowType::FrameCol));
        if frame_core_fallback && matches!(arg0, Some(CrowType::Unknown | CrowType::Void)) {
            return CrowType::Unknown;
        }
        // Non-overlapping module functions
        if !is_overlapping && !frame_core_fallback && self.registry.has_function(name) {
            // A name two modules export (stack, unique, load: frame + math)
            // resolves by its first argument's type — the same dispatch the
            // emitters use — so a qualified call that lowering made bare
            // keeps its type.
            if self.registry.providers_of(name).len() > 1 {
                let arg0 = args.first().map(|a| arg_ty(a));
                let arg0 = arg0.map(|t| if let CrowType::Result(i) = t { *i } else { t });
                let pick = match arg0 {
                    Some(CrowType::Frame) | Some(CrowType::FrameCol) => Some("frame"),
                    Some(CrowType::NumArray(_)) | Some(CrowType::Array(_)) => Some("math"),
                    _ => None,
                };
                if let Some(m) = pick {
                    if let Some(t) = self.registry.type_of_in(m, name, args.len()) {
                        return t;
                    }
                }
            }
            if let Some(t) = self.registry.type_of(name, args.len()) {
                return t;
            }
        }

        // Builtins (check before vars to prevent builtins from being shadowed by Fn inference)
        match name {
            "log" | "cpu" | "gpu" | "assert" => CrowType::Void,
            "type" => CrowType::Str,
            // an error has no value, only a reason: a failed nothing, which
            // any value it meets decides the type of (unify)
            "error" => CrowType::Result(Box::new(CrowType::Void)),
            "map" => CrowType::Map(Box::new(CrowType::Unknown)),
            "range" => CrowType::Array(Box::new(CrowType::Int)),
            // Math builtins (scalar)
            "abs" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Int
            }
            "sqrt" => CrowType::Float,
            "floor" | "ceil" | "round" => CrowType::Int,
            "min" | "max" => {
                if args.len() >= 2 {
                    let t1 = arg_ty(&args[0]);
                    let t2 = arg_ty(&args[1]);
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
            // an algorithm it does not know, or a stored hash it cannot read
            "hash" => CrowType::Result(Box::new(CrowType::Str)),
            "split" => CrowType::Array(Box::new(CrowType::Str)),
            "starts" | "ends" | "contains" => CrowType::Bool,
            // Conversion builtins — fallible when parsing from string
            "int" => {
                if let Some(arg) = args.first() {
                    let arg_type = arg_ty(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str | CrowType::Json) {
                        return CrowType::Result(Box::new(CrowType::Int));
                    }
                }
                CrowType::Int
            }
            "float" => {
                if let Some(arg) = args.first() {
                    let arg_type = arg_ty(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str | CrowType::Json) {
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
                    let arr_type = arg_ty(arg);
                    if args.len() == 2 {
                        // a value that can fail is handed on as the call's
                        // error, never pushed: the items are its value's
                        let val_type = Self::walked(arg_ty(&args[1]));
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
            // the two joined: the first's form (a list, a tensor, text), items
            // from both — an empty `[]` takes its items from what it is
            // joined with
            "concat" if args.len() >= 2 => {
                let first = Self::walked(arg_ty(&args[0]));
                let second = Self::walked(arg_ty(&args[1]));
                match (&first, &second) {
                    (CrowType::Array(a), CrowType::Array(b) | CrowType::NumArray(b)) => CrowType::Array(Box::new(a.unify(b))),
                    (CrowType::NumArray(a), CrowType::NumArray(b) | CrowType::Array(b)) => CrowType::NumArray(Box::new(a.unify(b))),
                    (CrowType::Unknown, _) => second,
                    _ => first,
                }
            }
            "pop" | "sort" | "reverse" | "concat" => {
                if let Some(arg) = args.first() {
                    // Base type = with fallible args already unwrapped (the
                    // propagation block feeds these builtins plain values).
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t {
                        return *inner;
                    }
                    return t;
                }
                CrowType::Unknown
            }
            "set" => {
                // set[map, key, val] returns Map with value type from val arg
                if let Some(arg) = args.first() {
                    let map_type = arg_ty(arg);
                    if args.len() == 3 {
                        let val_type = arg_ty(&args[2]);
                        // A fallible value propagates out of the call (the map
                        // is built from the unwrapped value or not at all): the
                        // result is a fallible map of plain values, never a map
                        // of results. The same holds for a fallible map argument.
                        let (map_fallible, map_type) = match map_type {
                            CrowType::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let (val_fallible, val_type) = match val_type {
                            CrowType::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let existing_inner = if let CrowType::Map(inner) = &map_type {
                            *inner.clone()
                        } else {
                            CrowType::Unknown
                        };
                        let out = CrowType::Map(Box::new(existing_inner.unify(&val_type)));
                        return if map_fallible || val_fallible { CrowType::Result(Box::new(out)) } else { out };
                    }
                    return map_type;
                }
                CrowType::Unknown
            }
            "del" => {
                if let Some(arg) = args.first() {
                    return arg_ty(arg);
                }
                CrowType::Unknown
            }
            // Map builtins
            "keys" => CrowType::Array(Box::new(CrowType::Str)),
            "has" => CrowType::Bool,
            "values" => {
                if let Some(arg) = args.first() {
                    let map_type = arg_ty(arg);
                    if let CrowType::Map(inner) = map_type {
                        return CrowType::Array(inner);
                    }
                }
                CrowType::Array(Box::new(CrowType::Unknown))
            }
            "slice" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Unknown
            }
            "len" => CrowType::Int,
            // shell/env are readers: error values on failure, stamped with a
            // capture moment on success (same shape as read[])
            "shell" => CrowType::Result(Box::new(CrowType::Struct("CrowShell".to_string()))),
            "calendar" => CrowType::Map(Box::new(CrowType::Int)),
            // Regex + search builtins
            "matches" => CrowType::Bool,
            "any" | "all" => CrowType::Bool,
            "find" => {
                // Dispatch: find[str, pattern] → Result(record with the groups);
                // find[array, predicate] → Result(element)
                let arg0 = args.first().map(|a| arg_ty(a)).unwrap_or(CrowType::Unknown);
                match arg0 {
                    CrowType::Array(inner) => CrowType::Result(inner),
                    CrowType::NumArray(inner) => match inner.as_ref() {
                        CrowType::NumArray(_) => CrowType::Result(inner),
                        _ => CrowType::Result(inner),
                    },
                    _ => CrowType::Result(Box::new(CrowType::Struct("CrowFindResult".to_string()))),
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
            // Moments are numbers (epoch seconds); constructors are fallible
            "now" => CrowType::Float,
            "captured" => CrowType::Result(Box::new(CrowType::Float)),
            "chart" => CrowType::Str,
            "moment" | "parse_time" => CrowType::Result(Box::new(CrowType::Float)),
            "sleep" => CrowType::Int,
            // Background work: later[] is fire-and-forget (Void — ask
            // running[] for in-flight status)
            "later" => CrowType::Void,
            "running" => CrowType::Bool,
            // the session's notice: an effect held to the handler's commit,
            // and the record the runtime keeps (notice.text, notice.at)
            "notice" => CrowType::Void,
            // the page's signed-in reader: a map of strings the runtime
            // provides, set by two effects held to the handler's commit
            "__reader" => CrowType::Map(Box::new(CrowType::Str)),
            // a read a guard narrowed (lower.rs): the value, never its failure
            "__known" => arg_types.first().cloned().map(Self::walked).unwrap_or(CrowType::Unknown),
            "sign_in" | "sign_out" | "download" => CrowType::Void,
            // the files a test's session was handed, as maps of name and text
            "downloads" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
            // a message: whether the server took it; and the messages a
            // test's session sent, as maps of to, subject and body
            "mail" => CrowType::Result(Box::new(CrowType::Bool)),
            "smtp" => CrowType::Mailer,
            "mails" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
            "__notice_text" => CrowType::Str,
            "__notice_at" => CrowType::Float,
            "env" => CrowType::Result(Box::new(CrowType::Str)),
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
    /// `x = db.connect['url']` (or bare `connect`) binds x to a database.
    fn note_connection(&mut self, name: &str, value: &Expr) {
        if let Expr::Call { callee, args } = value {
            let is_connect = match callee.as_ref() {
                Expr::Ident(n) => n == "connect",
                Expr::FieldAccess { object, field } => field == "connect" && matches!(object.as_ref(), Expr::Ident(m) if m == "db"),
                _ => false,
            };
            if is_connect {
                if let Some(Expr::Str(url)) = args.first() {
                    self.conn_urls.retain(|(n, _)| n != name);
                    self.conn_urls.push((name.to_string(), url.clone()));
                }
            }
        }
    }

    /// The columns of the frame an expression produces, when they can be
    /// known: a literal query (described by the database), a function that
    /// returns one, or a frame operation over a known frame.
    pub fn frame_schema_of(&mut self, e: &Expr) -> Option<BTreeMap<String, CrowType>> {
        if let Some(v) = read_var(e) {
            return self.frame_schemas.get(v).cloned();
        }
        match e {
            Expr::Ident(v) => self.frame_schemas.get(v).cloned(),
            Expr::Call { callee, args } => {
                let name = match callee.as_ref() {
                    Expr::Ident(n) => n.clone(),
                    Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::Ident(m) if m == "db" || m == "frame") => field.clone(),
                    _ => return None,
                };
                match name.as_str() {
                    "query" => match args.get(1) {
                        Some(Expr::Str(sql)) => {
                            let handle = match args.first() { Some(Expr::Ident(v)) => Some(v.clone()), _ => None };
                            self.describe_sql(sql, handle.as_deref())
                        }
                        _ => None,
                    },
                    "filter" | "sort" | "head" | "tail" | "fill" | "stack" => args.first().and_then(|a| self.frame_schema_of(a)),
                    "new" => Some(BTreeMap::new()),
                    "with" => {
                        let mut base = args.first().and_then(|a| self.frame_schema_of(a)).unwrap_or_default();
                        if let (Some(Expr::Str(col)), Some(v)) = (args.get(1), args.get(2)) {
                            let vt = self.infer_expr(v);
                            let vt = if let CrowType::Result(i) = vt { *i } else { vt };
                            let cell = match vt { CrowType::Array(inner) => *inner, CrowType::NumArray(_) => CrowType::Float, CrowType::Int => CrowType::Int, CrowType::Float => CrowType::Float, CrowType::Bool => CrowType::Bool, _ => CrowType::Str };
                            base.insert(col.clone(), cell);
                        }
                        Some(base)
                    }
                    "select" => {
                        let base = args.first().and_then(|a| self.frame_schema_of(a))?;
                        let mut out = BTreeMap::new();
                        if let Some(Expr::Array(cols)) = args.get(1) {
                            for c in cols {
                                if let Expr::Str(n) = c { if let Some(t) = base.get(n) { out.insert(n.clone(), t.clone()); } }
                            }
                        }
                        Some(out)
                    }
                    "join" => {
                        let mut out = args.first().and_then(|a| self.frame_schema_of(a))?;
                        if let Some(right) = args.get(1).and_then(|a| self.frame_schema_of(a)) {
                            for (k, v) in right { out.entry(k).or_insert(v); }
                        }
                        Some(out)
                    }
                    other => self.fn_frame_schemas.get(other).cloned(),
                }
            }
            _ => None,
        }
    }

    /// Ask each declared database, in order, what the statement returns.
    /// Interpolated spans and `?` placeholders are unknown at compile
    /// time and stand in as NULL. A database that cannot be reached or
    /// does not understand the statement is skipped; when none answers,
    /// the cells read as text, as before, and the build says so once.
    /// The query's own connection answers when the handle is a known
    /// top-level one; a handle that is a parameter (a helper's `conn`) is
    /// unknown here, and every connection is asked until one answers.
    fn describe_sql(&mut self, sql: &str, handle: Option<&str>) -> Option<BTreeMap<String, CrowType>> {
        if !self.describe_queries || self.conn_urls.is_empty() { return None; }
        if let Some(cached) = self.describe_cache.get(sql) {
            if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
                eprintln!("DESCRIBE cached {:?} for `{}…`", cached, sql.chars().take(50).collect::<String>());
            }
            return cached.clone();
        }
        let own: Vec<(String, String)> = match handle {
            Some(h) => self.conn_urls.iter().filter(|(n, _)| n == h).cloned().collect(),
            None => Vec::new(),
        };
        let candidates: Vec<(String, String)> = if own.is_empty() { self.conn_urls.clone() } else { own };
        // `...` spans → NULL
        let mut scrubbed = String::with_capacity(sql.len());
        let mut in_span = false;
        for c in sql.chars() {
            if c == '`' { if !in_span { scrubbed.push_str("NULL"); } in_span = !in_span; continue; }
            if !in_span { scrubbed.push(c); }
        }
        let mut reasons = Vec::new();
        let mut found: Option<BTreeMap<String, CrowType>> = None;
        let names: Vec<String> = candidates.iter().map(|(v, _)| v.clone()).collect();
        for (var, url) in candidates {
            match crate::type_infer::describe_with_tables(&url, &scrubbed, &self.ddl) {
                Ok(cols) => {
                    schema_record::write(&var, &scrubbed, &cols);
                    let mut schema = BTreeMap::new();
                    for (n, kind) in cols {
                        let t = match kind.as_str() { "int" => CrowType::Int, "float" | "datetime" => CrowType::Float, "bool" => CrowType::Bool, _ => CrowType::Str };
                        schema.insert(n, t);
                    }
                    found = Some(schema);
                    break;
                }
                // a backend without describe (sqlite) is not a failure to report
                Err(e) if e.contains("describe is not supported") => {}
                Err(e) => reasons.push(format!("{}: {}", var, e)),
            }
        }
        if found.is_none() && !reasons.is_empty() {
            let head: String = sql.chars().take(60).collect();
            // what the database answered last time, kept in the project
            match names.iter().find_map(|v| schema_record::read(v, &scrubbed)) {
                Some(schema) => {
                    self.describe_notes.push(format!("note: the columns of `{}…` are typed from .crow/schema — no database answered ({})", head, reasons.join("; ")));
                    found = Some(schema);
                }
                None => self.describe_notes.push(format!("note: the columns of `{}…` could not be typed at compile time — cells read as text ({})", head, reasons.join("; "))),
            }
        }
        if std::env::var("CROW_DEBUG_SCHEMAS").is_ok() {
            eprintln!("DESCRIBE {:?} via {:?} for `{}…`", found, handle, sql.chars().take(50).collect::<String>());
        }
        self.describe_cache.insert(sql.to_string(), found.clone());
        found
    }

    /// Names of the functions a body calls or references.
    fn collect_called_fns_expr(e: &Expr, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Expr::Call { callee, args } => {
                if let Expr::Ident(n) = callee.as_ref() { out.insert(n.clone()); } else { Self::collect_called_fns_expr(callee, out); }
                for a in args { Self::collect_called_fns_expr(a, out); }
            }
            Expr::Ident(n) => { out.insert(n.clone()); }
            Expr::Named { value, .. } => Self::collect_called_fns_expr(value, out),
            Expr::BinOp { left, right, .. } => { Self::collect_called_fns_expr(left, out); Self::collect_called_fns_expr(right, out); }
            Expr::UnaryOp { expr, .. } => Self::collect_called_fns_expr(expr, out),
            Expr::Array(items) => for i in items { Self::collect_called_fns_expr(i, out); },
            Expr::Map(pairs) => for (k, v) in pairs { Self::collect_called_fns_expr(k, out); Self::collect_called_fns_expr(v, out); },
            Expr::FieldAccess { object, .. } => Self::collect_called_fns_expr(object, out),
            Expr::ForExpr { iter, body, .. } => { Self::collect_called_fns_expr(iter, out); Self::collect_called_fns_stmts(body, out); }
            _ => {}
        }
    }
    fn collect_called_fns_stmts(stmts: &[Stmt], out: &mut std::collections::BTreeSet<String>) {
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } => Self::collect_called_fns_expr(value, out),
                Stmt::MultiAssign { values, .. } => for v in values { Self::collect_called_fns_expr(v, out); },
                Stmt::ExprStmt(e) => Self::collect_called_fns_expr(e, out),
                Stmt::Return(Some(e)) => Self::collect_called_fns_expr(e, out),
                Stmt::If { condition, then_body, else_body } => {
                    Self::collect_called_fns_expr(condition, out);
                    Self::collect_called_fns_stmts(then_body, out);
                    if let Some(eb) = else_body { Self::collect_called_fns_stmts(eb, out); }
                }
                Stmt::For { iter, body, .. } => { Self::collect_called_fns_expr(iter, out); Self::collect_called_fns_stmts(body, out); }
                Stmt::While { condition, body } => { Self::collect_called_fns_expr(condition, out); Self::collect_called_fns_stmts(body, out); }
                Stmt::FieldAssign { value, .. } | Stmt::IndexAssign { value, .. } => Self::collect_called_fns_expr(value, out),
                _ => {}
            }
        }
    }

    /// An expression's type as every reader of strings sees it: markup is
    /// a string. Placing a child is the one place that asks for more
    /// (`infer_expr_full`).
    pub fn infer_expr(&self, expr: &Expr) -> CrowType {
        self.infer_expr_full(expr).text()
    }

    /// An expression's type, markup told apart from text.
    pub fn infer_expr_full(&self, expr: &Expr) -> CrowType {
        // a cell of a column whose type the database told us: frame.col[i]
        if let Expr::Call { callee, args } = expr {
            if args.len() == 1 {
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Some(t) = self.column_type(object, field) {
                        // a cell, not a gather by a list of indices
                        if !matches!(self.infer_expr(&args[0]), CrowType::NumArray(_) | CrowType::Array(_)) {
                            return t;
                        }
                    }
                }
            }
        }
        match expr {
            Expr::Int(_) => CrowType::Int,
            Expr::Float(_) => CrowType::Float,
            Expr::Bool(_) => CrowType::Bool,
            Expr::Str(s) => {
                if self.string_has_fallible_span(s) { CrowType::Result(Box::new(CrowType::Str)) } else { CrowType::Str }
            }
            // Lowering resolves named args before inference; type is the value's
            Expr::Named { value, .. } => self.infer_expr_full(value),
            Expr::Ident(name) => self.ident_type(name),
            Expr::BinOp { left, op, right } => self.binop_type(op, self.infer_expr(left), self.infer_expr(right)),
            Expr::UnaryOp { op, expr: e } => self.unary_type(op, self.infer_expr(e)),
            Expr::Call { callee, args } => {
                // every argument inferred once, here
                let arg_types: Vec<CrowType> = args.iter().map(|a| self.infer_expr(a)).collect();
                if let Expr::Ident(name) = callee.as_ref() {
                    return self.named_call_type(name, args, callee, &arg_types);
                }
                // Module call: module.func[args] or module.Struct[args]
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(module) = object.as_ref() {
                        if let Some(t) = self.module_call_type(module, field, &arg_types) {
                            return t;
                        }
                    }
                }
                // Array/Map/Json indexing or indirect function call
                self.index_type(self.infer_expr(callee), arg_types.into_iter().next())
            }
            Expr::FieldAccess { object, field } => self.field_type(object, self.infer_expr(object), field),
            Expr::Array(items) => Self::array_type(items.iter().map(|i| self.infer_expr_full(i))),
            Expr::Map(pairs) => Self::map_type(pairs.iter().map(|(_, v)| self.infer_expr(v))),
            Expr::ForExpr { var, iter, body } => self.for_type(var, self.infer_expr(iter), body),
        }
    }

    // ── typing rules: a node's type from its children's ────────────────
    //
    // Each rule takes the types of a node's children, as `infer_expr` gives
    // them (text, not markup), so the walk above and the lowering, which
    // types each IR node from its children's IR types, read one set of rules.

    /// A name read as a value: a variable, a module constant, a
    /// derivation's value, or a function held as a value.
    pub(crate) fn ident_type(&self, name: &str) -> CrowType {
        // User variables shadow module constants (matches codegen +
        // interpreter scoping: environment first, then constants)
        if let Some(ty) = self.vars.get(name) {
            return ty.clone();
        }
        // Module constants: pi, e, inf, nan
        if self.registry.has_constant(name) {
            return CrowType::Float;
        }
        // A derivation's name is its value, wherever it is read — as
        // a call argument too — never a function reference
        if self.derives.contains(name) {
            return self.functions.get(name).map(|s| s.return_type.clone()).unwrap_or(CrowType::Unknown);
        }
        // Check if it's a known function name — return as CrowType::Fn
        if let Some(sig) = self.functions.get(name) {
            let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
            return CrowType::Fn(param_types, Box::new(sig.return_type.clone()));
        }
        CrowType::Unknown
    }

    /// The type the database told us for a frame's column, `frame.col`:
    /// the type of each of its cells.
    pub(crate) fn column_type(&self, object: &Expr, field: &str) -> Option<CrowType> {
        let v = read_var(object)?;
        self.frame_schemas.get(v).and_then(|s| s.get(field)).cloned()
    }

    /// An operator's type from its operands'.
    pub(crate) fn binop_type(&self, op: &BinOp, lt: CrowType, rt: CrowType) -> CrowType {
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
                // Element-wise tensor ops return NumArray; frame
                // columns bridge into tensors
                if matches!(&lt_inner, CrowType::FrameCol)
                    || matches!(&rt_inner, CrowType::FrameCol)
                {
                    CrowType::NumArray(Box::new(CrowType::Float))
                } else if matches!(&lt_inner, CrowType::NumArray(_)) {
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

    /// A unary operator's type from its operand's.
    pub(crate) fn unary_type(&self, op: &UnaryOp, t: CrowType) -> CrowType {
        match op {
            UnaryOp::Not => {
                // not on a boolean mask negates element-wise → Array(Bool)
                let inner = if let CrowType::Result(i) = t { *i } else { t };
                if matches!(&inner, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                    CrowType::Array(Box::new(CrowType::Bool))
                } else {
                    CrowType::Bool
                }
            }
            UnaryOp::Neg => t,
        }
    }

    /// A call by name's type from its arguments' types, an argument's
    /// failure made the call's (auto-propagation) unless the callee renders
    /// what it is given.
    pub(crate) fn named_call_type(&self, name: &str, args: &[Expr], callee: &Expr, arg_types: &[CrowType]) -> CrowType {
        let base = self.infer_call_base_typed(name, args, callee, arg_types);
        // Auto-propagation: wrap in CrowResult if any arg is CrowResult
        // a view renders what it is given: a failed argument shows
        // its reason inside the tree, it never fails the view; a record
        // holds a failed value in its field, as a module's record does
        let skip = matches!(name, "log" | "assert" | "type" | "cpu" | "gpu" | "error" | "table" | "chart" | "__known")
            || self.structs.contains_key(name)
            || crate::lower::UI_ELEMENTS.contains(&name)
            || (self.in_view && crate::lower::is_ui_control(name, matches!(arg_types.first().cloned().map(Self::walked), Some(CrowType::Frame))))
            || self.views.contains(name);
        if !skip && !matches!(base, CrowType::Result(_) | CrowType::Void) {
            let has_result_arg = arg_types.iter().any(|t| matches!(t, CrowType::Result(_)));
            if has_result_arg {
                return CrowType::Result(Box::new(base));
            }
        }
        base
    }

    /// A call through a module's name (`math.zeros[…]`, `m.f[…]` after
    /// `use m`): its type, when the module is known.
    pub(crate) fn module_call_type(&self, module: &str, field: &str, arg_types: &[CrowType]) -> Option<CrowType> {
        // web.new[] → WebApp
        if module == "web" && field == "new" {
            return Some(CrowType::WebApp);
        }
        // Registry-module prefixed call (math.zeros[...],
        // or via alias: m.zeros after `use math as m`)
        let real = self.module_aliases.get(module).cloned().unwrap_or_else(|| module.to_string());
        if matches!(real.as_str(), "math" | "io" | "frame" | "db" | "doc" | "chart" | "math.stats" | "math.nn") && self.registry.has_function(field) {
            if let Some(t) = self.registry.type_of_in(&real, field, arg_types.len()) {
                // auto-propagation applies to qualified calls too:
                // a fallible argument makes the result fallible
                if !matches!(t, CrowType::Result(_) | CrowType::Void) && arg_types.iter().any(|a| matches!(a, CrowType::Result(_))) {
                    return Some(CrowType::Result(Box::new(t)));
                }
                return Some(t);
            }
        }
        let prefixed = format!("{}_{}", module, field);
        // Module struct constructor
        if self.structs.contains_key(&prefixed) {
            return Some(CrowType::Struct(prefixed));
        }
        // Module function call
        self.functions.get(&prefixed).map(|sig| sig.return_type.clone())
    }

    /// An index's type (`xs[i]`, `m[k]`, a function value applied) from the
    /// indexed value's type and the first index's.
    pub(crate) fn index_type(&self, callee_type: CrowType, index: Option<CrowType>) -> CrowType {
        match callee_type {
            CrowType::Array(inner) => *inner,
            CrowType::NumArray(inner) => {
                // Advanced indexing: tensor[tensor] → tensor (gather)
                if matches!(index, Some(CrowType::NumArray(_))) {
                    return CrowType::NumArray(inner);
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

    /// A field's type (`x.f`) from the object's type; a frame's schema and a
    /// module's variables are looked up by the object's name.
    pub(crate) fn field_type(&self, object: &Expr, obj_type: CrowType, field: &str) -> CrowType {
        // Json object field access stays Json (dynamic values)
        {
            let inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
            if matches!(inner, CrowType::Json)
                && !matches!(field, "ok" | "reason" | "kind" | "trace")
            {
                return CrowType::Json;
            }
        }
        // .ok on a frame column → its rows' validity; every other
        // value's .ok is its own status, a list's included
        if field == "ok" && matches!(&obj_type, CrowType::FrameCol) {
            return CrowType::Array(Box::new(CrowType::Bool));
        }
        // DateTime properties on strings → Int
        if matches!(&obj_type, CrowType::Str) {
            match field {
                "year" | "month" | "day" | "hour" | "minute" | "second" | "offset" => return CrowType::Int,
                _ => {}
            }
        }
        // Error-value metadata fields
        match field {
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
            match field {
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
            match field {
                "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                "columns" | "dtypes" => return CrowType::Array(Box::new(CrowType::Str)),
                _ => return CrowType::FrameCol, // column access by name → typed column
            }
        }
        // FrameCol properties
        if matches!(obj_type, CrowType::FrameCol) {
            match field {
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
        if let Expr::Ident(mod_name) = object {
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

    /// A collecting loop's type from its list's: the list of what its body
    /// yields, typed with the loop variable bound to an element.
    pub(crate) fn for_type(&self, var: &str, iter_type: CrowType, body: &[Stmt]) -> CrowType {
        // Determine element type from iterator
        let elem = match Self::walked(iter_type) {
            CrowType::Array(inner) | CrowType::NumArray(inner) => *inner,
            _ => CrowType::Unknown,
        };

        // Infer the yield type from the body's last expression, with
        // the loop variable bound — otherwise a body like f[10]
        // (f = loop var holding a fn) infers Unknown. A trailing if
        // (the filter pattern) yields from inside its branches.
        fn body_yield_type(ctx: &TypeContext, body: &[Stmt]) -> CrowType {
            match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
                Some(Stmt::ExprStmt(e)) => ctx.infer_expr_full(e),
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
        let last_type = if elem != CrowType::Unknown {
            let mut sub = self.clone();
            sub.vars.insert(var.to_string(), elem);
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

    /// A list literal's type from its items': numbers make a tensor, and
    /// nested numeric rows a tensor of higher rank.
    pub(crate) fn array_type(items: impl Iterator<Item = CrowType>) -> CrowType {
        let mut any = false;
        let mut inner = CrowType::Unknown;
        for t in items {
            any = true;
            inner = inner.unify(&t);
        }
        if !any {
            return CrowType::Array(Box::new(CrowType::Unknown));
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

    /// A map literal's type from its values'.
    pub(crate) fn map_type(values: impl Iterator<Item = CrowType>) -> CrowType {
        let mut vtype = CrowType::Unknown;
        for t in values {
            vtype = vtype.unify(&t);
        }
        CrowType::Map(Box::new(vtype))
    }
}

thread_local! {
    static DESCRIBE_HANDLES: std::cell::RefCell<std::collections::HashMap<String, Result<crow_db::DbHandle, String>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Describe a statement against a database URL, keeping one connection per
/// URL for the build. A SQLite file that does not exist yet is not created
/// by asking: only the program makes its database. For a program that makes
/// its own tables, a SQLite database that
/// does not exist yet — or one missing a table the statement reads — is
/// described from a scratch copy the program's `CREATE` statements build,
/// so a new app's columns are typed on its first build as on every later one.
pub fn describe_with_tables(url: &str, sql: &str, ddl: &[String]) -> Result<Vec<(String, String)>, String> {
    let sqlite_path = url.strip_prefix("sqlite://");
    let missing = sqlite_path.map_or(false, |p| p == ":memory:" || !std::path::Path::new(p).exists());
    if missing {
        if ddl.is_empty() {
            return Err(format!("{} does not exist yet", url));
        }
        return describe_on(&format!("{}#program", url), "sqlite://:memory:", ddl, sql);
    }
    match describe_on(url, url, &[], sql) {
        Err(e) if sqlite_path.is_some() && !ddl.is_empty() && e.contains("no such table") => describe_on(&format!("{}#program", url), "sqlite://:memory:", ddl, sql),
        other => other,
    }
}

/// Describe on the build's connection for `key` (opened on `url`, built from
/// `ddl` when it is new).
fn describe_on(key: &str, url: &str, ddl: &[String], sql: &str) -> Result<Vec<(String, String)>, String> {
    DESCRIBE_HANDLES.with(|h| {
        let mut map = h.borrow_mut();
        let handle = map.entry(key.to_string()).or_insert_with(|| {
            let db = crow_db::connect(url, Some(1))?;
            for statement in ddl {
                let _ = db.exec(statement, &[]);
            }
            Ok(db)
        });
        match handle {
            Ok(db) => {
                let mut cols = db.describe(sql)?;
                // SQLite types no computed column, and a statement whose
                // parameters match no row shows none: type those from the
                // statement and the tables' declared columns
                if url.starts_with("sqlite://") && cols.iter().any(|(_, k)| k == "unknown") {
                    let columns_of = |table: &str| -> Vec<(String, String)> {
                        let frame = match db.query(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")), &[]) {
                            Ok(f) => f,
                            Err(_) => return Vec::new(),
                        };
                        let text = |name: &str| -> Vec<String> {
                            frame.columns.iter().position(|c| c == name)
                                .and_then(|i| match frame.data[i].as_ref() { crow_runtime::frame::Column::Str(v) => Some(v.clone()), _ => None })
                                .unwrap_or_default()
                        };
                        text("name").into_iter().zip(text("type")).collect()
                    };
                    if let Some(kinds) = crate::sql_types::select_kinds(sql, &columns_of) {
                        if kinds.len() == cols.len() {
                            for ((_, k), typed) in cols.iter_mut().zip(kinds) {
                                if k == "unknown" {
                                    if let Some(t) = typed {
                                        *k = t;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(cols)
            }
            Err(e) => Err(e.clone()),
        }
    })
}

/// The variable an expression reads, seen through a guard's narrowing:
/// `__known[x]` reads x's value, and what is known of x (a frame's schema)
/// holds for it.
fn read_var(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(v) => Some(v),
        Expr::Call { callee, args } if matches!(callee.as_ref(), Expr::Ident(k) if k == crate::lower::KNOWN) && args.len() == 1 => read_var(&args[0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SQLite computed column is typed from the statement when no row
    /// answers the describe probe — finch's blotter P&L read as text in the
    /// compiled program while the interpreter read a number
    #[test]
    fn a_sqlite_computed_column_is_typed_without_a_row() {
        let dir = std::env::temp_dir().join(format!("crow_describe_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("book.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crow_db::connect(&url, Some(1)).unwrap().exec("CREATE TABLE trades (session_id INTEGER, shares INTEGER, entry_px REAL, exit_px REAL)", &[]).unwrap();
        let cols = describe_with_tables(&url, "SELECT shares, round((exit_px - entry_px) * shares, 2) AS pnl FROM trades WHERE session_id = ?", &[]).unwrap();
        assert_eq!(cols, vec![("shares".to_string(), "int".to_string()), ("pnl".to_string(), "float".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A new app makes its own SQLite tables when it starts: before its
    /// database exists, its first build types the columns from its own
    /// CREATE statements (a first build read them as text, a second as
    /// numbers, and the interpreter as numbers) — and asking creates nothing
    #[test]
    fn a_program_s_own_tables_type_its_first_build() {
        let dir = std::env::temp_dir().join(format!("crow_describe_own_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("new.db");
        let _ = std::fs::remove_file(&path);
        let src = format!(
            "use db\nbook = db.connect['sqlite://{}']\nexec[book, 'CREATE TABLE IF NOT EXISTS cards (id INTEGER PRIMARY KEY, title TEXT, stars INTEGER)']\nrows = query[book, 'SELECT id, title, stars FROM cards']\nlog[rows.stars[0]]\n",
            path.display()
        );
        let stmts = crate::lower::lower_program(&crate::build(&src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.describe_queries = true;
        ctx.infer_program(&stmts);
        let schema = ctx.frame_schemas.get("rows").cloned().unwrap_or_default();
        assert_eq!(schema.get("stars"), Some(&CrowType::Int), "{:?}", schema);
        assert_eq!(schema.get("title"), Some(&CrowType::Str));
        assert!(!path.exists(), "describing creates no database");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A widget's handler takes what the module sends as a field's handler
    /// takes its text: its untyped parameter is a string, so `int[z]` parses
    /// it (a compiled build cast the string and did not compile)
    #[test]
    fn a_widget_handler_takes_text() {
        let src = "use web, ui\nzoom = 3\nfn set_zoom[z]\n    zoom = int[z]\nview v[]\n    return widget['map', props = map[z = zoom], on = map[zoom = set_zoom]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions["set_zoom"].params[0].1, CrowType::Str);
    }

    /// A handler a test clicks with arguments is a handler, as one clicked
    /// by name is, and the arguments type the parameters nothing else types:
    /// the compiled program dispatches it by them (`crow test --compiled`)
    #[test]
    fn a_handler_a_test_clicks_with_arguments_is_typed_by_them() {
        let src = "use web, ui\nbase = 0.0\nfn begin[start]\n    base = start\nview v[]\n    return p['`base`']\npage['/', v]\ntest t[]\n    a = app[v]\n    click[a, begin[1000.0]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert!(ctx.view_handlers.contains("begin"));
        assert_eq!(ctx.functions["begin"].params[0].1, CrowType::Float);
    }

    /// A frame function over a value not typed yet is not guessed to be
    /// the core builtin of its name: `join` of two derivations a function
    /// builds is a frame join (it was typed as text joining, in the first
    /// round, and the guess stuck in the derivation's type)
    #[test]
    fn a_join_of_derivations_is_a_frame_join() {
        let src = "use web, ui, frame\nn = 0\nfn make[]\n    return with[frame.new[], 'k', [1, 2]]\nderive a\n    return make[]\nderive b\n    return make[]\nderive both\n    return join[a, b, 'k']\nview v[]\n    return p['`len[both]` `n`']\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let both = ctx.vars.get("both").cloned().map(TypeContext::walked);
        assert_eq!(both, Some(CrowType::Frame), "{:?}", ctx.vars.get("both"));
    }

    /// A function's local is its own: one named like a module variable
    /// the program assigns from the function's value does not type it (the
    /// call-site scan read the local as the module variable, and the
    /// assignment was refused as a type change)
    #[test]
    fn a_functions_local_is_not_the_module_variable_of_its_name() {
        let src = "fn last[xs]\n    r = push[xs, 1.0]\n    return r[0]\nr = last[[2.0]]\nlog[r]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.vars.get("r"), Some(&CrowType::Float));
        assert!(crate::lower_ir::check(&stmts, &ctx).is_ok());
    }

    /// ... nor does the module variable type the local: a function's
    /// local passed to another function gives it the local's type (the
    /// call-site scan unified the local with the module variable, and the
    /// struct it held typed the other function's parameter)
    #[test]
    fn a_module_variable_does_not_type_a_functions_local_of_its_name() {
        let src = "struct Pair [a, b]\nfn inner[x, last]\n    return last - x\nfn outer[n]\n    f = 0\n    total = 0\n    while f lt n\n        total = total + inner[1, f]\n        f = f + 1\n    return total\nf = Pair[1, 2]\nlog[outer[3]]\nlog[f.a]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions["inner"].params[1].1, CrowType::Int);
        assert!(ctx.vars.get("f").is_some_and(|t| t.to_rust_type().contains("Pair")), "{:?}", ctx.vars.get("f"));
    }

    /// A local indexed before its value is typed is not a function: the
    /// function it is passed to takes a list (a function guessed from
    /// `hs[0]` early in the fixpoint stuck to `edge`'s parameter)
    #[test]
    fn an_indexed_local_is_not_guessed_a_function() {
        let src = "struct Bars [h]\nfn edge[ys]\n    return ys[0] + ys[1]\nfn around[cd]\n    hs = slice[cd.h, 0, 2]\n    top = hs[0]\n    return edge[hs] + top\nlog[around[Bars[[1.0, 2.0, 3.0]]]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let ys = &ctx.functions["edge"].params[0].1;
        assert!(!matches!(ys, CrowType::Fn(_, _)), "{:?}", ys);
    }

    /// A function that sends mail for a handler holds it for the handler's
    /// commit, so it acts for the session with its handler, as one that
    /// says a notice does, and so does a function between them; one no
    /// handler calls keeps its own scope, and a mailer is a handle
    #[test]
    fn a_function_that_mails_for_a_handler_acts_for_the_session() {
        let src = "use web, ui\nuse web.mail\nmailer = smtp['h', 587, 'u', 'p']\nn = 0\nfn notify[m, to]\n    mail[m, to, 'Hello', 'x']\nfn between[m]\n    notify[m, 'a@b.c']\nfn alone[m]\n    mail[m, 'z@b.c', 'Hi', 'y']\nfn go[]\n    n = n + 1\n    between[mailer]\nview v[]\n    return button['Go', click = go]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert!(ctx.view_handlers.contains("notify") && ctx.view_handlers.contains("between"), "{:?}", ctx.view_handlers);
        assert!(!ctx.view_handlers.contains("alone"));
        assert_eq!(ctx.functions["notify"].params[0].1, CrowType::Mailer);
    }

    /// The program's own evidence types a handler before a test's click
    /// does: a widget sends text, so its handler takes text however a test
    /// clicks it
    #[test]
    fn a_widget_handler_takes_text_whatever_a_test_clicks() {
        let src = "use web, ui\nzoom = 3\nfn set_zoom[z]\n    zoom = int[z]\nview v[]\n    return widget['map', props = map[z = zoom], on = map[zoom = set_zoom]]\npage['/', v]\ntest t[]\n    a = app[v]\n    click[a, set_zoom[4]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions["set_zoom"].params[0].1, CrowType::Str);
    }

    /// A map literal is a nested set[] chain after lowering; inferring each
    /// level twice doubled the work per entry, and a 30-entry literal never
    /// finished. Each argument is inferred once, so this is instant.
    #[test]
    fn a_large_map_literal_infers_in_one_pass() {
        let entries: Vec<String> = (0..80).map(|i| format!("'k{}' = {}.5", i, i)).collect();
        let src = format!("m = map[{}]\nlog[len[m]]\n", entries.join(", "));
        let stmts = crate::build(&src).unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.vars.get("m"), Some(&CrowType::Map(Box::new(CrowType::Float))));
    }

    /// A local reusing a module variable's name is a new binding (spec §7):
    /// the function's type is its own, the module's variable keeps its type.
    #[test]
    fn a_local_shadowing_a_module_name_has_its_own_type() {
        let src = "fn f[]\n    x = 1\n    return x\n\nx = 'module'\nlog[f[], x]\n";
        let stmts = crate::build(src).unwrap();
        let stmts = crate::lower::lower_program(&stmts).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions.get("f").map(|s| s.return_type.clone()), Some(CrowType::Int));
        assert_eq!(ctx.vars.get("x"), Some(&CrowType::Str));
    }

    /// A variable whose first value could not fail refuses a failed one
    /// where it is assigned, so the function can fail there, as at a
    /// condition; a name first bound in a branch is the branch's own, and a
    /// variable whose first value could fail holds whatever it is given
    #[test]
    fn a_function_that_refuses_an_assignment_can_fail() {
        let src = "fn f[s]\n    x = 1.0\n    for c in [s]\n        x = float[c]\n    return 1\nfn g[s]\n    if s eq ''\n        y = 1.0\n    y = float[s]\n    return 1\nfn h[s]\n    z = float[s]\n    z = float['2']\n    return 1\nlog[f['1'], g['1'], h['1']]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let ret = |n: &str| ctx.functions.get(n).map(|s| s.return_type.clone());
        assert_eq!(ret("f"), Some(CrowType::Result(Box::new(CrowType::Int))));
        assert_eq!(ret("g"), Some(CrowType::Int));
        assert_eq!(ret("h"), Some(CrowType::Int));
    }
}

/// What a program's databases answered about its statements, kept in the
/// project's `.crow/schema`: one file per statement and the connection it
/// was asked on, named by a hash of both (an FNV-1a, the same on every
/// machine and release), holding the statement and its columns — name and
/// kind (`int` `float` `str` `bool` `datetime`), in the statement's order.
/// A build reads it when no database answers — a server down, a machine
/// without the data — so the program is typed as it was; a rehearsal
/// generates rows from it.
pub(crate) mod schema_record {
    use super::CrowType;
    use std::collections::BTreeMap;

    /// The FNV-1a hash of the parts, a stable file name
    pub(crate) fn fnv(parts: &[&str]) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                h ^= b'\n' as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            for b in p.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        format!("{:016x}", h)
    }

    fn file_for(var: &str, sql: &str) -> Option<std::path::PathBuf> {
        let dir = crate::schema_dir()?;
        Some(dir.join(format!("{}.json", fnv(&[var, sql]))))
    }

    /// Keep what `var` answered for `sql` (written only when it changed).
    pub(crate) fn write(var: &str, sql: &str, columns: &[(String, String)]) {
        let Some(path) = file_for(var, sql) else { return };
        let columns: Vec<[&str; 2]> = columns.iter().map(|(n, k)| [n.as_str(), k.as_str()]).collect();
        let text = match serde_json::to_string_pretty(&serde_json::json!({ "via": var, "sql": sql, "columns": columns })) {
            Ok(t) => t + "\n",
            Err(_) => return,
        };
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, text);
    }

    /// A record's statement and columns, from its file's text.
    pub(crate) fn parse(text: &str) -> Option<(String, Vec<(String, String)>)> {
        let j: serde_json::Value = serde_json::from_str(text).ok()?;
        let sql = j.get("sql")?.as_str()?.to_string();
        let cols = j
            .get("columns")?
            .as_array()?
            .iter()
            .filter_map(|c| {
                let pair = c.as_array()?;
                Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))
            })
            .collect();
        Some((sql, cols))
    }

    /// What `var` answered for `sql` when it last could, if it is kept.
    pub(crate) fn read(var: &str, sql: &str) -> Option<BTreeMap<String, CrowType>> {
        let (_, cols) = parse(&std::fs::read_to_string(file_for(var, sql)?).ok()?)?;
        Some(
            cols.into_iter()
                .map(|(n, k)| {
                    let t = match k.as_str() {
                        "int" => CrowType::Int,
                        "float" | "datetime" => CrowType::Float,
                        "bool" => CrowType::Bool,
                        _ => CrowType::Str,
                    };
                    (n, t)
                })
                .collect(),
        )
    }
}
