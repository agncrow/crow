//! The Rust backend: the IR printed as a Rust crate (docs/ir-design.md §6).
//!
//! The IR decided the meaning — every node's type, which calls are
//! effects, where a result propagates, which loops are parallel, which
//! functions are handlers and views. This backend prints it as the Rust
//! the runtime crates expect: `crow_runtime` for the builtins, frames,
//! tensors and display; `crow_web` for pages, sessions and the live loop;
//! `crow_db` for databases. It reads types from the IR (never infers
//! them) and keeps the conventions the generated programs have always
//! had, so a program compiled by it behaves as it did.

use crate::ir::*;
use crate::lower_ir::crow_type;
use crate::type_infer::CrowType;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod builtins;
mod exprs;
mod program;
mod stmts;
#[cfg(test)]
mod tests;
mod ui;

/// What the build needs from the program besides its text: the crates
/// and features the generated `Cargo.toml` lists.
#[derive(Default)]
pub struct Emitted {
    pub source: String,
    pub needs_rayon: bool,
    pub needs_serde_json: bool,
    /// Records a fact holds are kept as JSON by the session: serde derives.
    pub needs_serde: bool,
    pub needs_crow_runtime: bool,
    pub needs_regex: bool,
    pub needs_chrono: bool,
    pub needs_crow_web: bool,
    pub needs_crow_db: bool,
    pub web_features: BTreeSet<String>,
    pub db_features: BTreeSet<String>,
    /// crow_runtime's features the program's paths may need (`net`,
    /// `parquet`: crow_runtime::sources)
    pub runtime_features: BTreeSet<String>,
}

/// Emit a lowered program as `main.rs`; for `crow test --compiled`
/// (`test_control`) a program that also answers the test harness.
pub fn emit_with(
    program: &Program,
    registry: crate::modules::ModuleRegistry,
    source_file: &str,
    shown_names: BTreeMap<String, String>,
    cpu: Option<usize>,
    test_control: bool,
    schema: Option<crow_db::schema::Mode>,
) -> Result<Emitted, String> {
    let part = program.parts.first().cloned().ok_or("an empty program")?;
    let mut e = Emitter::new(part, registry, source_file, shown_names);
    e.test_control = test_control;
    e.schema_mode = schema;
    if let Some(n) = cpu {
        e.cpu_count = Some(n);
    }
    let source = e.program()?;
    Ok(Emitted {
        source,
        needs_rayon: e.needs_rayon,
        needs_serde_json: e.needs_serde_json,
        needs_serde: !e.serde_records.is_empty(),
        needs_crow_runtime: e.needs_crow_runtime,
        needs_regex: e.needs_regex,
        needs_chrono: e.needs_chrono,
        needs_crow_web: e.needs_crow_web,
        needs_crow_db: e.needs_crow_db,
        web_features: e.web_features.clone(),
        db_features: e.db_features.clone(),
        runtime_features: e.runtime_features.clone(),
    })
}

/// A local the backend declared: the Rust type it printed, and whether it
/// holds markup (a view's or an element's HTML) rather than text.
#[derive(Clone, Debug)]
pub(crate) struct Local {
    pub ty: CrowType,
}

pub(crate) struct Emitter {
    pub(crate) part: Part,
    pub(crate) registry: crate::modules::ModuleRegistry,
    pub(crate) source_file: String,
    pub(crate) shown_names: BTreeMap<String, String>,
    pub(crate) out: String,
    pub(crate) indent: usize,

    // what the crate needs
    pub(crate) needs_rayon: bool,
    pub(crate) needs_serde_json: bool,
    /// The records a fact holds, directly or in a field: the session keeps
    /// them as JSON, so they derive serde's traits.
    pub(crate) serde_records: BTreeSet<String>,
    pub(crate) needs_crow_runtime: bool,
    pub(crate) needs_distributed: bool,
    pub(crate) needs_later: bool,
    pub(crate) needs_regex: bool,
    pub(crate) needs_find_struct: bool,
    pub(crate) needs_time_result: bool,
    pub(crate) needs_shell_struct: bool,
    pub(crate) needs_elapsed_struct: bool,
    pub(crate) needs_chrono: bool,
    pub(crate) needs_grad: bool,
    pub(crate) needs_crow_db: bool,
    pub(crate) needs_crow_web: bool,
    pub(crate) needs_ui: bool,
    pub(crate) db_features: BTreeSet<String>,
    pub(crate) web_features: BTreeSet<String>,
    pub(crate) runtime_features: BTreeSet<String>,

    // the program, by name
    pub(crate) fns: BTreeMap<String, Fn>,
    pub(crate) structs: BTreeMap<String, StructDef>,
    pub(crate) derives: BTreeMap<String, Derive>,
    pub(crate) views: BTreeSet<String>,
    /// module names from `use` (a user module's variables are `mod_x`)
    pub(crate) modules: HashSet<String>,
    pub(crate) cpu_count: Option<usize>,
    /// The mode the crowfile says a built program opens its connections in
    /// (spec, db, The schema a program declares); without one, verify.
    pub(crate) schema_mode: Option<crow_db::schema::Mode>,
    pub(crate) by_ref_params: BTreeMap<String, HashSet<String>>,
    pub(crate) grad_functions: BTreeSet<String>,
    pub(crate) web_handlers: HashSet<String>,
    pub(crate) live_render_fns: HashSet<String>,
    pub(crate) live_handle_fns: HashSet<String>,
    pub(crate) reactive_render_fns: HashSet<String>,
    pub(crate) reactive_event_handlers: BTreeSet<String>,
    pub(crate) has_page_routes: bool,
    /// `crow test --compiled`: the program answers the test harness over
    /// HTTP (`crow_web::live::test_control`), a part without a page too.
    pub(crate) test_control: bool,
    pub(crate) has_noapp_web: bool,
    /// facts: name → (type, the Rust default)
    pub(crate) reactive_vars: BTreeMap<String, (CrowType, String)>,
    /// Each fact's declared value as main computes it, where it reads a fact
    /// declared before it: a render or a handler cannot compute it (its
    /// getter falls back on the type's default, which no session reaches —
    /// every session starts from the declared values).
    pub(crate) fact_inits: BTreeMap<String, String>,
    /// top-level connections in a page program: process-wide statics
    pub(crate) static_handle_vars: BTreeSet<String>,
    /// facts a `fetch[]`/`poll[]` fills
    pub(crate) data_source_vars: BTreeSet<String>,
    pub(crate) derive_fns: BTreeSet<String>,
    /// a user module's top-level values, printed as consts
    pub(crate) module_consts: BTreeSet<String>,

    // the function being printed
    pub(crate) cur_fn: String,
    pub(crate) cur_ret: Option<CrowType>,
    pub(crate) returns_unit: bool,
    pub(crate) ref_params: HashSet<String>,
    pub(crate) scopes: Vec<HashMap<String, Local>>,
    pub(crate) mutated: HashSet<String>,
    pub(crate) emitting_dual: bool,
    pub(crate) reactive_epilogue: Option<Vec<String>>,
    pub(crate) reactive_own_writes: Vec<String>,
    pub(crate) handler_derivations: BTreeSet<String>,
    pub(crate) view_mark: bool,
    pub(crate) closure_depth: usize,
    pub(crate) current_line: usize,
    pub(crate) binop_counter: usize,
    pub(crate) move_target: Option<String>,
    pub(crate) pinned_module: Option<String>,
    /// The first thing the program asks that Rust cannot say: the build
    /// stops with it, in the program's words
    pub(crate) error: Option<String>,
}

impl Emitter {
    fn new(part: Part, registry: crate::modules::ModuleRegistry, source_file: &str, shown_names: BTreeMap<String, String>) -> Self {
        let fns: BTreeMap<String, Fn> = part.fns.iter().map(|f| (f.name.clone(), f.clone())).collect();
        let structs: BTreeMap<String, StructDef> = part.structs.iter().map(|s| (s.name.clone(), s.clone())).collect();
        let derives: BTreeMap<String, Derive> = part.derives.iter().map(|d| (d.name.clone(), d.clone())).collect();
        let views: BTreeSet<String> = part.fns.iter().filter(|f| f.kind == FnKind::View).map(|f| f.name.clone()).collect();
        Emitter {
            part,
            registry,
            source_file: source_file.to_string(),
            shown_names,
            out: String::new(),
            indent: 0,
            needs_rayon: false,
            needs_serde_json: false,
            serde_records: BTreeSet::new(),
            needs_crow_runtime: false,
            needs_distributed: false,
            needs_later: false,
            needs_regex: false,
            needs_find_struct: false,
            needs_time_result: false,
            needs_shell_struct: false,
            needs_elapsed_struct: false,
            needs_chrono: false,
            needs_grad: false,
            needs_crow_db: false,
            needs_crow_web: false,
            needs_ui: false,
            db_features: BTreeSet::new(),
            web_features: BTreeSet::new(),
            runtime_features: BTreeSet::new(),
            fns,
            structs,
            derives,
            views,
            modules: HashSet::new(),
            cpu_count: None,
            schema_mode: None,
            by_ref_params: BTreeMap::new(),
            grad_functions: BTreeSet::new(),
            web_handlers: HashSet::new(),
            live_render_fns: HashSet::new(),
            live_handle_fns: HashSet::new(),
            reactive_render_fns: HashSet::new(),
            reactive_event_handlers: BTreeSet::new(),
            has_page_routes: false,
            test_control: false,
            has_noapp_web: false,
            reactive_vars: BTreeMap::new(),
            fact_inits: BTreeMap::new(),
            static_handle_vars: BTreeSet::new(),
            data_source_vars: BTreeSet::new(),
            derive_fns: BTreeSet::new(),
            module_consts: BTreeSet::new(),
            cur_fn: String::new(),
            cur_ret: None,
            returns_unit: false,
            ref_params: HashSet::new(),
            scopes: vec![HashMap::new()],
            mutated: HashSet::new(),
            emitting_dual: false,
            reactive_epilogue: None,
            reactive_own_writes: Vec::new(),
            handler_derivations: BTreeSet::new(),
            view_mark: false,
            closure_depth: 0,
            current_line: 0,
            binop_counter: 0,
            move_target: None,
            pinned_module: None,
            error: None,
        }
    }

    /// Stop the build with a reason (the first one wins).
    pub(crate) fn fail(&mut self, reason: String) {
        if self.error.is_none() {
            self.error = Some(reason);
        }
    }

    // ── output ──────────────────────────────────────────────────────

    pub(crate) fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    // ── names ───────────────────────────────────────────────────────

    /// A user function's Rust name: `main` is Rust's entry point.
    pub(crate) fn rust_fn_name(name: &str) -> String {
        if name == "main" {
            "crow_main".to_string()
        } else {
            name.to_string()
        }
    }

    /// A struct field as a Rust identifier: a field Rust reserves is
    /// written raw (`r#type`); the four Rust cannot write raw take a suffix.
    pub(crate) fn rust_field(f: &str) -> String {
        match f {
            "self" | "Self" | "super" | "crate" => format!("{}_field", f),
            _ if crate::rust_names::RUST_KEYWORDS.contains(&f) => format!("r#{}", f),
            _ => f.to_string(),
        }
    }

    /// The function being printed, named as the program wrote it.
    pub(crate) fn shown_fn_name(&self) -> String {
        let base = self.cur_fn.split("__").next().unwrap_or(&self.cur_fn).to_string();
        let name = if self.fns.get(&self.cur_fn).is_some() && self.cur_fn.contains("__") && self.cur_fn.rsplit_once("__").map_or(false, |(_, s)| s.chars().all(|c| c.is_ascii_digit())) {
            base
        } else {
            self.cur_fn.clone()
        };
        self.shown_names.get(&name).cloned().unwrap_or(name)
    }

    /// A trace entry: `file.fn:N` inside a function, `file:N` at the top.
    pub(crate) fn trace_format_expr(&self) -> String {
        let stem = if self.source_file.is_empty() {
            "unknown"
        } else {
            self.source_file.strip_suffix(".co").or_else(|| self.source_file.strip_suffix(".crow")).unwrap_or(&self.source_file)
        };
        if self.cur_fn.is_empty() {
            format!("format!(\"{}:{{}}\", {})", stem, self.current_line)
        } else {
            format!("format!(\"{}.{}:{{}}\", {})", stem, self.shown_fn_name(), self.current_line)
        }
    }

    // ── types ───────────────────────────────────────────────────────

    /// An expression's type as the type tables read it: markup is text. A
    /// local reads as the type it was declared with here (its first value's,
    /// as the generated programs have always declared it), everything else
    /// as the IR types it.
    pub(crate) fn tt(&self, e: &Expr) -> CrowType {
        self.tt_full(e).text()
    }

    /// An expression's type with markup told apart from text.
    pub(crate) fn tt_full(&self, e: &Expr) -> CrowType {
        match e {
            Expr::Local(n, _) | Expr::Move(n, _) | Expr::Load(n, _) | Expr::Derived(n, _) => match self.local_ty(n) {
                Some(t) if !matches!(t, CrowType::Unknown) => t,
                _ => crow_type(&e.ty()),
            },
            Expr::Convert { how: Convert::Unwrap | Convert::Known, expr, ty } => {
                let inner = self.tt_full(expr);
                match inner {
                    CrowType::Result(i) => *i,
                    CrowType::Unknown => crow_type(ty),
                    other => other,
                }
            }
            _ => crow_type(&e.ty()),
        }
    }

    /// The payload of a result type, or the type itself.
    pub(crate) fn inner(t: &CrowType) -> CrowType {
        match t {
            CrowType::Result(i) => i.as_ref().clone(),
            o => o.clone(),
        }
    }

    /// What a runtime call that can refuse ends with: its value, or the
    /// program stops on this line with the runtime's reason.
    pub(crate) fn or_stop(&self) -> String {
        format!(".unwrap_or_else(|e| crow_stop({}, e))", self.current_line)
    }

    /// A call argument as the program wrote it: the lowering marks a
    /// fallible argument of a propagating call with an unwrap; the backend
    /// decides the propagation itself and reads through the mark.
    pub(crate) fn raw(e: &Expr) -> &Expr {
        match e {
            Expr::Convert { how: Convert::Unwrap, expr, .. } => raw(expr),
            other => other,
        }
    }

    pub(crate) fn is_copy_type(t: &CrowType) -> bool {
        matches!(t, CrowType::Int | CrowType::Float | CrowType::Bool)
    }

    // ── scopes ──────────────────────────────────────────────────────

    pub(crate) fn declare(&mut self, name: &str, ty: CrowType) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), Local { ty });
        }
    }

    pub(crate) fn declared(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains_key(name))
    }

    pub(crate) fn local_ty(&self, name: &str) -> Option<CrowType> {
        self.scopes.iter().rev().find_map(|s| s.get(name).map(|l| l.ty.clone()))
    }

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    // ── the program's kinds of function ─────────────────────────────

    /// Inside a view.
    pub(crate) fn in_view(&self) -> bool {
        self.needs_ui && self.views.contains(&self.cur_fn)
    }

    /// Inside a handler or a render the live loop runs: the session's
    /// state is in scope as `state`.
    pub(crate) fn in_live(&self) -> bool {
        self.reactive_event_handlers.contains(&self.cur_fn) || self.reactive_render_fns.contains(&self.cur_fn)
    }
}

fn raw(e: &Expr) -> &Expr {
    Emitter::raw(e)
}

pub(crate) use crate::ir::in_place_builtin;
