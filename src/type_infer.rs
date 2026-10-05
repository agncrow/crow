/// Type inference: the front end both modes share, before the IR.

use std::collections::BTreeMap;
use crate::ast::*;

/// A tensor's dimensions as its type knows them, outermost first: a size,
/// or unknown where the program computes it.
pub type Dims = Vec<Option<usize>>;

/// Two tensors' shapes where ways through meet, or a function's calls hand
/// it tensors: the dimensions they agree on; any other rank, or no shape,
/// is no shape. What is not known stays not known: a join never invents a
/// dimension.
/// A list's shape as a tensor: a vector of numbers, a matrix of rows (one
/// dimension more than its rows have), unknown for anything else.
fn list_dims(item: &CrowType) -> Option<Dims> {
    match item {
        CrowType::Int | CrowType::Float => Some(vec![None]),
        CrowType::NumArray(_, d) => {
            let mut out = vec![None];
            out.extend(d.clone().unwrap_or_else(|| vec![None]));
            Some(out)
        }
        _ => None,
    }
}

/// What two shapes agree on: each dimension both state, unknown where
/// they differ; the rank, where one side knows it and the other knows
/// nothing (a function handed a matrix literal by one call and a record's
/// tensor by another still walks rows); nothing where the ranks differ.
pub fn join_dims(a: &Option<Dims>, b: &Option<Dims>) -> Option<Dims> {
    match (a, b) {
        (Some(a), Some(b)) if a.len() == b.len() => Some(a.iter().zip(b).map(|(x, y)| if x == y { *x } else { None }).collect()),
        (Some(a), None) | (None, Some(a)) => Some(vec![None; a.len()]),
        _ => None,
    }
}

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
    /// Numeric array backed by CrowTensor (f64 internally). Inner type
    /// tracks original element type; the shape is its dimensions where the
    /// program states them (second-half-design.md §3) — none where its
    /// rank shows only as it runs.
    NumArray(Box<CrowType>, Option<Dims>),
    /// Sparse matrix backed by CrowSparse (CSR format).
    Sparse,
    /// Column-oriented data frame: its columns, when the program knows
    /// them (none: nothing is known of them).
    Frame(Option<std::sync::Arc<Cols>>),
    /// A single typed column from a frame (CrowColumn wrapper).
    FrameCol,
    /// Web application (crow_web::server::CrowApp).
    WebApp,
    /// Web HTTP response (crow_web::core::CrowResponse).
    WebResponse,
    /// Web HTTP request (crow_web::core::CrowRequest).
    WebRequest,
    /// Database connection handle (crow_db::DbHandle) — `use db` connect[].
    DbHandle,
    /// Database transaction handle (&dyn crow_db::DbTx) — transact[] callback param.
    DbTx,
    /// An SMTP mailer (crow_runtime::sys::Mailer) — `use web.mail` smtp[].
    Mailer,
    /// A WebSocket client's connection (crow_runtime::sys::Socket) — `socket[url]`.
    Socket,
    /// Bytes (crow_runtime::sys::Bytes): what a connection reads and writes.
    Bytes,
    /// A connection `sys.open` made (crow_runtime::sys::CrowStream).
    Stream,
    /// A test's app session (`a = app[render]`), which the harness's words
    /// read and drive: `html[a]`, `elements[a, …]`, `click[a, …]`.
    App,
    Void,
    Unknown,
}

/// The effects a session keeps or holds for its handler's commit: a
/// function that makes one acts for the session.
mod calls;
mod classify;
mod frames;
pub(crate) use frames::mark_nullable;
mod rules;
pub(crate) mod settle;
#[cfg(test)]
use frames::*;
pub use frames::describe_with_tables;

const SESSION_EFFECTS: &[&str] = &["notice", "sign_in", "sign_out", "download", "mail"];

/// What a type knows beyond its kind that an assignment can change
/// (`CrowType::detail`).
#[derive(Debug, PartialEq)]
pub enum Detail<'a> {
    Columns(&'a Option<std::sync::Arc<Cols>>),
    Shape(Option<&'a Dims>),
}

impl Detail<'_> {
    /// Whether two details are of one kind: columns and columns, a shape
    /// and a shape.
    pub fn same_kind(&self, other: &Detail<'_>) -> bool {
        matches!((self, other), (Detail::Columns(_), Detail::Columns(_)) | (Detail::Shape(_), Detail::Shape(_)))
    }
}

/// A shape as a message prints it: `[2, 3]`, `?` for an unknown dimension.
pub fn shown_dims(dims: &Dims) -> String {
    let each: Vec<String> = dims.iter().map(|d| d.map_or("?".to_string(), |n| n.to_string())).collect();
    format!("[{}]", each.join(", "))
}

impl CrowType {
    /// A tensor of `elem`s whose shape nothing states.
    pub fn tensor(elem: CrowType) -> CrowType {
        CrowType::NumArray(Box::new(elem), None)
    }

    /// What a local's type knows beyond its kind that an assignment can
    /// change: a frame's columns, a tensor's shape. A local holding either
    /// is bound anew where it changes, and has what ways through share
    /// where they meet (lower_ir/frames.rs).
    pub fn detail(&self) -> Option<Detail<'_>> {
        match self {
            CrowType::Frame(cols) => Some(Detail::Columns(cols)),
            CrowType::NumArray(_, dims) => Some(Detail::Shape(dims.as_ref())),
            CrowType::Result(inner) => inner.detail(),
            _ => None,
        }
    }

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
            CrowType::NumArray(..) => "crow_runtime::tensor::CrowTensor".into(),
            CrowType::Sparse => "crow_runtime::sparse::CrowSparse".into(),
            CrowType::Frame(_) => "crow_runtime::frame::CrowFrame".into(),
            CrowType::FrameCol => "crow_runtime::frame::CrowColumn".into(),
            CrowType::WebApp => "crow_web::server::CrowApp".into(),
            CrowType::WebResponse => "crow_web::core::CrowResponse".into(),
            CrowType::WebRequest => "crow_web::core::CrowRequest".into(),
            CrowType::DbHandle => "crow_db::DbHandle".into(),
            CrowType::DbTx => "&dyn crow_db::DbTx".into(),
            CrowType::Mailer => "crow_runtime::sys::Mailer".into(),
            CrowType::Socket => "crow_runtime::sys::Socket".into(),
            CrowType::Bytes => "crow_runtime::sys::Bytes".into(),
            CrowType::Stream => "crow_runtime::sys::CrowStream".into(),
            // (a test's session: a test block runs interpreted)
            CrowType::App => "()".into(),
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
            CrowType::NumArray(..) => "crow_runtime::tensor::CrowTensor::zeros(vec![0])".into(),
            CrowType::Sparse => "CrowSparse::from_coo(&[], &[], &[], 0, 0)".into(),
            CrowType::Frame(_) => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])".into(),
            CrowType::FrameCol => "crow_runtime::frame::CrowColumn { data: std::sync::Arc::new(crow_runtime::frame::Column::Int(vec![])), valid: std::sync::Arc::new(vec![]) }".into(),
            CrowType::WebApp => "crow_web::server::CrowApp::new()".into(),
            CrowType::WebResponse => "crow_web::core::CrowResponse::default()".into(),
            CrowType::WebRequest => "crow_web::core::CrowRequest::new()".into(),
            CrowType::DbHandle => "panic!(\"no default db connection\")".into(),
            CrowType::DbTx => "panic!(\"no default db transaction\")".into(),
            CrowType::Mailer => "crow_runtime::sys::Mailer::default()".into(),
            CrowType::Socket => "crow_runtime::sys::Socket::closed()".into(),
            CrowType::Bytes => "crow_runtime::sys::Bytes::default()".into(),
            CrowType::Stream => "crow_runtime::sys::CrowStream::closed()".into(),
            CrowType::App => "()".into(),
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

    /// A frame's columns — of a frame, or of one that can fail: two call
    /// signatures whose frames differ in their columns are two
    /// specializations, so each reads the columns it is handed.
    pub fn frame_cols(&self) -> Option<&Option<std::sync::Arc<Cols>>> {
        match self {
            CrowType::Frame(c) => Some(c),
            CrowType::Result(t) => t.frame_cols(),
            _ => None,
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
            CrowType::NumArray(..) => "array".into(),
            CrowType::Sparse => "sparse".into(),
            CrowType::Frame(_) => "frame".into(),
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
            CrowType::DbHandle => "db.conn".into(),
            CrowType::DbTx => "db.tx".into(),
            CrowType::Mailer => "mailer".into(),
            CrowType::Socket => "socket".into(),
            CrowType::Bytes => "bytes".into(),
            CrowType::Stream => "stream".into(),
            CrowType::App => "app".into(),
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
            (CrowType::NumArray(..), CrowType::Array(inner)) | (CrowType::Array(inner), CrowType::NumArray(..)) => {
                matches!(inner.as_ref(), CrowType::Int | CrowType::Float | CrowType::Unknown)
            }
            (CrowType::NumArray(..), CrowType::NumArray(..)) => true,
            // a frame is a frame whatever its columns: a reassignment that
            // changes them binds the variable anew
            (CrowType::Frame(_), CrowType::Frame(_)) => true,
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
            (CrowType::NumArray(a, da), CrowType::NumArray(b, db)) => CrowType::NumArray(Box::new(a.unify(b)), join_dims(da, db)),
            (CrowType::Array(a), CrowType::Array(b)) => CrowType::Array(Box::new(a.unify(b))),
            // numbers held as a list meet numbers held as a tensor: the
            // first form stays (the lowering converts the other where they
            // meet), and what either knows of the items is kept (an empty
            // list and a tensor of floats are floats)
            (CrowType::Array(a), CrowType::NumArray(b, _)) => CrowType::Array(Box::new(a.unify(b))),
            (CrowType::NumArray(a, da), CrowType::Array(b)) => {
                let elem = match a.unify(b) {
                    CrowType::NumArray(e, _) => *e,
                    e => e,
                };
                CrowType::NumArray(Box::new(elem), join_dims(da, &list_dims(b)))
            }
            (CrowType::Map(a), CrowType::Map(b)) => CrowType::Map(Box::new(a.unify(b))),
            // two ways a frame could have come: the columns both give (a
            // frame whose columns are not worked out yet adds nothing — one
            // whose columns show only as it runs is a frame of none known,
            // open)
            (CrowType::Frame(a), CrowType::Frame(b)) => CrowType::Frame(match (a, b) {
                (Some(a), Some(b)) => Some(std::sync::Arc::new(Cols::join(a, b))),
                (Some(c), None) | (None, Some(c)) => Some(c.clone()),
                (None, None) => None,
            }),
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

/// A frame's columns, as the program knows them: each one's cell type (a
/// `Result` where a cell can be missing), and `open` when a column the
/// program cannot name may be there too (one added under a name a variable
/// holds).
#[derive(Debug, Clone, PartialEq)]
pub struct Cols {
    pub cols: BTreeMap<String, CrowType>,
    pub open: bool,
}

impl Cols {
    /// Exactly these columns.
    pub fn closed(cols: BTreeMap<String, CrowType>) -> std::sync::Arc<Cols> {
        std::sync::Arc::new(Cols { cols, open: false })
    }

    /// A frame whose columns show only as the program runs (a file loaded
    /// then, a query no database described): none known, open.
    pub fn dynamic() -> std::sync::Arc<Cols> {
        std::sync::Arc::new(Cols { cols: BTreeMap::new(), open: true })
    }

    /// The columns two ways through give: those both have, their types
    /// joined (a cell one side can miss can be missed); open where their
    /// columns differ, or either is.
    pub fn join(a: &Cols, b: &Cols) -> Cols {
        let mut cols = BTreeMap::new();
        for (name, ta) in &a.cols {
            if let Some(tb) = b.cols.get(name) {
                cols.insert(name.clone(), ta.unify(tb));
            }
        }
        let open = a.open || b.open || cols.len() != a.cols.len() || cols.len() != b.cols.len();
        Cols { cols, open }
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
    outer_fn_name: Option<String>,
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
    /// The functions `grad` differentiates whose parameters the seek
    /// guessed floats: a call of the gradient that hands tensors retypes
    /// them (the lowering's Index path), and takes the name out
    pub grad_forced: std::collections::BTreeSet<String>,
    /// While a function body is inferred: the names it has assigned so far
    /// (its parameters to begin with). The first assignment of another
    /// name binds a new local (spec §7), whatever the module calls it.
    pub(crate) body_assigned: Option<std::collections::HashSet<String>>,
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
    /// What the program's own tests call its functions with, as they type
    /// them (`type_tested`): a program that imports this one as a part
    /// types the part's functions from them, since a part's tests are not
    /// its importer's.
    pub(crate) tested: Vec<(String, Vec<CrowType>)>,
    /// The parameters (and record fields) only a test's call or click
    /// typed, with the type it gave each: a later reading of the tests
    /// refines one nothing else has changed since, as the functions its
    /// arguments come from settle.
    by_tests: BTreeMap<(String, usize), CrowType>,
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
        // a request's answer (`request[url, method, body]`)
        structs.insert(
            "CrowReply".to_string(),
            StructSig {
                fields: vec![
                    ("status".to_string(), CrowType::Int),
                    ("body".to_string(), CrowType::Str),
                    ("headers".to_string(), CrowType::Map(Box::new(CrowType::Str))),
                ],
            },
        );
        // a connection's answer (`sys.exchange`, `sys.fetch`: the layer the
        // readers written in Crow stand on)
        structs.insert(
            "CrowAnswer".to_string(),
            StructSig {
                fields: vec![
                    ("status".to_string(), CrowType::Int),
                    ("phrase".to_string(), CrowType::Str),
                    ("headers".to_string(), CrowType::Map(Box::new(CrowType::Str))),
                    ("body".to_string(), CrowType::Bytes),
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
            grad_forced: std::collections::BTreeSet::new(),
            body_assigned: None,
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
            registry: crate::modules::ModuleRegistry::new(),
            conn_urls: Vec::new(),
            ddl: Vec::new(),
            record_envs: false,
            body_envs: BTreeMap::new(),
            describe_queries: false,
            describe_cache: BTreeMap::new(),
            describe_notes: Vec::new(),
            tested: Vec::new(),
            by_tests: BTreeMap::new(),
            current_fn_name: None,
        }
    }

    /// Main entry point: the program's signatures, found by lowering its
    /// bodies (F5, `lower_ir::infer_by_lowering`).
    pub fn infer_program(&mut self, stmts: &[Stmt]) {
        crate::lower_ir::infer_by_lowering(self, stmts);
        if std::env::var("CROW_DEBUG_TYPES").is_ok() {
            for (n, t) in &self.vars {
                eprintln!("VAR {} : {:?}", n, t);
            }
            for (n, sig) in &self.functions {
                eprintln!("FN {} params={:?} ret={:?}", n, sig.params, sig.return_type);
            }
        }
    }

    /// Before any body: the program's tables, declarations and handlers,
    /// and a derivation's name read as a variable of its body's type.
    pub(crate) fn begin_inference(&mut self, stmts: &[Stmt]) {
        self.is_ui = self.is_ui || stmts.iter().any(|st| matches!(st, Stmt::Use { path, .. } if path == "ui" || path == "web.ui"));
        self.collect_ddl(stmts);
        self.collect_declarations(stmts);
        self.classify_handlers(stmts);
        self.sync_derive_types();
    }

    /// Every function's return type and parameters: a round of inference
    /// that moves none of them has settled.
    pub(crate) fn signatures(&self) -> Vec<(String, CrowType, Vec<(String, CrowType)>)> {
        self.functions.iter().map(|(n, s)| (n.clone(), s.return_type.clone(), s.params.clone())).collect()
    }

    /// The handler calls the program's tests click (`click[a, h[4]]`), each
    /// with its arguments' types: a parameter nothing in the program types
    /// takes the click's (the program's own evidence — a view's call, a
    /// table's action, a widget's text — comes first). Whether any moved.
    pub(crate) fn type_clicked(&mut self, clicks: &[(String, Vec<CrowType>)]) -> bool {
        // every click of a handler met, argument by argument: an int and a
        // float make a float, whichever click comes first
        let mut met: BTreeMap<(String, usize), CrowType> = BTreeMap::new();
        for (h, types) in clicks {
            if self.functions.get(h).is_none_or(|sig| sig.params.len() != types.len()) {
                continue;
            }
            for (i, t) in types.iter().enumerate() {
                // a value that can fail reaches the handler as what it holds,
                // as a call's argument does (`clock.jump[sig - 120.0]` where
                // `sig` came from parse_time: compiled, the parameter had
                // been made a result, and the click's number never fit it)
                let t = reaching(t);
                if !matches!(t, CrowType::Unknown) {
                    met.entry((h.clone(), i)).and_modify(|m| *m = m.unify(&t)).or_insert_with(|| t.clone());
                }
            }
        }
        let mut moved = false;
        for ((h, i), t) in met {
            moved |= self.type_by_tests(&h, i, &t, true);
        }
        moved
    }

    /// A parameter (or a record's field) typed by what the tests hand it:
    /// one nothing typed yet, or one the tests typed before whose type
    /// nothing else has changed since — the tests are read again once the
    /// functions their arguments come from settle, and a frame whose
    /// columns were not worked out then is known now. Whether it moved.
    fn type_by_tests(&mut self, f: &str, i: usize, t: &CrowType, params_only: bool) -> bool {
        let before = self.by_tests.get(&(f.to_string(), i)).cloned();
        let slot = match self.functions.get_mut(f) {
            Some(sig) => sig.params.get_mut(i),
            None if params_only => None,
            None => self.structs.get_mut(f).and_then(|st| st.fields.get_mut(i)),
        };
        let Some(p) = slot else { return false };
        let ours = matches!(p.1, CrowType::Unknown) || before.as_ref() == Some(&p.1);
        // a tensor parameter the program's calls typed takes the shapes a
        // test's calls hand it too: what the sizes disagree on is unknown
        // (one call in the program had fixed a function's sizes, and a
        // test's call of another size was refused by the shape the program's
        // call gave the return)
        if !ours {
            if let (CrowType::NumArray(pe, pd), CrowType::NumArray(te, td)) = (&p.1, t) {
                if pe == te && pd != td {
                    let joined = CrowType::NumArray(pe.clone(), join_dims(pd, td));
                    if joined != p.1 {
                        p.1 = joined;
                        return true;
                    }
                }
            }
            return false;
        }
        if p.1 == *t {
            return false;
        }
        p.1 = t.clone();
        self.by_tests.insert((f.to_string(), i), t.clone());
        true
    }

    /// What only a test's calls type (F5): a parameter nothing in the
    /// program types takes the type every test call of its function agrees
    /// on — a function only tests call is called by nothing in the
    /// program, and its body had been typed with its parameters unknown. A
    /// parameter the calls disagree on stays unknown, so a test's call never
    /// makes the program's body refuse what it did not. The calls are the
    /// program's tests' and its parts' tests' (a part's function is typed
    /// alike whichever file is the entry); a type naming a struct the
    /// program does not declare types nothing. Whether any moved.
    pub(crate) fn type_tested(&mut self, calls: &[(String, Vec<CrowType>)]) -> bool {
        let mut agreed: BTreeMap<(String, usize), Option<CrowType>> = BTreeMap::new();
        for (f, types) in calls {
            // a function's parameters, or a record's fields: a test that
            // builds a record the program only receives (the layout engine's
            // node, which Rust builds) types its fields as a call does
            let takes = match (self.functions.get(f), self.structs.get(f)) {
                (Some(sig), _) => sig.params.len(),
                (None, Some(st)) => st.fields.len(),
                (None, None) => continue,
            };
            if takes != types.len() {
                continue;
            }
            // a test's call at a signature of its own is a call like any
            // other: the function gets a copy for it (F2), where the test
            // had been lowered against the program's signature and handed
            // a tensor to a parameter the program made a number
            if self.functions.contains_key(f) {
                let sig: Vec<CrowType> = types.iter().map(reaching).collect();
                if sig.iter().all(Self::type_is_concrete) {
                    let entry = self.call_signatures.entry(f.clone()).or_default();
                    if !entry.iter().any(|existing| Self::same_signature(existing, &sig)) {
                        entry.push(sig);
                    }
                }
            }
            for (i, t) in types.iter().enumerate() {
                let t = reaching(t);
                let entry = agreed.entry((f.clone(), i)).or_insert_with(|| Some(t.clone()));
                let keep = match entry.as_ref() {
                    Some(a) => !matches!(t, CrowType::Unknown) && self.declares(&t) && a.compatible(&t),
                    None => false,
                };
                *entry = if keep { entry.as_ref().map(|a| a.unify(&t)) } else { None };
            }
        }
        let mut moved = false;
        for ((f, i), t) in agreed {
            let Some(t) = t.filter(|t| !matches!(t, CrowType::Unknown)) else { continue };
            moved |= self.type_by_tests(&f, i, &t, false);
        }
        moved
    }

    /// Whether every struct a type names is one the program declares.
    fn declares(&self, t: &CrowType) -> bool {
        match t {
            CrowType::Struct(n) | CrowType::OptBoxStruct(n) => self.structs.contains_key(n),
            CrowType::Array(e) | CrowType::Map(e) | CrowType::Result(e) | CrowType::NumArray(e, _) => self.declares(e),
            CrowType::Fn(params, ret) => params.iter().all(|p| self.declares(p)) && self.declares(ret),
            _ => true,
        }
    }

    /// Propagate types from indirect function calls back to original function declarations.
    /// When we see `op = double` and `op[5]`, propagate i64 back to double's param.
    pub(crate) fn propagate_indirect_fn_types(&mut self, stmts: &[Stmt]) {
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

    /// A type is concrete if it contains no Unknown anywhere (used to decide
    /// whether a call signature is usable for monomorphization).
    pub(crate) fn type_is_concrete(t: &CrowType) -> bool {
        // (a frame whose columns are not worked out yet is not either)
        let shown = format!("{:?}", t);
        !shown.contains("Unknown") && !shown.contains("Frame(None)")
    }

    /// One round of the F2 monomorphization pass: each function called with
    /// several signatures, not specialized yet, gets a clone per signature.
    /// The clones' names; none when nothing is left to specialize.
    pub(crate) fn new_clones(&mut self) -> Vec<String> {
        let mut new_clones: Vec<String> = Vec::new();
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
                new_clones.push(mangled);
            }
        }
        new_clones
    }

    /// Every clone the monomorphization pass made.
    pub(crate) fn clone_names(&self) -> Vec<String> {
        self.specializations.values().flat_map(|v| v.iter().map(|(_, m)| m.clone())).collect()
    }

    /// A clone's return type forgotten, so a stale unification does not
    /// stick when its body is typed again; its body.
    pub(crate) fn reset_clone(&mut self, mangled: &str) -> Option<Vec<Stmt>> {
        let sig = self.functions.get_mut(mangled)?;
        sig.return_type = CrowType::Unknown;
        Some(sig.body.clone())
    }

    /// A function's parameters bound in the environment, typed by its
    /// signature; the environment before, to put back.
    pub(crate) fn bind_params(&mut self, name: &str) -> BTreeMap<String, CrowType> {
        let saved_vars = self.vars.clone();
        if let Some(sig) = self.functions.get(name) {
            let bind: Vec<(String, CrowType)> = sig.params.clone();
            for (p, t) in bind {
                self.vars.insert(p, t);
            }
        }
        saved_vars
    }

    /// What a loop walks: a list that can fail is refused before the loop
    /// runs (spec, Errors in Conditions), so its items are the value's.
    pub(crate) fn walked(t: CrowType) -> CrowType {
        match t {
            CrowType::Result(inner) => *inner,
            other => other,
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
        // (the function being lowered: inside a standard module's, the
        // program's functions are invisible to the calls typed here)
        let outer_fn_name = self.current_fn_name.replace(name.to_string());
        BodyScope { saved_vars, outer_assigned, outer_in_view, outer_fn_name }
    }

    pub(crate) fn leave_body(&mut self, scope: BodyScope) {
        self.vars = scope.saved_vars;
        self.body_assigned = scope.outer_assigned;
        self.in_view = scope.outer_in_view;
        self.current_fn_name = scope.outer_fn_name;
    }

    /// Out of a function's body once a provisional lowering typed it: its
    /// return type the values it returns joined (each typed where it is
    /// returned, as the lowering met it), widened to a Result when the
    /// lowering handed an error out of it (`fails`), and a fact a view or
    /// handler gives a float widened for the program.
    pub(crate) fn finish_body(&mut self, name: &str, scope: BodyScope, returns: &[CrowType], fails: bool) {
        let BodyScope { saved_vars, outer_assigned, outer_in_view, outer_fn_name } = scope;
        self.body_assigned = outer_assigned;
        self.in_view = outer_in_view;
        self.current_fn_name = outer_fn_name;

        // a value whose type is not known is still a value: returns that are
        // all unknown make an unknown, not the no-value of a bare return
        // (a function built without its callers returned "nothing", and a
        // local holding its value was declared as the unit type)
        let ret = returns.iter().fold(None::<CrowType>, |r, t| {
            Some(match (r, t) {
                (None, t) => t.clone(),
                (Some(CrowType::Void), CrowType::Unknown) | (Some(CrowType::Unknown), CrowType::Void) => CrowType::Unknown,
                (Some(r), t) => r.unify(t),
            })
        });
        let ret = ret.unwrap_or(CrowType::Void);
        // A decision on a value that can fail hands its error out of the
        // function (spec §11: Errors in Conditions), as a failed value
        // assigned to a variable that holds no error does: the return type
        // widens to Result
        if let Some(sig) = self.functions.get_mut(name) {
            sig.return_type = sig.return_type.unify(&ret);
            if fails && !matches!(sig.return_type, CrowType::Result(_)) {
                let inner = std::mem::replace(&mut sig.return_type, CrowType::Unknown);
                sig.return_type = CrowType::Result(Box::new(inner));
            }
        }

        // A variable ever assigned a float is a float for its whole scope,
        // and a fact's scope is the program: a function assigning a fact a
        // float widens it from its declaration. In a ui program a function's
        // assignment of a fact's name is the fact (fresh_binding), a helper's
        // too, which the lowering stores: a helper no handler reaches (a
        // part's, tested alone) stored a float into a fact typed int. A
        // frame (or a tensor) a function stores into a fact joins the fact's
        // columns (its shape): the fact holds either (it had kept its
        // default's, and a table naming a column the handler's frame has was
        // refused).
        let mut widened: Vec<(String, CrowType)> = Vec::new();
        if self.is_ui || self.views.contains(name) || self.view_handlers.contains(name) {
            for f in &self.facts {
                let before = saved_vars.get(f).cloned().unwrap_or(CrowType::Unknown);
                match (&before, self.vars.get(f)) {
                    (CrowType::Int | CrowType::Unknown, Some(CrowType::Float)) => widened.push((f.clone(), CrowType::Float)),
                    (was, Some(now)) if was.detail().zip(now.detail()).is_some_and(|(a, b)| a.same_kind(&b) && a != b) => {
                        widened.push((f.clone(), was.unify(now)));
                    }
                    _ => {}
                }
            }
        }
        self.vars = saved_vars;
        for (f, t) in widened {
            self.vars.insert(f, t);
        }
    }

    /// Inside a function, the first assignment of a name that is neither a
    /// parameter nor a fact is a new binding: its type is its own, not the
    /// module variable's of the same name. Reactive state keeps its type
    /// through a handler's assignment; every other module name is shadowed.
    pub(crate) fn fresh_binding(&mut self, name: &str) -> bool {
        let is_ui = self.is_ui;
        let facts = &self.facts;
        match &mut self.body_assigned {
            Some(assigned) => assigned.insert(name.to_string()) && !(is_ui && facts.contains(name)),
            None => false,
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
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.compatible(y) && x.has_markup() == y.has_markup() && x.frame_cols() == y.frame_cols())
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
    /// What a loop's variable holds: an item of the list, a number of the
    /// tensor (a row of a matrix), an item of a parsed JSON list.
    /// A tensor's item — what a loop walks and an index reads: a row when
    /// its dimensions are two or more (a literal's nesting, `transpose`,
    /// `zeros[2, 3]`); else one of its numbers. (The dimensions had been
    /// passed over: a loop over `transpose[m]`, or `zeros[2, 3][1]`, read
    /// numbers where both modes' values held rows, and the compiled program
    /// walked the numbers.)
    pub(crate) fn tensor_item(inner: &CrowType, dims: &Option<Dims>) -> CrowType {
        match dims {
            Some(d) if d.len() >= 2 => CrowType::NumArray(Box::new(inner.clone()), Some(d[1..].to_vec())),
            _ => inner.clone(),
        }
    }

    pub(crate) fn loop_item(iter_type: CrowType) -> CrowType {
        match iter_type {
            CrowType::Array(inner) => *inner,
            CrowType::NumArray(inner, dims) => Self::tensor_item(&inner, &dims),
            // a parsed value's items are parsed values, read as one type
            // where the body computes with them
            CrowType::Json => CrowType::Json,
            _ => CrowType::Unknown,
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

/// The type a call's or a click's argument gives the parameter it reaches
/// (note_call): what a value that can fail holds, since an error among the
/// arguments is handed on before the call; a boxed record as the record.
fn reaching(t: &CrowType) -> CrowType {
    match t {
        CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
        CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Void) => CrowType::Unknown,
        CrowType::Result(inner) => *inner.clone(),
        other => other.clone(),
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

    /// Two programs in one process, each making its own in-memory tables of
    /// one name, are described from their own tables (the second had been
    /// described from the first's, and its cells went untyped: a clean
    /// clone's test suite refused parity 92)
    #[test]
    fn each_program_is_described_from_its_own_tables() {
        let url = "sqlite://:memory:";
        let a = describe_with_tables(url, "SELECT v FROM t_own_tables", &["CREATE TABLE t_own_tables (v INTEGER NOT NULL)".to_string()]).unwrap();
        let b = describe_with_tables(url, "SELECT v, w FROM t_own_tables", &["CREATE TABLE t_own_tables (v TEXT NOT NULL, w REAL NOT NULL)".to_string()]).unwrap();
        assert_eq!(a, vec![("v".to_string(), "int".to_string())]);
        assert_eq!(b, vec![("v".to_string(), "str".to_string()), ("w".to_string(), "float".to_string())]);
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
        let schema = ctx.vars.get("rows").and_then(|t| t.frame_cols().cloned().flatten()).map(|c| c.cols.clone()).unwrap_or_default();
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

    /// A handler two clicks hand an int and a float takes a float, in
    /// either order (the clicks' types are met before the parameter takes
    /// them, so a later reading of the tests does not undo an earlier)
    #[test]
    fn a_handler_clicked_with_an_int_and_a_float_takes_a_float() {
        for (first, second) in [("1", "2.5"), ("2.5", "1")] {
            let src = format!("use web, ui\nbase = 0.0\nfn begin[start]\n    base = start * 1.0\nview v[]\n    return p['`base`']\npage['/', v]\ntest t[]\n    a = app[v]\n    click[a, begin[{}]]\n    click[a, begin[{}]]\n", first, second);
            let stmts = crate::lower::lower_program(&crate::build(&src).unwrap()).unwrap();
            let mut ctx = TypeContext::new();
            ctx.infer_program(&stmts);
            assert_eq!(ctx.functions["begin"].params[0].1, CrowType::Float, "{} then {}", first, second);
        }
    }

    /// A click handed a value that can fail types its handler's parameter as
    /// what the value holds, as a call does: finch's play-head jump, clicked
    /// with `sig - 120.0` where `sig` came from `parse_time`, had a result
    /// parameter compiled, and the harness's number never reached it
    #[test]
    fn a_click_handed_a_value_that_can_fail_types_what_it_holds() {
        let src = "use web, ui\nhead = 0.0\nfn jump[t]\n    head = t\nview v[]\n    return p['`head`']\npage['/', v]\ntest t[]\n    a = app[v]\n    click[a, jump[5.0]]\n    sig = parse_time['2026-05-26 07:05:00']\n    click[a, jump[sig - 120.0]]\n";
        let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        assert_eq!(ctx.functions["jump"].params[0].1, CrowType::Float);
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
        assert!(matches!(both, Some(CrowType::Frame(_))), "{:?}", ctx.vars.get("both"));
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
        // 80 entries are 80 nested calls, which inference lowers level by
        // level: an unoptimised build's frames outgrow a test thread's stack
        let infers = || {
            let entries: Vec<String> = (0..80).map(|i| format!("'k{}' = {}.5", i, i)).collect();
            let src = format!("m = map[{}]\nlog[len[m]]\n", entries.join(", "));
            let stmts = crate::build(&src).unwrap();
            let stmts = crate::lower::lower_program(&stmts).unwrap();
            let mut ctx = TypeContext::new();
            ctx.infer_program(&stmts);
            assert_eq!(ctx.vars.get("m"), Some(&CrowType::Map(Box::new(CrowType::Float))));
        };
        std::thread::Builder::new().stack_size(64 << 20).spawn(infers).unwrap().join().unwrap();
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

