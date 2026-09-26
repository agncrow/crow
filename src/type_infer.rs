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
mod classify;
mod frames;
mod rules;
#[cfg(test)]
use frames::*;
pub use frames::describe_with_tables;
pub(crate) use frames::schema_record;

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
    saved_frames: Vec<(String, Option<BTreeMap<String, CrowType>>)>,
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
    /// The columns of the frame each function's parameter is given, when
    /// its calls agree on them — `(function, parameter) → columns`, `None`
    /// when they do not — as the last pass over the call sites found them:
    /// a frame handed to a function keeps its cells' types there
    pub param_frame_schemas: BTreeMap<(String, String), Option<BTreeMap<String, CrowType>>>,
    /// What the pass under way finds (the one before it is read meanwhile)
    param_frame_schemas_next: BTreeMap<(String, String), Option<BTreeMap<String, CrowType>>>,
    /// Keep each body's locals as the lowering types them (`crow build
    /// --types`, the oracle every step of inference's rewrite is held to).
    pub record_envs: bool,
    /// Each body's parameters and locals, as its lowering began with them.
    pub body_envs: BTreeMap<String, BTreeMap<String, CrowType>>,
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
            param_frame_schemas: BTreeMap::new(),
            record_envs: false,
            body_envs: BTreeMap::new(),
            param_frame_schemas_next: BTreeMap::new(),
            describe_queries: false,
            describe_cache: BTreeMap::new(),
            describe_notes: Vec::new(),
            current_fn_name: None,
        }
    }

    /// Main entry point: run multi-pass type inference on a program.
    pub fn infer_program(&mut self, stmts: &[Stmt]) {
        self.is_ui = self.is_ui || stmts.iter().any(|st| matches!(st, Stmt::Use { path, .. } if path == "ui" || path == "web.ui"));
        self.collect_ddl(stmts);
        self.collect_declarations(stmts);
        self.classify_handlers(stmts);

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
            self.param_frame_schemas_next.clear();
            for stmt in stmts {
                self.collect_call_types_stmt(stmt);
            }
            let frames_moved = self.param_frame_schemas_next != self.param_frame_schemas;
            self.param_frame_schemas = std::mem::take(&mut self.param_frame_schemas_next);

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
            if round >= 4 && before == after && !frames_moved {
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
                let saved_frames = self.param_frames_in_view(name);
                let saved_assigned = self.body_assigned.replace(assigned);
                for s in body { self.collect_call_types_stmt(s); }
                self.body_assigned = saved_assigned;
                self.param_frames_out_of_view(saved_frames);
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

                        // a frame argument's columns are its parameter's, when
                        // every call agrees on them
                        for (i, arg) in args.iter().enumerate() {
                            let Some((pname, _)) = sig.params.get(i) else { continue };
                            let Some(schema) = self.frame_schema_of(arg) else { continue };
                            if schema.is_empty() {
                                continue;
                            }
                            let key = (name.clone(), pname.clone());
                            match self.param_frame_schemas_next.get(&key) {
                                None => {
                                    self.param_frame_schemas_next.insert(key, Some(schema));
                                }
                                Some(Some(seen)) if *seen != schema => {
                                    self.param_frame_schemas_next.insert(key, None);
                                }
                                _ => {}
                            }
                        }

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
                    // Clones of a view are views (component monomorphization),
                    // and a handler's are handlers: each writes the session's
                    // state as the original does (a clone lowered as a plain
                    // function made its fact writes locals)
                    if self.views.contains(name.as_str()) {
                        self.views.insert(mangled.clone());
                    }
                    if self.view_handlers.contains(name.as_str()) {
                        self.view_handlers.insert(mangled.clone());
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
        let saved_frames = self.param_frames_in_view(name);
        BodyScope { saved_vars, saved_frames, outer_assigned, outer_in_view }
    }

    pub(crate) fn leave_body(&mut self, scope: BodyScope) {
        self.vars = scope.saved_vars;
        self.body_assigned = scope.outer_assigned;
        self.in_view = scope.outer_in_view;
        self.param_frames_out_of_view(scope.saved_frames);
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
        self.param_frames_out_of_view(scope.saved_frames);
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

    /// The specialization of `name` a call reaches, given its arguments'
    /// types with markup told apart: the one rule the inference, the
    /// compiler and the interpreter share. The caller has the types (the
    /// walker from the tree, the lowering from its lowered arguments).
    pub fn specialization_for(&self, name: &str, full: &[CrowType]) -> Option<String> {
        let clones = self.specializations.get(name)?;
        let call_sig: Vec<CrowType> = full
            .iter()
            .map(|t| match t {
                CrowType::Result(inner) => *inner.clone(),
                CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                other => other.clone(),
            })
            .collect();
        clones
            .iter()
            .find(|(csig, _)| Self::same_signature(csig, &call_sig))
            .or_else(|| clones.first())
            .map(|(_, m)| m.clone())
    }

    /// What a collecting loop's body yields, walked with the loop variable
    /// bound to the element type: its last expression, or from inside the
    /// branches of a trailing `if` (the filter pattern). Unknown when the
    /// body says nothing yet (a later pass settles it).
    pub(crate) fn loop_yield_type(&self, var: &str, iter_type: CrowType, body: &[Stmt]) -> CrowType {
        let elem = match Self::walked(iter_type) {
            CrowType::Array(inner) | CrowType::NumArray(inner) => *inner,
            _ => CrowType::Unknown,
        };
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
        if elem != CrowType::Unknown {
            let mut sub = self.clone();
            sub.vars.insert(var.to_string(), elem);
            body_yield_type(&sub, body)
        } else {
            body_yield_type(self, body)
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
                // every argument inferred once, here: its type with markup
                // told apart, which choosing a specialization reads, and the
                // text every other rule reads (walked twice, a nesting of
                // calls cost twice per level — the layout engine never ended)
                let full: Vec<CrowType> = args.iter().map(|a| self.infer_expr_full(a)).collect();
                let arg_types: Vec<CrowType> = full.iter().map(|t| t.text()).collect();
                if let Expr::Ident(name) = callee.as_ref() {
                    return self.named_call_type(name, args, callee, &arg_types, &full);
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
            Expr::ForExpr { var, iter, body } => Self::for_type(self.loop_yield_type(var, self.infer_expr(iter), body)),
        }
    }

    // ── typing rules: a node's type from its children's ────────────────
    //
    // Each rule takes the types of a node's children, as `infer_expr` gives
    // them (text, not markup), so the walk above and the lowering, which
    // types each IR node from its children's IR types, read one set of rules.

}

/// Every type inference found, in one canonical text: each function's
/// signature with its body's locals as the lowering began with them, the
/// specializations, the top level's variables, the frames' columns, the
/// classification and the records. `crow build --types` prints it; each
/// step of inference's rewrite keeps it the same (core-rewrite-notes §10).
pub fn dump_types(ctx: &TypeContext) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "functions:");
    for (name, sig) in &ctx.functions {
        let params: Vec<String> = sig.params.iter().map(|(p, t)| format!("{}: {:?}", p, t)).collect();
        let _ = writeln!(out, "  {}({}) -> {:?}", name, params.join(", "), sig.return_type);
        if let Some(env) = ctx.body_envs.get(name) {
            for (n, t) in env {
                let _ = writeln!(out, "    {}: {:?}", n, t);
            }
        }
    }
    for (name, env) in ctx.body_envs.iter().filter(|(n, _)| n.starts_with("test ")) {
        let _ = writeln!(out, "  {}", name);
        for (n, t) in env {
            let _ = writeln!(out, "    {}: {:?}", n, t);
        }
    }
    let _ = writeln!(out, "specializations:");
    for (name, clones) in &ctx.specializations {
        for (sig, clone) in clones {
            let _ = writeln!(out, "  {} {:?} -> {}", name, sig, clone);
        }
    }
    let _ = writeln!(out, "top:");
    for (n, t) in &ctx.vars {
        let _ = writeln!(out, "  {}: {:?}", n, t);
    }
    let _ = writeln!(out, "frames:");
    for (n, s) in &ctx.frame_schemas {
        let _ = writeln!(out, "  {}: {:?}", n, s);
    }
    let _ = writeln!(out, "function frames:");
    for (n, s) in &ctx.fn_frame_schemas {
        let _ = writeln!(out, "  {}: {:?}", n, s);
    }
    let _ = writeln!(out, "parameter frames:");
    for ((f, p), s) in &ctx.param_frame_schemas {
        let _ = writeln!(out, "  {}.{}: {:?}", f, p, s);
    }
    let set = |s: &std::collections::BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join(" ");
    let _ = writeln!(out, "views: {}", set(&ctx.views));
    let _ = writeln!(out, "handlers: {}", set(&ctx.view_handlers));
    let _ = writeln!(out, "derives: {}", set(&ctx.derives));
    let _ = writeln!(out, "facts: {}", set(&ctx.facts));
    let _ = writeln!(out, "records:");
    for (n, s) in &ctx.structs {
        let fields: Vec<String> = s.fields.iter().map(|(f, t)| format!("{}: {:?}", f, t)).collect();
        let _ = writeln!(out, "  {} {{ {} }}", n, fields.join(", "));
    }
    out
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
        // (no column is declared NOT NULL: each can hold NULL)
        assert_eq!(cols, vec![("shares".to_string(), "int?".to_string()), ("pnl".to_string(), "float?".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A statement with a list parameter (`IN ?`) is asked about as the list
    /// of one it is sent as: no database prepares `IN ?`
    #[test]
    fn a_list_parameter_is_described_as_a_list_of_one() {
        assert_eq!(in_lists_as_one("SELECT a FROM t WHERE b IN ? AND c NOT IN  ? AND d = '? IN ?' AND e = ? AND MIN(f) > ?"), "SELECT a FROM t WHERE b IN (?) AND c NOT IN  (?) AND d = '? IN ?' AND e = ? AND MIN(f) > ?");
        let dir = std::env::temp_dir().join(format!("crow_describe_in_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("in.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crow_db::connect(&url, Some(1)).unwrap().exec("CREATE TABLE news (sym TEXT NOT NULL, at REAL NOT NULL)", &[]).unwrap();
        let cols = describe_with_tables(&url, "SELECT sym, at FROM news WHERE sym IN ?", &[]).unwrap();
        assert_eq!(cols, vec![("sym".to_string(), "str".to_string()), ("at".to_string(), "float".to_string())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A statement described twice within the minute is asked once: the
    /// second program lowered in a process (each test block's) takes the
    /// first's answer
    #[test]
    fn a_statement_is_described_once_a_minute() {
        let dir = std::env::temp_dir().join(format!("crow_describe_kept_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("kept.db");
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}", path.display());
        crow_db::connect(&url, Some(1)).unwrap().exec("CREATE TABLE bars (sym TEXT NOT NULL, px REAL)", &[]).unwrap();
        let before = DESCRIBES_ASKED.with(|n| n.get());
        let first = describe_with_tables(&url, "SELECT sym, px FROM bars WHERE sym = ?", &[]).unwrap();
        let second = describe_with_tables(&url, "SELECT sym, px FROM bars WHERE sym = ?", &[]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, vec![("sym".to_string(), "str".to_string()), ("px".to_string(), "float?".to_string())]);
        assert_eq!(DESCRIBES_ASKED.with(|n| n.get()) - before, 1, "the second describe asked again");
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
        // an INTEGER PRIMARY KEY is the row's id, never NULL; the others
        // are not declared NOT NULL, so their cells can be missing
        assert_eq!(schema.get("id"), Some(&CrowType::Int), "{:?}", schema);
        assert_eq!(schema.get("stars"), Some(&CrowType::Result(Box::new(CrowType::Int))), "{:?}", schema);
        assert_eq!(schema.get("title"), Some(&CrowType::Result(Box::new(CrowType::Str))));
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

