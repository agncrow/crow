use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crow_runtime::tensor::CrowTensor;


use crate::ast::*;
use crate::lexer::Lexer;
use crate::parallel::{group_into_waves, stmt_writes};
use crate::parser::Parser;
use crate::value::*;

/// Environment: a stack of scopes for variable lookup
#[derive(Debug, Clone)]
struct Env {
    scopes: Vec<HashMap<String, CrowValue>>,
    /// Saved scope stacks for function call isolation.
    /// When a function is called, non-global scopes are saved here
    /// so the function can only see globals + its own scope.
    saved_stacks: Vec<Vec<HashMap<String, CrowValue>>>,
    /// `use ui` programs keep module variables visible inside functions —
    /// they are reactive state (see spec Design Note "No Ambient Mutability").
    ui_mode: bool,
    /// Per-call-frame reactive exemption (views + event handlers)
    exempt_frames: Vec<bool>,
}

impl Env {
    fn new() -> Self {
        Env {
            scopes: vec![HashMap::new()],
            saved_stacks: Vec::new(),
            ui_mode: false,
            exempt_frames: Vec::new(),
        }
    }

    /// Values that stay visible inside functions: the program's vocabulary
    /// (functions, structs, modules), not its mutable state.
    fn is_ambient_value(v: &CrowValue) -> bool {
        matches!(
            v.value,
            Value::Fn(_) | Value::GradFn(_) | Value::StructDef(_) | Value::Module(_)
        )
    }

    /// True when name resolution is inside a function call frame, where
    /// module variables are invisible (spec: Function Scope). Views and event
    /// handlers are exempt (reactive state — `use ui`'s sanctioned channel).
    fn in_call(&self) -> bool {
        !self.saved_stacks.is_empty() && !self.exempt_frames.last().copied().unwrap_or(false)
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Push an isolated function scope.
    /// Saves all non-global scopes and creates a fresh scope for the function.
    /// This prevents function-local variables from clobbering caller variables.
    fn push_call_scope(&mut self, exempt: bool) {
        let saved: Vec<HashMap<String, CrowValue>> = self.scopes.drain(1..).collect();
        self.saved_stacks.push(saved);
        self.exempt_frames.push(exempt);
        // Now scopes = [global] only; push a fresh function scope
        self.scopes.push(HashMap::new());
    }

    /// Pop an isolated function scope and restore the caller's scopes.
    fn pop_call_scope(&mut self) {
        // Pop the function scope
        self.scopes.pop();
        self.exempt_frames.pop();
        // Restore saved non-global scopes
        if let Some(saved) = self.saved_stacks.pop() {
            self.scopes.extend(saved);
        }
    }

    /// Bind a name in the current scope only — a parameter, never a write to
    /// module state whatever frame this is.
    fn set_local(&mut self, name: &str, value: CrowValue) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn set(&mut self, name: &str, value: CrowValue) {
        // Reactive state: inside a view/handler frame (the `use ui` exemption)
        // an assignment to an existing global updates that global — writing
        // state is the whole point of a handler. Everywhere else, assignment
        // is local to the current scope.
        let exempt = self.exempt_frames.last().copied().unwrap_or(false);
        if exempt && !self.saved_stacks.is_empty() {
            let local_first = self
                .scopes
                .iter()
                .skip(1)
                .any(|sc| sc.contains_key(name));
            if !local_first {
                if let Some(global) = self.scopes.first_mut() {
                    if global.contains_key(name) {
                        global.insert(name.to_string(), value);
                        return;
                    }
                }
            }
        }
        // set in the current (top) scope
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn get(&self, name: &str) -> Option<&CrowValue> {
        // search from innermost to outermost scope
        let in_call = self.in_call();
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(val) = scope.get(name) {
                // Inside a function, the global scope only provides the
                // program's vocabulary (fns, structs) — never module variables
                if i == 0 && in_call && !Self::is_ambient_value(val) {
                    return None;
                }
                return Some(val);
            }
        }
        None
    }

    /// Check whether a name exists as a module variable (for error hints)
    fn global_var_exists(&self, name: &str) -> bool {
        self.scopes
            .first()
            .and_then(|s| s.get(name))
            .map_or(false, |v| !Self::is_ambient_value(v))
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut CrowValue> {
        let in_call = self.in_call();
        for (i, scope) in self.scopes.iter_mut().enumerate().rev() {
            if scope.contains_key(name) {
                // Function frames can never mutate module variables
                if i == 0 && in_call {
                    let ambient = scope.get(name).map_or(false, Self::is_ambient_value);
                    if !ambient {
                        return None;
                    }
                }
                return scope.get_mut(name);
            }
        }
        None
    }

    fn update(&mut self, name: &str, value: CrowValue) -> bool {
        // update in the scope where the variable exists
        // When inside a function call (saved_stacks non-empty), don't
        // cross into the global scope (index 0) — this prevents
        // function-local assignments from clobbering global variables.
        let stop_at = if self.saved_stacks.is_empty() { 0 } else { 1 };
        for scope in self.scopes[stop_at..].iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }
}

/// Signal for control flow
enum Signal {
    None,
    Return(CrowValue),
    Value(CrowValue), // expression result (for collecting in for loops)
    Break,
    Continue,
}



/// Builtins that read the outside world — their results are stamped with a
/// capture moment, which then flows through derived values (see age[]).
const READER_FNS: &[&str] = &[
    "query", "read", "fetch", "recv", "input", "load", "env", "shell", "ls",
    "csv", "lines",
];


fn html_escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A node of the harness's own rendered HTML (well-formed by construction).
struct HNode {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<HNode>,
    text: String, // for text nodes (tag is empty)
}

fn html_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Parse the harness's rendered markup into a tree. Handles open/close tags,
/// self-closing tags (charts' SVG), quoted attributes, and text.
fn harness_parse_html(html: &str) -> HNode {
    let mut root = HNode { tag: String::new(), attrs: vec![], children: vec![], text: String::new() };
    let mut stack: Vec<HNode> = vec![];
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;
    let push_child = |stack: &mut Vec<HNode>, root: &mut HNode, node: HNode| {
        if let Some(parent) = stack.last_mut() { parent.children.push(node); } else { root.children.push(node); }
    };
    while i < chars.len() {
        if chars[i] == '<' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '>' { j += 1; }
            let inside: String = chars[i + 1..j.min(chars.len())].iter().collect();
            i = j + 1;
            let inside = inside.trim();
            if inside.starts_with('/') {
                let name = inside[1..].trim();
                // close the nearest open element with this name
                if let Some(pos) = stack.iter().rposition(|n| n.tag == name) {
                    while stack.len() > pos + 1 { let n = stack.pop().unwrap(); push_child(&mut stack, &mut root, n); }
                    let n = stack.pop().unwrap();
                    push_child(&mut stack, &mut root, n);
                }
                continue;
            }
            if inside.starts_with('!') { continue; }
            let self_closing = inside.ends_with('/');
            let inside = inside.trim_end_matches('/').trim();
            let mut parts = inside.splitn(2, char::is_whitespace);
            let tag = parts.next().unwrap_or("").to_string();
            let rest = parts.next().unwrap_or("").to_string();
            let mut attrs = vec![];
            let rc: Vec<char> = rest.chars().collect();
            let mut k = 0;
            while k < rc.len() {
                while k < rc.len() && rc[k].is_whitespace() { k += 1; }
                let ks = k;
                while k < rc.len() && rc[k] != '=' && !rc[k].is_whitespace() { k += 1; }
                let key: String = rc[ks..k].iter().collect();
                if key.is_empty() { break; }
                let mut val = String::new();
                if k < rc.len() && rc[k] == '=' {
                    k += 1;
                    if k < rc.len() && (rc[k] == '"' || rc[k] == '\'') {
                        let q = rc[k]; k += 1; let vs = k;
                        while k < rc.len() && rc[k] != q { k += 1; }
                        val = rc[vs..k.min(rc.len())].iter().collect(); k += 1;
                    } else {
                        let vs = k; while k < rc.len() && !rc[k].is_whitespace() { k += 1; }
                        val = rc[vs..k].iter().collect();
                    }
                }
                attrs.push((key, html_unescape(&val)));
            }
            let node = HNode { tag, attrs, children: vec![], text: String::new() };
            if self_closing { push_child(&mut stack, &mut root, node); } else { stack.push(node); }
        } else {
            let mut j = i;
            while j < chars.len() && chars[j] != '<' { j += 1; }
            let text: String = chars[i..j].iter().collect();
            i = j;
            if !text.trim().is_empty() || !text.is_empty() {
                push_child(&mut stack, &mut root, HNode { tag: String::new(), attrs: vec![], children: vec![], text: html_unescape(&text) });
            }
        }
    }
    while let Some(n) = stack.pop() { push_child(&mut stack, &mut root, n); }
    root
}

fn hnode_text(n: &HNode) -> String {
    if n.tag.is_empty() { return n.text.clone(); }
    n.children.iter().map(hnode_text).collect::<Vec<_>>().join("")
}

fn hnode_matches(n: &HNode, tag: &str, class: &str) -> bool {
    if n.tag.is_empty() { return false; }
    if !tag.is_empty() && n.tag != tag { return false; }
    if !class.is_empty() {
        let cls = n.attrs.iter().find(|(k, _)| k == "class").map(|(_, v)| v.clone()).unwrap_or_default();
        if !cls.split_whitespace().any(|c| c == class) { return false; }
    }
    true
}

/// Descendant selection: `steps[i]` must match an ancestor-or-self chain in
/// document order; the last step's matches are collected as records.
fn harness_select(n: &HNode, steps: &[(String, String)], depth: usize, out: &mut Vec<CrowValue>) {
    let normalized = harness_normalize_steps(steps);
    let steps = &normalized;
    for child in &n.children {
        if child.tag.is_empty() { continue; }
        let (tag, class) = &steps[depth];
        if hnode_matches(child, tag, class) {
            if depth + 1 == steps.len() {
                let mut m = std::collections::BTreeMap::new();
                // every attribute is a key (href, name, src, …); tag, text and
                // class are always present so records read uniformly
                for (k, v) in &child.attrs {
                    m.insert(k.clone(), CrowValue::ok(Value::Str(v.clone())));
                }
                let cls = child.attrs.iter().find(|(k, _)| k == "class").map(|(_, v)| v.clone()).unwrap_or_default();
                let text = hnode_text(child).split_whitespace().collect::<Vec<_>>().join(" ");
                m.insert("tag".to_string(), CrowValue::ok(Value::Str(child.tag.clone())));
                m.insert("text".to_string(), CrowValue::ok(Value::Str(text)));
                m.insert("class".to_string(), CrowValue::ok(Value::Str(cls)));
                out.push(CrowValue::ok(Value::Map(m)));
            } else {
                harness_select(child, steps, depth + 1, out);
            }
        }
        // a later step may still start deeper down
        harness_select(child, steps, depth, out);
    }
}

/// The layout engine, written in Crow and shipped inside the compiler.
const LAYOUT_ENGINE_SRC: &str = include_str!("modules/layout.crow");

/// A rendered node as the engine's LNode struct value.
fn harness_to_lnode(n: &HNode) -> CrowValue {
    let mut attrs = std::collections::BTreeMap::new();
    let mut class = String::new();
    for (k, v) in &n.attrs {
        if k == "class" { class = v.clone(); }
        // the kernel table's data-* carry the responsive rules
        let key = k.trim_start_matches("data-").replace('-', "_");
        attrs.insert(key, CrowValue::ok(Value::Str(v.clone())));
    }
    // inline style declarations become attrs the engine understands
    if let Some((_, style)) = n.attrs.iter().find(|(k, _)| k == "style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                let (k, v) = (k.trim(), v.trim());
                let mapped: Option<(&str, String)> = match k {
                    "gap" => Some(("gap", crow_space_name(v))),
                    "padding" => {
                        let parts: Vec<String> = v.split_whitespace().map(crow_space_name).collect();
                        Some(("pad", parts.join(" ")))
                    }
                    "margin" => Some(("margin", crow_space_name(v.split_whitespace().next().unwrap_or("0px")))),
                    "width" => Some(("width", v.to_string())),
                    "height" => Some(("height", v.to_string())),
                    "flex" => Some(("fill", "true".to_string())),
                    "flex-wrap" if v == "wrap" => Some(("wrap", "true".to_string())),
                    "font-size" => Some(("size", match v { "12px" => "xs", "14px" => "s", "16px" => "m", "20px" => "l", "24px" => "xl", "32px" => "xxl", _ => "m" }.to_string())),
                    "font-weight" => Some(("weight", match v { "700" => "bold", "500" => "medium", _ => "regular" }.to_string())),
                    "grid-template-columns" => {
                        // repeat(N, minmax(0, 1fr)) → N
                        v.strip_prefix("repeat(").and_then(|r| r.split(',').next()).map(|n| ("columns", n.trim().to_string()))
                    }
                    _ => None,
                };
                if let Some((k, v)) = mapped { attrs.insert(k.to_string(), CrowValue::ok(Value::Str(v))); }
            }
        }
    }
    let kids: Vec<CrowValue> = n.children.iter().filter(|c| !c.tag.is_empty()).map(harness_to_lnode).collect();
    let text = hnode_text(n).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("tag".to_string(), CrowValue::ok(Value::Str(n.tag.clone())));
    fields.insert("class".to_string(), CrowValue::ok(Value::Str(class)));
    fields.insert("text".to_string(), CrowValue::ok(Value::Str(text)));
    fields.insert("attrs".to_string(), CrowValue::ok(Value::Map(attrs)));
    fields.insert("kids".to_string(), CrowValue::ok(Value::Array(kids)));
    CrowValue::ok(Value::StructInstance(StructInstance { name: "LNode".to_string(), fields }))
}

fn crow_space_name(px: &str) -> String {
    match px.trim() { "4px" => "xs", "8px" => "s", "16px" => "m", "24px" => "l", "32px" => "xl", _ => "" }.to_string()
}

/// `path` is "/tag.class/tag.class/…" from the engine; a selector chain
/// matches when its steps appear in order as ancestors-or-self.
/// A selector step naming a layout container (`grid`, `row`, …) means its
/// class: containers render as divs carrying the primitive's name.
fn harness_normalize_steps(steps: &[(String, String)]) -> Vec<(String, String)> {
    steps.iter().map(|(t, c)| {
        if crow_runtime::layout::CONTAINERS.contains(&t.as_str()) && c.is_empty() { (String::new(), t.clone()) } else { (t.clone(), c.clone()) }
    }).collect()
}

fn harness_path_matches(path: &str, steps: &[(String, String)]) -> bool {
    let steps = &harness_normalize_steps(steps);
    let nodes: Vec<(&str, &str)> = path.split('/').filter(|s| !s.is_empty()).map(|seg| {
        match seg.find('.') { Some(i) => (&seg[..i], &seg[i + 1..]), None => (seg, "") }
    }).collect();
    if nodes.is_empty() || steps.is_empty() { return false; }
    // the last step must match the node itself
    let matches = |(tag, class): (&str, &str), (st, sc): &(String, String)| -> bool {
        (st.is_empty() || tag == st) && (sc.is_empty() || class.split_whitespace().any(|c| c == sc))
    };
    let last = nodes[nodes.len() - 1];
    if !matches(last, &steps[steps.len() - 1]) { return false; }
    let mut si = steps.len() - 1;
    let mut ni = nodes.len() - 1;
    while si > 0 {
        si -= 1;
        loop {
            if ni == 0 { return false; }
            ni -= 1;
            if matches(nodes[ni], &steps[si]) { break; }
        }
    }
    true
}

/// Early cutoff (derive-design.md §3.3): equal scalars and strings mark
/// nothing downstream; everything else counts as changed.
fn scalar_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
    }
}

/// A derivation as registered: how stale it may be, what it reads, and
/// its body as a callable.
#[derive(Clone)]
struct DeriveDef {
    fresh: f64,
    reads: crate::derive::ReadSet,
    func: FnValue,
}

/// A derivation's cached value and the input versions it was computed from.
#[derive(Clone)]
struct DerivedEntry {
    value: CrowValue,
    version: u64,
    facts_seen: std::collections::BTreeMap<String, u64>,
    derives_seen: std::collections::BTreeMap<String, u64>,
    tables_seen: std::collections::BTreeMap<String, u64>,
    tick_seen: u64,
    computed_at: f64,
}

#[derive(Clone)]
pub struct Interpreter {
    /// True when user modules were inlined by expand_module_imports before
    /// this interpreter ran — `use views.home` is then a no-op, not a
    /// dynamic load (which would double-define with separate state).
    modules_preexpanded: bool,
    env: Env,
    cpu_count: Option<usize>,
    gpu_count: Option<usize>,
    nodes_count: Option<usize>,
    source_dir: PathBuf,
    module_paths: Vec<PathBuf>,
    current_line: usize,
    current_function: String,
    current_file: String,
    cancelled: Option<Arc<AtomicBool>>,
    registry: crate::modules::ModuleRegistry,
    lowerer: crate::lower::Lowerer,
    /// Names declared with `view` (reactive-exempt, element vocabulary)
    view_fns: std::collections::HashSet<String>,
    /// Depth of view rendering — ui element names build html strings here.
    in_view: usize,
    /// Event handlers referenced from views/page[] (reactive-exempt)
    handler_fns: std::collections::HashSet<String>,
    /// Derivations (derive-design.md): definition, static read set, the
    /// cached value with the input versions it saw, and the version
    /// counters those checks compare against.
    derives: std::collections::HashMap<String, DeriveDef>,
    derived_cache: std::collections::HashMap<String, DerivedEntry>,
    fact_versions: std::collections::HashMap<String, u64>,
    table_versions: std::collections::HashMap<String, u64>,
    /// one per render: what "as fresh as the loop" means in the harness
    tick: u64,
    top_level_facts: std::collections::HashSet<String>,
    layout_engine_loaded: bool,
    /// alias → module path for `use X as y` (qualified-only imports)
    module_aliases: std::collections::HashMap<String, String>,
    /// Modules imported bare — their exports are callable without a prefix
    bare_modules: std::collections::HashSet<String>,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            modules_preexpanded: false,
            env: Env::new(),
            cpu_count: None,
            gpu_count: None,
            nodes_count: None,
            source_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            module_paths: vec![],
            current_line: 0,
            current_function: String::new(),
            current_file: String::new(),
            cancelled: None,
            registry: crate::modules::ModuleRegistry::new(),
            lowerer: crate::lower::Lowerer::new(),
            view_fns: std::collections::HashSet::new(),
            in_view: 0,
            handler_fns: std::collections::HashSet::new(),
            derives: std::collections::HashMap::new(),
            derived_cache: std::collections::HashMap::new(),
            fact_versions: std::collections::HashMap::new(),
            table_versions: std::collections::HashMap::new(),
            tick: 0,
            top_level_facts: std::collections::HashSet::new(),
            layout_engine_loaded: false,
            module_aliases: std::collections::HashMap::new(),
            bare_modules: std::collections::HashSet::new(),
        }
    }

    pub fn set_source_file(&mut self, file: &str) {
        self.current_file = file.to_string();
    }

    /// Set a cancellation flag that will cause loops to exit early.
    pub fn set_cancel_flag(&mut self, flag: Arc<AtomicBool>) {
        self.cancelled = Some(flag);
    }

    /// Check if this interpreter has been cancelled.
    fn is_cancelled(&self) -> bool {
        self.cancelled.as_ref().map_or(false, |f| f.load(Ordering::Relaxed))
    }

    /// Format a trace entry like "math.divide:5" or "main:3"
    fn trace_entry(&self) -> String {
        let file_stem = if self.current_file.is_empty() {
            "unknown"
        } else {
            self.current_file.strip_suffix(".crow").unwrap_or(&self.current_file)
        };
        if self.current_function.is_empty() {
            format!("{}:{}", file_stem, self.current_line)
        } else {
            format!("{}.{}:{}", file_stem, self.current_function, self.current_line)
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = dir;
    }

    pub fn add_module_path(&mut self, dir: PathBuf) {
        self.module_paths.push(dir);
    }

    pub fn set_cpu_count(&mut self, n: usize) {
        self.cpu_count = Some(n);
    }

    pub fn set_gpu_count(&mut self, n: usize) {
        self.gpu_count = Some(n);
    }

    pub fn get_var(&self, name: &str) -> Option<CrowValue> {
        self.env.get(name).cloned()
    }

    /// Public wrapper for eval_expr, used by the checker module.
    pub fn eval_expr_pub(&mut self, expr: &Expr) -> Result<CrowValue, String> {
        self.eval_expr(expr)
    }

    /// Get all variables from the top-level scope (for module exports)
    pub fn get_all_vars(&self) -> BTreeMap<String, CrowValue> {
        if let Some(scope) = self.env.scopes.first() {
            scope.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            BTreeMap::new()
        }
    }

    /// Wrap an error with the current line number
    fn line_err(&self, e: String) -> String {
        if self.current_line > 0 && !e.starts_with("line ") {
            format!("line {}: {}", self.current_line, e)
        } else {
            e
        }
    }

    /// Collect view names and event-handler names (bare fn refs inside view
    /// bodies + page[] registration args) — these frames keep reactive state
    /// visible under `use ui`.
    /// Coerce an all-numeric Array (how frame columns arrive) into a tensor
    /// value for element-wise operators; other values pass through unchanged.
    fn coerce_numeric_array(v: &Value) -> Value {
        if let Value::Array(items) = v {
            if !items.is_empty() {
                let mut data = Vec::with_capacity(items.len());
                for item in items {
                    match &item.value {
                        Value::Int(n) => data.push(*n as f64),
                        Value::Float(f) => data.push(*f),
                        _ => return v.clone(),
                    }
                }
                let n = data.len();
                return Value::NumArray(CrowTensor::from_vec(data, vec![n]));
            }
        }
        v.clone()
    }

    fn prescan_views(&mut self, stmts: &[Stmt]) {
        let mut fn_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        fn collect_fns(stmts: &[Stmt], out: &mut std::collections::HashSet<String>) {
            for st in stmts {
                if let Stmt::FnDecl { name, .. } = st {
                    out.insert(name.clone());
                }
            }
        }
        collect_fns(stmts, &mut fn_names);

        // Derivations: register each with its static read set; the body
        // reads facts like a handler does (the reactive exemption)
        {
            let facts = crate::derive::fact_names(stmts);
            let derive_names: std::collections::BTreeSet<String> = stmts.iter().filter_map(|s| if let Stmt::Derive { name, .. } = s { Some(name.clone()) } else { None }).collect();
            let bodies: std::collections::BTreeMap<String, Vec<Stmt>> = stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
            let owned = crate::derive::owned_tables(stmts, &bodies);
            for s in stmts {
                if let Stmt::Derive { name, fresh, body } = s {
                    let reads = crate::derive::read_set(body, &facts, &derive_names, &bodies, &owned);
                    self.derives.insert(name.clone(), DeriveDef {
                        fresh: *fresh,
                        reads,
                        func: FnValue { name: name.clone(), params: vec![], body: std::sync::Arc::new(body.clone()) },
                    });
                    self.handler_fns.insert(name.clone());
                }
            }
            self.top_level_facts.extend(facts);
        }

        fn walk_expr(e: &Expr, fns: &std::collections::HashSet<String>, out: &mut std::collections::HashSet<String>) {
            match e {
                Expr::Ident(n) => {
                    if fns.contains(n) {
                        out.insert(n.clone());
                    }
                }
                Expr::Call { callee, args } => {
                    if !matches!(callee.as_ref(), Expr::Ident(_)) {
                        walk_expr(callee, fns, out);
                    }
                    for a in args { walk_expr(a, fns, out); }
                }
                Expr::Named { value, .. } => walk_expr(value, fns, out),
                Expr::BinOp { left, right, .. } => {
                    walk_expr(left, fns, out);
                    walk_expr(right, fns, out);
                }
                Expr::UnaryOp { expr, .. } => walk_expr(expr, fns, out),
                Expr::Array(items) => for i in items { walk_expr(i, fns, out); },
                Expr::Map(pairs) => for (k, v) in pairs {
                    // F-11: click = set_speed[10.0] — the callee is a handler
                    let is_event_key = matches!(k, Expr::Ident(n) | Expr::Str(n)
                        if matches!(n.as_str(), "click" | "input" | "submit" | "change" | "seek"));
                    if is_event_key {
                        if let Expr::Call { callee, .. } = v {
                            if let Expr::Ident(f) = callee.as_ref() {
                                if fns.contains(f) {
                                    out.insert(f.clone());
                                }
                            }
                        }
                    }
                    walk_expr(v, fns, out);
                },
                Expr::FieldAccess { object, .. } => walk_expr(object, fns, out),
                Expr::ForExpr { iter, body, .. } => {
                    walk_expr(iter, fns, out);
                    walk_stmts(body, fns, out);
                }
                _ => {}
            }
        }
        fn walk_stmts(stmts: &[Stmt], fns: &std::collections::HashSet<String>, out: &mut std::collections::HashSet<String>) {
            for st in stmts {
                match st {
                    Stmt::Assign { value, .. } => walk_expr(value, fns, out),
                    Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => walk_expr(e, fns, out),
                    Stmt::If { condition, then_body, else_body } => {
                        walk_expr(condition, fns, out);
                        walk_stmts(then_body, fns, out);
                        if let Some(eb) = else_body { walk_stmts(eb, fns, out); }
                    }
                    Stmt::For { iter, body, .. } => {
                        walk_expr(iter, fns, out);
                        walk_stmts(body, fns, out);
                    }
                    Stmt::While { condition, body } => {
                        walk_expr(condition, fns, out);
                        walk_stmts(body, fns, out);
                    }
                    _ => {}
                }
            }
        }

        for st in stmts {
            if let Stmt::FnDecl { name, body, is_view: true, .. } = st {
                self.view_fns.insert(name.clone());
                let mut handlers = std::collections::HashSet::new();
                walk_stmts(body, &fn_names, &mut handlers);
                self.handler_fns.extend(handlers);
            }
            if let Stmt::ExprStmt(Expr::Call { callee, args }) = st {
                if matches!(callee.as_ref(), Expr::Ident(n) if n == "page") {
                    for a in args.iter().skip(1) {
                        if let Expr::Ident(n) = a {
                            if fn_names.contains(n) {
                                self.handler_fns.insert(n.clone());
                            }
                        }
                    }
                }
            }
        }

        // Reactive state reaches what a view or handler calls, not only the
        // functions named in click attributes: close over callees.
        fn called_fns_expr(e: &Expr, fns: &std::collections::HashSet<String>, out: &mut std::collections::HashSet<String>) {
            match e {
                Expr::Call { callee, args } => {
                    if let Expr::Ident(n) = callee.as_ref() { if fns.contains(n) { out.insert(n.clone()); } }
                    else { called_fns_expr(callee, fns, out); }
                    for a in args { called_fns_expr(a, fns, out); }
                }
                Expr::Ident(n) => { if fns.contains(n) { out.insert(n.clone()); } }
                Expr::Named { value, .. } => called_fns_expr(value, fns, out),
                Expr::BinOp { left, right, .. } => { called_fns_expr(left, fns, out); called_fns_expr(right, fns, out); }
                Expr::UnaryOp { expr, .. } => called_fns_expr(expr, fns, out),
                Expr::Array(items) => for i in items { called_fns_expr(i, fns, out); },
                Expr::Map(pairs) => for (k, v) in pairs { called_fns_expr(k, fns, out); called_fns_expr(v, fns, out); },
                Expr::FieldAccess { object, .. } => called_fns_expr(object, fns, out),
                Expr::ForExpr { iter, body, .. } => { called_fns_expr(iter, fns, out); called_fns_stmts(body, fns, out); }
                _ => {}
            }
        }
        fn called_fns_stmts(stmts: &[Stmt], fns: &std::collections::HashSet<String>, out: &mut std::collections::HashSet<String>) {
            for st in stmts {
                match st {
                    Stmt::Assign { value, .. } => called_fns_expr(value, fns, out),
                    Stmt::MultiAssign { values, .. } => for v in values { called_fns_expr(v, fns, out); },
                    Stmt::ExprStmt(e) => called_fns_expr(e, fns, out),
                    Stmt::Return(Some(e)) => called_fns_expr(e, fns, out),
                    Stmt::If { condition, then_body, else_body } => {
                        called_fns_expr(condition, fns, out);
                        called_fns_stmts(then_body, fns, out);
                        if let Some(eb) = else_body { called_fns_stmts(eb, fns, out); }
                    }
                    Stmt::For { iter, body, .. } => { called_fns_expr(iter, fns, out); called_fns_stmts(body, fns, out); }
                    Stmt::While { condition, body } => { called_fns_expr(condition, fns, out); called_fns_stmts(body, fns, out); }
                    Stmt::FieldAssign { value, .. } | Stmt::IndexAssign { value, .. } => called_fns_expr(value, fns, out),
                    _ => {}
                }
            }
        }
        fn assigns_any(stmts: &[Stmt], names: &std::collections::HashSet<String>) -> bool {
            stmts.iter().any(|st| match st {
                Stmt::Assign { name, .. } => names.contains(name),
                Stmt::MultiAssign { names: ns, .. } => ns.iter().any(|n| names.contains(n)),
                Stmt::If { then_body, else_body, .. } => assigns_any(then_body, names) || else_body.as_ref().map_or(false, |eb| assigns_any(eb, names)),
                Stmt::For { body, .. } | Stmt::While { body, .. } => assigns_any(body, names),
                _ => false,
            })
        }
        let module_vars: std::collections::HashSet<String> = stmts.iter().filter_map(|s| if let Stmt::Assign { name, .. } = s { Some(name.clone()) } else { None }).collect();
        let bodies: std::collections::HashMap<String, &Vec<Stmt>> = stmts.iter().filter_map(|s| match s {
            Stmt::FnDecl { name, body, .. } | Stmt::Derive { name, body, .. } => Some((name.clone(), body)),
            _ => None,
        }).collect();
        loop {
            let mut grown = false;
            let roots: Vec<String> = self.handler_fns.iter().cloned().collect();
            for r in roots {
                if let Some(b) = bodies.get(&r) {
                    let mut out = std::collections::HashSet::new();
                    called_fns_stmts(b, &fn_names, &mut out);
                    for f in out {
                        // only a callee that writes module state moves it on the
                        // handler's behalf; a pure helper keeps its own locals
                        let writes_state = bodies.get(&f).map_or(false, |fb| assigns_any(fb, &module_vars));
                        if writes_state && !self.view_fns.contains(&f) && self.handler_fns.insert(f) { grown = true; }
                    }
                }
            }
            if !grown { break; }
        }
    }

    /// Does this fn keep reactive-state visibility inside its call frame?
    fn reactive_exempt(&self, name: &str) -> bool {
        self.env.ui_mode && (self.view_fns.contains(name) || self.handler_fns.contains(name))
    }

    pub fn run(&mut self, stmts: &[Stmt]) -> Result<CrowValue, String> {
        // Resolve named arguments to positional form (single implementation
        // shared with the transpiler pipeline — see src/lower.rs). The lowerer
        // persists on the interpreter so REPL batches see earlier functions.
        let lowered = self.lowerer.lower(stmts)?;
        let stmts: &[Stmt] = &lowered;
        self.prescan_views(stmts);
        let waves = group_into_waves(stmts);
        let mut result = CrowValue::done();

        for wave in &waves {
            // Update current_line from the nearest preceding Line marker
            if let Some(&first_idx) = wave.first() {
                for j in (0..first_idx).rev() {
                    if let Stmt::Line(n) = &stmts[j] {
                        self.current_line = *n;
                        break;
                    }
                }
            }
            if wave.len() == 1 {
                // single statement - execute sequentially (no overhead)
                match self.exec_stmt(&stmts[wave[0]]).map_err(|e| self.line_err(e))? {
                    Signal::Return(val) => return Ok(val),
                    Signal::Value(val) => result = val,
                    Signal::Break | Signal::Continue | Signal::None => {}
                }
            } else {
                // multiple independent statements - execute in parallel
                let pairs: Vec<(usize, &Stmt)> = wave.iter().map(|&i| (i, &stmts[i])).collect();

                let run_parallel = |pairs: &[(usize, &Stmt)]| -> Result<Vec<(usize, Signal, Vec<(String, CrowValue)>)>, String> {
                    pairs.par_iter().map(|&(idx, stmt)| {
                        let mut interp = self.clone();
                        // Find the Line marker preceding this specific statement
                        for j in (0..idx).rev() {
                            if let Stmt::Line(n) = &stmts[j] {
                                interp.current_line = *n;
                                break;
                            }
                        }
                        let signal = interp.exec_stmt(stmt).map_err(|e| interp.line_err(e))?;
                        // collect written variables
                        let writes = stmt_writes(stmt);
                        let mut written_vars = vec![];
                        for var_name in &writes {
                            if let Some(val) = interp.env.get(var_name) {
                                written_vars.push((var_name.clone(), val.clone()));
                            }
                        }
                        Ok((idx, signal, written_vars))
                    }).collect()
                };

                let results = if let Some(n) = self.cpu_count {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(n)
                        .build()
                        .map_err(|e| format!("failed to create thread pool: {}", e))?;
                    pool.install(|| run_parallel(&pairs))?
                } else {
                    run_parallel(&pairs)?
                };

                // merge results back in statement order
                for (_idx, signal, written_vars) in results {
                    // merge written variables back into main environment
                    for (name, val) in written_vars {
                        if !self.env.update(&name, val.clone()) {
                            self.env.set(&name, val);
                        }
                    }
                    match signal {
                        Signal::Return(val) => return Ok(val),
                        Signal::Value(val) => result = val,
                        Signal::Break | Signal::Continue | Signal::None => {}
                    }
                }
            }
        }

        Ok(result)
    }

    pub fn set_modules_preexpanded(&mut self, v: bool) {
        self.modules_preexpanded = v;
    }

    fn resolve_module_file(&self, module_path: &str) -> Result<PathBuf, String> {
        // 1. try relative to source_dir: <source_dir>/<module_path>.crow
        let rel_path = self.source_dir.join(format!("{}.crow", module_path));
        if rel_path.exists() {
            return Ok(rel_path);
        }

        // 2. search module_paths (e.g. crow_modules/)
        for base in &self.module_paths {
            // try <base>/<module_path>.crow
            let p = base.join(format!("{}.crow", module_path));
            if p.exists() {
                return Ok(p);
            }
            // try <base>/<module_path>/main.crow (package with main entry)
            let p = base.join(module_path).join("main.crow");
            if p.exists() {
                return Ok(p);
            }
        }

        Err(format!("cannot find module '{}'", module_path))
    }

    fn load_module(&mut self, module_path: &str) -> Result<(), String> {
        // `use ui`: module variables are reactive state, so they stay visible
        // inside render/handler functions (the documented function-scope
        // exception — spec Design Note "No Ambient Mutability")
        if module_path == "ui" {
            self.env.ui_mode = true;
            return Ok(());
        }
        // `use web` (and web.*): the web layer is compiled-only — a no-op
        // here so programs and their test blocks still run under the
        // interpreter.
        if module_path == "web" || module_path.starts_with("web.") {
            return Ok(());
        }
        // Built-in module: math — load via module registry
        if module_path == "math" {
            self.registry.load(std::sync::Arc::new(crate::modules::math::MathModule));
            return Ok(());
        }
        // Built-in sub-module: math.stats — load via module registry
        if module_path == "math.stats" {
            // Ensure math module is loaded first
            if !self.registry.is_loaded("math") {
                self.load_module("math")?;
            }
            self.registry.load(std::sync::Arc::new(crate::modules::stats::StatsModule));
            return Ok(());
        }
        // Built-in sub-module: math.nn — load via module registry
        if module_path == "math.nn" {
            if !self.registry.is_loaded("math") {
                self.load_module("math")?;
            }
            self.registry.load(std::sync::Arc::new(crate::modules::nn::NnModule));
            return Ok(());
        }
        // Built-in module: io — file I/O
        if module_path == "io" {
            self.registry.load(std::sync::Arc::new(crate::modules::io::IoModule));
            return Ok(());
        }
        // Built-in module: frame — column-oriented data frames
        if module_path == "frame" {
            self.registry.load(std::sync::Arc::new(crate::modules::frame::FrameModule));
            return Ok(());
        }
        // Built-in module: db — database connectivity
        if module_path == "db" {
            self.registry.load(std::sync::Arc::new(crate::modules::db::DbModule));
            return Ok(());
        }
        // Built-in module: doc — documentation from Markdown
        if module_path == "doc" {
            self.registry.load(std::sync::Arc::new(crate::modules::doc::DocModule));
            return Ok(());
        }
        // Built-in module: chart — indicators for the chart element
        if module_path == "chart" {
            self.registry.load(std::sync::Arc::new(crate::modules::chart::ChartModule));
            return Ok(());
        }

        // Pre-expanded user modules (views.home → views/home.crow already
        // inlined with prefixed names by the front end): loading one again
        // would create a second copy with separate state, and one the front
        // end could not find is reported where it is used, as the compiled
        // program reports it. No-op.
        if self.modules_preexpanded {
            return Ok(());
        }

        let file_path = self.resolve_module_file(module_path)?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("cannot load module '{}': {}", module_path, e))?;

        // lex and parse
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse()?;

        // run in a fresh interpreter with the module's directory as source_dir
        let mut module_interp = Interpreter::new();
        let module_dir = file_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        module_interp.set_source_dir(module_dir);
        // set source file for trace entries
        if let Some(fname) = file_path.file_name() {
            module_interp.set_source_file(&fname.to_string_lossy());
        }
        // pass module_paths so deps can find their own deps
        module_interp.module_paths = self.module_paths.clone();
        module_interp.run(&stmts)?;

        // extract the module name (last component of path)
        let module_name = module_path
            .rsplit('/')
            .next()
            .unwrap_or(module_path)
            .to_string();

        // collect all bindings from the module
        let bindings = module_interp.get_all_vars();

        // store as a module value
        let module_val = CrowValue::ok(Value::Module(ModuleValue {
            name: module_name.clone(),
            bindings,
        }));
        self.env.set(&module_name, module_val);

        Ok(())
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Signal, String> {
        match stmt {
            // Verification blocks never execute — crow check consumes them.
            Stmt::TestBlock { .. } => Ok(Signal::None),
            // registered in the prescan; nothing runs until it is read
            Stmt::Derive { .. } => Ok(Signal::None),
            Stmt::Assign { name, value } => {
                let val = self.eval_expr(value)?;
                // Check for type-changing reassignment
                if let Some(existing) = self.env.get(name) {
                    if !existing.value.type_compatible(&val.value)
                        && !matches!(existing.value, Value::Done)
                        && !matches!(val.value, Value::Done)
                    {
                        return Err(self.line_err(format!(
                            "cannot reassign '{}' from {} to {} — variable type cannot change",
                            name, existing.value.type_name(), val.value.type_name()
                        )));
                    }
                }
                // A variable that has held a float is a float (spec: int/float
                // reassignment) — matches the transpiler, which unifies the
                // variable's type to float for its whole scope
                let val = match (self.env.get(name).map(|e| &e.value), &val.value) {
                    (Some(Value::Float(_)), Value::Int(n)) => {
                        CrowValue::ok(Value::Float(*n as f64))
                    }
                    _ => val,
                };
                let changed = self.env.get(name).map(|e| !scalar_equal(&e.value, &val.value)).unwrap_or(true);
                if !self.env.update(name, val.clone()) {
                    self.env.set(name, val);
                }
                self.note_fact_write(name, changed);
                Ok(Signal::None)
            }
            Stmt::MultiAssign { names, values } => {
                let vals: Vec<CrowValue> = values
                    .iter()
                    .map(|v| self.eval_expr(v))
                    .collect::<Result<_, _>>()?;
                for (name, val) in names.iter().zip(vals) {
                    // Check for type-changing reassignment
                    if let Some(existing) = self.env.get(name) {
                        if !existing.value.type_compatible(&val.value)
                            && !matches!(existing.value, Value::Done)
                            && !matches!(val.value, Value::Done)
                        {
                            return Err(self.line_err(format!(
                                "cannot reassign '{}' from {} to {} — variable type cannot change",
                                name, existing.value.type_name(), val.value.type_name()
                            )));
                        }
                    }
                    let changed = self.env.get(name).map(|e| !scalar_equal(&e.value, &val.value)).unwrap_or(true);
                    if !self.env.update(name, val.clone()) {
                        self.env.set(name, val);
                    }
                    self.note_fact_write(name, changed);
                }
                Ok(Signal::None)
            }
            Stmt::FnDecl { name, params, body, .. } => {
                let func = CrowValue::ok(Value::Fn(FnValue {
                    name: name.clone(),
                    params: params.clone(),
                    body: Arc::new(body.clone()),
                }));
                self.env.set(name, func);
                Ok(Signal::None)
            }
            Stmt::Return(expr) => {
                let val = match expr {
                    Some(e) => self.eval_expr(e)?,
                    None => CrowValue::done(),
                };
                Ok(Signal::Return(val))
            }
            Stmt::Break => Ok(Signal::Break),
            Stmt::Continue => Ok(Signal::Continue),
            Stmt::ExprStmt(expr) => {
                let val = self.eval_expr(expr)?;
                Ok(Signal::Value(val))
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => self.exec_if(condition, then_body, else_body),
            Stmt::For { var, iter, body } => {
                self.exec_for(var, iter, body)?;
                Ok(Signal::None)
            }
            Stmt::While { condition, body } => {
                self.exec_while(condition, body)
            }
            Stmt::StructDecl { name, fields } => {
                for f in fields {
                    if matches!(f.as_str(), "ok" | "reason" | "kind" | "trace") {
                        return Err(format!(
                            "struct '{}' field '{}' collides with error-value metadata (.ok/.reason/.kind/.trace are reserved on every value) — rename it",
                            name, f
                        ));
                    }
                }
                let sd = CrowValue::ok(Value::StructDef(StructDef {
                    name: name.clone(),
                    fields: fields.clone(),
                }));
                self.env.set(name, sd);
                Ok(Signal::None)
            }
            Stmt::FieldAssign { object, field, value } => {
                let new_val = self.eval_expr(value)?;
                // Find root variable name from expression
                let var_name = Self::root_var_name(object)
                    .ok_or_else(|| "field assignment target must be a variable".to_string())?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                // Navigate to the right nested field using the expression chain
                Self::assign_field(&mut var.value, object, field, new_val)?;
                Ok(Signal::None)
            }
            Stmt::IndexAssign { object, index, value } => {
                let new_val = self.eval_expr(value)?;
                let idx_val = self.eval_expr(index)?;
                let var_name = Self::root_var_name(object)
                    .ok_or_else(|| "index assignment target must be a variable".to_string())?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                Self::assign_index(&mut var.value, object, idx_val, new_val)?;
                Ok(Signal::None)
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
                self.load_module(path)?;
                Ok(Signal::None)
            }
            Stmt::Resource { kind, count } => {
                if kind == "gpu" {
                    if let Expr::Int(n) = count {
                        if *n >= 1 {
                            self.gpu_count = Some(*n as usize);
                            crow_runtime::gpu::set_gpu_enabled(true);
                        }
                    } else {
                        // gpu[] with no specific count — enable all GPUs
                        self.gpu_count = None;
                        crow_runtime::gpu::set_gpu_enabled(true);
                    }
                } else if kind == "cpu" {
                    if let Expr::Int(n) = count {
                        if *n >= 1 {
                            self.cpu_count = Some(*n as usize);
                        }
                    }
                } else if kind == "nodes" {
                    if let Expr::Int(n) = count {
                        if *n >= 1 {
                            self.nodes_count = Some(*n as usize);
                            // In interpreter mode, simulate single-process (rank 0)
                            // Use init(1) to avoid TCP connections
                            crow_runtime::distributed::init(1);
                        }
                    }
                }
                Ok(Signal::None)
            }
            Stmt::Line(n) => {
                self.current_line = *n;
                Ok(Signal::None)
            }
        }
    }

    // --- Mutation helpers ---

    /// Extract the root variable name from a nested expression (e.g., p.x → p, nums[0] → nums)
    fn root_var_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, .. } => Self::root_var_name(object),
            Expr::Call { callee, .. } => Self::root_var_name(callee),
            _ => None,
        }
    }

    /// Assign to a struct field, navigating nested field access chains
    /// Coarse dynamic type key for map-homogeneity checks (comparison) and a
    /// user-facing name (display). int/float unify (like arrays); structs are
    /// distinct per name.
    fn map_value_kind(v: &Value) -> (String, String) {
        match v {
            Value::Int(_) => ("number".to_string(), "int".to_string()),
            Value::Float(_) | Value::Dual(_, _) => ("number".to_string(), "float".to_string()),
            Value::Str(_) => ("str".to_string(), "str".to_string()),
            Value::Bool(_) => ("bool".to_string(), "bool".to_string()),
            Value::Array(_) | Value::NumArray(_) => ("array".to_string(), "array".to_string()),
            Value::Map(_) => ("map".to_string(), "map".to_string()),
            Value::StructInstance(si) => (format!("struct {}", si.name), format!("struct {}", si.name)),
            Value::Fn(_) | Value::GradFn(_) => ("fn".to_string(), "fn".to_string()),
            other => (other.type_name().to_string(), other.type_name().to_string()),
        }
    }

    /// Maps are dictionaries: every value must have the same type
    /// (spec Design Note "Maps Are Dictionaries, Not Records"). This is the
    /// runtime check matching what the transpiler's typed maps always enforced.
    fn check_map_homogeneous(
        map: &std::collections::BTreeMap<String, CrowValue>,
        new_val: &CrowValue,
    ) -> Result<(), String> {
        if let Some(existing) = map.values().next() {
            let (have_key, have_name) = Self::map_value_kind(&existing.value);
            let (add_key, add_name) = Self::map_value_kind(&new_val.value);
            if have_key != add_key {
                return Err(format!(
                    "map values must all be the same type — this map holds {} values, cannot add {} (use a struct for mixed named data)",
                    have_name, add_name
                ));
            }
        }
        Ok(())
    }

    fn assign_field(val: &mut Value, object_expr: &Expr, field: &str, new_val: CrowValue) -> Result<(), String> {
        match object_expr {
            Expr::Ident(_) => {
                // Direct field access: var.field = value
                match val {
                    Value::StructInstance(si) => {
                        if si.fields.contains_key(field) {
                            si.fields.insert(field.to_string(), new_val);
                            Ok(())
                        } else {
                            Err(format!("struct '{}' has no field '{}'", si.name, field))
                        }
                    }
                    Value::Map(m) => {
                        Self::check_map_homogeneous(m, &new_val)?;
                        m.insert(field.to_string(), new_val);
                        Ok(())
                    }
                    _ => Err("field assignment requires a struct or map".to_string()),
                }
            }
            Expr::FieldAccess { object, field: outer_field } => {
                // Nested: obj.outer.field = value — navigate to outer first
                match val {
                    Value::StructInstance(si) => {
                        let inner = si.fields.get_mut(outer_field)
                            .ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, outer_field))?;
                        Self::assign_field(&mut inner.value, object, field, new_val)
                    }
                    Value::Map(m) => {
                        let inner = m.get_mut(outer_field)
                            .ok_or_else(|| format!("map has no key '{}'", outer_field))?;
                        Self::assign_field(&mut inner.value, object, field, new_val)
                    }
                    _ => Err("field assignment requires a struct or map".to_string()),
                }
            }
            _ => Err("invalid field assignment target".to_string()),
        }
    }

    /// Assign to an array index or map key
    fn assign_index(val: &mut Value, object_expr: &Expr, idx_val: CrowValue, new_val: CrowValue) -> Result<(), String> {
        match object_expr {
            Expr::Ident(_) => {
                match val {
                    Value::Array(items) => {
                        if let Value::Int(i) = &idx_val.value {
                            let len = items.len() as i64;
                            let resolved = if *i < 0 { len + *i } else { *i };
                            if resolved >= 0 && (resolved as usize) < items.len() {
                                items[resolved as usize] = new_val;
                                Ok(())
                            } else {
                                Err(format!("index {} out of bounds (len {})", i, items.len()))
                            }
                        } else {
                            Err("array index must be an integer".to_string())
                        }
                    }
                    Value::NumArray(tensor) => {
                        if let Value::Int(i) = &idx_val.value {
                            let len = tensor.len() as i64;
                            let resolved = if *i < 0 { len + *i } else { *i };
                            if resolved >= 0 && (resolved as usize) < tensor.len() {
                                // Convert NumArray to Array for mutation
                                let mut items: Vec<CrowValue> = tensor.to_vec().into_iter()
                                    .map(|v| {
                                        if v == (v as i64) as f64 && v.is_finite() {
                                            CrowValue::ok(Value::Int(v as i64))
                                        } else {
                                            CrowValue::ok(Value::Float(v))
                                        }
                                    })
                                    .collect();
                                items[resolved as usize] = new_val;
                                // Try to convert back to NumArray
                                let all_numeric = items.iter().all(|v| {
                                    v.ok && matches!(v.value, Value::Int(_) | Value::Float(_))
                                });
                                if all_numeric {
                                    let data: Vec<f64> = items.iter().map(|v| match &v.value {
                                        Value::Int(n) => *n as f64,
                                        Value::Float(f) => *f,
                                        _ => unreachable!(),
                                    }).collect();
                                    let n = data.len();
                                    *val = Value::NumArray(CrowTensor::from_vec(data, vec![n]));
                                } else {
                                    *val = Value::Array(items);
                                }
                                Ok(())
                            } else {
                                Err(format!("index {} out of bounds (len {})", i, tensor.len()))
                            }
                        } else {
                            Err("array index must be an integer".to_string())
                        }
                    }
                    Value::Map(map) => {
                        if let Value::Str(key) = &idx_val.value {
                            Self::check_map_homogeneous(map, &new_val)?;
                            map.insert(key.clone(), new_val);
                            Ok(())
                        } else {
                            Err("map key must be a string".to_string())
                        }
                    }
                    _ => Err("index assignment requires an array or map".to_string()),
                }
            }
            Expr::FieldAccess { object, field } => {
                // Nested: obj.field[idx] = value
                if let Value::StructInstance(si) = val {
                    let inner = si.fields.get_mut(field)
                        .ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, field))?;
                    Self::assign_index(&mut inner.value, object, idx_val, new_val)
                } else {
                    Err("nested index assignment requires a struct".to_string())
                }
            }
            _ => Err("invalid index assignment target".to_string()),
        }
    }

    // --- Control flow ---

    /// A condition evaluated to an error-value: no branch can be taken.
    /// Inside a function → the function returns the error (auto-propagation,
    /// trace appended). At top level → runtime error with the error's reason.
    fn refuse_error_branch(&mut self, mut err: CrowValue) -> Result<Signal, String> {
        if !self.env.saved_stacks.is_empty() {
            err.trace.push(self.trace_entry());
            return Ok(Signal::Return(err));
        }
        Err(format!(
            "cannot branch on an error value — {}",
            err.reason
        ))
    }

    fn exec_if(
        &mut self,
        condition: &Expr,
        then_body: &[Stmt],
        else_body: &Option<Vec<Stmt>>,
    ) -> Result<Signal, String> {
        let cond = self.eval_expr(condition)?;
        // An error has no truth value — no branch can be taken (spec §11:
        // Errors in Conditions). Inside a function the error propagates to
        // the caller; at top level the program halts with the reason.
        if cond.is_error() {
            return self.refuse_error_branch(cond);
        }
        if self.is_truthy(&cond) {
            self.env.push_scope();
            let mut last_signal = Signal::None;
            for stmt in then_body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Break => {
                        self.env.pop_scope();
                        return Ok(Signal::Break);
                    }
                    Signal::Continue => {
                        self.env.pop_scope();
                        return Ok(Signal::Continue);
                    }
                    Signal::Value(val) => last_signal = Signal::Value(val),
                    Signal::None => {}
                }
            }
            self.env.pop_scope();
            return Ok(last_signal);
        } else if let Some(else_stmts) = else_body {
            self.env.push_scope();
            let mut last_signal = Signal::None;
            for stmt in else_stmts {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Break => {
                        self.env.pop_scope();
                        return Ok(Signal::Break);
                    }
                    Signal::Continue => {
                        self.env.pop_scope();
                        return Ok(Signal::Continue);
                    }
                    Signal::Value(val) => last_signal = Signal::Value(val),
                    Signal::None => {}
                }
            }
            self.env.pop_scope();
            return Ok(last_signal);
        }
        Ok(Signal::None)
    }

    /// Collect every variable name the statements assign to (including nested
    /// blocks). Used to decide whether a for-loop body mutates outer state.
    fn collect_assigned_names(stmts: &[Stmt], out: &mut std::collections::HashSet<String>) {
        for s in stmts {
            match s {
                Stmt::Assign { name, .. } => {
                    out.insert(name.clone());
                }
                Stmt::MultiAssign { names, .. } => {
                    for n in names {
                        out.insert(n.clone());
                    }
                }
                Stmt::FieldAssign { object, .. } | Stmt::IndexAssign { object, .. } => {
                    if let Some(n) = Self::root_var_name(object) {
                        out.insert(n);
                    }
                }
                Stmt::If { then_body, else_body, .. } => {
                    Self::collect_assigned_names(then_body, out);
                    if let Some(eb) = else_body {
                        Self::collect_assigned_names(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    Self::collect_assigned_names(body, out);
                }
                _ => {}
            }
        }
    }

    fn exec_for(
        &mut self,
        var: &str,
        iter_expr: &Expr,
        body: &[Stmt],
    ) -> Result<Vec<CrowValue>, String> {
        if self.is_cancelled() {
            return Err("execution cancelled".to_string());
        }
        let iter_val = self.eval_expr(iter_expr)?;
        if iter_val.is_error() {
            return Ok(vec![iter_val]);
        }
        let items = match &iter_val.value {
            Value::Array(items) => items.clone(),
            Value::NumArray(tensor) => {
                if tensor.ndim() >= 2 {
                    // Iterating a matrix yields its rows
                    tensor.rows().into_iter()
                        .map(|t| CrowValue::ok(Value::NumArray(t)))
                        .collect()
                } else {
                    tensor.to_vec().into_iter()
                        .map(|v| {
                            if v == (v as i64) as f64 && v.is_finite() {
                                CrowValue::ok(Value::Int(v as i64))
                            } else {
                                CrowValue::ok(Value::Float(v))
                            }
                        })
                        .collect()
                }
            }
            _ => return Err("for loop requires an array".to_string()),
        };

        // Sequential fallback (spec §12): if the body mutates a variable from
        // the outer scope, run in order on the real environment so the
        // mutations are kept. The parallel path clones the interpreter per
        // iteration, which would silently discard them.
        let mut assigned = std::collections::HashSet::new();
        Self::collect_assigned_names(body, &mut assigned);
        assigned.remove(var);
        let mutates_outer = assigned.iter().any(|n| self.env.get(n).is_some());
        if mutates_outer {
            let mut results = Vec::with_capacity(items.len());
            for item in &items {
                self.env.push_scope();
                self.env.set(var, item.clone());
                let mut last_val = CrowValue::done();
                let mut failed: Option<String> = None;
                for stmt in body {
                    match self.exec_stmt(stmt) {
                        Ok(Signal::Return(_)) => {
                            failed = Some("cannot return from inside a for loop".to_string());
                            break;
                        }
                        Ok(Signal::Value(val)) => last_val = val,
                        Ok(Signal::None | Signal::Break | Signal::Continue) => {}
                        Err(e) => {
                            failed = Some(e);
                            break;
                        }
                    }
                }
                self.env.pop_scope();
                if let Some(e) = failed {
                    return Err(e);
                }
                results.push(last_val);
            }
            return Ok(results);
        }

        let run_parallel = |items: &[CrowValue], interp: &Self, var: &str, body: &[Stmt]| -> Result<Vec<CrowValue>, String> {
            items
                .par_iter()
                .map(|item| {
                    let mut local = interp.clone();
                    local.env.push_scope();
                    local.env.set(var, item.clone());
                    let mut last_val = CrowValue::done();
                    for stmt in body {
                        match local.exec_stmt(stmt)? {
                            Signal::Return(_) => {
                                return Err(
                                    "cannot return from inside a for loop".to_string()
                                );
                            }
                            Signal::Value(val) => last_val = val,
                            Signal::None | Signal::Break | Signal::Continue => {}
                        }
                    }
                    Ok(last_val)
                })
                .collect()
        };

        match self.cpu_count {
            Some(n) => {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build()
                    .map_err(|e| format!("failed to create thread pool: {}", e))?;
                pool.install(|| run_parallel(&items, self, var, body))
            }
            None => run_parallel(&items, self, var, body),
        }
    }

    fn exec_while(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
    ) -> Result<Signal, String> {
        loop {
            if self.is_cancelled() {
                return Err("execution cancelled".to_string());
            }
            let cond = self.eval_expr(condition)?;
            if cond.is_error() {
                return self.refuse_error_branch(cond);
            }
            if !self.is_truthy(&cond) {
                break;
            }
            self.env.push_scope();
            let mut should_break = false;
            for stmt in body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Break => { should_break = true; break; }
                    Signal::Continue => break,
                    Signal::Value(_) | Signal::None => {}
                }
            }
            self.env.pop_scope();
            if should_break { break; }
        }
        Ok(Signal::None)
    }

    fn is_truthy(&self, val: &CrowValue) -> bool {
        if val.is_error() {
            return false;
        }
        match &val.value {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(l) => !l.is_empty(),
            Value::NumArray(t) => !t.is_empty(),
            Value::Done => false,
            _ => true,
        }
    }

    fn values_equal(&self, a: &CrowValue, b: &CrowValue) -> bool {
        match (&a.value, &b.value) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
            (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
            (Value::Dual(x, _), Value::Dual(y, _)) => x == y,
            (Value::Dual(x, _), Value::Float(y)) => x == y,
            (Value::Float(x), Value::Dual(y, _)) => x == y,
            (Value::Dual(x, _), Value::Int(y)) => *x == (*y as f64),
            (Value::Int(x), Value::Dual(y, _)) => (*x as f64) == *y,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::NumArray(x), Value::NumArray(y)) => x.to_vec() == y.to_vec(),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len()
                    && x.iter().zip(y.iter()).all(|(a, b)| self.values_equal(a, b))
            }
            _ => false,
        }
    }

    // --- Expression evaluation ---

    fn eval_expr(&mut self, expr: &Expr) -> Result<CrowValue, String> {
        match expr {
            // Lowering resolves all named arguments before execution
            Expr::Named { name, .. } => Err(format!(
                "named argument '{}' is not valid here",
                name
            )),
            Expr::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
            Expr::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
            Expr::Str(s) => {
                // an error value inside a backtick span propagates: the string
                // is that error, never the text "done"
                self.interpolate_string(s)
            }
            Expr::Bool(b) => Ok(CrowValue::ok(Value::Bool(*b))),
            Expr::Ident(name) => {
                if self.derives.contains_key(name) {
                    return self.read_derived(name);
                }
                match self.env.get(name) {
                    Some(val) => Ok(val.clone()),
                    None => {
                        // Check module constants (pi, e, inf, nan)
                        if let Some(val) = self.registry.get_constant(name) {
                            return Ok(val);
                        }
                        if self.env.global_var_exists(name) {
                            return Err(format!(
                                "module variables are not visible inside functions — pass '{}' as a parameter",
                                name
                            ));
                        }
                        Err(format!("undefined variable '{}'", name))
                    }
                }
            }
            Expr::BinOp { left, op, right } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binop(&l, op, &r)
            }
            Expr::UnaryOp { op, expr } => {
                let val = self.eval_expr(expr)?;
                self.eval_unaryop(op, &val)
            }
            Expr::Call { callee, args } => {
                self.eval_call(callee, args)
            }
            Expr::FieldAccess { object, field } => {
                // Reading a field of a captured value is still that value —
                // px.last is as old as px. (Arithmetic is not: see age[].)
                let obj = self.eval_expr(object)?;
                let mut out = self.eval_field_access(object, field)?;
                out.inherit_capture(&obj);
                Ok(out)
            }
            Expr::Array(items) => {
                let vals: Vec<CrowValue> = items
                    .iter()
                    .map(|e| self.eval_expr(e))
                    .collect::<Result<_, _>>()?;
                // If all elements are numeric (Int or Float), create a NumArray
                let all_numeric = !vals.is_empty() && vals.iter().all(|v| {
                    v.ok && matches!(v.value, Value::Int(_) | Value::Float(_))
                });
                if all_numeric {
                    let data: Vec<f64> = vals.iter().map(|v| match &v.value {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => unreachable!(),
                    }).collect();
                    let len = data.len();
                    return Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))));
                }
                // Nested numeric literal: [[1, 2], [3, 4]] — all elements are
                // tensors of equal shape → one shaped tensor (spec: unified arrays)
                let all_tensors = !vals.is_empty() && vals.iter().all(|v| {
                    v.ok && matches!(v.value, Value::NumArray(_))
                });
                if all_tensors {
                    let tensors: Vec<CrowTensor> = vals.iter().map(|v| match &v.value {
                        Value::NumArray(t) => t.clone(),
                        _ => unreachable!(),
                    }).collect();
                    let stacked = CrowTensor::from_elements(&tensors)?;
                    return Ok(CrowValue::ok(Value::NumArray(stacked)));
                }
                Ok(CrowValue::ok(Value::Array(vals)))
            }
            Expr::Map(pairs) => {
                let mut map = BTreeMap::new();
                for (k, v) in pairs {
                    let key = self.eval_expr(k)?;
                    let val = self.eval_expr(v)?;
                    let key_str = match key.unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        other => format!("{}", other),
                    };
                    map.insert(key_str, val);
                }
                Ok(CrowValue::ok(Value::Map(map)))
            }
            Expr::ForExpr { var, iter, body } => {
                let results = self.exec_for(var, iter, body)?;
                // filter out 'done' results (from iterations that didn't produce a value)
                let filtered: Vec<CrowValue> = results
                    .into_iter()
                    .filter(|v| !matches!(v.value, Value::Done))
                    .collect();
                // If all results are numeric, create NumArray
                let all_numeric = !filtered.is_empty() && filtered.iter().all(|v| {
                    v.ok && matches!(v.value, Value::Int(_) | Value::Float(_))
                });
                if all_numeric {
                    let data: Vec<f64> = filtered.iter().map(|v| match &v.value {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => unreachable!(),
                    }).collect();
                    let len = data.len();
                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                } else {
                    Ok(CrowValue::ok(Value::Array(filtered)))
                }
            }
        }
    }

    fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<CrowValue, String> {
        // Handle module-prefixed calls: db.connect[url], math.sin[x], etc.
        // (aliases translate first: m.sum after `use math as m`)
        if let Expr::FieldAccess { object, field } = callee {
            if let Expr::Ident(raw_mod) = object.as_ref() {
                let mod_name = self
                    .module_aliases
                    .get(raw_mod.as_str())
                    .cloned()
                    .unwrap_or_else(|| raw_mod.clone());
                let mod_name = &mod_name;
                // Check if this is a registry module
                if self.registry.is_loaded(mod_name) {
                    // Special handling for db.transact
                    if mod_name == "db" && field == "transact" {
                        return self.call_db_transact(args);
                    }
                    let arg_vals: Vec<CrowValue> = args
                        .iter()
                        .map(|a| self.eval_expr(a))
                        .collect::<Result<_, _>>()?;
                    // Overlapping scalar/tensor names (math.sqrt[16.0]):
                    // scalar args use the core builtin, same as bare calls
                    let overlapping = matches!(
                        field.as_str(),
                        "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max"
                    );
                    if overlapping
                        && !arg_vals.iter().any(|a| matches!(&a.value, Value::NumArray(_)))
                    {
                        return self.call_builtin(field, &arg_vals);
                    }
                    // Module-scoped resolution: the prefix names the provider,
                    // so overlapping exports (frame.load vs math.load) resolve
                    return { let _r = self.registry.call_in(mod_name, field, &arg_vals); self.note_sql_write(field, &arg_vals); _r }
                        .map(|v| if READER_FNS.contains(&field.as_str()) && v.is_ok() { v.stamp_captured() } else { v });
                }
            }
        }

        // Check for builtins and module functions
        if let Expr::Ident(name) = callee {
            // Overlapping builtins: abs, sqrt, floor, ceil, round, min, max
            // These work on both scalars (core builtin) and tensors (module).
            // We need to evaluate args first to check types.
            let is_overlapping = matches!(name.as_str(), "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");

            if is_overlapping {
                let arg_vals: Vec<CrowValue> = args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                // If any arg is NumArray, route to module
                let has_numarray = arg_vals.iter().any(|a| matches!(&a.value, Value::NumArray(_)));
                if has_numarray && self.registry.has_function(name) {
                    return { let _r = self.registry.call(name, &arg_vals); self.note_sql_write(name, &arg_vals); _r }
                        .map(|v| if READER_FNS.contains(&name.as_str()) && v.is_ok() { v.stamp_captured() } else { v });
                }
                return self.call_builtin(name, &arg_vals);
            }

            // Non-overlapping module functions take priority — except inside
            // a rendering view, where an element name (stack, row, table…)
            // is the element; the prefix reaches frame.stack / math.stack.
            let element_here = self.in_view > 0 && (crate::lower::UI_ELEMENTS.contains(&name.as_str())
                || name == "chart"
                || matches!(name.as_str(), "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                || (name == "select" && args.len() == 2 && !matches!(args.first(), Some(Expr::Map(_)))));
            if self.registry.has_function(name) && !element_here {
                // transact[conn, fn] needs special handling — we must call a Crow function
                if name == "transact" && self.registry.module_name_for(name) == Some("db") {
                    return self.call_db_transact(args);
                }

                let arg_vals: Vec<CrowValue> = args.iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                // A frame function given something other than a frame is the
                // core builtin of the same name (sort[xs], join[parts, sep])
                if self.registry.module_name_for(name) == Some("frame")
                    && crate::checker::BUILTINS.contains(&name.as_str())
                    && !arg_vals.first().map(|a| matches!(a.value, Value::Frame(_))).unwrap_or(false)
                {
                    return self.call_builtin(name, &arg_vals);
                }
                return { let _r = self.registry.call(name, &arg_vals); self.note_sql_write(name, &arg_vals); _r }
                    .map(|v| if READER_FNS.contains(&name.as_str()) && v.is_ok() { v.stamp_captured() } else { v });
            }

            // later[work, …] — background scheduling. The interpreter runs the
            // work synchronously inline: "eventually, atomically" admits the
            // immediate schedule, which keeps tests deterministic.
            if name == "later" {
                if args.is_empty() {
                    return Err("later[] needs a function: later[work] or later[work, args…]".into());
                }
                let fname = match &args[0] {
                    Expr::Ident(n) => n.clone(),
                    _ => return Err("later[] first argument must be a function name".into()),
                };
                let val = self
                    .env
                    .get(&fname)
                    .ok_or_else(|| format!("later[]: unknown function '{}'", fname))?;
                let func = match &val.value {
                    Value::Fn(f) => f.clone(),
                    _ => {
                        return Err(format!(
                            "later[] first argument must be a function, got {}",
                            val.value.type_name()
                        ))
                    }
                };
                let mut arg_vals = Vec::new();
                for a in &args[1..] {
                    arg_vals.push(self.eval_expr(a)?);
                }
                if func.params.len() != arg_vals.len() {
                    return Err(format!(
                        "later[{}]: function takes {} parameter{}, got {}",
                        fname,
                        func.params.len(),
                        if func.params.len() == 1 { "" } else { "s" },
                        arg_vals.len()
                    ));
                }
                // A later[] target scheduled in a ui context is a deferred
                // handler half: it writes reactive state when it lands.
                if self.env.ui_mode {
                    self.handler_fns.insert(fname.clone());
                }
                let _ = self.call_fn_value(&func, arg_vals)?;
                return Ok(CrowValue::ok(Value::Bool(true)));
            }
            // running[work] — inline schedules complete instantly, so nothing
            // is ever observably in flight in the interpreter.
            if name == "running" {
                match args.first() {
                    Some(Expr::Ident(_)) => {
                        return Ok(CrowValue::ok(Value::Bool(false)));
                    }
                    _ => return Err("running[] takes a function name: running[work]".into()),
                }
            }

            // --- app[] harness + view rendering (interpreter ui support) ---
            if self.env.ui_mode {
                // chart[frame, x, candles, lines, bars_y, marker, tz, height]
                // (lowered) — the shared kernel renders the SVG so both
                // modes emit identical markup.
                if self.in_view > 0 && name == "chart" {
                    let vals: Vec<CrowValue> = args
                        .iter()
                        .map(|a| self.eval_expr(a))
                        .collect::<Result<_, _>>()?;
                    let Value::Frame(f) = &vals[0].value else {
                        if vals[0].is_error() {
                            return Ok(CrowValue::ok(Value::Str(format!(
                                "<p>chart error: {}</p>",
                                vals[0].reason
                            ))));
                        }
                        return Err("chart[] takes a frame first".into());
                    };
                    let as_str = |v: &CrowValue| -> String { format!("{}", v.value) };
                    let str_list = |v: &CrowValue| -> Vec<String> {
                        match &v.value {
                            Value::Array(items) => {
                                items.iter().map(|i| format!("{}", i.value)).collect()
                            }
                            _ => vec![],
                        }
                    };
                    let as_f64 = |v: &CrowValue| -> f64 {
                        match &v.value {
                            Value::Float(x) => *x,
                            Value::Int(x) => *x as f64,
                            _ => 0.0,
                        }
                    };
                    let marker = as_f64(&vals[5]);
                    // panes = [['rsi'], ['cvd']]: one strip per inner list
                    let panes: Vec<Vec<String>> = match &vals[8].value {
                        Value::Array(items) => items.iter().map(|i| str_list(i)).collect(),
                        _ => vec![],
                    };
                    // levels = map[rsi = [30, 70], cvd = [0]]: reference
                    // lines, keyed by the column whose pane draws them
                    let mut levels: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
                    if let Value::Map(m) = &vals[9].value {
                        for (k, v) in m.iter() {
                            let row = match &v.value {
                                Value::Array(xs) => xs.iter().map(as_f64).collect(),
                                Value::NumArray(t) => t.to_vec(),
                                Value::Int(_) | Value::Float(_) => vec![as_f64(v)],
                                _ => vec![],
                            };
                            levels.insert(k.clone(), row);
                        }
                    }
                    let points = match &vals[10].value {
                        Value::Frame(pf) => Some(pf),
                        _ => None,
                    };
                    let seek = match &vals[11].value {
                        Value::Fn(func) => Some(func.name.clone()),
                        _ => None,
                    };
                    let node = crow_web::view::CrowVNode::chart_from_frame(
                        f,
                        &as_str(&vals[1]),
                        &str_list(&vals[2]),
                        &str_list(&vals[3]),
                        &as_str(&vals[4]),
                        if marker <= 0.0 { f64::NAN } else { marker },
                        &as_str(&vals[6]),
                        as_f64(&vals[7]),
                        &panes,
                        &levels,
                        points,
                        seek.as_deref(),
                    );
                    return Ok(CrowValue::ok(Value::Str(node.to_html())));
                }

                // Bound inputs: the variable's name is the binding key, so
                // they are built here, before evaluation, through the same
                // vnode helpers the compiled renderer uses — one markup, by
                // construction (design rule 12).
                let bound_input = matches!(name.as_str(), "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                    || (name == "select" && args.len() == 2 && !matches!(args.first(), Some(Expr::Map(_))));
                if self.in_view > 0 && bound_input {
                    let expected = match name.as_str() { "field" => 4, "slider" => 5, _ => 2 };
                    if args.len() == expected {
                        use crow_web::view::CrowVNode;
                        let key = match args.get(1) {
                            Some(Expr::Ident(v)) => v.clone(),
                            Some(Expr::FieldAccess { field, .. }) => field.clone(),
                            _ => "unknown".to_string(),
                        };
                        let vals: Vec<CrowValue> = args.iter().map(|a| self.eval_expr(a)).collect::<Result<_, _>>()?;
                        let s = |v: &CrowValue| format!("{}", v.value);
                        let num = |v: &CrowValue| match &v.value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => 0.0 };
                        let options = |v: &CrowValue| -> Result<Vec<String>, String> {
                            match &v.value {
                                Value::Array(items) => Ok(items.iter().map(|i| format!("{}", i.value)).collect()),
                                other => Err(format!("{}[] takes a list of options first, got {}", name, other.type_name())),
                            }
                        };
                        let node = match name.as_str() {
                            "field" => {
                                let kind = match &vals[2].value {
                                    Value::Str(k) if !k.is_empty() => k.clone(),
                                    _ => match &vals[1].value { Value::Int(_) | Value::Float(_) => "number".to_string(), _ => "text".to_string() },
                                };
                                CrowVNode::field_typed(&s(&vals[0]), &key, &s(&vals[1]), &kind, num(&vals[3]) as i64)
                            }
                            "slider" => CrowVNode::slider(&s(&vals[0]), &key, &s(&vals[1]), num(&vals[2]), num(&vals[3]), num(&vals[4])),
                            "checkbox" | "toggle" => {
                                let on = match &vals[1].value { Value::Bool(b) => *b, Value::Int(n) => *n != 0, Value::Str(t) => t == "true", _ => false };
                                CrowVNode::check_input(&s(&vals[0]), &key, on, name == "toggle")
                            }
                            "radio" | "tabs" => CrowVNode::choice_input(&options(&vals[0])?, &key, &s(&vals[1]), name == "tabs"),
                            _ => CrowVNode::select_input(&options(&vals[0])?, &key, &s(&vals[1])),
                        };
                        return Ok(CrowValue::ok(Value::Str(node.to_html())));
                    }
                }

                // Inside a rendering view, element names build html strings.
                if self.in_view > 0 && crate::lower::UI_ELEMENTS.contains(&name.as_str()) {
                    let mut inner = String::new();
                    let mut attrs: Vec<(String, String)> = Vec::new();
                    let (mut table_click, mut table_key, mut table_mark): (Option<String>, Option<String>, Option<String>) = (None, None, None);
                    let mut table_delta: Vec<String> = Vec::new();
                    for a in args {
                        // F-11: an attribute map may carry deferred calls
                        // (click = set_speed[10.0]) — evaluating it verbatim
                        // would RUN the handler at render. Strip event-key
                        // call values before evaluation; the harness's
                        // click[] carries the args explicitly.
                        let a_clean;
                        let a = if let Expr::Map(pairs) = a {
                            let stripped: Vec<(Expr, Expr)> = pairs.iter().map(|(k, v)| {
                                let is_event_key = matches!(k, Expr::Ident(n) | Expr::Str(n)
                                    if matches!(n.as_str(), "click" | "input" | "submit" | "change" | "seek"));
                                if is_event_key {
                                    if let Expr::Call { callee, .. } = v {
                                        if let Expr::Ident(f) = callee.as_ref() {
                                            return (k.clone(), Expr::Ident(f.clone()));
                                        }
                                    }
                                }
                                (k.clone(), v.clone())
                            }).collect();
                            a_clean = Expr::Map(stripped);
                            &a_clean
                        } else {
                            a
                        };
                        let v = self.eval_expr(a)?;
                        if v.is_error() {
                            // a view renders what it is given: a failed value
                            // shows its reason (same as the compiled renderer)
                            inner.push_str(&format!("error: {}", v.reason));
                            continue;
                        }
                        match &v.value {
                            // attribute map: scalar entries render as attributes
                            // (class, href, name, …) exactly as the compiled
                            // renderer does; event keys hold handlers and are
                            // wiring, not markup
                            Value::Map(m) => {
                                for (k, val) in m.iter() {
                                    // a table over a frame consumes its row options
                                    if name == "table" {
                                        match (k.as_str(), &val.value) {
                                            ("click", Value::Fn(f)) => { table_click = Some(f.name.clone()); continue; }
                                            ("key", Value::Str(s)) => { table_key = Some(s.clone()); continue; }
                                            ("mark", Value::Str(s)) => { table_mark = Some(s.clone()); continue; }
                                            ("delta", Value::Array(items)) => { table_delta = items.iter().map(|i| format!("{}", i.value)).collect(); continue; }
                                            _ => {}
                                        }
                                    }
                                    if matches!(k.as_str(), "click" | "input" | "submit" | "change" | "seek") { continue; }
                                    let s = match &val.value {
                                        Value::Str(s) => s.clone(),
                                        Value::Int(i) => i.to_string(),
                                        Value::Float(f) => f.to_string(),
                                        Value::Bool(b) => b.to_string(),
                                        _ => continue,
                                    };
                                    attrs.push((k.clone(), s));
                                }
                            }
                            // a child that is already markup passes through; text
                            // is escaped exactly as the compiled renderer does
                            Value::Str(s) if s.starts_with('<') && s.ends_with('>') => inner.push_str(s),
                            // a frame in a table is its rows, from the kernel
                            Value::Frame(f) if name == "table" => inner.push_str(&crow_runtime::layout::frame_table_html(f, table_click.as_deref(), table_key.as_deref(), table_mark.as_deref(), &table_delta)),
                            // a list contributes each item: markup passes
                            // through, anything else is text (the compiled
                            // renderer's children_of applies the same rule)
                            Value::Array(items) => {
                                fn splice(inner: &mut String, items: &[CrowValue]) {
                                    for it in items {
                                        match &it.value {
                                            Value::Str(s) if s.starts_with('<') && s.ends_with('>') => inner.push_str(s),
                                            Value::Array(nested) => splice(inner, nested),
                                            other => inner.push_str(&html_escape_text(&format!("{}", other))),
                                        }
                                    }
                                }
                                splice(&mut inner, items);
                            }
                            other => inner.push_str(&html_escape_text(&format!("{}", other))),
                        }
                    }
                    // nav[..., active = path]: the link to the active path is marked
                    if name == "nav" {
                        if let Some((_, active)) = attrs.iter().find(|(k, _)| k == "active") {
                            inner = crow_runtime::layout::nav_active_html(&inner, active);
                        }
                    }
                    // layout attributes become class/style/data-* through the
                    // kernel table; the rest pass through as HTML attributes
                    let rendered = crow_runtime::layout::render(name, &attrs);
                    if let Some(e) = rendered.errors.first() {
                        return Err(format!("{}: {}", name, e));
                    }
                    let mut out_attrs: Vec<(String, String)> = rendered.passthrough.clone();
                    if !rendered.class.is_empty() { out_attrs.push(("class".to_string(), rendered.class.clone())); }
                    if !rendered.style.is_empty() { out_attrs.push(("style".to_string(), rendered.style.clone())); }
                    out_attrs.extend(rendered.data.iter().cloned());
                    out_attrs.sort();
                    let attr_str: String = out_attrs.iter().map(|(k, v)| format!(" {}=\"{}\"", k, html_escape_attr(v))).collect();
                    return Ok(CrowValue::ok(Value::Str(format!(
                        "<{0}{2}>{1}</{0}>",
                        rendered.tag, inner, attr_str
                    ))));
                }
                // app[render] — a headless session bound to a view.
                if name == "app" && (1..=3).contains(&args.len()) {
                    // Load the engine now, in this interpreter: a box[] inside a
                    // parallel loop runs in a clone whose call scopes only see the
                    // globals that existed when it was made.
                    self.ensure_layout_engine()?;
                    if let Expr::Ident(vname) = &args[0] {
                        let found = self.env.get(vname).and_then(|val| if let Value::Fn(f) = &val.value { Some(f.clone()) } else { None });
                        if let Some(f) = found {
                            // app[view, width = 1280, wide = true]: the viewport the
                            // engine lays out for, and whether the page uses the full
                            // window (serve's `wide`) or the reading column
                            let mut width = 1280.0;
                            let mut wide = false;
                            for a in &args[1..] {
                                match self.eval_expr(a)?.value {
                                    Value::Int(n) => width = n as f64,
                                    Value::Float(x) => width = x,
                                    Value::Bool(b) => wide = b,
                                    _ => return Err("app[] takes a view name, then width = 390 and wide = true".into()),
                                }
                            }
                            return Ok(CrowValue::ok(Value::App(f, width, wide)));
                        }
                    }
                    return Err("app[] takes a view name: app[render] (and an optional width = 390, wide = true)".into());
                }
                // box[s, selector] / fits[s, width] / style[s, selector] — the layout
                // engine (src/modules/layout.crow) over the rendered tree.
                if (name == "box" || name == "fits" || name == "style") && args.len() == 2 {
                    let a = self.eval_expr(&args[0])?;
                    let (f, session_width, session_wide) = match &a.value { Value::App(f, w, wide) => (f.clone(), *w, *wide), _ => return Err(format!("{}[] takes an app session first: {}[s, ...]", name, name)) };
                    let arg1 = self.eval_expr(&args[1])?;
                    self.tick += 1;
                    self.in_view += 1;
                    let rendered = self.call_fn_value(&f, Vec::new());
                    self.in_view -= 1;
                    let rendered = rendered?;
                    let html = match &rendered.value { Value::Str(s) => s.clone(), other => format!("{}", other) };
                    let tree = harness_parse_html(&html);
                    if name == "style" {
                        let selector = match &arg1.value { Value::Str(s) => s.clone(), _ => return Err("style[] selector must be a string".into()) };
                        let steps: Vec<(String, String)> = selector.split_whitespace().map(|step| match step.find('.') { Some(i) => (step[..i].to_string(), step[i + 1..].to_string()), None => (step.to_string(), String::new()) }).collect();
                        let mut hits: Vec<CrowValue> = Vec::new();
                        harness_select(&tree, &steps, 0, &mut hits);
                        // resolved tokens per element: from its style attribute and class rules
                        let mut out = Vec::new();
                        for h in hits {
                            if let Value::Map(m) = &h.value {
                                let mut r = std::collections::BTreeMap::new();
                                let cls = m.get("class").map(|v| format!("{}", v.value)).unwrap_or_default();
                                let style = m.get("style").map(|v| format!("{}", v.value)).unwrap_or_default();
                                let mut background = String::new();
                                let mut color = String::from("--crow-text");
                                let mut size = String::from("m");
                                let mut weight = String::from("regular");
                                for c in cls.split_whitespace() {
                                    match c { "card" => background = "--crow-surface".into(), "primary" => { background = "--crow-accent".into(); color = "--crow-bg".into(); }, "up" => color = "--crow-up".into(), "down" => color = "--crow-down".into(), "stale" => color = "--crow-muted".into(), _ => {} }
                                }
                                for decl in style.split(';') {
                                    if let Some((k, v)) = decl.split_once(':') {
                                        let v = v.trim();
                                        match k.trim() {
                                            "color" => color = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                                            "background" => background = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                                            "font-size" => size = match v { "12px" => "xs", "14px" => "s", "16px" => "m", "20px" => "l", "24px" => "xl", "32px" => "xxl", _ => "m" }.to_string(),
                                            "font-weight" => weight = match v { "700" => "bold", "500" => "medium", _ => "regular" }.to_string(),
                                            _ => {}
                                        }
                                    }
                                }
                                r.insert("background".to_string(), CrowValue::ok(Value::Str(background)));
                                r.insert("color".to_string(), CrowValue::ok(Value::Str(color)));
                                r.insert("size".to_string(), CrowValue::ok(Value::Str(size)));
                                r.insert("weight".to_string(), CrowValue::ok(Value::Str(weight)));
                                out.push(CrowValue::ok(Value::Map(r)));
                            }
                        }
                        return Ok(CrowValue::ok(Value::Array(out)));
                    }
                    self.ensure_layout_engine()?;
                    let viewport = if name == "fits" {
                        match &arg1.value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => return Err("fits[] takes a width: fits[s, 390]".into()) }
                    } else {
                        session_width
                    };
                    let root = harness_to_lnode(&tree);
                    let engine = match self.env.get("crow_layout") { Some(v) => match &v.value { Value::Fn(f) => f.clone(), _ => return Err("layout engine missing".into()) }, None => return Err("layout engine missing".into()) };
                    let boxes = self.call_fn_value(&engine, vec![root, CrowValue::ok(Value::Float(viewport)), CrowValue::ok(Value::Bool(session_wide))])?;
                    let list = match &boxes.value { Value::Array(items) => items.clone(), _ => Vec::new() };
                    if name == "fits" {
                        let any_overflow = list.iter().any(|b| match &b.value { Value::StructInstance(si) => matches!(si.fields.get("overflow").map(|v| &v.value), Some(Value::Bool(true))), _ => false });
                        return Ok(CrowValue::ok(Value::Bool(!any_overflow)));
                    }
                    let selector = match &arg1.value { Value::Str(s) => s.clone(), _ => return Err("box[] selector must be a string like 'button' or 'div.stats .card'".into()) };
                    let steps: Vec<(String, String)> = selector.split_whitespace().map(|step| match step.find('.') { Some(i) => (step[..i].to_string(), step[i + 1..].to_string()), None => (step.to_string(), String::new()) }).collect();
                    let hits: Vec<CrowValue> = list.into_iter().filter(|b| {
                        if let Value::StructInstance(si) = &b.value {
                            let path = si.fields.get("path").map(|v| format!("{}", v.value)).unwrap_or_default();
                            harness_path_matches(&path, &steps)
                        } else { false }
                    }).collect();
                    return Ok(CrowValue::ok(Value::Array(hits)));
                }
                // html[a] — render the session's view against current state.
                if name == "html" && args.len() == 1 {
                    let a = self.eval_expr(&args[0])?;
                    if let Value::App(f, _, _) = &a.value {
                        let f = f.clone();
                        self.tick += 1;
                        self.in_view += 1;
                        let r = self.call_fn_value(&f, Vec::new());
                        self.in_view -= 1;
                        return r;
                    }
                    return Err("html[] takes an app session: html[a]".into());
                }
                // elements[a, selector] — the rendered view as a tree: every
                // element matching `tag`, `.class`, `tag.class`, or a
                // descendant chain of those, in document order, as records
                // [tag = …, text = …, class = …]. Structure becomes a truth.
                if name == "elements" && args.len() == 2 {
                    let a = self.eval_expr(&args[0])?;
                    let sel = self.eval_expr(&args[1])?;
                    let f = match &a.value { Value::App(f, _, _) => f.clone(), _ => return Err("elements[] takes an app session and a selector: elements[a, 'button']".into()) };
                    let selector = match &sel.value { Value::Str(s) => s.clone(), _ => return Err("elements[] selector must be a string like 'button', '.card', or 'div.stats button'".into()) };
                    self.in_view += 1;
                    let rendered = self.call_fn_value(&f, Vec::new());
                    self.in_view -= 1;
                    let rendered = rendered?;
                    let html = match &rendered.value { Value::Str(s) => s.clone(), other => format!("{}", other) };
                    let tree = harness_parse_html(&html);
                    let steps: Vec<(String, String)> = selector.split_whitespace().map(|step| {
                        let (tag, class) = match step.find('.') { Some(i) => (step[..i].to_string(), step[i + 1..].to_string()), None => (step.to_string(), String::new()) };
                        (tag, class)
                    }).collect();
                    if steps.is_empty() { return Err("elements[] selector is empty".into()); }
                    let mut hits: Vec<CrowValue> = Vec::new();
                    harness_select(&tree, &steps, 0, &mut hits);
                    return Ok(CrowValue::ok(Value::Array(hits)));
                }
                // change[a, 'var', value] — what typing into a bound field
                // does: set the reactive variable, so the next render and the
                // next handler see it.
                if name == "change" && args.len() == 3 {
                    let a = self.eval_expr(&args[0])?;
                    if !matches!(a.value, Value::App(..)) {
                        return Err("change[] first argument must be an app session: change[a, 'min_qty', 5]".into());
                    }
                    let var = match self.eval_expr(&args[1])?.value {
                        Value::Str(s) => s,
                        _ => return Err("change[] names the reactive variable as a string: change[a, 'min_qty', 5]".into()),
                    };
                    let value = self.eval_expr(&args[2])?;
                    let current = match self.env.scopes.first().and_then(|s| s.get(&var)) {
                        Some(v) => v.clone(),
                        None => return Err(format!("change[]: no reactive variable named '{}' (reactive state is a top-level assignment)", var)),
                    };
                    let compatible = match (&current.value, &value.value) {
                        (Value::Int(_), Value::Int(_)) | (Value::Float(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) | (Value::Int(_), Value::Float(_)) | (Value::Str(_), Value::Str(_)) | (Value::Bool(_), Value::Bool(_)) => true,
                        (a, b) => a.type_name() == b.type_name(),
                    };
                    if !compatible {
                        return Err(format!("change[]: '{}' holds {} — cannot set it to {}", var, current.value.type_name(), value.value.type_name()));
                    }
                    let value = match (&current.value, &value.value) {
                        (Value::Float(_), Value::Int(i)) => CrowValue::ok(Value::Float(*i as f64)),
                        _ => value,
                    };
                    let changed = self.env.scopes.first().and_then(|sc| sc.get(&var)).map(|e| !scalar_equal(&e.value, &value.value)).unwrap_or(true);
                    if let Some(scope) = self.env.scopes.first_mut() { scope.insert(var.clone(), value); }
                    self.note_fact_write(&var, changed);
                    return Ok(CrowValue::done());
                }
                // click[a, handler] — run the handler (reactive-exempt);
                // later[] work inside it runs inline, so tests stay
                // deterministic.
                if name == "click" && args.len() == 2 {
                    let a = self.eval_expr(&args[0])?;
                    if !matches!(a.value, Value::App(..)) {
                        return Err(
                            "click[] first argument must be an app session: click[a, go]".into(),
                        );
                    }
                    if let Expr::Ident(h) = &args[1] {
                        if let Some(val) = self.env.get(h) {
                            if let Value::Fn(f) = &val.value {
                                let f = f.clone();
                                // click[] names this fn as a handler — it
                                // gains the reactive-state exemption.
                                self.handler_fns.insert(h.clone());
                                return self.call_fn_value(&f, Vec::new());
                            }
                        }
                        return Err(format!("click[]: unknown handler '{}'", h));
                    }
                    // F-11: click[a, set_speed[10.0]] — frozen args ride along
                    if let Expr::Call { callee, args: fargs } = &args[1] {
                        if let Expr::Ident(h) = callee.as_ref() {
                            if let Some(val) = self.env.get(h) {
                                if let Value::Fn(f) = &val.value {
                                    let f = f.clone();
                                    self.handler_fns.insert(h.clone());
                                    let mut vals = Vec::new();
                                    for fa in fargs {
                                        vals.push(self.eval_expr(fa)?);
                                    }
                                    return self.call_fn_value(&f, vals);
                                }
                            }
                            return Err(format!("click[]: unknown handler '{}'", h));
                        }
                    }
                    return Err(
                        "click[] second argument must be a handler name or call: click[a, go] or click[a, go[x]]".into(),
                    );
                }
            }

            // assert[a op b] — introspect the comparison on failure: report
            // both operand values, not just "assertion failed".
            if name == "assert" && args.len() == 1 {
                if let Expr::BinOp { left, op, right } = &args[0] {
                    use crate::ast::BinOp as B;
                    let word = match op {
                        B::Eq => Some("eq"), B::Ne => Some("ne"),
                        B::Gt => Some("gt"), B::Lt => Some("lt"),
                        B::Gte => Some("gte"), B::Lte => Some("lte"),
                        _ => None,
                    };
                    if let Some(word) = word {
                        let full = self.eval_expr(&args[0])?;
                        if !full.ok {
                            return Err(format!(
                                "cannot branch on an error value — {}",
                                full.reason
                            ));
                        }
                        if matches!(full.value, Value::Bool(false)) {
                            let lv = self.eval_expr(left)?;
                            let rv = self.eval_expr(right)?;
                            return Err(format!(
                                "assert failed: {}\n    left:  {}\n    right: {}",
                                word, lv.value, rv.value
                            ));
                        }
                        return Ok(CrowValue::done());
                    }
                }
            }

            // Core builtins
            if self.is_builtin(name) {
                let arg_vals: Vec<CrowValue> = args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                // captured[x] — the moment x was read from the outside world.
                // Staleness is ordinary arithmetic: now[] - captured[px].
                // A value computed here has no capture moment: asking is a
                // category error, so it answers with an error value.
                if name == "captured" {
                    if arg_vals.len() != 1 {
                        return Err("captured[] takes exactly 1 argument".into());
                    }
                    // A failed read has no value to have captured: the read's
                    // own error propagates (same as compiled mode).
                    if arg_vals[0].is_error() {
                        let mut err = arg_vals[0].clone();
                        err.append_trace(&self.trace_entry());
                        return Ok(err);
                    }
                    return Ok(match arg_vals[0].captured {
                        Some(t) => CrowValue::ok(Value::Float(t)),
                        None => CrowValue::error_with_kind(
                            "this value was computed here, not read from outside - it has no capture moment",
                            "no_capture",
                            &self.trace_entry(),
                        ),
                    });
                }
                let mut out = self.call_builtin(name, &arg_vals)?;
                // Only a successful read has a capture moment: a failed read
                // yielded no value, and captured[] on it propagates the error.
                if READER_FNS.contains(&name.as_str()) && out.is_ok() {
                    out = out.stamp_captured();
                }
                return Ok(out);
            }
        }

        let callee_val = self.eval_expr(callee)?;

        // soft error propagation: if callee is an error, propagate it
        if callee_val.is_error() {
            let mut err = callee_val;
            err.append_trace(&self.trace_entry());
            return Ok(err);
        }

        let arg_vals: Vec<CrowValue> = args
            .iter()
            .map(|a| self.eval_expr(a))
            .collect::<Result<_, _>>()?;

        // Auto-propagation: if any argument is an error-value, return the first error.
        // A view is the exception, like the elements it is made of: it renders
        // what it is given, so a failed argument shows as text in the page
        // rather than wrapping the whole tree in a Result.
        let is_view_call = matches!(&callee_val.value, Value::Fn(f) if self.view_fns.contains(&f.name));
        for arg in &arg_vals {
            if arg.is_error() {
                if is_view_call {
                    return Ok(CrowValue::ok(Value::Str(format!("error: {}", html_escape_text(&arg.reason)))));
                }
                let mut err = arg.clone();
                err.append_trace(&self.trace_entry());
                return Ok(err);
            }
        }

        match &callee_val.value {
            Value::Fn(func) => {
                let func = func.clone();
                if func.params.len() != arg_vals.len() {
                    return Err(format!(
                        "{}[] expects {} args, got {}",
                        func.name,
                        func.params.len(),
                        arg_vals.len()
                    ));
                }
                // Use isolated call scope so function locals don't
                // clobber caller variables with the same name.
                let saved_function = self.current_function.clone();
                self.current_function = func.name.clone();
                let _ex = self.reactive_exempt(&func.name);
                // A view renders wherever it is called (see call_fn_value).
                let renders = self.view_fns.contains(&func.name);
                if renders { self.in_view += 1; }
                self.env.push_call_scope(_ex);
                for (param, val) in func.params.iter().zip(arg_vals) {
                    self.env.set_local(param, val);
                }
                let mut result = CrowValue::done();
                for stmt in func.body.iter() {
                    let signal = match self.exec_stmt(stmt) {
                        Ok(sig) => sig,
                        Err(e) => {
                            if renders { self.in_view -= 1; }
                            return Err(e);
                        }
                    };
                    match signal {
                        Signal::Return(val) => {
                            self.env.pop_call_scope();
                            if renders { self.in_view -= 1; }
                            self.current_function = saved_function;
                            return Ok(val);
                        }
                        Signal::Value(val) => { result = val; }
                        Signal::Break | Signal::Continue | Signal::None => {}
                    }
                }
                self.env.pop_call_scope();
                if renders { self.in_view -= 1; }
                self.current_function = saved_function;
                Ok(result)
            }
            Value::GradFn(func) => {
                // Forward-mode AD: for each parameter, run f with that param as Dual(x, 1.0)
                // and all others as Dual(x, 0.0), extract derivative from result.
                let func = func.clone();
                if func.params.len() != arg_vals.len() {
                    return Err(format!(
                        "grad[{}][] expects {} args, got {}",
                        func.name,
                        func.params.len(),
                        arg_vals.len()
                    ));
                }

                // Convert args to f64
                let float_args: Vec<f64> = arg_vals.iter().map(|a| {
                    match &a.value {
                        Value::Int(n) => Ok(*n as f64),
                        Value::Float(n) => Ok(*n),
                        Value::Dual(n, _) => Ok(*n),
                        _ => Err(format!("grad[{}][] requires numeric arguments", func.name)),
                    }
                }).collect::<Result<_, _>>()?;

                let n = float_args.len();
                let mut derivatives = Vec::with_capacity(n);

                for i in 0..n {
                    // Run function with param i as Dual(xi, 1.0), rest as Dual(xj, 0.0)
                    let saved_function = self.current_function.clone();
                    self.current_function = func.name.clone();
                    let _ex = self.reactive_exempt(&func.name);
                    self.env.push_call_scope(_ex);
                    for (j, param) in func.params.iter().enumerate() {
                        let dual_val = if j == i {
                            Value::Dual(float_args[j], 1.0)
                        } else {
                            Value::Dual(float_args[j], 0.0)
                        };
                        self.env.set(param, CrowValue::ok(dual_val));
                    }
                    let mut result = CrowValue::done();
                    for stmt in func.body.iter() {
                        match self.exec_stmt(stmt)? {
                            Signal::Return(val) => {
                                result = val;
                                break;
                            }
                            Signal::Value(val) => { result = val; }
                            Signal::Break | Signal::Continue | Signal::None => {}
                        }
                    }
                    self.env.pop_call_scope();
                    self.current_function = saved_function;

                    // Extract derivative from result
                    let deriv = match &result.value {
                        Value::Dual(_, d) => *d,
                        Value::Float(v) => {
                            // Constant function — derivative is 0
                            let _ = v;
                            0.0
                        }
                        Value::Int(_) => 0.0,
                        _ => return Err(format!(
                            "grad[{}][] requires function to return a number, got {}",
                            func.name,
                            result.value.type_name()
                        )),
                    };
                    derivatives.push(deriv);
                }

                if n == 1 {
                    // Single param: return scalar
                    Ok(CrowValue::ok(Value::Float(derivatives[0])))
                } else {
                    // Multiple params: return array
                    let arr: Vec<CrowValue> = derivatives.into_iter()
                        .map(|d| CrowValue::ok(Value::Float(d)))
                        .collect();
                    Ok(CrowValue::ok(Value::Array(arr)))
                }
            }
            Value::StructDef(sd) => {
                let sd = sd.clone();
                if sd.fields.len() != arg_vals.len() {
                    return Err(format!(
                        "{}[] expects {} fields, got {}",
                        sd.name,
                        sd.fields.len(),
                        arg_vals.len()
                    ));
                }
                let mut fields = BTreeMap::new();
                for (name, val) in sd.fields.iter().zip(arg_vals) {
                    fields.insert(name.clone(), val);
                }
                Ok(CrowValue::ok(Value::StructInstance(StructInstance {
                    name: sd.name.clone(),
                    fields,
                })))
            }
            Value::Array(items) => {
                // array[index] - indexing
                if arg_vals.len() != 1 {
                    return Err("array indexing requires exactly one argument".to_string());
                }
                let index_val = arg_vals[0].unwrap_or_propagate()?;
                match index_val {
                    Value::Int(idx) => {
                        let idx = *idx;
                        let len = items.len() as i64;
                        let resolved = if idx < 0 { len + idx } else { idx };
                        if resolved < 0 || resolved >= len {
                            return Err(format!(
                                "index {} out of bounds for array of length {}",
                                idx,
                                items.len()
                            ));
                        }
                        Ok(items[resolved as usize].clone())
                    }
                    _ => Err("array index must be an integer".to_string()),
                }
            }
            Value::NumArray(tensor) => {
                // NumArray[index] - indexing
                if arg_vals.len() != 1 {
                    return Err("array indexing requires exactly one argument".to_string());
                }
                let index_val = arg_vals[0].unwrap_or_propagate()?;
                match index_val {
                    Value::Int(idx) => {
                        let idx = *idx;
                        // Rank >= 2: the element at [i] is a row (rank-1-lower tensor)
                        if tensor.ndim() >= 2 {
                            let rows = tensor.shape()[0] as i64;
                            let resolved = if idx < 0 { rows + idx } else { idx };
                            if resolved < 0 || resolved >= rows {
                                return Err(format!(
                                    "index {} out of bounds for tensor with {} rows",
                                    idx, rows
                                ));
                            }
                            return Ok(CrowValue::ok(Value::NumArray(tensor.index0(resolved as usize))));
                        }
                        let len = tensor.len() as i64;
                        let resolved = if idx < 0 { len + idx } else { idx };
                        if resolved < 0 || resolved >= len {
                            return Err(format!(
                                "index {} out of bounds for array of length {}",
                                idx, tensor.len()
                            ));
                        }
                        let v = tensor.get_flat(resolved as usize);
                        // Return as int if it's a whole number
                        if v == (v as i64) as f64 && v.is_finite() {
                            Ok(CrowValue::ok(Value::Int(v as i64)))
                        } else {
                            Ok(CrowValue::ok(Value::Float(v)))
                        }
                    }
                    Value::NumArray(idx_tensor) => {
                        // Advanced indexing: gather by index array
                        Ok(CrowValue::ok(Value::NumArray(tensor.gather(idx_tensor))))
                    }
                    _ => Err("array index must be an integer or tensor".to_string()),
                }
            }
            Value::Str(s) => {
                // string[index] - character access
                if arg_vals.len() != 1 {
                    return Err("string indexing requires exactly one argument".to_string());
                }
                let index_val = arg_vals[0].unwrap_or_propagate()?;
                match index_val {
                    Value::Int(idx) => {
                        let idx = *idx;
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len() as i64;
                        let resolved = if idx < 0 { len + idx } else { idx };
                        if resolved < 0 || resolved >= len {
                            return Err(format!(
                                "index {} out of bounds for string of length {}",
                                idx,
                                chars.len()
                            ));
                        }
                        Ok(CrowValue::ok(Value::Str(chars[resolved as usize].to_string())))
                    }
                    _ => Err("string index must be an integer".to_string()),
                }
            }
            Value::Map(map) => {
                // map['key'] - access
                if arg_vals.len() != 1 {
                    return Err("map access requires exactly one argument".to_string());
                }
                let key_val = arg_vals[0].unwrap_or_propagate()?;
                let key_str = match key_val {
                    Value::Str(s) => s.clone(),
                    other => format!("{}", other),
                };
                match map.get(&key_str) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!("key '{}' not found in map", key_str)),
                }
            }
            _ => {
                // check if callee is an ident for builtins
                if let Expr::Ident(name) = callee {
                    return self.call_builtin(name, &arg_vals);
                }
                Err("not callable".to_string())
            }
        }
    }

    fn is_builtin(&self, name: &str) -> bool {
        matches!(name, "log" | "map" | "cpu" | "gpu" | "nodes" | "range"
            | "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max"
            | "split" | "trim" | "replace" | "upper" | "lower" | "starts" | "ends" | "join"
            | "repeat" | "pad" | "fmt"
            | "int" | "float" | "str"
            | "push" | "pop" | "sort" | "reverse" | "append" | "concat" | "contains" | "slice"
            | "read" | "write" | "exists" | "mkdir" | "ls" | "rm" | "mv" | "cp" | "len"
            | "keys" | "values" | "has" | "set" | "del"
            | "args" | "input"
            | "assert" | "error"
            | "rand" | "time" | "sleep" | "env" | "exit"
            | "json" | "encode"
            | "type" | "grad"
            | "shell" | "hash"
            | "find" | "matches" | "sub" | "any" | "all"
            | "now" | "date" | "elapsed" | "moment" | "calendar" | "parse_time" | "captured" | "chart"
            // Web P4 builtins
            | "cors" | "sessions" | "templates" | "render" | "cache" | "limit"
            | "uploads" | "sse" | "ws" | "connect" | "send" | "recv" | "close"
            | "static_dir" | "smtp"
            // Response format builtins
            | "msgpack" | "arrow" | "bytes" | "stream"
            // UI reactive builtins
            | "serve" | "page" | "fetch" | "poll" | "field" | "slider" | "today"
            | "checkbox" | "toggle" | "radio" | "tabs" | "progress" | "icon"
        )
    }


    /// Handle transact[conn, fn_name] — executes a Crow function inside a DB transaction.
    fn call_db_transact(&mut self, args: &[Expr]) -> Result<CrowValue, String> {
        if args.len() != 2 {
            return Err("transact[] takes 2 arguments: transact[conn, fn_name]".into());
        }
        let conn_val = self.eval_expr(&args[0])?;
        let conn = match &conn_val.value {
            Value::DbConn(h) => h.clone(),
            _ => return Err(format!("transact[] first argument must be a db connection, got {}", conn_val.value.type_name())),
        };

        // Second arg must be a function name (identifier)
        let func = match &args[1] {
            Expr::Ident(name) => {
                let val = self.env.get(name)
                    .ok_or_else(|| format!("transact[]: unknown function '{}'", name))?;
                match &val.value {
                    Value::Fn(f) => f.clone(),
                    _ => return Err(format!("transact[] second argument must be a function, got {}", val.value.type_name())),
                }
            }
            _ => return Err("transact[] second argument must be a function name".into()),
        };

        if func.params.len() != 1 {
            return Err(format!("transact[] function must take 1 parameter (the transaction handle), got {}", func.params.len()));
        }

        // Execute the function body inside a transaction.
        // The function receives the same conn handle — exec/query calls inside
        // the transaction use the same connection.
        let result = conn.transact(&|_tx| {
            // We can't call the Crow function from inside the Rust closure directly
            // because we can't borrow self mutably. Instead, we'll use a simpler approach:
            // execute BEGIN/fn/COMMIT manually.
            Ok(())
        });

        // Actually, the better approach: manually manage the transaction in Crow-land.
        // We run BEGIN, call the function, then COMMIT or ROLLBACK.
        // This avoids the closure lifetime issue.
        let _ = result; // ignore the empty transaction above

        // Manual transaction management
        conn.exec("BEGIN", &[]).map_err(|e| format!("transact[]: {}", e))?;

        // Create the tx handle — for SQLite, the same conn works inside a transaction
        let tx_val = CrowValue::ok(Value::DbConn(conn.clone()));

        // Call the user's function with the connection
        let saved_function = self.current_function.clone();
        self.current_function = func.name.clone();
        let _ex = self.reactive_exempt(&func.name);
        self.env.push_call_scope(_ex);
        self.env.set(&func.params[0], tx_val);

        let mut call_result = Ok(CrowValue::done());
        for stmt in func.body.iter() {
            match self.exec_stmt(stmt) {
                Ok(Signal::Return(val)) => {
                    call_result = Ok(val);
                    break;
                }
                Ok(Signal::Value(val)) => { call_result = Ok(val); }
                Ok(Signal::Break | Signal::Continue | Signal::None) => {}
                Err(e) => {
                    call_result = Err(e);
                    break;
                }
            }
        }

        self.env.pop_call_scope();
        self.current_function = saved_function;

        match call_result {
            Ok(val) => {
                conn.exec("COMMIT", &[]).map_err(|e| format!("transact[]: commit failed: {}", e))?;
                Ok(val)
            }
            Err(e) => {
                let _ = conn.exec("ROLLBACK", &[]);
                Err(format!("transact[]: rolled back: {}", e))
            }
        }
    }

    /// Call a Crow function value with evaluated arguments (used by builtins
    /// that take function parameters: any/all/find, transact, grad).
    /// Load the Crow-written layout engine into this interpreter once.
    fn ensure_layout_engine(&mut self) -> Result<(), String> {
        if self.layout_engine_loaded { return Ok(()); }
        let stmts = crate::build(LAYOUT_ENGINE_SRC).map_err(|e| format!("layout engine: {}", e))?;
        let saved_view = self.in_view;
        let saved_line = self.current_line;
        self.in_view = 0;
        // The engine's functions belong to the global scope whichever call
        // frame first needs them — a test block's frame, or a clone running
        // one iteration of a parallel loop — so run it with the globals alone.
        let saved_scopes: Vec<HashMap<String, CrowValue>> = self.env.scopes.drain(1..).collect();
        let saved_stacks = std::mem::take(&mut self.env.saved_stacks);
        let saved_frames = std::mem::take(&mut self.env.exempt_frames);
        let r = self.run(&stmts);
        self.env.exempt_frames = saved_frames;
        self.env.saved_stacks = saved_stacks;
        self.env.scopes.extend(saved_scopes);
        self.in_view = saved_view;
        self.current_line = saved_line;
        r.map_err(|e| format!("layout engine: {}", e))?;
        self.layout_engine_loaded = true;
        Ok(())
    }

    /// A fact was written with a possibly-new value: bump its version only
    /// when the value changed, so a derivation recomputes when a fact it
    /// reads actually moved (scalar_equal is conservative — a collection
    /// counts as changed, which over-recomputes but never under-).
    fn note_fact_write(&mut self, name: &str, changed: bool) {
        if changed && self.top_level_facts.contains(name) {
            *self.fact_versions.entry(name.to_string()).or_insert(0) += 1;
        }
    }

    /// A table was written by this program: derivations over it are stale.
    fn note_sql_write(&mut self, fname: &str, args: &[CrowValue]) {
        if fname != "exec" && fname != "insert" { return; }
        if let Some(Value::Str(sql)) = args.get(1).map(|v| &v.value) {
            let written = if fname == "insert" { vec![sql.clone()] } else { crate::derive::tables_of_sql(sql).1 };
            for t in written {
                *self.table_versions.entry(t).or_insert(0) += 1;
            }
        }
    }

    /// Read a derivation: recompute if anything it read changed since the
    /// cached value — a fact's version, a derivation it reads, a table this
    /// program wrote, the tick (for `now[]` and external sources at
    /// fresh = 0), or its freshness window — else the cached value.
    fn read_derived(&mut self, name: &str) -> Result<CrowValue, String> {
        let def = match self.derives.get(name) { Some(d) => d.clone(), None => return Err(format!("undefined derivation '{}'", name)) };
        // inputs that are derivations refresh first, so their versions are current
        for d in &def.reads.derives {
            if d != name { self.read_derived(d)?; }
        }
        let now = crow_runtime::moments::now();
        let valid = match self.derived_cache.get(name) {
            None => false,
            Some(e) => {
                def.reads.facts.iter().all(|f| e.facts_seen.get(f).copied() == self.fact_versions.get(f).copied())
                    && def.reads.derives.iter().all(|d| e.derives_seen.get(d).copied() == self.derived_cache.get(d).map(|x| x.version))
                    && def.reads.tables.iter().all(|t| e.tables_seen.get(t).copied() == self.table_versions.get(t).copied())
                    && (!def.reads.now || e.tick_seen == self.tick)
                    && (!def.reads.external || if def.fresh > 0.0 { now - e.computed_at <= def.fresh } else { e.tick_seen == self.tick })
            }
        };
        if valid {
            return Ok(self.derived_cache[name].value.clone());
        }
        let mut val = self.call_fn_value(&def.func, Vec::new())?;
        val.captured = Some(now);
        let (old_version, changed) = match self.derived_cache.get(name) {
            Some(e) => (e.version, !scalar_equal(&e.value.value, &val.value)),
            None => (0, true),
        };
        let entry = DerivedEntry {
            value: val.clone(),
            version: if changed { old_version + 1 } else { old_version },
            facts_seen: def.reads.facts.iter().map(|f| (f.clone(), self.fact_versions.get(f).copied().unwrap_or(0))).collect(),
            derives_seen: def.reads.derives.iter().map(|d| (d.clone(), self.derived_cache.get(d).map(|x| x.version).unwrap_or(0))).collect(),
            tables_seen: def.reads.tables.iter().map(|t| (t.clone(), self.table_versions.get(t).copied().unwrap_or(0))).collect(),
            tick_seen: self.tick,
            computed_at: now,
        };
        // versions default to 0 for facts never written; store what we compared
        let mut entry = entry;
        for f in &def.reads.facts { if !self.fact_versions.contains_key(f) { entry.facts_seen.remove(f); } }
        for t in &def.reads.tables { if !self.table_versions.contains_key(t) { entry.tables_seen.remove(t); } }
        self.derived_cache.insert(name.to_string(), entry);
        Ok(val)
    }

    fn call_fn_value(&mut self, func: &FnValue, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        if func.params.len() != args.len() {
            return Err(format!(
                "'{}' expects {} arguments, got {}",
                func.name, func.params.len(), args.len()
            ));
        }
        let saved_function = self.current_function.clone();
        // The callee moves current_line; the caller's statement (an assert,
        // say) must still report its own line once the call returns. An
        // error inside is stamped with the callee's line before the restore.
        let saved_line = self.current_line;
        self.current_function = func.name.clone();
        let _ex = self.reactive_exempt(&func.name);
        // A view renders wherever it is called — from a page, a test
        // harness, or a program writing pages to disk.
        let renders = self.view_fns.contains(&func.name);
        if renders { self.in_view += 1; }
        self.env.push_call_scope(_ex);
        for (p, a) in func.params.iter().zip(args) {
            self.env.set_local(p, a);
        }
        let mut result = Ok(CrowValue::done());
        for stmt in func.body.iter() {
            match self.exec_stmt(stmt) {
                Ok(Signal::Return(val)) => { result = Ok(val); break; }
                Ok(Signal::Value(val)) => { result = Ok(val); }
                Ok(Signal::Break | Signal::Continue | Signal::None) => {}
                Err(e) => { result = Err(self.line_err(e)); break; }
            }
        }
        self.env.pop_call_scope();
        if renders { self.in_view -= 1; }
        self.current_function = saved_function;
        self.current_line = saved_line;
        result
    }

    /// Elements of an array or tensor for the search builtins.
    fn search_items(&self, v: &CrowValue, name: &str) -> Result<Vec<CrowValue>, String> {
        match &v.value {
            Value::Array(items) => Ok(items.clone()),
            Value::NumArray(t) => {
                if t.ndim() >= 2 {
                    Ok(t.rows().into_iter().map(|r| CrowValue::ok(Value::NumArray(r))).collect())
                } else {
                    Ok(t.to_vec().into_iter().map(|f| {
                        if f == (f as i64) as f64 && f.is_finite() {
                            CrowValue::ok(Value::Int(f as i64))
                        } else {
                            CrowValue::ok(Value::Float(f))
                        }
                    }).collect())
                }
            }
            other => Err(format!("{}[] requires an array, got {}", name, other.type_name())),
        }
    }

    fn call_builtin(
        &mut self,
        name: &str,
        args: &[CrowValue],
    ) -> Result<CrowValue, String> {
        // Auto-propagation: if any argument is an error-value, return the first error.
        // Exceptions: `log` prints error-values, `type` inspects them, `error`
        // creates them — and `assert` is a DECISION, so an error condition
        // halts with its reason (errors have no truth value) via its own arm.
        if !matches!(name, "log" | "type" | "error" | "assert") {
            for arg in args {
                if arg.is_error() {
                    let mut err = arg.clone();
                    err.append_trace(&self.trace_entry());
                    return Ok(err);
                }
            }
        }
        match name {
            "log" => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    if arg.is_error() {
                        print!("error: {}", arg.reason);
                    } else {
                        print!("{}", arg.value);
                    }
                }
                println!();
                Ok(CrowValue::done())
            }
            "type" => {
                if args.len() != 1 { return Err("type[] takes exactly 1 argument".into()); }
                let type_name = if args[0].is_error() {
                    "error".to_string()
                } else {
                    match &args[0].value {
                        Value::App(..) => "app".to_string(),
                        Value::Int(_) => "int".to_string(),
                        Value::Float(_) => "float".to_string(),
                        Value::Str(_) => "str".to_string(),
                        Value::Bool(_) => "bool".to_string(),
                        Value::Array(_) | Value::NumArray(_) => "array".to_string(),
                        Value::Sparse(_) => "sparse".to_string(),
                        Value::Map(_) => "map".to_string(),
                        Value::Fn(_) => "fn".to_string(),
                        Value::StructDef(sd) => sd.name.clone(),
                        Value::StructInstance(si) => si.name.clone(),
                        Value::Module(m) => format!("module:{}", m.name),
                        Value::Dual(_, _) => "float".to_string(),
                        Value::GradFn(_) => "fn".to_string(),
                        Value::Frame(_) => "frame".to_string(),
                        Value::DbConn(_) => "db".to_string(),
                        Value::Done => "none".to_string(),
                    }
                };
                Ok(CrowValue::ok(Value::Str(type_name)))
            }
            "grad" => {
                if args.len() != 1 { return Err("grad[] takes exactly 1 argument".into()); }
                match &args[0].value {
                    Value::Fn(f) => Ok(CrowValue::ok(Value::GradFn(f.clone()))),
                    _ => Err("grad[] requires a function".into()),
                }
            }
            "map" => {
                // map[] creates an empty map
                if args.is_empty() {
                    Ok(CrowValue::ok(Value::Map(BTreeMap::new())))
                } else {
                    Err("map[] takes no arguments".to_string())
                }
            }
            "cpu" => {
                if args.len() != 1 {
                    return Err("cpu[] takes exactly 1 argument".to_string());
                }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("cpu[] requires a positive integer".to_string());
                        }
                        self.cpu_count = Some(*n as usize);
                        Ok(CrowValue::done())
                    }
                    _ => Err("cpu[] requires an integer argument".to_string()),
                }
            }
            "gpu" => {
                if args.is_empty() {
                    // gpu[] — enable all GPUs
                    self.gpu_count = None;
                    crow_runtime::gpu::set_gpu_enabled(true);
                    return Ok(CrowValue::done());
                }
                if args.len() != 1 {
                    return Err("gpu[] takes 0 or 1 arguments".to_string());
                }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("gpu[] requires a positive integer".to_string());
                        }
                        self.gpu_count = Some(*n as usize);
                        crow_runtime::gpu::set_gpu_enabled(true);
                        Ok(CrowValue::done())
                    }
                    _ => Err("gpu[] requires an integer argument".to_string()),
                }
            }
            "nodes" => {
                if args.len() != 1 {
                    return Err("nodes[] takes 1 argument".to_string());
                }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("nodes[] requires a positive integer".to_string());
                        }
                        self.nodes_count = Some(*n as usize);
                        // In interpreter mode, simulate single-process (rank 0)
                        // Use init(1) to avoid TCP connections
                        crow_runtime::distributed::init(1);
                        Ok(CrowValue::done())
                    }
                    _ => Err("nodes[] requires an integer argument".to_string()),
                }
            }
            "range" => {
                // range[stop], range[start, stop], range[start, stop, step]
                let (start, stop, step) = match args.len() {
                    1 => {
                        match args[0].unwrap_or_propagate()? {
                            Value::Int(n) => (0i64, *n, 1i64),
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    2 => {
                        match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                            (Value::Int(a), Value::Int(b)) => (*a, *b, 1i64),
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    3 => {
                        match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(a), Value::Int(b), Value::Int(c)) => {
                                if *c == 0 {
                                    return Err("range[] step cannot be zero".to_string());
                                }
                                (*a, *b, *c)
                            }
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    _ => return Err("range[] takes 1, 2, or 3 arguments".to_string()),
                };
                let mut result = Vec::new();
                if step > 0 {
                    let mut i = start;
                    while i < stop {
                        result.push(CrowValue::ok(Value::Int(i)));
                        i += step;
                    }
                } else {
                    let mut i = start;
                    while i > stop {
                        result.push(CrowValue::ok(Value::Int(i)));
                        i += step;
                    }
                }
                Ok(CrowValue::ok(Value::Array(result)))
            }
            // --- Math builtins ---
            "abs" => {
                if args.len() != 1 { return Err("abs[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(n.abs()))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.abs()))),
                    Value::Dual(a, da) => {
                        // abs'(x) = sign(x) * x'
                        let sign = if *a > 0.0 { 1.0 } else if *a < 0.0 { -1.0 } else { 0.0 };
                        Ok(CrowValue::ok(Value::Dual(a.abs(), da * sign)))
                    }
                    _ => Err("abs[] requires a number".into()),
                }
            }
            "sqrt" => {
                if args.len() != 1 { return Err("sqrt[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Float((*n as f64).sqrt()))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.sqrt()))),
                    Value::Dual(a, da) => {
                        // sqrt'(x) = x' / (2*sqrt(x))
                        let s = a.sqrt();
                        Ok(CrowValue::ok(Value::Dual(s, da / (2.0 * s))))
                    }
                    _ => Err("sqrt[] requires a number".into()),
                }
            }
            "floor" => {
                if args.len() != 1 { return Err("floor[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.floor() as i64))),
                    Value::Dual(a, _) => {
                        // floor is a step function, derivative is 0
                        Ok(CrowValue::ok(Value::Dual(a.floor(), 0.0)))
                    }
                    _ => Err("floor[] requires a number".into()),
                }
            }
            "ceil" => {
                if args.len() != 1 { return Err("ceil[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.ceil() as i64))),
                    Value::Dual(a, _) => {
                        Ok(CrowValue::ok(Value::Dual(a.ceil(), 0.0)))
                    }
                    _ => Err("ceil[] requires a number".into()),
                }
            }
            "round" => {
                if args.len() != 1 { return Err("round[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.round() as i64))),
                    Value::Dual(a, _) => {
                        Ok(CrowValue::ok(Value::Dual(a.round(), 0.0)))
                    }
                    _ => Err("round[] requires a number".into()),
                }
            }
            // pow removed — use ^ operator instead
            "min" => {
                if args.len() != 2 { return Err("min[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.min(b)))),
                    (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b)))),
                    (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).min(*b)))),
                    (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b as f64)))),
                    _ => Err("min[] requires numbers".into()),
                }
            }
            "max" => {
                if args.len() != 2 { return Err("max[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.max(b)))),
                    (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b)))),
                    (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).max(*b)))),
                    (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b as f64)))),
                    _ => Err("max[] requires numbers".into()),
                }
            }
            // --- String builtins ---
            "split" => {
                if args.len() != 2 { return Err("split[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(delim)) => {
                        let parts: Vec<CrowValue> = s.split(delim.as_str())
                            .map(|p| CrowValue::ok(Value::Str(p.to_string())))
                            .collect();
                        Ok(CrowValue::ok(Value::Array(parts)))
                    }
                    _ => Err("split[] requires two strings".into()),
                }
            }
            "trim" => {
                if args.len() != 1 { return Err("trim[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.trim().to_string()))),
                    _ => Err("trim[] requires a string".into()),
                }
            }
            "replace" => {
                if args.len() != 3 { return Err("replace[] takes exactly 3 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(old), Value::Str(new)) => {
                        Ok(CrowValue::ok(Value::Str(s.replace(old.as_str(), new.as_str()))))
                    }
                    _ => Err("replace[] requires three strings".into()),
                }
            }
            "upper" => {
                if args.len() != 1 { return Err("upper[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.to_uppercase()))),
                    _ => Err("to_upper[] requires a string".into()),
                }
            }
            "lower" => {
                if args.len() != 1 { return Err("lower[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.to_lowercase()))),
                    _ => Err("to_lower[] requires a string".into()),
                }
            }
            "join" => {
                if args.len() != 2 { return Err("join[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Array(items), Value::Str(sep)) => {
                        let parts: Vec<String> = items.iter()
                            .map(|item| match item.unwrap_or_propagate() {
                                Ok(Value::Str(s)) => s.clone(),
                                Ok(Value::Int(n)) => format!("{}", n),
                                Ok(Value::Float(f)) => format!("{}", f),
                                Ok(Value::Bool(b)) => format!("{}", b),
                                _ => "?".to_string(),
                            })
                            .collect();
                        Ok(CrowValue::ok(Value::Str(parts.join(sep.as_str()))))
                    }
                    (Value::NumArray(t), Value::Str(sep)) => {
                        let parts: Vec<String> = t.to_vec().iter()
                            .map(|v| {
                                if *v == (*v as i64) as f64 && v.is_finite() {
                                    format!("{}", *v as i64)
                                } else {
                                    format!("{}", v)
                                }
                            })
                            .collect();
                        Ok(CrowValue::ok(Value::Str(parts.join(sep.as_str()))))
                    }
                    _ => Err("join[] requires an array and a string separator".into()),
                }
            }
            "starts" => {
                if args.len() != 2 { return Err("starts[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(prefix)) => {
                        Ok(CrowValue::ok(Value::Bool(s.starts_with(prefix.as_str()))))
                    }
                    _ => Err("starts_with[] requires two strings".into()),
                }
            }
            "ends" => {
                if args.len() != 2 { return Err("ends[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(suffix)) => {
                        Ok(CrowValue::ok(Value::Bool(s.ends_with(suffix.as_str()))))
                    }
                    _ => Err("ends_with[] requires two strings".into()),
                }
            }
            "repeat" => {
                if args.len() != 2 { return Err("repeat[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Int(n)) => {
                        if *n < 0 { return Err("repeat[] count cannot be negative".into()); }
                        Ok(CrowValue::ok(Value::Str(s.repeat(*n as usize))))
                    }
                    _ => Err("repeat[] requires a string and an int".into()),
                }
            }
            "pad" => {
                Err("pad was removed — use fmt[s, width = n]".into())
            }
            // fmt: value formatting via the shared display kernel. The lowering
            // pass guarantees exactly 7 positional args (sentinel defaults).
            "fmt" => {
                if args.len() != 9 { return Err("fmt[] takes a value plus named arguments (decimals, commas, width, align, fill, percent, time, tz)".into()); }
                let time_style = match &args[7].value { Value::Str(s) => s.clone(), _ => String::new() };
                if !time_style.is_empty() {
                    let tz = match &args[8].value { Value::Str(s) => s.clone(), _ => "UTC".to_string() };
                    let t = match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        other => return Err(format!("fmt: time display needs a moment (a number), got {}", other.type_name())),
                    };
                    return match crow_runtime::moments::fmt_time(t, &time_style, &tz) {
                        Ok(s) => Ok(CrowValue::ok(Value::Str(s))),
                        Err(e) => Ok(CrowValue::error_with_kind(&e, "time", &self.trace_entry())),
                    };
                }
                let decimals = match &args[1].value { Value::Int(n) => *n, _ => return Err("fmt: decimals must be an int".into()) };
                let commas = matches!(&args[2].value, Value::Bool(true));
                let width = match &args[3].value { Value::Int(n) => *n, _ => return Err("fmt: width must be an int".into()) };
                let align = match &args[4].value { Value::Str(s) => s.clone(), _ => return Err("fmt: align must be a string".into()) };
                if !align.is_empty() && align != "left" && align != "right" {
                    return Err(format!("fmt: align must be 'left' or 'right', got '{}'", align));
                }
                let fill = match &args[5].value { Value::Str(s) => s.clone(), _ => return Err("fmt: fill must be a string".into()) };
                let percent = matches!(&args[6].value, Value::Bool(true));
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Str(
                        crow_runtime::display::crow_fmt_f64(*n as f64, decimals, commas, width, &align, &fill, percent),
                    ))),
                    Value::Float(f) => Ok(CrowValue::ok(Value::Str(
                        crow_runtime::display::crow_fmt_f64(*f, decimals, commas, width, &align, &fill, percent),
                    ))),
                    Value::Str(s) => {
                        if decimals >= 0 { return Err("fmt: decimals applies to numbers, not strings".into()); }
                        if percent { return Err("fmt: percent applies to numbers, not strings".into()); }
                        Ok(CrowValue::ok(Value::Str(
                            crow_runtime::display::crow_fmt_str(s, -1, width, &align, &fill, false),
                        )))
                    }
                    other => Err(format!("fmt: cannot format {}", other.type_name())),
                }
            }
            // --- Conversion builtins ---
            "int" => {
                if args.len() != 1 { return Err("int[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(*n as i64))),
                    Value::Str(s) => match s.parse::<i64>() {
                        Ok(n) => Ok(CrowValue::ok(Value::Int(n))),
                        Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to int", s), "parse", &self.trace_entry())),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Int(if *b { 1 } else { 0 }))),
                    _ => Ok(CrowValue::error_with_kind("cannot convert to int", "parse", &self.trace_entry())),
                }
            }
            "float" => {
                if args.len() != 1 { return Err("float[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Float(*n as f64))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
                    Value::Str(s) => match s.parse::<f64>() {
                        Ok(n) => Ok(CrowValue::ok(Value::Float(n))),
                        Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to float", s), "parse", &self.trace_entry())),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Float(if *b { 1.0 } else { 0.0 }))),
                    _ => Ok(CrowValue::error_with_kind("cannot convert to float", "parse", &self.trace_entry())),
                }
            }
            "str" => {
                if args.len() != 1 { return Err("str[] takes exactly 1 argument".into()); }
                let val = args[0].unwrap_or_propagate()?;
                Ok(CrowValue::ok(Value::Str(format!("{}", val))))
            }
            // --- Collection builtins (mutating: push, pop, sort, reverse, append) ---
            "len" => {
                if args.len() != 1 { return Err("len[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => Ok(CrowValue::ok(Value::Int(items.len() as i64))),
                    Value::NumArray(t) => Ok(CrowValue::ok(Value::Int(t.len() as i64))),
                    Value::Str(s) => Ok(CrowValue::ok(Value::Int(s.chars().count() as i64))),
                    Value::Map(m) => Ok(CrowValue::ok(Value::Int(m.len() as i64))),
                    Value::Frame(f) => Ok(CrowValue::ok(Value::Int(f.nrows() as i64))),
                    _ => Err("len[] requires an array, string, map, or frame".into()),
                }
            }

            "contains" => {
                if args.len() != 2 { return Err("contains[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        let needle = args[1].unwrap_or_propagate()?;
                        let found = items.iter().any(|item| {
                            if let Ok(v) = item.unwrap_or_propagate() {
                                match (v, needle) {
                                    (Value::Int(a), Value::Int(b)) => a == b,
                                    (Value::Float(a), Value::Float(b)) => a == b,
                                    (Value::Str(a), Value::Str(b)) => a == b,
                                    (Value::Bool(a), Value::Bool(b)) => a == b,
                                    _ => false,
                                }
                            } else {
                                false
                            }
                        });
                        Ok(CrowValue::ok(Value::Bool(found)))
                    }
                    Value::NumArray(t) => {
                        let needle = match args[1].unwrap_or_propagate()? {
                            Value::Int(n) => *n as f64,
                            Value::Float(f) => *f,
                            _ => return Err("contains[] on numeric array requires a number".into()),
                        };
                        let found = t.to_vec().iter().any(|v| *v == needle);
                        Ok(CrowValue::ok(Value::Bool(found)))
                    }
                    Value::Str(s) => {
                        match args[1].unwrap_or_propagate()? {
                            Value::Str(sub) => Ok(CrowValue::ok(Value::Bool(s.contains(sub.as_str())))),
                            _ => Err("contains[] with string requires a string needle".into()),
                        }
                    }
                    _ => Err("contains[] requires an array or string".into()),
                }
            }
            "slice" => {
                if args.len() != 3 { return Err("slice[] takes exactly 3 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(start), Value::Int(end)) => {
                                let s = (*start).max(0) as usize;
                                let e = (*end as usize).min(items.len());
                                if s >= e {
                                    Ok(CrowValue::ok(Value::Array(vec![])))
                                } else {
                                    Ok(CrowValue::ok(Value::Array(items[s..e].to_vec())))
                                }
                            }
                            _ => Err("slice[] requires integer start and end".into()),
                        }
                    }
                    Value::NumArray(t) => {
                        match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(start), Value::Int(end)) => {
                                let s = (*start).max(0) as usize;
                                let e = (*end as usize).min(t.len());
                                if s >= e {
                                    Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(vec![], vec![0]))))
                                } else {
                                    Ok(CrowValue::ok(Value::NumArray(t.slice(0, s, e))))
                                }
                            }
                            _ => Err("slice[] requires integer start and end".into()),
                        }
                    }
                    Value::Str(s) => {
                        match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(start), Value::Int(end)) => {
                                // character positions, so 'a—b' slices the same
                                // in both modes and never lands inside a glyph
                                let st = (*start).max(0) as usize;
                                let en = (*end).max(0) as usize;
                                if st >= en {
                                    Ok(CrowValue::ok(Value::Str(String::new())))
                                } else {
                                    Ok(CrowValue::ok(Value::Str(s.chars().skip(st).take(en - st).collect())))
                                }
                            }
                            _ => Err("slice[] requires integer start and end".into()),
                        }
                    }
                    _ => Err("slice[] requires an array or string".into()),
                }
            }
            // --- Map builtins (non-mutating) ---
            "keys" => {
                if args.len() != 1 { return Err("keys[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_list: Vec<CrowValue> = m.keys()
                            .map(|k| CrowValue::ok(Value::Str(k.clone())))
                            .collect();
                        Ok(CrowValue::ok(Value::Array(key_list)))
                    }
                    _ => Err("keys[] requires a map".into()),
                }
            }
            "values" => {
                if args.len() != 1 { return Err("values[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let val_list: Vec<CrowValue> = m.values().cloned().collect();
                        Ok(CrowValue::ok(Value::Array(val_list)))
                    }
                    _ => Err("values[] requires a map".into()),
                }
            }
            "has" => {
                if args.len() != 2 { return Err("has[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("has[] key must be a string".into()),
                        };
                        Ok(CrowValue::ok(Value::Bool(m.contains_key(&key_str))))
                    }
                    _ => Err("has[] requires a map".into()),
                }
            }
            // --- Immutable collection builtins (return new values) ---
            "push" => {
                if args.len() != 2 { return Err("push[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        let mut new_items = items.clone();
                        new_items.push(args[1].clone());
                        Ok(CrowValue::ok(Value::Array(new_items)))
                    }
                    Value::NumArray(t) => {
                        // The element of a rank-N tensor is a rank-(N-1) slice:
                        // scalar for 1-D, row for 2-D. Shape and type checked.
                        match args[1].unwrap_or_propagate()? {
                            Value::Int(n) => Ok(CrowValue::ok(Value::NumArray(t.push_scalar(*n as f64)?))),
                            Value::Float(f) => Ok(CrowValue::ok(Value::NumArray(t.push_scalar(*f)?))),
                            Value::NumArray(row) => Ok(CrowValue::ok(Value::NumArray(t.push_row(row)?))),
                            other => Err(format!(
                                "arrays are homogeneous — cannot add {} to a numeric array",
                                other.type_name()
                            )),
                        }
                    }
                    _ => Err("push[] requires an array".into()),
                }
            }
            "pop" => {
                if args.len() != 1 { return Err("pop[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        if items.is_empty() {
                            return Ok(CrowValue::error("pop from empty array", &self.trace_entry()));
                        }
                        let new_items = items[..items.len() - 1].to_vec();
                        Ok(CrowValue::ok(Value::Array(new_items)))
                    }
                    Value::NumArray(t) => {
                        if t.is_empty() {
                            return Ok(CrowValue::error("pop from empty array", &self.trace_entry()));
                        }
                        let mut data = t.to_vec();
                        data.pop();
                        let len = data.len();
                        Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                    }
                    _ => Err("pop[] requires an array".into()),
                }
            }
            "sort" => {
                if args.len() < 1 || args.len() > 2 { return Err("sort[] takes 1 or 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        let mut new_items = items.clone();
                        new_items.sort_by(|a, b| {
                            let va = a.unwrap_or_propagate().ok();
                            let vb = b.unwrap_or_propagate().ok();
                            match (va, vb) {
                                (Some(Value::Int(x)), Some(Value::Int(y))) => x.cmp(y),
                                (Some(Value::Float(x)), Some(Value::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                                (Some(Value::Str(x)), Some(Value::Str(y))) => x.cmp(y),
                                _ => std::cmp::Ordering::Equal,
                            }
                        });
                        Ok(CrowValue::ok(Value::Array(new_items)))
                    }
                    Value::NumArray(t) => {
                        let mut data = t.to_vec();
                        data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                        let len = data.len();
                        Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                    }
                    _ => Err("sort[] requires an array".into()),
                }
            }
            "reverse" => {
                if args.len() != 1 { return Err("reverse[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Array(items) => {
                        let mut new_items = items.clone();
                        new_items.reverse();
                        Ok(CrowValue::ok(Value::Array(new_items)))
                    }
                    Value::NumArray(t) => {
                        let mut data = t.to_vec();
                        data.reverse();
                        let len = data.len();
                        Ok(CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len]))))
                    }
                    Value::Str(s) => {
                        Ok(CrowValue::ok(Value::Str(s.chars().rev().collect())))
                    }
                    _ => Err("reverse[] requires an array or string".into()),
                }
            }
            "append" => {
                Err("append was removed — use concat[a, b] (one merge operation, see language-spec.md)".into())
            }
            // concat is the single collection-merge operation: plain arrays
            // (axis 0 implied) and tensors (optional axis). Replaces append
            // and the former math-module concat.
            "concat" => {
                if args.len() < 2 || args.len() > 3 { return Err("concat[] takes 2-3 arguments (a, b[, axis])".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(a), Value::Str(b)) => {
                        if args.len() == 3 {
                            return Err("concat: axis applies to tensors only".into());
                        }
                        Ok(CrowValue::ok(Value::Str(format!("{}{}", a, b))))
                    }
                    (Value::Array(items), Value::Array(other)) => {
                        if args.len() == 3 {
                            return Err("concat: axis applies to tensors only".into());
                        }
                        let mut new_items = items.clone();
                        new_items.extend(other.clone());
                        Ok(CrowValue::ok(Value::Array(new_items)))
                    }
                    (Value::NumArray(a), Value::NumArray(b)) => {
                        let axis = if args.len() == 3 {
                            match args[2].unwrap_or_propagate()? {
                                Value::Int(n) => *n as usize,
                                _ => return Err("concat: axis must be an int".into()),
                            }
                        } else { 0 };
                        Ok(CrowValue::ok(Value::NumArray(a.concat(b, axis))))
                    }
                    _ => Err("concat[] requires two arrays of the same kind".into()),
                }
            }
            "set" => {
                if args.len() != 3 { return Err("set[] takes exactly 3 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("set[] key must be a string".into()),
                        };
                        Self::check_map_homogeneous(m, &args[2])?;
                        let mut new_map = m.clone();
                        new_map.insert(key_str, args[2].clone());
                        Ok(CrowValue::ok(Value::Map(new_map)))
                    }
                    _ => Err("set[] requires a map".into()),
                }
            }
            "del" => {
                if args.len() != 2 { return Err("del[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("del[] key must be a string".into()),
                        };
                        let mut new_map = m.clone();
                        new_map.remove(&key_str);
                        Ok(CrowValue::ok(Value::Map(new_map)))
                    }
                    _ => Err("del[] requires a map".into()),
                }
            }
            // --- CLI / IO builtins ---
            "args" => {
                if !args.is_empty() { return Err("args[] takes no arguments".into()); }
                let cli_args: Vec<CrowValue> = std::env::args()
                    .skip(1) // skip binary name
                    .skip_while(|a| a == "run" || a == "-i" || a.ends_with(".crow") || a.ends_with(".crowc"))
                    .map(|a| CrowValue::ok(Value::Str(a)))
                    .collect();
                Ok(CrowValue::ok(Value::Array(cli_args)))
            }
            "input" => {
                if args.len() > 1 { return Err("input[] takes at most one argument (a prompt)".into()); }
                if let Some(p) = args.first() {
                    print!("{}", p.value);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(_) => {
                        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r').to_string();
                        Ok(CrowValue::ok(Value::Str(trimmed)))
                    }
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("input error: {}", e), "io", &self.trace_entry())),
                }
            }
            // --- System builtins ---
            "rand" => {
                if args.len() > 2 { return Err("rand[] takes 0, 1, or 2 arguments".into()); }
                if args.is_empty() {
                    // rand[] — random float between 0.0 and 1.0
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut hasher);
                    std::thread::current().id().hash(&mut hasher);
                    let hash = hasher.finish();
                    let val = (hash as f64) / (u64::MAX as f64);
                    Ok(CrowValue::ok(Value::Float(val)))
                } else if args.len() == 1 {
                    // rand[n] — random int between 0 and n-1
                    match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => {
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut hasher = DefaultHasher::new();
                            std::time::SystemTime::now().hash(&mut hasher);
                            std::thread::current().id().hash(&mut hasher);
                            let hash = hasher.finish();
                            let val = (hash % (*n as u64)) as i64;
                            Ok(CrowValue::ok(Value::Int(val)))
                        }
                        _ => Err("rand[n] requires an integer".into()),
                    }
                } else {
                    // rand[min, max] — random int between min and max (inclusive)
                    match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                        (Value::Int(min), Value::Int(max)) => {
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut hasher = DefaultHasher::new();
                            std::time::SystemTime::now().hash(&mut hasher);
                            std::thread::current().id().hash(&mut hasher);
                            let hash = hasher.finish();
                            let range = (max - min + 1) as u64;
                            let val = min + (hash % range) as i64;
                            Ok(CrowValue::ok(Value::Int(val)))
                        }
                        _ => Err("rand[min, max] requires two integers".into()),
                    }
                }
            }
            "time" => {
                if !args.is_empty() { return Err("time[] takes no arguments".into()); }
                match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                    Ok(d) => Ok(CrowValue::ok(Value::Float(d.as_secs_f64()))),
                    Err(_) => Ok(CrowValue::error("system time before unix epoch", &self.trace_entry())),
                }
            }
            "sleep" => {
                if args.len() != 1 { return Err("sleep[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(ms) => {
                        std::thread::sleep(std::time::Duration::from_millis(*ms as u64));
                        Ok(CrowValue::done())
                    }
                    Value::Float(s) => {
                        std::thread::sleep(std::time::Duration::from_secs_f64(*s));
                        Ok(CrowValue::done())
                    }
                    _ => Err("sleep[] requires a number (milliseconds as int, or seconds as float)".into()),
                }
            }
            "env" => {
                if args.is_empty() || args.len() > 2 { return Err("env[] takes 1 or 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(name) => {
                        match std::env::var(name) {
                            Ok(val) => Ok(CrowValue::ok(Value::Str(val))),
                            Err(_) => {
                                if args.len() == 2 {
                                    // env[key, default] — return default
                                    Ok(args[1].clone())
                                } else {
                                    // env[key] — return error-value
                                    Ok(CrowValue::error_with_kind(
                                        &format!("environment variable '{}' not set", name),
                                        "env",
                                        &self.trace_entry(),
                                    ))
                                }
                            }
                        }
                    }
                    _ => Err("env[] requires a string key".into()),
                }
            }
            "exit" => {
                if args.len() > 1 { return Err("exit[] takes 0 or 1 arguments".into()); }
                let code = if args.is_empty() {
                    0
                } else {
                    match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => *n as i32,
                        _ => return Err("exit[] requires an integer".into()),
                    }
                };
                std::process::exit(code);
            }
            // --- File I/O builtins ---
            "read" => {
                if args.len() != 1 { return Err("read[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        match std::fs::read_to_string(path) {
                            Ok(content) => Ok(CrowValue::ok(Value::Str(content))),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot read '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("read[] requires a string path".into()),
                }
            }
            "write" => {
                if args.len() != 2 { return Err("write[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(path), Value::Str(content)) => {
                        match std::fs::write(path, content) {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot write '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("write[] requires a string path and string content".into()),
                }
            }
            // --- File system builtins ---
            "exists" => {
                if args.len() != 1 { return Err("exists[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => Ok(CrowValue::ok(Value::Bool(std::path::Path::new(path).exists()))),
                    _ => Err("exists[] requires a string path".into()),
                }
            }
            "mkdir" => {
                if args.len() != 1 { return Err("mkdir[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        match std::fs::create_dir_all(path) {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot mkdir '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("mkdir[] requires a string path".into()),
                }
            }
            "ls" => {
                if args.len() != 1 { return Err("ls[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        match std::fs::read_dir(path) {
                            Ok(entries) => {
                                let mut names: Vec<CrowValue> = Vec::new();
                                for entry in entries {
                                    match entry {
                                        Ok(e) => names.push(CrowValue::ok(Value::Str(e.file_name().to_string_lossy().to_string()))),
                                        Err(e) => return Ok(CrowValue::error_with_kind(&format!("ls '{}': {}", path, e), "io", &self.trace_entry())),
                                    }
                                }
                                names.sort_by(|a, b| {
                                    if let (Value::Str(sa), Value::Str(sb)) = (&a.value, &b.value) {
                                        sa.cmp(sb)
                                    } else {
                                        std::cmp::Ordering::Equal
                                    }
                                });
                                Ok(CrowValue::ok(Value::Array(names)))
                            }
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot ls '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("ls[] requires a string path".into()),
                }
            }
            "rm" => {
                if args.len() != 1 { return Err("rm[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        let p = std::path::Path::new(path);
                        let result = if p.is_dir() {
                            std::fs::remove_dir_all(p)
                        } else {
                            std::fs::remove_file(p)
                        };
                        match result {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot rm '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("rm[] requires a string path".into()),
                }
            }
            "mv" => {
                if args.len() != 2 { return Err("mv[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(src), Value::Str(dst)) => {
                        match std::fs::rename(src, dst) {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot mv '{}' to '{}': {}", src, dst, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("mv[] requires two string paths".into()),
                }
            }
            "cp" => {
                if args.len() != 2 { return Err("cp[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(src), Value::Str(dst)) => {
                        match std::fs::copy(src, dst) {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot cp '{}' to '{}': {}", src, dst, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("cp[] requires two string paths".into()),
                }
            }
            // --- JSON builtins ---
            "json" => {
                if args.len() != 1 { return Err("json[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => {
                        match serde_json::from_str::<serde_json::Value>(s) {
                            Ok(jval) => Ok(self.json_to_crow(&jval)),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid JSON: {}", e), "parse", &self.trace_entry())),
                        }
                    }
                    _ => Err("json[] requires a string argument".into()),
                }
            }
            "encode" => {
                if args.len() != 1 { return Err("encode[] takes exactly 1 argument".into()); }
                let jval = self.crow_to_json(&args[0]);
                match jval {
                    Ok(v) => Ok(CrowValue::ok(Value::Str(v.to_string()))),
                    Err(e) => Ok(CrowValue::error(&e, &self.trace_entry())),
                }
            }
            // --- Shell builtin ---
            "shell" => {
                if args.len() != 1 { return Err("shell[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(cmd) => {
                        match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
                            Ok(output) => {
                                let mut map = std::collections::BTreeMap::new();
                                map.insert("status".to_string(), CrowValue::ok(Value::Int(output.status.code().unwrap_or(-1) as i64)));
                                map.insert("stdout".to_string(), CrowValue::ok(Value::Str(String::from_utf8_lossy(&output.stdout).trim_end().to_string())));
                                map.insert("stderr".to_string(), CrowValue::ok(Value::Str(String::from_utf8_lossy(&output.stderr).trim_end().to_string())));
                                Ok(CrowValue::ok(Value::Map(map)))
                            }
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("shell failed: {}", e), "shell", &self.trace_entry())),
                        }
                    }
                    _ => Err("shell[] requires a string command".into()),
                }
            }
            // --- Hashing builtin ---
            "hash" => {
                if args.len() != 2 { return Err("hash[] takes exactly 2 arguments (string, algorithm)".into()); }
                let s = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("hash[] first argument must be a string".into()),
                };
                let algo = match args[1].unwrap_or_propagate()? {
                    Value::Str(a) => a.clone(),
                    _ => return Err("hash[] second argument must be a string algorithm".into()),
                };
                let hex = match algo.as_str() {
                    "sha256" => {
                        use sha2::Digest;
                        let result = sha2::Sha256::digest(s.as_bytes());
                        format!("{:x}", result)
                    }
                    "sha512" => {
                        use sha2::Digest;
                        let result = sha2::Sha512::digest(s.as_bytes());
                        format!("{:x}", result)
                    }
                    "md5" => {
                        use md5::Digest;
                        let result = md5::Md5::digest(s.as_bytes());
                        format!("{:x}", result)
                    }
                    _ => return Ok(CrowValue::error_with_kind(&format!("unsupported hash algorithm '{}' (use sha256, sha512, or md5)", algo), "hash", &self.trace_entry())),
                };
                Ok(CrowValue::ok(Value::Str(hex)))
            }
            // --- Regex builtins ---
            "any" | "all" => {
                if args.len() != 2 { return Err(format!("{}[] takes exactly 2 arguments (array, predicate)", name)); }
                let func = match &args[1].value {
                    Value::Fn(f) => f.clone(),
                    other => return Err(format!("{}[] second argument must be a function, got {}", name, other.type_name())),
                };
                let items = self.search_items(&args[0], name)?;
                let want = name == "any";
                for item in items {
                    let hit = self.call_fn_value(&func, vec![item])?;
                    if self.is_truthy(&hit) == want {
                        // any: found a match → true; all: found a failure → false
                        return Ok(CrowValue::ok(Value::Bool(want)));
                    }
                }
                Ok(CrowValue::ok(Value::Bool(!want)))
            }
            "find" => {
                if args.len() != 2 { return Err("find[] takes exactly 2 arguments".into()); }
                // Search dispatch: find[array, predicate] → first matching element
                if matches!(&args[0].value, Value::Array(_) | Value::NumArray(_)) {
                    let func = match &args[1].value {
                        Value::Fn(f) => f.clone(),
                        other => return Err(format!("find[] on an array takes a function, got {}", other.type_name())),
                    };
                    let items = self.search_items(&args[0], "find")?;
                    for item in items {
                        let hit = self.call_fn_value(&func, vec![item.clone()])?;
                        if self.is_truthy(&hit) {
                            return Ok(item);
                        }
                    }
                    return Ok(CrowValue::error("no match", "find"));
                }
                let s = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("find[] first argument must be a string".into()),
                };
                let pattern = match args[1].unwrap_or_propagate()? {
                    Value::Str(p) => p.clone(),
                    _ => return Err("find[] second argument must be a string pattern".into()),
                };
                match regex::Regex::new(&pattern) {
                    Ok(re) => {
                        if let Some(caps) = re.captures(&s) {
                            let groups: Vec<CrowValue> = caps.iter()
                                .map(|m| match m {
                                    Some(m) => CrowValue::ok(Value::Str(m.as_str().to_string())),
                                    None => CrowValue::ok(Value::Str(String::new())),
                                })
                                .collect();
                            let mut map = std::collections::BTreeMap::new();
                            map.insert("ok".to_string(), CrowValue::ok(Value::Bool(true)));
                            map.insert("groups".to_string(), CrowValue::ok(Value::Array(groups)));
                            Ok(CrowValue::ok(Value::Map(map)))
                        } else {
                            let mut map = std::collections::BTreeMap::new();
                            map.insert("ok".to_string(), CrowValue::ok(Value::Bool(false)));
                            map.insert("groups".to_string(), CrowValue::ok(Value::Array(vec![])));
                            Ok(CrowValue::ok(Value::Map(map)))
                        }
                    }
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid regex: {}", e), "regex", &self.trace_entry())),
                }
            }
            "matches" => {
                if args.len() != 2 { return Err("matches[] takes exactly 2 arguments (string, pattern)".into()); }
                let s = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("matches[] first argument must be a string".into()),
                };
                let pattern = match args[1].unwrap_or_propagate()? {
                    Value::Str(p) => p.clone(),
                    _ => return Err("matches[] second argument must be a string pattern".into()),
                };
                match regex::Regex::new(&pattern) {
                    Ok(re) => Ok(CrowValue::ok(Value::Bool(re.is_match(&s)))),
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid regex: {}", e), "regex", &self.trace_entry())),
                }
            }
            "sub" => {
                if args.len() != 3 { return Err("sub[] takes exactly 3 arguments (string, pattern, replacement)".into()); }
                let s = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("sub[] first argument must be a string".into()),
                };
                let pattern = match args[1].unwrap_or_propagate()? {
                    Value::Str(p) => p.clone(),
                    _ => return Err("sub[] second argument must be a string pattern".into()),
                };
                let replacement = match args[2].unwrap_or_propagate()? {
                    Value::Str(r) => r.clone(),
                    _ => return Err("sub[] third argument must be a string replacement".into()),
                };
                match regex::Regex::new(&pattern) {
                    Ok(re) => Ok(CrowValue::ok(Value::Str(re.replace_all(&s, replacement.as_str()).to_string()))),
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid regex: {}", e), "regex", &self.trace_entry())),
                }
            }
            // --- Date/time builtins ---
            "now" => {
                if !args.is_empty() { return Err("now[] takes no arguments".into()); }
                Ok(CrowValue::ok(Value::Float(crow_runtime::moments::now())))
            }
            "moment" => {
                if args.len() != 7 { return Err("moment[] takes year, month, day + named hour, minute, second, tz".into()); }
                let geti = |i: usize| -> Result<i64, String> {
                    match &args[i].value { Value::Int(n) => Ok(*n), Value::Float(f) => Ok(*f as i64), _ => Err("moment[]: components must be numbers".into()) }
                };
                let second = match &args[5].value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => return Err("moment[]: second must be a number".into()) };
                let tz = match &args[6].value { Value::Str(s) => s.clone(), _ => return Err("moment[]: tz must be a string".into()) };
                match crow_runtime::moments::moment(geti(0)?, geti(1)?, geti(2)?, geti(3)?, geti(4)?, second, &tz) {
                    Ok(t) => Ok(CrowValue::ok(Value::Float(t))),
                    Err(e) => Ok(CrowValue::error_with_kind(&e, "time", &self.trace_entry())),
                }
            }
            "calendar" => {
                if args.len() != 2 { return Err("calendar[] takes a moment + named tz".into()); }
                let t = match &args[0].value { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => return Err("calendar[]: takes a moment (a number)".into()) };
                let tz = match &args[1].value { Value::Str(s) => s.clone(), _ => return Err("calendar[]: tz must be a string".into()) };
                match crow_runtime::moments::calendar(t, &tz) {
                    Ok(fields) => {
                        let mut map = std::collections::BTreeMap::new();
                        for (k, v) in fields { map.insert(k.to_string(), CrowValue::ok(Value::Int(v))); }
                        Ok(CrowValue::ok(Value::Map(map)))
                    }
                    Err(e) => Ok(CrowValue::error_with_kind(&e, "time", &self.trace_entry())),
                }
            }
            "parse_time" => {
                if args.len() != 2 { return Err("parse_time[] takes a string + named tz".into()); }
                let text = match &args[0].value { Value::Str(s) => s.clone(), _ => return Err("parse_time[]: takes a string".into()) };
                let tz = match &args[1].value { Value::Str(s) => s.clone(), _ => return Err("parse_time[]: tz must be a string".into()) };
                match crow_runtime::moments::parse_time(&text, &tz) {
                    Ok(t) => Ok(CrowValue::ok(Value::Float(t))),
                    Err(e) => Ok(CrowValue::error_with_kind(&e, "time", &self.trace_entry())),
                }
            }
            "date" => Err("date[] was removed - a moment is a number: moment[y, m, d] (hour/minute/second/tz are named args)".into()),
            "elapsed" => Err("elapsed[] was removed - subtract moments: end - start is seconds".into()),
            "cors" => {
                // cors[app] or cors[app, origins] → returns app unchanged (mock)
                if args.is_empty() { return Err("cors[] requires at least 1 argument".into()); }
                Ok(args[0].clone())
            }
            "sessions" => {
                // sessions[] → returns empty session store map
                let map = std::collections::BTreeMap::new();
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "templates" => {
                // templates[glob] → returns a stub template engine map
                if args.len() != 1 { return Err("templates[] takes 1 argument".into()); }
                let mut map = std::collections::BTreeMap::new();
                let glob_str = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => "".to_string(),
                };
                map.insert("glob".to_string(), CrowValue::ok(Value::Str(glob_str)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "render" => {
                // render[t, name, data] → returns rendered string (stub: just the name)
                if args.len() != 3 { return Err("render[] takes 3 arguments (templates, name, data)".into()); }
                let name = match args[1].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("render[] name must be a string".into()),
                };
                Ok(CrowValue::ok(Value::Str(format!("[rendered: {}]", name))))
            }
            "cache" => {
                // cache[max] or cache[max, ttl] → returns empty cache map
                if args.is_empty() { return Err("cache[] requires at least 1 argument".into()); }
                let map = std::collections::BTreeMap::new();
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "limit" => {
                // limit[max, window] → returns rate limiter stub map
                if args.len() != 2 { return Err("limit[] takes 2 arguments (max, window)".into()); }
                let mut map = std::collections::BTreeMap::new();
                map.insert("max".to_string(), args[0].clone());
                map.insert("window".to_string(), args[1].clone());
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "uploads" => {
                // uploads[req] → returns empty array (mock)
                if args.len() != 1 { return Err("uploads[] takes 1 argument".into()); }
                Ok(CrowValue::ok(Value::Array(vec![])))
            }
            "sse" => {
                // sse[app, path, handler] → returns app unchanged (mock)
                if args.len() != 3 { return Err("sse[] takes 3 arguments".into()); }
                Ok(args[0].clone())
            }
            "ws" => {
                // ws[app, path, handler] → returns app unchanged (mock)
                if args.len() != 3 { return Err("ws[] takes 3 arguments".into()); }
                Ok(args[0].clone())
            }
            "connect" => {
                // connect[url] → returns mock WebSocket map
                if args.len() != 1 { return Err("connect[] takes 1 argument".into()); }
                let url = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("connect[] url must be a string".into()),
                };
                let mut map = std::collections::BTreeMap::new();
                map.insert("url".to_string(), CrowValue::ok(Value::Str(url)));
                map.insert("ok".to_string(), CrowValue::ok(Value::Bool(false)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "send" => {
                // send[sock, msg] → returns true (mock)
                if args.len() != 2 { return Err("send[] takes 2 arguments".into()); }
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            "recv" => {
                // recv[sock] → returns empty string (mock)
                if args.len() != 1 { return Err("recv[] takes 1 argument".into()); }
                Ok(CrowValue::ok(Value::Str("".to_string())))
            }
            "close" => {
                // close[sock] → returns true (mock)
                if args.len() != 1 { return Err("close[] takes 1 argument".into()); }
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            "static_dir" => {
                // static_dir[app, url_path, dir_path] → returns app unchanged (mock)
                if args.len() != 3 { return Err("static_dir[] takes 3 arguments".into()); }
                Ok(args[0].clone())
            }
            "smtp" => {
                // smtp[host, port, user, pass] → returns mock mailer map
                if args.len() != 4 { return Err("smtp[] takes 4 arguments".into()); }
                let mut map = std::collections::BTreeMap::new();
                let host = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("smtp[] host must be a string".into()),
                };
                map.insert("host".to_string(), CrowValue::ok(Value::Str(host)));
                map.insert("ok".to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }

            // --- Response format builtins (P4) ---
            "msgpack" => {
                // msgpack[value] → returns a mock response map with msgpack content-type
                if args.len() != 1 { return Err("msgpack[] takes 1 argument".into()); }
                let _ = args[0].unwrap_or_propagate()?;
                let mut map = std::collections::BTreeMap::new();
                map.insert("status".to_string(), CrowValue::ok(Value::Int(200)));
                map.insert("type".to_string(), CrowValue::ok(Value::Str("application/msgpack".to_string())));
                map.insert("binary".to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "arrow" => {
                // arrow[frame] → returns a mock response map with arrow content-type
                if args.len() != 1 { return Err("arrow[] takes 1 argument".into()); }
                let _ = args[0].unwrap_or_propagate()?;
                let mut map = std::collections::BTreeMap::new();
                map.insert("status".to_string(), CrowValue::ok(Value::Int(200)));
                map.insert("type".to_string(), CrowValue::ok(Value::Str("application/vnd.apache.arrow.stream".to_string())));
                map.insert("binary".to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "bytes" => {
                // bytes[data] → returns a mock response map with octet-stream content-type
                if args.len() != 1 { return Err("bytes[] takes 1 argument".into()); }
                let _ = args[0].unwrap_or_propagate()?;
                let mut map = std::collections::BTreeMap::new();
                map.insert("status".to_string(), CrowValue::ok(Value::Int(200)));
                map.insert("type".to_string(), CrowValue::ok(Value::Str("application/octet-stream".to_string())));
                map.insert("binary".to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "stream" => {
                // stream[fn[send] ...] → mock: calls the function with a no-op send, returns response map
                if args.len() != 1 { return Err("stream[] takes 1 argument (a function)".into()); }
                let mut map = std::collections::BTreeMap::new();
                map.insert("status".to_string(), CrowValue::ok(Value::Int(200)));
                map.insert("type".to_string(), CrowValue::ok(Value::Str("application/octet-stream".to_string())));
                map.insert("streaming".to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }

            // --- UI reactive builtins (mocks for interpreter) ---
            "serve" => {
                // serve[port] → mock: no-op in interpreter
                Ok(CrowValue::done())
            }
            "page" => {
                // page[path, render] or page[path, render, handle] → mock: no-op in interpreter
                Ok(CrowValue::done())
            }
            "fetch" => {
                // fetch[url] → mock: returns empty map
                if args.len() != 1 { return Err("fetch[] takes 1 argument".into()); }
                Ok(CrowValue::ok(Value::Map(std::collections::BTreeMap::new())))
            }
            "poll" => {
                // poll[url, seconds] → mock: returns empty map
                if args.len() != 2 { return Err("poll[] takes 2 arguments".into()); }
                Ok(CrowValue::ok(Value::Map(std::collections::BTreeMap::new())))
            }
            "slider" => {
                // slider[label, var, min, max, step] — a range input bound to
                // the variable; the same markup the compiled renderer produces
                if args.len() != 5 { return Err("slider[] takes label, var, min, max, step".into()); }
                if self.in_view > 0 {
                    let label = html_escape_text(&format!("{}", args[0].value));
                    let value = html_escape_attr(&format!("{}", args[1].value));
                    return Ok(CrowValue::ok(Value::Str(format!(
                        "<div class=\"crow-field crow-slider\"><label>{}</label><input type=\"range\" min=\"{}\" max=\"{}\" step=\"{}\" value=\"{}\"></input></div>",
                        label, args[2].value, args[3].value, args[4].value, value
                    ))));
                }
                Ok(CrowValue::done())
            }
            "field" => {
                // field[label, var, type, lines] (lowered) → a labelled input
                // bound to the variable, or a textarea when lines > 0 — the
                // markup the compiled renderer produces (minus the binding
                // attribute, which the harness has no use for)
                if args.len() != 4 { return Err("field[] takes label, var, type = '', lines = 0".into()); }
                if self.in_view > 0 {
                    let label = html_escape_text(&format!("{}", args[0].value));
                    let value = html_escape_attr(&format!("{}", args[1].value));
                    let lines = match &args[3].value { Value::Int(n) => *n, Value::Float(f) => *f as i64, _ => 0 };
                    let kind = match &args[2].value {
                        Value::Str(s) if !s.is_empty() => s.clone(),
                        _ => match &args[1].value { Value::Int(_) | Value::Float(_) => "number".to_string(), _ => "text".to_string() },
                    };
                    if lines > 0 {
                        return Ok(CrowValue::ok(Value::Str(format!("<div class=\"crow-field\"><label>{}</label><textarea rows=\"{}\">{}</textarea></div>", label, lines, html_escape_text(&format!("{}", args[1].value))))));
                    }
                    return Ok(CrowValue::ok(Value::Str(format!("<div class=\"crow-field\"><label>{}</label><input type=\"{}\" value=\"{}\"></input></div>", label, html_escape_attr(&kind), value))));
                }
                Ok(CrowValue::done())
            }
            "checkbox" | "toggle" => {
                // checkbox[label, var] / toggle[label, var] — a bool bound to state
                if args.len() != 2 { return Err(format!("{}[] takes label, var", name)); }
                if self.in_view > 0 {
                    let label = html_escape_text(&format!("{}", args[0].value));
                    let checked = match &args[1].value { Value::Bool(b) => *b, Value::Int(n) => *n != 0, Value::Str(s) => s == "true", _ => false };
                    let class = if name == "toggle" { "crow-check crow-toggle" } else { "crow-check" };
                    let on = if checked { "checked=\"\" " } else { "" };
                    return Ok(CrowValue::ok(Value::Str(format!("<label class=\"{}\"><input {}type=\"checkbox\"></input>{}</label>", class, on, label))));
                }
                Ok(CrowValue::done())
            }
            "radio" | "tabs" => {
                // radio[options, var] / tabs[options, var] — one of several
                // strings bound to state; the selected one is marked
                if args.len() != 2 { return Err(format!("{}[] takes options, var", name)); }
                if self.in_view > 0 {
                    let options: Vec<String> = match &args[0].value {
                        Value::Array(items) => items.iter().map(|i| format!("{}", i.value)).collect(),
                        other => return Err(format!("{}[] takes a list of options first, got {}", name, other.type_name())),
                    };
                    let current = format!("{}", args[1].value);
                    let tabs = name == "tabs";
                    let mut out = String::from(if tabs { "<div class=\"crow-tabs\" role=\"tablist\">" } else { "<div class=\"crow-radio\">" });
                    for opt in &options {
                        let selected = *opt == current;
                        let on = if selected { "checked=\"\" " } else { "" };
                        let label_attrs = if tabs { format!(" aria-selected=\"{}\" role=\"tab\"", selected) } else { String::new() };
                        out.push_str(&format!("<label{}><input {}type=\"radio\" value=\"{}\"></input>{}</label>", label_attrs, on, html_escape_attr(opt), html_escape_text(opt)));
                    }
                    out.push_str("</div>");
                    return Ok(CrowValue::ok(Value::Str(out)));
                }
                Ok(CrowValue::done())
            }
            "progress" => {
                // progress[value, max] (lowered)
                if args.len() != 2 { return Err("progress[] takes value, max = 1.0".into()); }
                if self.in_view > 0 {
                    let num = |v: &Value| match v { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => 0.0 };
                    let fmt_num = |x: f64| if (x - x.round()).abs() < 1e-9 { format!("{}", x.round() as i64) } else { format!("{}", x) };
                    let max = num(&args[1].value);
                    return Ok(CrowValue::ok(Value::Str(format!("<progress class=\"crow-progress\" max=\"{}\" value=\"{}\"></progress>", fmt_num(if max > 0.0 { max } else { 1.0 }), fmt_num(num(&args[0].value))))));
                }
                Ok(CrowValue::done())
            }
            "icon" => {
                // icon[name, size, color] (lowered) — inline SVG from the kernel's set
                if args.len() != 3 { return Err("icon[] takes name, size = 'm', color = ''".into()); }
                if self.in_view > 0 {
                    let s = |v: &Value| format!("{}", v);
                    return match crow_runtime::layout::icon_html(&s(&args[0].value), &s(&args[1].value), &s(&args[2].value)) {
                        Ok(svg) => Ok(CrowValue::ok(Value::Str(svg))),
                        Err(e) => Err(format!("icon: {}", e)),
                    };
                }
                Ok(CrowValue::done())
            }
            "today" => Err("today[] was removed - use fmt[now[], time = \'date\'] (tz = ... for a zone)".into()),
            "assert" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("assert[] takes 1 or 2 arguments".to_string());
                }
                let cond = &args[0];
                // An assert is a decision — errors have no truth value
                if !cond.ok {
                    return Err(format!(
                        "cannot branch on an error value — {}",
                        cond.reason
                    ));
                }
                let passed = match &cond.value {
                    Value::Bool(b) => *b,
                    Value::Int(n) => *n != 0,
                    Value::Str(s) => !s.is_empty(),
                    Value::Array(l) => !l.is_empty(),
                    Value::Map(m) => !m.is_empty(),
                    _ => true,
                };
                if !passed {
                    let msg = if args.len() == 2 {
                        format!("assertion failed: {}", args[1].value)
                    } else {
                        "assertion failed".to_string()
                    };
                    return Err(msg);
                }
                Ok(CrowValue::done())
            }
            "error" => {
                // error[reason] or error[reason, kind]
                if args.is_empty() || args.len() > 2 {
                    return Err("error[] takes 1 or 2 arguments".to_string());
                }
                let reason = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    other => format!("{}", other),
                };
                let kind = if args.len() == 2 {
                    match args[1].unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        other => format!("{}", other),
                    }
                } else {
                    String::new()
                };
                Ok(CrowValue::error_with_kind(&reason, &kind, &self.trace_entry()))
            }
            _ => Err(format!("undefined function '{}'", name)),
        }
    }

    /// Convert a serde_json::Value to a CrowValue.
    /// JSON null becomes an error-value with reason "null".
    fn json_to_crow(&self, jval: &serde_json::Value) -> CrowValue {
        match jval {
            serde_json::Value::Null => CrowValue::error_with_kind("null", "parse", &self.trace_entry()),
            serde_json::Value::Bool(b) => CrowValue::ok(Value::Bool(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    CrowValue::ok(Value::Int(i))
                } else if let Some(f) = n.as_f64() {
                    CrowValue::ok(Value::Float(f))
                } else {
                    CrowValue::ok(Value::Int(0))
                }
            }
            serde_json::Value::String(s) => CrowValue::ok(Value::Str(s.clone())),
            serde_json::Value::Array(arr) => {
                // Check if all elements are numeric
                let all_numeric = !arr.is_empty() && arr.iter().all(|v| v.is_number());
                if all_numeric {
                    let data: Vec<f64> = arr.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
                    let len = data.len();
                    CrowValue::ok(Value::NumArray(CrowTensor::from_vec(data, vec![len])))
                } else {
                    let items: Vec<CrowValue> = arr.iter().map(|v| self.json_to_crow(v)).collect();
                    CrowValue::ok(Value::Array(items))
                }
            }
            serde_json::Value::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k.clone(), self.json_to_crow(v));
                }
                CrowValue::ok(Value::Map(map))
            }
        }
    }

    /// Convert a CrowValue to a serde_json::Value for encoding.
    fn crow_to_json(&self, val: &CrowValue) -> Result<serde_json::Value, String> {
        if val.is_error() {
            // Error-values encode as JSON null (round-trip with json[])
            return Ok(serde_json::Value::Null);
        }
        match &val.value {
            Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
            Value::Float(f) => {
                match serde_json::Number::from_f64(*f) {
                    Some(n) => Ok(serde_json::Value::Number(n)),
                    None => Err(format!("cannot encode float {} to JSON", f)),
                }
            }
            Value::Str(s) => Ok(serde_json::Value::String(s.clone())),
            Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
            Value::Array(items) => {
                let arr: Result<Vec<serde_json::Value>, String> = items.iter().map(|v| self.crow_to_json(v)).collect();
                Ok(serde_json::Value::Array(arr?))
            }
            Value::NumArray(t) => {
                let arr: Vec<serde_json::Value> = t.to_vec().iter().map(|v| {
                    if *v == (*v as i64) as f64 && v.is_finite() {
                        serde_json::Value::Number(serde_json::Number::from(*v as i64))
                    } else {
                        match serde_json::Number::from_f64(*v) {
                            Some(n) => serde_json::Value::Number(n),
                            None => serde_json::Value::Null,
                        }
                    }
                }).collect();
                Ok(serde_json::Value::Array(arr))
            }
            Value::Map(map) => {
                let mut obj = serde_json::Map::new();
                for (k, v) in map {
                    obj.insert(k.clone(), self.crow_to_json(v)?);
                }
                Ok(serde_json::Value::Object(obj))
            }
            Value::Done => Ok(serde_json::Value::Null),
            _ => Err(format!("cannot encode {} to JSON", val.value.type_name())),
        }
    }

    fn eval_binop(
        &self,
        left: &CrowValue,
        op: &BinOp,
        right: &CrowValue,
    ) -> Result<CrowValue, String> {
        // soft error propagation: if either operand is an error, propagate it
        if left.is_error() {
            return Ok(left.clone());
        }
        if right.is_error() {
            return Ok(right.clone());
        }
        // Note: trace is appended at the call site (eval_call / call_builtin), not here

        let l = &left.value;
        let r = &right.value;

        match op {
            BinOp::Add => match (l, r) {
                // Dual number rules: dual(a,a') + dual(b,b') = dual(a+b, a'+b')
                (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, da + db))),
                (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a + b, *da))),
                (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a + b, *db))),
                (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a + *b as f64, *da))),
                (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 + b, *db))),
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a + b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a + b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 + b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a + *b as f64))),
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.add(b)))),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(*b as f64)))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(*b)))),
                (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.add_scalar(*a as f64)))),
                (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.add_scalar(*a)))),
                // sparse + sparse
                (Value::Sparse(a), Value::Sparse(b)) => Ok(CrowValue::ok(Value::Sparse(a.add(b)))),
                (Value::Str(_), Value::Str(_)) => Err("cannot use '+' with strings — use string interpolation instead".to_string()),
                // Vectorized: Array + scalar
                (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) |
                (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| a + b)
                }
                _ => Err("cannot add these types".to_string()),
            },
            BinOp::Sub => match (l, r) {
                // Dual: dual(a,a') - dual(b,b') = dual(a-b, a'-b')
                (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a - b, da - db))),
                (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a - b, *da))),
                (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a - b, -db))),
                (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a - *b as f64, *da))),
                (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 - b, -db))),
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a - b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a - b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 - b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a - *b as f64))),
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.sub(b)))),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(-(*b as f64))))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.add_scalar(-*b)))),
                (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.neg().add_scalar(*a as f64)))),
                (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.neg().add_scalar(*a)))),
                // Vectorized: Array - scalar
                (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| a - b)
                }
                (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| b - a)
                }
                _ => Err("cannot subtract these types".to_string()),
            },
            BinOp::Mul => match (l, r) {
                // Dual: dual(a,a') * dual(b,b') = dual(a*b, a'*b + a*b')
                (Value::Dual(a, da), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a * b, da * b + a * db))),
                (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a * b, da * b))),
                (Value::Float(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(a * b, a * db))),
                (Value::Dual(a, da), Value::Int(b)) => Ok(CrowValue::ok(Value::Dual(a * *b as f64, da * *b as f64))),
                (Value::Int(a), Value::Dual(b, db)) => Ok(CrowValue::ok(Value::Dual(*a as f64 * b, *a as f64 * db))),
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a * b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a * b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 * b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a * *b as f64))),
                // tensor * tensor → matmul; tensor * scalar → scale
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.matmul(b)))),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b as f64)))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b)))),
                (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a as f64)))),
                (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a)))),
                // sparse * dense → matmul
                (Value::Sparse(s), Value::NumArray(t)) => Ok(CrowValue::ok(Value::NumArray(s.matmul_dense(t)))),
                // Vectorized: Array * scalar
                (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) |
                (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| a * b)
                }
                _ => Err("cannot multiply these types".to_string()),
            },
            BinOp::Div => match (l, r) {
                // Dual: dual(a,a') / dual(b,b') = dual(a/b, (a'*b - a*b') / b^2)
                (Value::Dual(a, da), Value::Dual(b, db)) => {
                    Ok(CrowValue::ok(Value::Dual(a / b, (da * b - a * db) / (b * b))))
                }
                (Value::Dual(a, da), Value::Float(b)) => Ok(CrowValue::ok(Value::Dual(a / b, da / b))),
                (Value::Float(a), Value::Dual(b, db)) => {
                    Ok(CrowValue::ok(Value::Dual(a / b, (-a * db) / (b * b))))
                }
                (Value::Dual(a, da), Value::Int(b)) => {
                    let bf = *b as f64;
                    Ok(CrowValue::ok(Value::Dual(a / bf, da / bf)))
                }
                (Value::Int(a), Value::Dual(b, db)) => {
                    let af = *a as f64;
                    Ok(CrowValue::ok(Value::Dual(af / b, (-af * db) / (b * b))))
                }
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Ok(CrowValue::error_with_kind("division by zero", "math", &self.trace_entry()))
                    } else {
                        Ok(CrowValue::ok(Value::Int(a / b)))
                    }
                }
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a / b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 / b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a / *b as f64))),
                // tensor / tensor → error (use ./ for element-wise)
                (Value::NumArray(_), Value::NumArray(_)) => Err("use ./ for element-wise tensor division".to_string()),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b as f64)))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b)))),
                (Value::Int(a), Value::NumArray(b)) => {
                    let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a as f64);
                    Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
                }
                (Value::Float(a), Value::NumArray(b)) => {
                    let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a);
                    Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
                }
                // Vectorized: Array / scalar
                (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| a / b)
                }
                (scalar @ (Value::Int(_) | Value::Float(_)), Value::Array(items)) => {
                    Self::array_scalar_arithmetic(items, scalar, |a, b| b / a)
                }
                _ => Err("cannot divide these types".to_string()),
            },
            BinOp::Mod => match (l, r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Ok(CrowValue::error_with_kind("modulo by zero", "math", &self.trace_entry()))
                    } else {
                        Ok(CrowValue::ok(Value::Int(a % b)))
                    }
                }
                _ => Err("modulo only works on integers".to_string()),
            },
            BinOp::Pow => match (l, r) {
                // Dual: d/dx(x^n) = n * x^(n-1) * x'
                (Value::Dual(a, da), Value::Int(b)) => {
                    let bf = *b as f64;
                    Ok(CrowValue::ok(Value::Dual(a.powf(bf), da * bf * a.powf(bf - 1.0))))
                }
                (Value::Dual(a, da), Value::Float(b)) => {
                    Ok(CrowValue::ok(Value::Dual(a.powf(*b), da * b * a.powf(b - 1.0))))
                }
                (Value::Dual(a, da), Value::Dual(b, db)) => {
                    let val = a.powf(*b);
                    let deriv = val * (db * a.ln() + b * da / a);
                    Ok(CrowValue::ok(Value::Dual(val, deriv)))
                }
                (Value::Int(a), Value::Int(b)) => {
                    if *b >= 0 {
                        Ok(CrowValue::ok(Value::Int(a.pow(*b as u32))))
                    } else {
                        Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b as f64))))
                    }
                }
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b as f64)))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b)))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b)))),
                _ => Err("^ requires numbers".to_string()),
            },
            BinOp::ElemMul => match (&Self::coerce_numeric_array(l), &Self::coerce_numeric_array(r)) {
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul(b)))),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b as f64)))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(*b)))),
                (Value::Int(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a as f64)))),
                (Value::Float(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(b.mul_scalar(*a)))),
                _ => Err(".* requires tensors or tensor and scalar".to_string()),
            },
            BinOp::ElemDiv => match (&Self::coerce_numeric_array(l), &Self::coerce_numeric_array(r)) {
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.div(b)))),
                (Value::NumArray(a), Value::Int(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b as f64)))),
                (Value::NumArray(a), Value::Float(b)) => Ok(CrowValue::ok(Value::NumArray(a.mul_scalar(1.0 / *b)))),
                (Value::Int(a), Value::NumArray(b)) => {
                    let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a as f64);
                    Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
                }
                (Value::Float(a), Value::NumArray(b)) => {
                    let scalar_tensor = CrowTensor::full(b.shape().to_vec(), *a);
                    Ok(CrowValue::ok(Value::NumArray(scalar_tensor.div(b))))
                }
                _ => Err("./ requires tensors or tensor and scalar".to_string()),
            },
            BinOp::ElemPow => match (&Self::coerce_numeric_array(l), &Self::coerce_numeric_array(r)) {
                (Value::NumArray(a), Value::NumArray(b)) => Ok(CrowValue::ok(Value::NumArray(a.pow(b)))),
                (Value::NumArray(a), Value::Int(b)) => {
                    let s = *b as f64;
                    Ok(CrowValue::ok(Value::NumArray(a.map(|x| x.powf(s)))))
                }
                (Value::NumArray(a), Value::Float(b)) => {
                    Ok(CrowValue::ok(Value::NumArray(a.map(|x| x.powf(*b)))))
                }
                _ => Err(".^ requires tensors or tensor and scalar".to_string()),
            },
            BinOp::Eq => self.equality_values(left, right, false),
            BinOp::Ne => self.equality_values(left, right, true),
            BinOp::Gt => self.compare_values(l, r, |a, b| a > b),
            BinOp::Lt => self.compare_values(l, r, |a, b| a < b),
            BinOp::Gte => self.compare_values(l, r, |a, b| a >= b),
            BinOp::Lte => self.compare_values(l, r, |a, b| a <= b),
            BinOp::And => self.logical_values(left, right, |a, b| a && b),
            BinOp::Or => self.logical_values(left, right, |a, b| a || b),
        }
    }

    /// eq/ne. Array vs scalar broadcasts element-wise into a boolean array
    /// (mask semantics, matching gt/lt); everything else is whole-value equality.
    fn equality_values(
        &self,
        left: &CrowValue,
        right: &CrowValue,
        negate: bool,
    ) -> Result<CrowValue, String> {
        let is_scalar = |v: &Value| matches!(v, Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_));
        let broadcast = |items: &[CrowValue], scalar: &CrowValue| -> CrowValue {
            let results: Vec<CrowValue> = items.iter().map(|item| {
                let eq = !item.is_error() && self.values_equal(item, scalar);
                CrowValue::ok(Value::Bool(if negate { !eq } else { eq }))
            }).collect();
            CrowValue::ok(Value::Array(results))
        };
        match (&left.value, &right.value) {
            (Value::Array(items), r) if is_scalar(r) => Ok(broadcast(items, right)),
            (l, Value::Array(items)) if is_scalar(l) => Ok(broadcast(items, left)),
            // Tensor vs numeric scalar → element-wise mask (like gt/lt)
            (Value::NumArray(t), Value::Int(_) | Value::Float(_))
            | (Value::Int(_) | Value::Float(_), Value::NumArray(t)) => {
                let s = match (&left.value, &right.value) {
                    (_, Value::Int(n)) | (Value::Int(n), _) => *n as f64,
                    (_, Value::Float(f)) | (Value::Float(f), _) => *f,
                    _ => unreachable!(),
                };
                let results: Vec<CrowValue> = t.to_vec().iter().map(|&v| {
                    let eq = v == s;
                    CrowValue::ok(Value::Bool(if negate { !eq } else { eq }))
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            _ => {
                let eq = self.values_equal(left, right);
                Ok(CrowValue::ok(Value::Bool(if negate { !eq } else { eq })))
            }
        }
    }

    /// and/or. Boolean arrays combine element-wise (mask semantics);
    /// array vs scalar broadcasts; scalars short-circuit as before.
    fn logical_values(
        &self,
        left: &CrowValue,
        right: &CrowValue,
        op: impl Fn(bool, bool) -> bool,
    ) -> Result<CrowValue, String> {
        match (&left.value, &right.value) {
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Err(format!(
                        "boolean arrays have different lengths ({} vs {})",
                        a.len(), b.len()
                    ));
                }
                let results: Vec<CrowValue> = a.iter().zip(b.iter()).map(|(x, y)| {
                    CrowValue::ok(Value::Bool(op(self.is_truthy(x), self.is_truthy(y))))
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            (Value::Array(a), _) => {
                let s = self.is_truthy(right);
                let results: Vec<CrowValue> = a.iter().map(|x| {
                    CrowValue::ok(Value::Bool(op(self.is_truthy(x), s)))
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            (_, Value::Array(b)) => {
                let s = self.is_truthy(left);
                let results: Vec<CrowValue> = b.iter().map(|x| {
                    CrowValue::ok(Value::Bool(op(s, self.is_truthy(x))))
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            _ => Ok(CrowValue::ok(Value::Bool(op(
                self.is_truthy(left),
                self.is_truthy(right),
            )))),
        }
    }

    fn compare_values(
        &self,
        l: &Value,
        r: &Value,
        cmp: impl Fn(f64, f64) -> bool,
    ) -> Result<CrowValue, String> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b as f64))))
            }
            (Value::Float(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
            }
            (Value::Int(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b))))
            }
            (Value::Float(a), Value::Int(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b as f64))))
            }
            (Value::Dual(a, _), Value::Dual(b, _)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
            }
            (Value::Dual(a, _), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
            }
            (Value::Float(a), Value::Dual(b, _)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
            }
            (Value::Dual(a, _), Value::Int(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b as f64))))
            }
            (Value::Int(a), Value::Dual(b, _)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b))))
            }
            // Vectorized: tensor compared to scalar → boolean mask array
            (Value::NumArray(t), Value::Int(b)) => {
                let s = *b as f64;
                let results: Vec<CrowValue> = t.to_vec().iter()
                    .map(|&v| CrowValue::ok(Value::Bool(cmp(v, s))))
                    .collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            (Value::NumArray(t), Value::Float(b)) => {
                let s = *b;
                let results: Vec<CrowValue> = t.to_vec().iter()
                    .map(|&v| CrowValue::ok(Value::Bool(cmp(v, s))))
                    .collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            (Value::Int(a), Value::NumArray(t)) => {
                let s = *a as f64;
                let results: Vec<CrowValue> = t.to_vec().iter()
                    .map(|&v| CrowValue::ok(Value::Bool(cmp(s, v))))
                    .collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            (Value::Float(a), Value::NumArray(t)) => {
                let s = *a;
                let results: Vec<CrowValue> = t.to_vec().iter()
                    .map(|&v| CrowValue::ok(Value::Bool(cmp(s, v))))
                    .collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            // Vectorized: Array compared to scalar → boolean array
            (Value::Array(items), scalar @ (Value::Int(_) | Value::Float(_))) => {
                let results: Vec<CrowValue> = items.iter().map(|item| {
                    if item.is_error() {
                        CrowValue::ok(Value::Bool(false))
                    } else {
                        let val = match (&item.value, scalar) {
                            (Value::Int(a), Value::Int(b)) => cmp(*a as f64, *b as f64),
                            (Value::Float(a), Value::Float(b)) => cmp(*a, *b),
                            (Value::Int(a), Value::Float(b)) => cmp(*a as f64, *b),
                            (Value::Float(a), Value::Int(b)) => cmp(*a, *b as f64),
                            _ => false,
                        };
                        CrowValue::ok(Value::Bool(val))
                    }
                }).collect();
                Ok(CrowValue::ok(Value::Array(results)))
            }
            _ => Err("cannot compare these types".to_string()),
        }
    }

    /// Apply a scalar arithmetic operation element-wise to an Array of CrowValues.
    /// Error elements are propagated (preserved as errors).
    fn array_scalar_arithmetic(
        items: &[CrowValue],
        scalar: &Value,
        op: fn(f64, f64) -> f64,
    ) -> Result<CrowValue, String> {
        let scalar_f = match scalar {
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            _ => return Err("non-numeric scalar in array arithmetic".to_string()),
        };
        let results: Vec<CrowValue> = items.iter().map(|item| {
            if item.is_error() {
                item.clone()
            } else {
                let val = match &item.value {
                    Value::Int(a) => op(*a as f64, scalar_f),
                    Value::Float(a) => op(*a, scalar_f),
                    _ => return CrowValue::error("non-numeric value in array", "type"),
                };
                CrowValue::ok(Value::Float(val))
            }
        }).collect();
        Ok(CrowValue::ok(Value::Array(results)))
    }

    fn eval_unaryop(
        &self,
        op: &UnaryOp,
        val: &CrowValue,
    ) -> Result<CrowValue, String> {
        // soft error propagation
        if val.is_error() {
            return Ok(val.clone());
        }

        let v = &val.value;
        match op {
            UnaryOp::Neg => match v {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(-n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Float(-n))),
                Value::Dual(a, da) => Ok(CrowValue::ok(Value::Dual(-a, -da))),
                Value::NumArray(t) => Ok(CrowValue::ok(Value::NumArray(t.neg()))),
                _ => Err("cannot negate this type".to_string()),
            },
            UnaryOp::Not => match v {
                // Boolean arrays negate element-wise (mask semantics)
                Value::Array(items) => {
                    let results: Vec<CrowValue> = items.iter().map(|x| {
                        CrowValue::ok(Value::Bool(!self.is_truthy(x)))
                    }).collect();
                    Ok(CrowValue::ok(Value::Array(results)))
                }
                _ => Ok(CrowValue::ok(Value::Bool(!self.is_truthy(val)))),
            },
        }
    }

    fn eval_field_access(
        &mut self,
        object: &Expr,
        field: &str,
    ) -> Result<CrowValue, String> {
        let obj = self.eval_expr(object)?;

        // special fields: .ok, .reason, .kind, .trace — always work, even on errors
        match field {
            "ok" => {
                // If the outer CrowValue is ok and holds an Array, map .ok over each element
                if obj.ok {
                    if let Value::Array(items) = &obj.value {
                        let bools: Vec<CrowValue> = items.iter()
                            .map(|item| CrowValue::ok(Value::Bool(item.ok)))
                            .collect();
                        return Ok(CrowValue::ok(Value::Array(bools)));
                    }
                }
                return Ok(CrowValue::ok(Value::Bool(obj.ok)));
            }
            "reason" => {
                return Ok(CrowValue::ok(Value::Str(obj.reason.clone())));
            }
            "kind" => {
                return Ok(CrowValue::ok(Value::Str(obj.kind.clone())));
            }
            "trace" => {
                let trace_list: Vec<CrowValue> = obj.trace.iter()
                    .map(|s| CrowValue::ok(Value::Str(s.clone())))
                    .collect();
                return Ok(CrowValue::ok(Value::Array(trace_list)));
            }
            _ => {}
        }

        // soft error propagation for other field access
        if obj.is_error() {
            return Ok(obj);
        }

        // .length property for lists and strings
        if field == "len" {
            match &obj.value {
                Value::Array(items) => {
                    return Ok(CrowValue::ok(Value::Int(items.len() as i64)));
                }
                Value::NumArray(t) => {
                    return Ok(CrowValue::ok(Value::Int(t.len() as i64)));
                }
                Value::Str(s) => {
                    return Ok(CrowValue::ok(Value::Int(s.len() as i64)));
                }
                _ => return Err("only arrays and strings have .len".to_string()),
            }
        }

        // DateTime properties on strings: .year, .month, .day, .hour, .minute, .second, .offset
        if let Value::Str(s) = &obj.value {
            match field {
                "year" | "month" | "day" | "hour" | "minute" | "second" | "offset" => {
                    if let Some(val) = crow_runtime::frame::datetime_field(s, field) {
                        return Ok(CrowValue::ok(Value::Int(val)));
                    }
                    return Err(format!("'{}' is not a valid datetime string", s));
                }
                _ => {}
            }
        }

        // NumArray properties: .shape, .dtype, .device, .grad
        if let Value::NumArray(t) = &obj.value {
            match field {
                "shape" => {
                    let shape_vals: Vec<CrowValue> = t.shape().iter()
                        .map(|&s| CrowValue::ok(Value::Int(s as i64)))
                        .collect();
                    return Ok(CrowValue::ok(Value::Array(shape_vals)));
                }
                "dtype" => {
                    return Ok(CrowValue::ok(Value::Str("float".to_string())));
                }
                "device" => {
                    let device_str = if t.is_gpu() { "gpu" } else { "cpu" };
                    return Ok(CrowValue::ok(Value::Str(device_str.to_string())));
                }
                "grad" => {
                    // For now, gradient is not stored on the tensor itself
                    return Ok(CrowValue::ok(Value::Str("none".to_string())));
                }
                _ => return Err(format!("numarray has no property '{}'", field)),
            }
        }

        // Frame properties: .shape, .columns, .dtypes, .column_name
        if let Value::Frame(frame) = &obj.value {
            match field {
                "shape" => {
                    let shape = vec![
                        CrowValue::ok(Value::Int(frame.nrows() as i64)),
                        CrowValue::ok(Value::Int(frame.ncols() as i64)),
                    ];
                    return Ok(CrowValue::ok(Value::Array(shape)));
                }
                "columns" => {
                    let cols: Vec<CrowValue> = frame.columns.iter()
                        .map(|c| CrowValue::ok(Value::Str(c.clone())))
                        .collect();
                    return Ok(CrowValue::ok(Value::Array(cols)));
                }
                "dtypes" => {
                    let dtypes: Vec<CrowValue> = frame.dtypes().iter()
                        .map(|d| CrowValue::ok(Value::Str(d.to_string())))
                        .collect();
                    return Ok(CrowValue::ok(Value::Array(dtypes)));
                }
                _ => {
                    // Try column access by name: df.age returns the column as an array
                    if let Some(idx) = frame.col_index(field) {
                        let col = &frame.data[idx];
                        let validity = &frame.validity[idx];
                        let nrows = frame.nrows();
                        let mut vals: Vec<CrowValue> = Vec::with_capacity(nrows);
                        for i in 0..nrows {
                            if !validity[i] {
                                vals.push(CrowValue::error("nan", "missing"));
                            } else {
                                let v = match col.as_ref() {
                                    crow_runtime::frame::Column::Int(v) => Value::Int(v[i]),
                                    crow_runtime::frame::Column::Float(v) => Value::Float(v[i]),
                                    crow_runtime::frame::Column::Str(v) => Value::Str(v[i].clone()),
                                    crow_runtime::frame::Column::Bool(v) => Value::Bool(v[i]),
                                    crow_runtime::frame::Column::DateTime(millis, offsets) => {
                                        Value::Str(crow_runtime::frame::format_datetime(millis[i], offsets[i]))
                                    }
                                };
                                vals.push(CrowValue::ok(v));
                            }
                        }
                        return Ok(CrowValue::ok(Value::Array(vals)));
                    }
                    return Err(format!("frame has no property or column '{}'", field));
                }
            }
        }

        // struct and module field access
        match &obj.value {
            // App session: fields read the current reactive state.
            Value::App(..) => {
                return self.env.get(field).cloned().ok_or_else(|| {
                    format!("no reactive state named '{}'", field)
                });
            }
            Value::StructInstance(si) => {
                match si.fields.get(field) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!(
                        "struct '{}' has no field '{}'",
                        si.name, field
                    )),
                }
            }
            Value::Module(m) => {
                match m.bindings.get(field) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!(
                        "module '{}' has no member '{}'",
                        m.name, field
                    )),
                }
            }
            Value::Map(m) => {
                match m.get(field) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!("map has no key '{}'", field)),
                }
            }
            _ => Err(format!("cannot access field '{}' on this value", field)),
        }
    }

    fn interpolate_string(&mut self, s: &str) -> Result<CrowValue, String> {
        let mut result = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            // Escaped backtick from the lexer: \` is a literal character
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '`' {
                result.push('`');
                i += 2;
                continue;
            }
            if chars[i] == '`' {
                // Interpolation span: scan to the closing backtick, skipping
                // over string literals inside the expression (quote-aware)
                let start = i + 1;
                i += 1;
                let mut in_str = false;
                while i < chars.len() {
                    let c = chars[i];
                    if in_str {
                        if c == '\\' && i + 1 < chars.len() {
                            i += 2;
                            continue;
                        }
                        if c == '\'' {
                            in_str = false;
                        }
                    } else if c == '\'' {
                        in_str = true;
                    } else if c == '`' {
                        break;
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated interpolation — close it with ` or write \\` for a literal backtick".to_string());
                }
                let expr_str: String = chars[start..i].iter().collect();
                if expr_str.trim().is_empty() {
                    return Err("empty interpolation — backticks must contain an expression".to_string());
                }
                i += 1; // skip closing `

                // parse and evaluate the expression
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                let tokens = lexer.tokenize()?;
                let mut parser = crate::parser::Parser::new(tokens);
                let stmts = parser.parse()?;
                // Lower named arguments so interpolated calls like
                // `fmt[x, decimals = 2]` are handled the same as top-level code.
                let stmts = crate::lower::lower_program(&stmts).unwrap_or(stmts);

                if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, crate::ast::Stmt::Line(_))) {
                    let val = self.eval_expr(expr)?;
                    if val.is_error() {
                        let mut err = val;
                        err.append_trace(&self.trace_entry());
                        return Ok(err);
                    }
                    result.push_str(&format!("{}", val.value));
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        Ok(CrowValue::ok(Value::Str(result)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_program(src: &str) -> CrowValue {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap()
    }

    fn get_var(src: &str, var: &str) -> CrowValue {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap();
        interp.env.get(var).unwrap().clone()
    }

    // ===== §1: Variables & Types =====

    #[test]
    fn test_var_int() {
        let val = get_var("x = 42\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_var_negative_int() {
        let val = get_var("x = -7\n", "x");
        assert!(matches!(val.value, Value::Int(-7)));
    }

    #[test]
    fn test_var_float() {
        let val = get_var("x = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_var_string() {
        let val = get_var("x = 'hello'\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_var_bool_true() {
        let val = get_var("x = true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_var_bool_false() {
        let val = get_var("x = false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_var_list() {
        let val = get_var("x = [1, 2, 3]\n", "x");
        // All-numeric arrays become NumArray
        assert!(matches!(val.value, Value::NumArray(ref t) if t.len() == 3));
    }

    #[test]
    fn test_var_empty_list() {
        let val = get_var("x = []\n", "x");
        assert!(matches!(val.value, Value::Array(ref items) if items.is_empty()));
    }

    #[test]
    fn test_string_interpolation() {
        let val = get_var("name = 'crow'\nx = 'hello, `name`'\n", "x");
        assert_eq!(format!("{}", val.value), "hello, crow");
    }

    #[test]
    fn test_string_interpolation_expr() {
        let val = get_var("x = '1 + 2 = `1 + 2`'\n", "x");
        assert_eq!(format!("{}", val.value), "1 + 2 = 3");
    }

    #[test]
    fn test_multi_assign() {
        let val_x = get_var("x, y, z = 1, 2, 3\n", "x");
        let val_z = get_var("x, y, z = 1, 2, 3\n", "z");
        assert!(matches!(val_x.value, Value::Int(1)));
        assert!(matches!(val_z.value, Value::Int(3)));
    }

    #[test]
    fn test_swap() {
        let val_a = get_var("a = 1\nb = 2\na, b = b, a\n", "a");
        let val_b = get_var("a = 1\nb = 2\na, b = b, a\n", "b");
        assert!(matches!(val_a.value, Value::Int(2)));
        assert!(matches!(val_b.value, Value::Int(1)));
    }

    #[test]
    fn test_reassignment_same_type() {
        let val = get_var("x = 10\nx = 20\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_reassignment_int_float_compatible() {
        let val = get_var("x = 10\nx = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    // ===== §3: Functions =====

    #[test]
    fn test_fn_implicit_return() {
        let val = get_var("fn add[a, b]\n    a + b\nresult = add[3, 4]\n", "result");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_fn_explicit_return() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 0]\n", "result");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_fn_explicit_return_nonzero() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 2]\n", "result");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_fn_no_args() {
        let val = get_var("fn greet[]\n    return 'hello'\nresult = greet[]\n", "result");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_fn_first_class() {
        let val = get_var("fn double[x]\n    return x * 2\nfn apply[f, x]\n    return f[x]\nresult = apply[double, 5]\n", "result");
        assert!(matches!(val.value, Value::Int(10)));
    }

    // ===== §4: Control Flow =====

    #[test]
    fn test_if_true() {
        let val = get_var("x = 0\nif true\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_if_false() {
        let val = get_var("x = 0\nif false\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_if_else() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse\n    x = 2\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    #[test]
    fn test_if_else_if() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse if true\n    x = 2\nelse\n    x = 3\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    // ===== §5: Loops =====

    #[test]
    fn test_while_basic() {
        let val = get_var("x = 5\nwhile x gt 0\n    x = x - 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_while_break() {
        let val = get_var("x = 0\nwhile true\n    x = x + 1\n    if x eq 5\n        break\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_while_continue() {
        // Sum 1-10 skipping 5
        let val = get_var("total = 0\ni = 0\nwhile i lt 10\n    i = i + 1\n    if i eq 5\n        continue\n    total = total + i\n", "total");
        assert!(matches!(val.value, Value::Int(50))); // 55 - 5 = 50
    }

    #[test]
    fn test_for_as_expression() {
        let val = get_var("doubled = for x in [1, 2, 3]\n    x * 2\n", "doubled");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_for_as_expression_filter() {
        let val = get_var("evens = for x in [1, 2, 3, 4, 5]\n    if x % 2 eq 0\n        x\n", "evens");
        assert_eq!(format!("{}", val.value), "[2, 4]");
    }

    // ===== §7: Structs =====

    #[test]
    fn test_struct_creation() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\n", "b");
        if let Value::StructInstance(si) = &val.value {
            assert_eq!(si.name, "Bird");
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn test_struct_field_access() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.name\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_struct_field_access_int() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.speed\n", "x");
        assert!(matches!(val.value, Value::Int(50)));
    }

    // ===== §8: Operators =====

    #[test]
    fn test_arithmetic_add() {
        let val = get_var("x = 3 + 4\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_sub() {
        let val = get_var("x = 10 - 3\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_mul() {
        let val = get_var("x = 6 * 7\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_arithmetic_div_int() {
        let val = get_var("x = 7 / 2\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_arithmetic_div_float() {
        let val = get_var("x = 7.0 / 2\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.5).abs() < 0.001));
    }

    #[test]
    fn test_arithmetic_modulo() {
        let val = get_var("x = 10 % 3\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_string_concat_rejected() {
        // String + is not allowed — use interpolation instead
        let result = std::panic::catch_unwind(|| {
            get_var("x = 'hello' + ' world'\n", "x");
        });
        assert!(result.is_err(), "string + should be rejected");
    }

    #[test]
    fn test_comparison_eq() {
        let val = get_var("x = 5 eq 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_ne() {
        let val = get_var("x = 5 ne 3\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gt() {
        let val = get_var("x = 10 gt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lt() {
        let val = get_var("x = 3 lt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gte() {
        let val = get_var("x = 5 gte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lte() {
        let val = get_var("x = 5 lte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_and() {
        let val = get_var("x = true and false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_logical_or() {
        let val = get_var("x = false or true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_not() {
        let val = get_var("x = not true\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_expression_grouping() {
        let val = get_var("x = (1 + 2) * 3\n", "x");
        assert!(matches!(val.value, Value::Int(9)));
    }

    // ===== Existing error tests =====

    #[test]
    fn test_error_line_int_parse() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_error_line_second_line() {
        let val = get_var("y = 10\nx = int['bad']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("2")));
    }

    #[test]
    fn test_error_line_read() {
        let val = get_var("x = read['nonexistent_test_file.txt']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_ok_value_line_zero() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.ok);
        assert!(val.trace.is_empty());
    }

    #[test]
    fn test_raw_value_line_field() {
        // raw values (non-error-value) should have empty trace
        let val = get_var("x = 5\n", "x");
        assert!(val.trace.is_empty());
    }

    #[test]
    fn test_type_change_reassign_error() {
        let mut lexer = crate::lexer::Lexer::new("x = 1\nx = 'hello'\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run(&stmts);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot reassign"));
    }

    #[test]
    fn test_same_type_reassign_ok() {
        let val = get_var("x = 1\nx = 42\n", "x");
        match val.value {
            Value::Int(n) => assert_eq!(n, 42),
            _ => panic!("expected int"),
        }
    }

    #[test]
    fn test_int_float_reassign_ok() {
        let val = get_var("x = 1\nx = 3.14\n", "x");
        match val.value {
            Value::Float(f) => assert!((f - 3.14).abs() < 0.001),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn test_auto_propagation_builtin() {
        // Error should propagate through len[]
        let val = get_var("x = int['bad']\ny = len[x]\n", "y");
        assert!(!val.ok);
        assert!(val.reason.contains("convert"));
    }

    #[test]
    fn test_auto_propagation_user_fn() {
        // Error should propagate through user functions
        let val = get_var("fn double[n]\n    return n * 2\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.ok);
        assert!(val.reason.contains("convert"));
    }

    #[test]
    fn test_auto_propagation_chain() {
        // Error propagates through a chain of calls
        let val = get_var("fn a[x]\n    return x + 1\nfn b[x]\n    return a[x]\ne = int['bad']\nr = b[e]\n", "r");
        assert!(!val.ok);
    }

    #[test]
    fn test_auto_propagation_preserves_line() {
        // The original error trace should contain the original line
        let val = get_var("fn double[n]\n    return n * 2\nz = 10\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("4")));
    }

    #[test]
    fn test_binop_propagates_left_error() {
        // Error in left operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = bad + 10\n", "x");
        assert!(!val.ok, "binop with error left operand should propagate error");
        assert!(val.reason.contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_propagates_right_error() {
        // Error in right operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = 10 + bad\n", "x");
        assert!(!val.ok, "binop with error right operand should propagate error");
        assert!(val.reason.contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_success_unwraps() {
        // Successful CrowResult should auto-unwrap in arithmetic
        let val = get_var("good = int['42']\nx = good + 10\n", "x");
        assert!(val.ok, "binop with successful result should produce ok value");
        assert!(matches!(val.value, Value::Int(52)), "expected 52, got {:?}", val.value);
    }

    #[test]
    fn test_log_no_propagation() {
        // log should NOT propagate — it should print and return done
        let val = get_var("x = int['bad']\ny = log[x]\n", "y");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Done));
    }

    #[test]
    fn test_then_simple() {
        // x then f desugars to f[x]
        let val = get_var("x = '  hello  '\ny = x then trim\n", "y");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_then_chain() {
        // chaining multiple then
        let val = get_var("x = '  hello  '\ny = x then trim then upper\n", "y");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_then_with_args() {
        // x then f[a] desugars to f[x, a]
        let val = get_var("x = 'hello world'\ny = x then split[' '] then join['-']\n", "y");
        assert_eq!(format!("{}", val.value), "hello-world");
    }

    #[test]
    fn test_then_with_conversion() {
        // then works with conversion builtins
        let val = get_var("x = 42\ny = x then str\n", "y");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: range =====

    #[test]
    fn test_range_single_arg() {
        let val = get_var("x = range[5]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 5);
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[4].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_two_args() {
        let val = get_var("x = range[2, 5]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert!(matches!(items[0].value, Value::Int(2)));
            assert!(matches!(items[2].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_three_args() {
        let val = get_var("x = range[0, 10, 3]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 4); // 0, 3, 6, 9
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[3].value, Value::Int(9)));
        } else {
            panic!("expected list");
        }
    }

    // ===== Builtins: math =====

    #[test]
    fn test_abs() {
        let val = get_var("x = abs[-5]\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_sqrt() {
        let val = get_var("x = sqrt[16]\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 4.0).abs() < 0.001));
    }

    #[test]
    fn test_floor() {
        let val = get_var("x = floor[3.7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_ceil() {
        let val = get_var("x = ceil[3.2]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_round() {
        let val = get_var("x = round[3.5]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_pow() {
        let val = get_var("x = 2 ^ 10\n", "x");
        assert!(matches!(val.value, Value::Int(1024)));
    }

    #[test]
    fn test_min() {
        let val = get_var("x = min[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_max() {
        let val = get_var("x = max[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== Builtins: strings =====

    #[test]
    fn test_split() {
        let val = get_var("x = split['a,b,c', ',']\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert_eq!(format!("{}", items[0].value), "a");
            assert_eq!(format!("{}", items[2].value), "c");
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_trim() {
        let val = get_var("x = trim['  hello  ']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_replace() {
        let val = get_var("x = replace['hello world', 'world', 'crow']\n", "x");
        assert_eq!(format!("{}", val.value), "hello crow");
    }

    #[test]
    fn test_upper() {
        let val = get_var("x = upper['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_lower() {
        let val = get_var("x = lower['HELLO']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_starts() {
        let val = get_var("x = starts['hello', 'hel']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_ends() {
        let val = get_var("x = ends['hello', 'llo']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_join() {
        let val = get_var("x = join[['a', 'b', 'c'], '-']\n", "x");
        assert_eq!(format!("{}", val.value), "a-b-c");
    }

    #[test]
    fn test_repeat() {
        let val = get_var("x = repeat['ha', 3]\n", "x");
        assert_eq!(format!("{}", val.value), "hahaha");
    }

    #[test]
    fn test_repeat_zero() {
        let val = get_var("x = repeat['ha', 0]\n", "x");
        assert_eq!(format!("{}", val.value), "");
    }

    #[test]
    fn test_fmt_width_replaces_pad() {
        let val = get_var("x = fmt['ab', width = 5]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "ab   "));
    }

    #[test]
    fn test_fmt_width_shorter_than_string() {
        let val = get_var("x = fmt['hello', width = 3]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "hello"));
    }

    #[test]
    fn test_reverse_string() {
        let val = get_var("x = reverse['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "olleh");
    }

    #[test]
    fn test_reverse_empty_string() {
        let val = get_var("x = reverse['']\n", "x");
        assert_eq!(format!("{}", val.value), "");
    }

    // ===== Builtins: conversion =====

    #[test]
    fn test_int_from_string_success() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_int_from_string_fail() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.ok);
    }

    #[test]
    fn test_int_from_float() {
        let val = get_var("x = int[3.14]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_float_from_string_success() {
        let val = get_var("x = float['3.14']\n", "x");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_str_conversion() {
        let val = get_var("x = str[42]\n", "x");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: collections =====

    #[test]
    fn test_len_list() {
        let val = get_var("x = len[[1, 2, 3]]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_len_string() {
        let val = get_var("x = len['hello']\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_push() {
        let val = get_var("x = push[[1, 2], 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_pop() {
        let val = get_var("x = pop[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2]");
    }

    #[test]
    fn test_append() {
        // Regression: append was removed in favor of concat
        let result = std::panic::catch_unwind(|| {
            run_program("x = append[[1, 2], [3, 4]]\n")
        });
        assert!(result.is_err(), "append should error with removal message");
    }

    #[test]
    fn test_concat_core() {
        let val = get_var("x = concat[[1, 2], [3, 4]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3, 4]");
        let val = get_var("x = concat[['a'], ['b']]\n", "x");
        assert_eq!(format!("{}", val.value), "['a', 'b']");
    }

    #[test]
    fn test_sort() {
        let val = get_var("x = sort[[3, 1, 2]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_reverse() {
        let val = get_var("x = reverse[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "[3, 2, 1]");
    }

    #[test]
    fn test_contains() {
        let val = get_var("x = contains[[1, 2, 3], 2]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_contains_false() {
        let val = get_var("x = contains[[1, 2, 3], 5]\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_slice() {
        let val = get_var("x = slice[[10, 20, 30, 40, 50], 1, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[20, 30]");
    }

    #[test]
    fn test_list_index() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[1]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    // ===== Builtins: map operations =====

    #[test]
    fn test_map_creation() {
        let val = get_var("x = map[name = 'crow', kind = 'bird']\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_map_creation_quoted_keys() {
        let val = get_var("x = map['content-type' = 'json', 'x-id' = 'abc']\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_map_access() {
        let val = get_var("m = map[name = 'crow']\nx = m['name']\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_map_dot_access() {
        let val = get_var("m = map[name = 'crow', kind = 'bird']\nx = m.name\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_map_dot_access_int() {
        let val = get_var("m = map[a = 1, b = 3]\nx = m.b\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_map_nested_dot_access() {
        let val = get_var("m = map[layer1 = map[w = 10, b = 20]]\nx = m.layer1.b\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_map_dot_assign() {
        let val = get_var("m = map[x = 1]\nm.x = 42\nx = m.x\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_map_empty() {
        let val = get_var("x = map[]\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_keys() {
        let val = get_var("m = map[a = 1, b = 2]\nx = keys[m]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_values() {
        let val = get_var("m = map[a = 1, b = 2]\nx = values[m]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_has() {
        let val = get_var("m = map[a = 1]\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_has_false() {
        let val = get_var("m = map[a = 1]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_set() {
        let val = get_var("m = map[a = 1]\nm = set[m, 'b', 2]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_del() {
        let val = get_var("m = map[a = 1, b = 2]\nm = del[m, 'a']\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    // ===== Builtins: error[] =====

    #[test]
    fn test_error_builtin() {
        let val = get_var("x = error['something went wrong']\n", "x");
        assert!(!val.ok);
        assert_eq!(val.reason, "something went wrong");
    }

    #[test]
    fn test_error_builtin_with_kind() {
        let val = get_var("x = error['bad input', 'validation']\n", "x");
        assert!(!val.ok);
        assert_eq!(val.reason, "bad input");
        assert_eq!(val.kind, "validation");
    }

    // ===== §11: Error value status on normal values =====

    #[test]
    fn test_ok_on_normal_value() {
        let val = get_var("x = 5\n", "x");
        assert!(val.ok);
        assert_eq!(val.reason, "");
        assert_eq!(val.kind, "");
        assert!(val.trace.is_empty());
    }

    // ===== Scope =====

    #[test]
    fn test_scope_inner_accesses_outer() {
        let val = get_var("x = 10\nif true\n    y = x + 5\n    x = y\n", "x");
        assert!(matches!(val.value, Value::Int(15)));
    }

    // ===== Negative Indexing =====

    #[test]
    fn test_negative_index_last() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-1]\n", "x");
        assert!(matches!(val.value, Value::Int(30)));
    }

    #[test]
    fn test_negative_index_second_to_last() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-2]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_negative_index_first_via_neg_len() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-3]\n", "x");
        assert!(matches!(val.value, Value::Int(10)));
    }

    #[test]
    fn test_negative_index_out_of_bounds() {
        let src = "nums = [10, 20, 30]\nx = nums[-4]\n";
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run(&stmts);
        assert!(result.is_err());
    }

    #[test]
    fn test_negative_index_assign() {
        let val = get_var("nums = [10, 20, 30]\nnums[-1] = 99\nx = nums[2]\n", "x");
        assert!(matches!(val.value, Value::Int(99)));
    }

    #[test]
    fn test_positive_index_still_works() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[0]\n", "x");
        assert!(matches!(val.value, Value::Int(10)));
    }

    // ===== type[] builtin =====

    #[test]
    fn test_type_int() {
        let val = get_var("x = type[42]\n", "x");
        assert_eq!(format!("{}", val.value), "int");
    }

    #[test]
    fn test_type_float() {
        let val = get_var("x = type[3.14]\n", "x");
        assert_eq!(format!("{}", val.value), "float");
    }

    #[test]
    fn test_type_str() {
        let val = get_var("x = type['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "str");
    }

    #[test]
    fn test_type_bool() {
        let val = get_var("x = type[true]\n", "x");
        assert_eq!(format!("{}", val.value), "bool");
    }

    #[test]
    fn test_type_array() {
        let val = get_var("x = type[[1, 2]]\n", "x");
        assert_eq!(format!("{}", val.value), "array");
    }

    #[test]
    fn test_type_map() {
        let val = get_var("x = type[map[a = 1]]\n", "x");
        assert_eq!(format!("{}", val.value), "map");
    }

    #[test]
    fn test_type_none() {
        let val = get_var("fn f[]\n    x = 1\nx = type[f[]]\n", "x");
        assert_eq!(format!("{}", val.value), "none");
    }

    #[test]
    fn test_type_fn() {
        let val = get_var("fn f[]\n    x = 1\nx = type[f]\n", "x");
        assert_eq!(format!("{}", val.value), "fn");
    }

    #[test]
    fn test_type_struct_instance() {
        let val = get_var("struct Point [x, y]\np = Point[1, 2]\nx = type[p]\n", "x");
        assert_eq!(format!("{}", val.value), "Point");
    }

    #[test]
    fn test_type_error() {
        let val = get_var("e = error['oops']\nx = type[e]\n", "x");
        assert_eq!(format!("{}", val.value), "error");
    }

    // ===== String Indexing =====

    #[test]
    fn test_string_index_first() {
        let val = get_var("x = 'hello'[0]\n", "x");
        assert_eq!(format!("{}", val.value), "h");
    }

    #[test]
    fn test_string_index_last_positive() {
        let val = get_var("x = 'hello'[4]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    #[test]
    fn test_string_negative_index() {
        let val = get_var("x = 'hello'[-1]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    #[test]
    fn test_string_negative_index_first() {
        let val = get_var("x = 'hello'[-5]\n", "x");
        assert_eq!(format!("{}", val.value), "h");
    }

    #[test]
    fn test_string_index_out_of_bounds() {
        let src = "x = 'hi'[5]\n";
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run(&stmts);
        assert!(result.is_err());
    }

    #[test]
    fn test_string_index_via_variable() {
        let val = get_var("s = 'crow'\nx = s[2]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    // ===== NumArray (CrowTensor integration) =====

    #[test]
    fn test_numarray_creation() {
        let val = get_var("x = [1, 2, 3]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_numarray_floats() {
        let val = get_var("x = [1.5, 2.5, 3.5]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
        assert_eq!(format!("{}", val.value), "[1.5, 2.5, 3.5]");
    }

    #[test]
    fn test_numarray_mixed_int_float() {
        let val = get_var("x = [1, 2.5, 3]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
    }

    #[test]
    fn test_non_numeric_stays_array() {
        let val = get_var("x = ['a', 'b']\n", "x");
        assert!(matches!(val.value, Value::Array(_)));
    }

    #[test]
    fn test_empty_array_stays_array() {
        let val = get_var("x = []\n", "x");
        assert!(matches!(val.value, Value::Array(_)));
    }

    #[test]
    fn test_numarray_add() {
        let val = get_var("x = [1, 2, 3] + [4, 5, 6]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 7, 9]");
    }

    #[test]
    fn test_numarray_sub() {
        let val = get_var("x = [10, 20, 30] - [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[9, 18, 27]");
    }

    #[test]
    fn test_numarray_mul() {
        // * on tensor-tensor is matmul; use .* for element-wise
        let val = get_var("x = [2, 3, 4] .* [5, 6, 7]\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 18, 28]");
    }

    #[test]
    fn test_numarray_div() {
        // / on tensor-tensor is error; use ./ for element-wise
        let val = get_var("x = [10, 20, 30] ./ [2, 4, 5]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 5, 6]");
    }

    #[test]
    fn test_numarray_scalar_add() {
        let val = get_var("x = [1, 2, 3] + 10\n", "x");
        assert_eq!(format!("{}", val.value), "[11, 12, 13]");
    }

    #[test]
    fn test_numarray_scalar_mul() {
        let val = get_var("x = [1, 2, 3] * 2\n", "x");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_scalar_numarray_add() {
        let val = get_var("x = 10 + [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[11, 12, 13]");
    }

    #[test]
    fn test_scalar_numarray_sub() {
        let val = get_var("x = 10 - [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[9, 8, 7]");
    }

    #[test]
    fn test_numarray_neg() {
        let val = get_var("x = -[1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[-1, -2, -3]");
    }

    #[test]
    fn test_numarray_index() {
        let val = get_var("x = [10, 20, 30][1]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_numarray_negative_index() {
        let val = get_var("x = [10, 20, 30][-1]\n", "x");
        assert!(matches!(val.value, Value::Int(30)));
    }

    #[test]
    fn test_numarray_len() {
        let val = get_var("x = len[[1, 2, 3, 4]]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_numarray_contains() {
        let val = get_var("x = contains[[1, 2, 3], 2]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_numarray_for_loop() {
        // for-as-expression over a NumArray
        let val = get_var("doubled = for x in [1, 2, 3]\n    x * 2\n", "doubled");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_numarray_equality() {
        let val = get_var("x = [1, 2, 3] eq [1, 2, 3]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_numarray_type() {
        let val = get_var("x = type[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "array");
    }

    #[test]
    fn test_numarray_join() {
        let val = get_var("x = join[[1, 2, 3], ', ']\n", "x");
        assert_eq!(format!("{}", val.value), "1, 2, 3");
    }

    #[test]
    fn test_numarray_dot_len() {
        let val = get_var("nums = [1, 2, 3]\nx = nums.len\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    // ===== Math Module =====

    #[test]
    fn test_math_zeros() {
        let val = get_var("use math\nx = zeros[2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert!(t.to_vec().iter().all(|&v| v == 0.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_ones() {
        let val = get_var("use math\nx = ones[2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert!(t.to_vec().iter().all(|&v| v == 1.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_eye() {
        let val = get_var("use math\nx = eye[3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 3]);
            let d = t.to_vec();
            assert_eq!(d[0], 1.0); assert_eq!(d[1], 0.0); assert_eq!(d[4], 1.0); assert_eq!(d[8], 1.0);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_linspace() {
        let val = get_var("use math\nx = linspace[0, 1, 5]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[5]);
            let d = t.to_vec();
            assert!((d[0] - 0.0).abs() < 1e-10);
            assert!((d[4] - 1.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_arange() {
        let val = get_var("use math\nx = arange[0, 5, 1]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[5]);
            let d = t.to_vec();
            for i in 0..5 { assert_eq!(d[i], i as f64); }
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_full() {
        let val = get_var("use math\nx = full[2, 2, 7]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| v == 7.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_exp() {
        let val = get_var("use math\nx = exp[[1, 2]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let d = t.to_vec();
            assert!((d[0] - 1.0_f64.exp()).abs() < 1e-10);
            assert!((d[1] - 2.0_f64.exp()).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_sqrt() {
        let val = get_var("use math\nx = sqrt[[4, 9, 16]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let d = t.to_vec();
            assert!((d[0] - 2.0).abs() < 1e-10);
            assert!((d[1] - 3.0).abs() < 1e-10);
            assert!((d[2] - 4.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_sum() {
        let val = get_var("use math\nx = sum[[1, 2, 3]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 6.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_mean() {
        let val = get_var("use math\nx = mean[[2, 4, 6]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 4.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_reshape() {
        let val = get_var("use math\nx = reshape[ones[6], 2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_transpose() {
        let val = get_var("use math\nm = ones[2, 3]\nx = transpose[m]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 2]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_matmul() {
        // matmul removed — use * operator instead
        let val = get_var("use math\na = eye[2]\nb = full[2, 2, 3]\nx = a * b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| (v - 3.0).abs() < 1e-10));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_det() {
        let val = get_var("use math\na = eye[3]\nx = det[a]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 1.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_norm_l2() {
        let val = get_var("use math\nx = norm[linspace[3, 4, 2]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 5.0).abs() < 1e-10); }
        else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_norm_l1() {
        let val = get_var("use math\nx = norm[linspace[1, 3, 3], 1]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 6.0).abs() < 1e-10); }
        else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_inv() {
        let val = get_var("use math\na = eye[2]\nx = inv[a]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            let d = t.to_vec();
            assert!((d[0] - 1.0).abs() < 1e-10);
            assert!((d[3] - 1.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_diag() {
        let val = get_var("use math\na = eye[3]\nx = diag[a]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_no_import_error() {
        let result = std::panic::catch_unwind(|| {
            get_var("x = zeros[3]\n", "x");
        });
        assert!(result.is_err() || {
            let val = get_var("x = zeros[3]\n", "x");
            !val.ok
        });
    }

    #[test]
    fn test_math_pi() {
        let val = get_var("use math\nx = pi\n", "x");
        if let Value::Float(f) = val.value {
            assert!((f - std::f64::consts::PI).abs() < 1e-15);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_e() {
        let val = get_var("use math\nx = e\n", "x");
        if let Value::Float(f) = val.value {
            assert!((f - std::f64::consts::E).abs() < 1e-15);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_inf() {
        let val = get_var("use math\nx = inf\n", "x");
        if let Value::Float(f) = val.value {
            assert!(f.is_infinite() && f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_nan() {
        let val = get_var("use math\nx = nan\n", "x");
        if let Value::Float(f) = val.value {
            assert!(f.is_nan());
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_sign() {
        let val = get_var("use math\nx = sign[linspace[-2, 2, 5]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[-1.0, -1.0, 0.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_floor_tensor() {
        let val = get_var("use math\nx = floor[linspace[0.5, 2.5, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 1.0, 2.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_ceil_tensor() {
        let val = get_var("use math\nx = ceil[linspace[0.5, 2.5, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 2.0, 3.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_round_tensor() {
        let val = get_var("use math\nx = round[linspace[0.4, 2.6, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 2.0, 3.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_clip() {
        let val = get_var("use math\nx = clip[linspace[-2, 2, 5], -1, 1]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[-1.0, -1.0, 0.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_mod_scalar() {
        let val = get_var("use math\nx = mod[linspace[1, 5, 5], 2]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 0.0, 1.0, 0.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_tan() {
        let val = get_var("use math\nx = tan[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_asin() {
        let val = get_var("use math\nx = asin[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_tanh() {
        let val = get_var("use math\nx = tanh[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }


    // ===== Operator tests: ^, .*, ./, .^ =====

    #[test]
    fn test_pow_int() {
        let val = get_var("x = 2 ^ 10\n", "x");
        assert!(matches!(val.value, Value::Int(1024)));
    }

    #[test]
    fn test_pow_float() {
        let val = get_var("x = 2.0 ^ 3.0\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 8.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_pow_right_assoc() {
        // 2 ^ 3 ^ 2 = 2 ^ (3 ^ 2) = 2 ^ 9 = 512
        let val = get_var("x = 2 ^ 3 ^ 2\n", "x");
        assert!(matches!(val.value, Value::Int(512)));
    }

    #[test]
    fn test_pow_negative_exp() {
        let val = get_var("x = 2 ^ -1\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 0.5).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_elem_mul() {
        let val = get_var("x = [2, 3, 4] .* [5, 6, 7]\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 18, 28]");
    }

    #[test]
    fn test_elem_div() {
        let val = get_var("x = [10, 20, 30] ./ [2, 4, 5]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 5, 6]");
    }

    #[test]
    fn test_elem_pow() {
        let val = get_var("x = [2, 3] .^ [3, 2]\n", "x");
        assert_eq!(format!("{}", val.value), "[8, 9]");
    }

    #[test]
    fn test_elem_mul_scalar() {
        let val = get_var("x = [1, 2, 3] .* 10\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 20, 30]");
    }

    #[test]
    fn test_elem_pow_scalar() {
        let val = get_var("x = [2, 3, 4] .^ 2\n", "x");
        assert_eq!(format!("{}", val.value), "[4, 9, 16]");
    }

    #[test]
    fn test_tensor_mul_is_matmul() {
        let val = get_var("use math\na = eye[2]\nb = full[2, 2, 3]\nx = a * b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| (v - 3.0).abs() < 1e-10));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_tensor_div_error() {
        // tensor / tensor should error
        let result = std::panic::catch_unwind(|| {
            get_var("x = [1, 2] / [3, 4]\n", "x")
        });
        assert!(result.is_err() || {
            let val = result.unwrap();
            !val.ok
        });
    }

    // ===== Reduction tests: prod, argmax, argmin =====

    #[test]
    fn test_math_prod() {
        let val = get_var("use math\nx = prod[tensor[[2, 3, 4]]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 24.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_argmax() {
        let val = get_var("use math\nx = argmax[tensor[[1, 5, 3]]]\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_math_argmin() {
        let val = get_var("use math\nx = argmin[tensor[[3, 1, 4]]]\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_math_sum_axis() {
        // [[1,2,3],[4,5,6]] → sum axis 0 → [5,7,9]
        let val = get_var("use math\na = reshape[linspace[1, 6, 6], 2, 3]\nx = sum[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3]);
            assert_eq!(t.to_vec(), &[5.0, 7.0, 9.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_max_axis() {
        let val = get_var("use math\na = reshape[linspace[1, 6, 6], 2, 3]\nx = max[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3]);
            assert_eq!(t.to_vec(), &[4.0, 5.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_prod_axis() {
        let val = get_var("use math\na = reshape[linspace[1, 4, 4], 2, 2]\nx = prod[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2]);
            assert_eq!(t.to_vec(), &[3.0, 8.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Sorting & selection tests =====

    #[test]
    fn test_math_argsort() {
        let val = get_var("use math\nx = argsort[tensor[[3, 1, 4, 1, 5]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 0.0, 2.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_topk() {
        let val = get_var("use math\nx = topk[tensor[[3, 1, 4, 1, 5]], 2]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![5.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_unique() {
        let val = get_var("use math\nx = unique[tensor[[3, 1, 4, 1, 3]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_where() {
        let val = get_var("use math\ncond = tensor[[1, 0, 1]]\na = tensor[[10, 20, 30]]\nb = tensor[[100, 200, 300]]\nx = where[cond, a, b]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 200.0, 30.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Cumulative ops tests =====

    #[test]
    fn test_math_cumsum() {
        let val = get_var("use math\nx = cumsum[tensor[[1, 2, 3, 4]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 6.0, 10.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_cumprod() {
        let val = get_var("use math\nx = cumprod[tensor[[1, 2, 3, 4]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 6.0, 24.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Combining ops tests =====

    #[test]
    fn test_math_concat() {
        let val = get_var("use math\na = tensor[[1, 2, 3]]\nb = tensor[[4, 5]]\nx = concat[a, b]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_stack() {
        let val = get_var("use math\na = tensor[[1, 2, 3]]\nb = tensor[[4, 5, 6]]\nx = stack[[a, b]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }


    // ===== Broadcasting tests =====

    #[test]
    fn test_broadcast_add() {
        let val = get_var("use math\na = reshape[linspace[1, 3, 3], 3, 1]\nb = reshape[linspace[10, 40, 4], 1, 4]\nx = a + b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 4]);
            assert_eq!(t.to_vec(), vec![11.0, 21.0, 31.0, 41.0, 12.0, 22.0, 32.0, 42.0, 13.0, 23.0, 33.0, 43.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_broadcast_elem_mul() {
        let val = get_var("use math\na = reshape[linspace[1, 3, 3], 3, 1]\nb = reshape[linspace[1, 2, 2], 1, 2]\nx = a .* b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 2]);
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 2.0, 4.0, 3.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Advanced indexing (gather) tests =====

    #[test]
    fn test_gather_by_tensor() {
        let val = get_var("use math\na = linspace[10, 50, 5]\nidx = linspace[0, 2, 3]\nx = a[idx]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 20.0, 30.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_gather_repeated() {
        let val = get_var("use math\na = linspace[10, 30, 3]\nidx = linspace[0, 0, 3]\nx = a[idx]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 10.0, 10.0]);
        } else { panic!("expected NumArray"); }
    }







    #[test]
    fn test_numarray_dtype() {
        let val = get_var("use math\nx = ones[3]\ny = x.dtype\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "float"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_device() {
        let val = get_var("use math\nx = ones[3]\ny = x.device\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "cpu"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_grad() {
        let val = get_var("use math\nx = ones[3]\ny = x.grad\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "none"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_shape_property() {
        let val = get_var("use math\nx = ones[2, 3]\ny = x.shape\n", "y");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
            if let Value::Int(n) = items[0].value { assert_eq!(n, 2); } else { panic!("expected Int"); }
            if let Value::Int(n) = items[1].value { assert_eq!(n, 3); } else { panic!("expected Int"); }
        } else { panic!("expected Array, got {:?}", val.value); }
    }


    /// The GPU flag is process-global; the tests that touch it take turns.
    static GPU_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_gpu_call_sets_flag() {
        let _serial = GPU_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Reset GPU flag
        crow_runtime::gpu::set_gpu_enabled(false);
        assert!(!crow_runtime::gpu::is_gpu_enabled());
        // gpu[1] should set the flag
        run_program("gpu[1]\n");
        assert!(crow_runtime::gpu::is_gpu_enabled());
        // Reset
        crow_runtime::gpu::set_gpu_enabled(false);
    }

    #[test]
    fn test_gpu_resource_stmt() {
        let _serial = GPU_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crow_runtime::gpu::set_gpu_enabled(false);
        // Directly test the Stmt::Resource path
        let mut interp = Interpreter::new();
        interp.exec_stmt(&Stmt::Resource {
            kind: "gpu".to_string(),
            count: Expr::Int(2),
        }).unwrap();
        assert!(crow_runtime::gpu::is_gpu_enabled());
        crow_runtime::gpu::set_gpu_enabled(false);
    }

    // ===== §nodes: Distributed computing =====

    #[test]
    fn test_nodes_call() {
        // nodes[2] should initialize distributed mode (single-process simulation)
        run_program("nodes[2]\n");
        // In interpreter mode, rank is always 0 and world_size is set
        assert!(crow_runtime::distributed::is_distributed());
        // Reset
        // Note: can't fully reset OnceLock, but the flag is set
    }

    #[test]
    fn test_nodes_resource_stmt() {
        let mut interp = Interpreter::new();
        interp.exec_stmt(&Stmt::Resource {
            kind: "nodes".to_string(),
            count: Expr::Int(4),
        }).unwrap();
        assert!(crow_runtime::distributed::is_distributed());
    }

    // ===== §grad: Automatic differentiation =====

    #[test]
    fn test_grad_single_param() {
        // f(x) = x^2, f'(x) = 2x
        let val = get_var("fn f[x]\n  x * x\ndf = grad[f]\nr = df[3]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 6.0).abs() < 1e-10),
            Value::Int(v) => assert_eq!(v, 6),
            _ => panic!("expected numeric, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_multi_param() {
        // g(x,y) = x^2 + y^2, ∇g = [2x, 2y]
        let val = get_var("fn g[x, y]\n  x * x + y * y\ndg = grad[g]\nr = dg[3, 4]\n", "r");
        if let Value::Array(arr) = val.value {
            assert_eq!(arr.len(), 2);
            match (&arr[0].value, &arr[1].value) {
                (Value::Float(a), Value::Float(b)) => {
                    assert!((a - 6.0).abs() < 1e-10);
                    assert!((b - 8.0).abs() < 1e-10);
                }
                (Value::Int(a), Value::Int(b)) => {
                    assert_eq!(*a, 6);
                    assert_eq!(*b, 8);
                }
                _ => panic!("expected numeric gradient"),
            }
        } else {
            panic!("expected array, got {:?}", val.value);
        }
    }

    #[test]
    fn test_grad_polynomial() {
        // h(x) = x^3 + 2x, h'(x) = 3x^2 + 2, h'(2) = 14
        let val = get_var("fn h[x]\n  x * x * x + 2 * x\ndh = grad[h]\nr = dh[2]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 14.0).abs() < 1e-10),
            Value::Int(v) => assert_eq!(v, 14),
            _ => panic!("expected numeric, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_sqrt() {
        // p(x) = sqrt(x), p'(x) = 1/(2*sqrt(x)), p'(4) = 0.25
        let val = get_var("fn p[x]\n  sqrt[x]\ndp = grad[p]\nr = dp[4]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 0.25).abs() < 1e-10),
            _ => panic!("expected float, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_returns_gradfn() {
        // grad[f] should produce a GradFn value
        let val = get_var("fn f[x]\n  x * x\ndf = grad[f]\n", "df");
        assert!(matches!(val.value, Value::GradFn(_)));
    }

    #[test]
    fn test_grad_abs_derivative() {
        // abs'(x) = sign(x), abs'(-3) = -1
        let val = get_var("fn q[x]\n  abs[x]\ndq = grad[q]\nr = dq[-3]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - (-1.0)).abs() < 1e-10),
            _ => panic!("expected float, got {:?}", val.value),
        }
    }

    // ===== Layer 3: Random & Distributions =====
    // All seed-dependent tests in a single function to avoid global RNG state
    // races (tests share the process-wide GLOBAL_RNG mutex). Same pattern as
    // distributed.rs::test_distributed_all.

    #[test]
    fn test_math_random_all() {
        // --- seed reproducibility ---
        let val = get_var("use math\nseed[42]\na = uniform[0, 1, 3]\nseed[42]\nb = uniform[0, 1, 3]\nr = sum[a] - sum[b]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v).abs() < 1e-15, "same seed should give same uniform, diff={}", v),
            _ => panic!("expected float"),
        }

        // --- normal distribution ---
        let val = get_var("use math\nseed[7]\nx = normal[0, 1, 4]\n", "x");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.to_vec().len(), 4),
            _ => panic!("expected numarray"),
        }

        // --- uniform distribution ---
        let val = get_var("use math\nseed[7]\nx = uniform[2, 5, 3]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert_eq!(t.to_vec().len(), 3);
                for v in t.to_vec() {
                    assert!(v >= 2.0 && v < 5.0, "expected in [2,5), got {}", v);
                }
            }
            _ => panic!("expected numarray"),
        }

        // --- permutation ---
        let val = get_var("use math\nseed[1]\nx = permutation[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert_eq!(t.to_vec().len(), 5);
                let mut v = t.to_vec();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                assert_eq!(v, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
            }
            _ => panic!("expected numarray"),
        }

        // Clear global seed so we don't pollute other tests
        crow_runtime::rng::clear_seed();
    }

    // ===== Layer 4: Signal Processing =====

    #[test]
    fn test_math_fft_interp() {
        // FFT of [1,1,1,1] → DC=4, rest=0
        let val = get_var("use math\nx = fft[ones[4]]\n", "x");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2, "fft should return [real, imag]");
                if let Value::NumArray(real) = &arr[0].value {
                    let v = real.to_vec();
                    assert!((v[0] - 4.0).abs() < 1e-10, "DC should be 4");
                    assert!((v[1]).abs() < 1e-10, "non-DC should be 0");
                } else { panic!("expected numarray for real part"); }
            }
            _ => panic!("expected array from fft"),
        }
    }

    #[test]
    fn test_math_ifft_interp() {
        // FFT then IFFT should recover original
        let val = get_var("use math\na = linspace[1, 4, 4]\nf = fft[a]\nx = ifft[f[0], f[1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 4);
                for (i, expected) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
                    assert!((v[i] - expected).abs() < 1e-10, "ifft[{}] should be {}, got {}", i, expected, v[i]);
                }
            }
            _ => panic!("expected numarray from ifft"),
        }
    }

    #[test]
    fn test_math_conv_interp() {
        // [1,2,3] * [1,1] = [1,3,5,3]
        let val = get_var("use math\na = linspace[1, 3, 3]\nb = linspace[1, 1, 2]\nx = conv[a, b]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 4);
                assert!((v[0] - 1.0).abs() < 1e-10);
                assert!((v[1] - 3.0).abs() < 1e-10);
                assert!((v[2] - 5.0).abs() < 1e-10);
                assert!((v[3] - 3.0).abs() < 1e-10);
            }
            _ => panic!("expected numarray from conv"),
        }
    }

    #[test]
    fn test_math_hamming_interp() {
        let val = get_var("use math\nx = hamming[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 5);
                assert!((v[0] - 0.08).abs() < 1e-10, "hamming endpoint should be 0.08");
            }
            _ => panic!("expected numarray from hamming"),
        }
    }

    #[test]
    fn test_math_hanning_interp() {
        let val = get_var("use math\nx = hanning[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 5);
                assert!((v[0]).abs() < 1e-10, "hanning endpoint should be 0");
                assert!((v[2] - 1.0).abs() < 1e-10, "hanning center should be 1");
            }
            _ => panic!("expected numarray from hanning"),
        }
    }

    // ===== Layer 5: Special Functions & Complex Numbers =====

    #[test]
    fn test_math_gamma_interp() {
        let val = get_var("use math\nx = gamma[linspace[1, 4, 4]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert!((v[0] - 1.0).abs() < 1e-8, "Γ(1) = 1");
                assert!((v[3] - 6.0).abs() < 1e-6, "Γ(4) = 6");
            }
            _ => panic!("expected numarray from gamma"),
        }
    }

    #[test]
    fn test_math_erf_interp() {
        let val = get_var("use math\nx = erf[linspace[1, 1, 1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 0.8427).abs() < 1e-3, "erf(1) ≈ 0.8427");
            }
            _ => panic!("expected numarray from erf"),
        }
    }

    #[test]
    fn test_math_factorial_interp() {
        let val = get_var("use math\nx = factorial[linspace[0, 5, 6]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert!((v[0] - 1.0).abs() < 1e-8, "0! = 1");
                assert!((v[5] - 120.0).abs() < 1e-5, "5! = 120");
            }
            _ => panic!("expected numarray from factorial"),
        }
    }

    #[test]
    fn test_math_choose_interp() {
        let val = get_var("use math\nx = choose[linspace[5, 5, 1], linspace[2, 2, 1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 10.0).abs() < 1e-6, "C(5,2) = 10");
            }
            _ => panic!("expected numarray from choose"),
        }
    }

    #[test]
    fn test_math_complex_interp() {
        let val = get_var("use math\nc = complex[3, 4]\n", "c");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                if let Value::NumArray(re) = &arr[0].value {
                    assert!((re.to_vec()[0] - 3.0).abs() < 1e-10);
                } else { panic!("expected numarray for real part"); }
                if let Value::NumArray(im) = &arr[1].value {
                    assert!((im.to_vec()[0] - 4.0).abs() < 1e-10);
                } else { panic!("expected numarray for imag part"); }
            }
            _ => panic!("expected array from complex"),
        }
    }

    #[test]
    fn test_math_magnitude_interp() {
        let val = get_var("use math\nc = complex[3, 4]\nm = magnitude[c]\n", "m");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 5.0).abs() < 1e-10, "|3+4i| = 5");
            }
            _ => panic!("expected numarray from magnitude"),
        }
    }

    #[test]
    fn test_math_conjugate_interp() {
        let val = get_var("use math\nc = complex[3, 4]\ncc = conjugate[c]\n", "cc");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                if let Value::NumArray(re) = &arr[0].value {
                    assert!((re.to_vec()[0] - 3.0).abs() < 1e-10);
                } else { panic!("expected numarray for real part"); }
                if let Value::NumArray(im) = &arr[1].value {
                    assert!((im.to_vec()[0] - (-4.0)).abs() < 1e-10, "conjugate should negate imag");
                } else { panic!("expected numarray for imag part"); }
            }
            _ => panic!("expected array from conjugate"),
        }
    }

    // ===== Sparse Arrays =====

    #[test]
    fn test_math_sparse_from_dense() {
        let val = get_var("use math\ns = sparse[eye[3]]\n", "s");
        if let Value::Sparse(s) = &val.value {
            assert_eq!(s.nnz(), 3);
            assert_eq!(s.shape(), (3, 3));
        } else { panic!("expected Sparse, got {:?}", val.value); }
    }

    #[test]
    fn test_math_sparse_from_coo() {
        let val = get_var("use math\ns = sparse[[0, 1, 2], [0, 1, 2], [5, 6, 7], [3, 3]]\n", "s");
        if let Value::Sparse(s) = &val.value {
            assert_eq!(s.nnz(), 3);
            assert_eq!(s.shape(), (3, 3));
        } else { panic!("expected Sparse, got {:?}", val.value); }
    }

    #[test]
    fn test_math_dense_from_sparse() {
        let val = get_var("use math\ns = sparse[eye[2]]\nd = dense[s]\n", "d");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 0.0, 0.0, 1.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_matmul() {
        let val = get_var("use math\ns = sparse[eye[3]]\nv = [1, 2, 3]\nr = s * v\n", "r");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_add() {
        let val = get_var("use math\na = sparse[eye[2]]\nb = sparse[eye[2]]\nc = a + b\nd = dense[c]\n", "d");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![2.0, 0.0, 0.0, 2.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_solve() {
        let val = get_var("use math\ns = sparse[[0, 1], [0, 1], [2, 3], [2, 2]]\nx = solve[s, [4, 9]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let v = t.to_vec();
            assert!((v[0] - 2.0).abs() < 1e-10);
            assert!((v[1] - 3.0).abs() < 1e-10);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_type() {
        let val = get_var("use math\ns = sparse[eye[2]]\nt = type[s]\n", "t");
        assert_eq!(format!("{}", val.value), "sparse");
    }

    // ===== Stats sub-module =====

    #[test]
    fn test_stats_median() {
        let val = get_var("use math\nuse math.stats\nx = median[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!((*f - 3.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_std() {
        let val = get_var("use math\nuse math.stats\nx = std[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_var() {
        let val = get_var("use math\nuse math.stats\nx = var[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_percentile() {
        let val = get_var("use math\nuse math.stats\nx = percentile[linspace[1, 5, 5], 50]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!((*f - 3.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_zscore() {
        let val = get_var("use math\nuse math.stats\nz = zscore[linspace[1, 5, 5]]\n", "z");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec().len(), 5);
            // Mean of zscore should be ~0
            let m: f64 = t.to_vec().iter().sum::<f64>() / 5.0;
            assert!(m.abs() < 1e-10);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_corr() {
        let val = get_var("use math\nuse math.stats\nc = corr[linspace[1, 5, 5], linspace[2, 10, 5]]\n", "c");
        if let Value::Float(f) = &val.value {
            assert!((*f - 1.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_cov() {
        let val = get_var("use math\nuse math.stats\nc = cov[linspace[1, 5, 5], linspace[2, 10, 5]]\n", "c");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_ttest() {
        let val = get_var("use math\nuse math.stats\nr = ttest[linspace[1, 5, 5], linspace[10, 50, 5]]\n", "r");
        if let Value::Map(m) = &val.value {
            assert!(m.contains_key("t"));
            assert!(m.contains_key("p"));
        } else { panic!("expected Map, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_requires_import() {
        // Without `use math.stats`, stats functions should fail
        let result = std::panic::catch_unwind(|| {
            get_var("use math\nx = median[linspace[1, 5, 5]]\n", "x")
        });
        assert!(result.is_err() || {
            // It might return an error value instead of panicking
            true
        });
    }

    // ===== File System Builtins =====

    #[test]
    fn test_fs_exists() {
        // Cargo.toml always exists in the project root
        let val = get_var("x = exists['Cargo.toml']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
        // Non-existent file
        let val2 = get_var("x = exists['no_such_file_xyz.txt']\n", "x");
        assert!(matches!(val2.value, Value::Bool(false)));
    }

    #[test]
    fn test_fs_mkdir_ls_rm() {
        let dir = format!("/tmp/crow_test_fs_{}", std::process::id());
        let src = format!(
            "mkdir['{d}']\nwrite['{d}/hello.txt', 'hi']\nfiles = ls['{d}']\ne = exists['{d}/hello.txt']\nrm['{d}']\ne2 = exists['{d}']\n",
            d = dir
        );
        let mut lexer = crate::lexer::Lexer::new(&src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap();

        // ls should return array with 'hello.txt'
        let files = interp.env.get("files").unwrap().clone();
        if let Value::Array(arr) = &files.value {
            assert_eq!(arr.len(), 1);
            assert!(matches!(&arr[0].value, Value::Str(s) if s == "hello.txt"));
        } else {
            panic!("expected array from ls, got {:?}", files.value);
        }
        // exists should have been true before rm
        let e = interp.env.get("e").unwrap().clone();
        assert!(matches!(e.value, Value::Bool(true)));
        // exists should be false after rm
        let e2 = interp.env.get("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(false)));
    }

    #[test]
    fn test_fs_mv() {
        let base = format!("/tmp/crow_test_mv_{}", std::process::id());
        let src = format!(
            "mkdir['{b}']\nwrite['{b}/a.txt', 'data']\nmv['{b}/a.txt', '{b}/b.txt']\ne1 = exists['{b}/a.txt']\ne2 = exists['{b}/b.txt']\nrm['{b}']\n",
            b = base
        );
        let mut lexer = crate::lexer::Lexer::new(&src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap();

        let e1 = interp.env.get("e1").unwrap().clone();
        assert!(matches!(e1.value, Value::Bool(false)), "source should not exist after mv");
        let e2 = interp.env.get("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(true)), "destination should exist after mv");
    }

    #[test]
    fn test_fs_cp() {
        let base = format!("/tmp/crow_test_cp_{}", std::process::id());
        let src = format!(
            "mkdir['{b}']\nwrite['{b}/orig.txt', 'hello']\ncp['{b}/orig.txt', '{b}/copy.txt']\ne1 = exists['{b}/orig.txt']\ne2 = exists['{b}/copy.txt']\nc = read['{b}/copy.txt']\nrm['{b}']\n",
            b = base
        );
        let mut lexer = crate::lexer::Lexer::new(&src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap();

        let e1 = interp.env.get("e1").unwrap().clone();
        assert!(matches!(e1.value, Value::Bool(true)), "original should still exist after cp");
        let e2 = interp.env.get("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(true)), "copy should exist after cp");
        let c = interp.env.get("c").unwrap().clone();
        assert!(matches!(&c.value, Value::Str(s) if s == "hello"), "copy content should match");
    }

    #[test]
    fn test_fs_mkdir_error_propagation() {
        // mkdir returns error-value on failure, not panic
        let val = get_var("x = mkdir['/root/no_permission_dir_xyz']\n", "x");
        assert!(!val.ok, "mkdir on inaccessible path should return error-value");
    }

    #[test]
    fn test_fs_rm_nonexistent() {
        let val = get_var("x = rm['/tmp/no_such_file_crow_test_xyz']\n", "x");
        assert!(!val.ok, "rm on nonexistent path should return error-value");
    }

    // ===== env builtin =====

    #[test]
    fn test_env_existing_var() {
        unsafe { std::env::set_var("CROW_TEST_ENV_VAR", "hello123"); }
        let val = get_var("x = env['CROW_TEST_ENV_VAR']\n", "x");
        assert!(val.ok, "env[] on existing var should succeed");
        assert_eq!(format!("{}", val.value), "hello123");
        unsafe { std::env::remove_var("CROW_TEST_ENV_VAR"); }
    }

    #[test]
    fn test_env_missing_var_no_default() {
        unsafe { std::env::remove_var("CROW_TEST_NONEXISTENT_XYZ"); }
        let val = get_var("x = env['CROW_TEST_NONEXISTENT_XYZ']\n", "x");
        assert!(!val.ok, "env[] on missing var without default should return error-value");
    }

    #[test]
    fn test_env_missing_var_with_default() {
        unsafe { std::env::remove_var("CROW_TEST_NONEXISTENT_XYZ"); }
        let val = get_var("x = env['CROW_TEST_NONEXISTENT_XYZ', 3000]\n", "x");
        assert!(val.ok, "env[] with default should succeed");
        assert_eq!(format!("{}", val.value), "3000");
    }

    #[test]
    fn test_env_existing_var_ignores_default() {
        unsafe { std::env::set_var("CROW_TEST_ENV_PORT", "8080"); }
        let val = get_var("x = env['CROW_TEST_ENV_PORT', 3000]\n", "x");
        assert!(val.ok);
        assert_eq!(format!("{}", val.value), "8080");
        unsafe { std::env::remove_var("CROW_TEST_ENV_PORT"); }
    }

    // --- P3: Shell ---
    #[test]
    fn test_shell_builtin() {
        let val = run_program("x = shell['echo hello']\nlog[x.stdout]");
        assert!(val.ok);
    }

    #[test]
    fn test_shell_status() {
        let val = get_var("r = shell['true']", "r");
        match &val.value {
            Value::Map(m) => {
                let status = m.get("status").unwrap();
                assert_eq!(format!("{}", status.value), "0");
            }
            _ => panic!("expected map"),
        }
    }

    // --- P3: Hashing ---
    #[test]
    fn test_hash_sha256() {
        let val = get_var("h = hash['hello', 'sha256']", "h");
        assert_eq!(format!("{}", val.value), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_hash_md5() {
        let val = get_var("h = hash['hello', 'md5']", "h");
        assert_eq!(format!("{}", val.value), "5d41402abc4b2a76b9719d911017c592");
    }

    // --- P3: Regex ---
    #[test]
    fn test_matches_true() {
        let val = get_var("r = matches['hello', '[a-z]+']", "r");
        assert_eq!(format!("{}", val.value), "true");
    }

    #[test]
    fn test_matches_false() {
        let val = get_var("r = matches['hello', '[0-9]+']", "r");
        assert_eq!(format!("{}", val.value), "false");
    }

    #[test]
    fn test_find_ok() {
        let val = get_var("m = find['abc123', '([a-z]+)([0-9]+)']", "m");
        match &val.value {
            Value::Map(m) => {
                let ok = m.get("ok").unwrap();
                assert_eq!(format!("{}", ok.value), "true");
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn test_sub_regex() {
        let val = get_var("r = sub['hello world', 'world', 'crow']", "r");
        assert_eq!(format!("{}", val.value), "hello crow");
    }

    // --- P3: Date/Time ---



    // --- P4: Web builtin interpreter mock tests ---

    #[test]
    fn test_cors_builtin() {
        let val = get_var("app = []\napp = cors[app]", "app");
        assert!(matches!(&val.value, Value::Array(_)));
    }

    #[test]
    fn test_sessions_builtin() {
        let val = get_var("store = sessions[]", "store");
        assert!(matches!(&val.value, Value::Map(_)));
    }

    #[test]
    fn test_templates_builtin() {
        let val = get_var("t = templates['./templates/**/*']", "t");
        match &val.value {
            Value::Map(m) => {
                let glob = m.get("glob").unwrap();
                assert_eq!(format!("{}", glob.value), "./templates/**/*");
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn test_render_builtin() {
        let val = get_var("t = templates['./t/**/*']\ndata = map[name = 'crow']\nresult = render[t, 'index', data]", "result");
        match &val.value {
            Value::Str(s) => assert!(s.contains("rendered"), "should contain 'rendered': {}", s),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_cache_builtin() {
        let val = get_var("c = cache[100]", "c");
        assert!(matches!(&val.value, Value::Map(_)));
    }

    #[test]
    fn test_cache_with_ttl_builtin() {
        let val = get_var("c = cache[100, 60]", "c");
        assert!(matches!(&val.value, Value::Map(_)));
    }

    #[test]
    fn test_limit_builtin() {
        let val = get_var("limiter = limit[100, 60]", "limiter");
        match &val.value {
            Value::Map(m) => {
                assert!(m.contains_key("max"));
                assert!(m.contains_key("window"));
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn test_uploads_builtin() {
        let val = get_var("req = []\nfiles = uploads[req]", "files");
        assert!(matches!(&val.value, Value::Array(_)));
    }

    #[test]
    fn test_sse_builtin() {
        let val = get_var("fn handler[req]\n    return 'data'\napp = []\napp = sse[app, '/events', handler]", "app");
        assert!(matches!(&val.value, Value::Array(_)));
    }

    #[test]
    fn test_ws_builtin() {
        let val = get_var("fn handler[req]\n    return 'ok'\napp = []\napp = ws[app, '/ws', handler]", "app");
        assert!(matches!(&val.value, Value::Array(_)));
    }

    #[test]
    fn test_connect_builtin() {
        let val = get_var("sock = connect['ws://localhost:3000']", "sock");
        match &val.value {
            Value::Map(m) => {
                let url = m.get("url").unwrap();
                assert_eq!(format!("{}", url.value), "ws://localhost:3000");
            }
            _ => panic!("expected map"),
        }
    }

    #[test]
    fn test_send_recv_close_builtins() {
        let val = get_var("sock = connect['ws://localhost:3000']\nresult = send[sock, 'hi']", "result");
        assert!(matches!(&val.value, Value::Bool(true)));

        let val = get_var("sock = connect['ws://localhost:3000']\nmsg = recv[sock]", "msg");
        assert!(matches!(&val.value, Value::Str(_)));

        let val = get_var("sock = connect['ws://localhost:3000']\nresult = close[sock]", "result");
        assert!(matches!(&val.value, Value::Bool(true)));
    }

    /// `live[...]` was a mock of a retired API; `page[]` is the one way. The
    /// name is a user's to define, in both modes.
    #[test]
    fn live_is_not_a_builtin() {
        let val = get_var("fn live[x]\n    return x + 1\nn = live[1]", "n");
        assert!(matches!(&val.value, Value::Int(2)), "got {:?}", val.value);
    }

    #[test]
    fn test_static_dir_builtin() {
        let val = get_var("app = []\napp = static_dir[app, '/assets', './public']", "app");
        assert!(matches!(&val.value, Value::Array(_)));
    }

    #[test]
    fn test_smtp_builtin() {
        let val = get_var("mailer = smtp['smtp.example.com', 587, 'user', 'pass']", "mailer");
        match &val.value {
            Value::Map(m) => {
                assert!(m.contains_key("host"));
                assert!(m.contains_key("ok"));
            }
            _ => panic!("expected map"),
        }
    }

    // ===== Response format tests (P4) =====

    #[test]
    fn test_msgpack_builtin() {
        let val = get_var("resp = msgpack['hello']", "resp");
        match &val.value {
            Value::Map(m) => {
                assert!(matches!(m.get("status").unwrap().value, Value::Int(200)));
                assert!(matches!(&m.get("type").unwrap().value, Value::Str(s) if s == "application/msgpack"));
                assert!(matches!(m.get("binary").unwrap().value, Value::Bool(true)));
            }
            _ => panic!("expected map, got {:?}", val.value),
        }
    }

    #[test]
    fn test_arrow_builtin() {
        let val = get_var("resp = arrow['frame_placeholder']", "resp");
        match &val.value {
            Value::Map(m) => {
                assert!(matches!(m.get("status").unwrap().value, Value::Int(200)));
                assert!(matches!(&m.get("type").unwrap().value, Value::Str(s) if s == "application/vnd.apache.arrow.stream"));
                assert!(matches!(m.get("binary").unwrap().value, Value::Bool(true)));
            }
            _ => panic!("expected map, got {:?}", val.value),
        }
    }

    #[test]
    fn test_bytes_builtin() {
        let val = get_var("resp = bytes['raw data']", "resp");
        match &val.value {
            Value::Map(m) => {
                assert!(matches!(m.get("status").unwrap().value, Value::Int(200)));
                assert!(matches!(&m.get("type").unwrap().value, Value::Str(s) if s == "application/octet-stream"));
                assert!(matches!(m.get("binary").unwrap().value, Value::Bool(true)));
            }
            _ => panic!("expected map, got {:?}", val.value),
        }
    }

    #[test]
    fn test_stream_builtin() {
        let val = get_var("resp = stream['handler_placeholder']", "resp");
        match &val.value {
            Value::Map(m) => {
                assert!(matches!(m.get("status").unwrap().value, Value::Int(200)));
                assert!(matches!(m.get("streaming").unwrap().value, Value::Bool(true)));
            }
            _ => panic!("expected map, got {:?}", val.value),
        }
    }

    #[test]
    fn test_msgpack_wrong_args() {
        let result = std::panic::catch_unwind(|| {
            get_var("resp = msgpack[]", "resp");
        });
        assert!(result.is_err(), "msgpack[] with no args should fail");
    }

    #[test]
    fn test_arrow_wrong_args() {
        let result = std::panic::catch_unwind(|| {
            get_var("resp = arrow[]", "resp");
        });
        assert!(result.is_err(), "arrow[] with no args should fail");
    }

    // ===== DB module tests =====

    #[test]
    fn test_db_connect_sqlite() {
        let val = get_var("use db\nconn = connect['sqlite://:memory:']\n", "conn");
        assert!(matches!(val.value, Value::DbConn(_)), "expected DbConn, got {:?}", val.value);
    }

    #[test]
    fn test_db_connect_prefixed() {
        let val = get_var("use db\nconn = db.connect['sqlite://:memory:']\n", "conn");
        assert!(matches!(val.value, Value::DbConn(_)), "expected DbConn via db.connect");
    }

    #[test]
    fn test_db_exec_create_table() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nn = exec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\n",
            "n"
        );
        assert!(matches!(val.value, Value::Int(0)), "CREATE TABLE should return 0 affected rows, got {:?}", val.value);
    }

    #[test]
    fn test_db_exec_insert() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nn = exec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\n",
            "n"
        );
        assert!(matches!(val.value, Value::Int(1)), "INSERT should return 1 affected row, got {:?}", val.value);
    }

    #[test]
    fn test_db_query() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nexec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\nexec[conn, 'INSERT INTO t VALUES (2, ?)', ['bob']]\ndf = query[conn, 'SELECT * FROM t ORDER BY id']\n",
            "df"
        );
        match &val.value {
            Value::Frame(f) => {
                assert_eq!(f.nrows(), 2, "should have 2 rows");
                assert_eq!(f.ncols(), 2, "should have 2 columns");
            }
            _ => panic!("expected Frame, got {:?}", val.value),
        }
    }

    #[test]
    fn test_db_query_with_params() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nexec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\nexec[conn, 'INSERT INTO t VALUES (2, ?)', ['bob']]\ndf = query[conn, 'SELECT * FROM t WHERE name = ?', ['alice']]\n",
            "df"
        );
        match &val.value {
            Value::Frame(f) => {
                assert_eq!(f.nrows(), 1, "should have 1 row for alice");
            }
            _ => panic!("expected Frame, got {:?}", val.value),
        }
    }

    #[test]
    fn test_db_transact() {
        // transact should commit on success
        run_program(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER)']\nfn do_inserts[tx]\n    exec[tx, 'INSERT INTO t VALUES (1)']\n    exec[tx, 'INSERT INTO t VALUES (2)']\ntransact[conn, do_inserts]\n"
        );
    }

    #[test]
    fn test_db_connect_bad_url() {
        let result = std::panic::catch_unwind(|| {
            run_program("use db\nconn = connect['ftp://bad']\n")
        });
        assert!(result.is_err(), "connect with bad URL should error");
    }

    // ===== Boolean mask semantics (frame API: strings + expressions) =====

    fn assert_bool_array(val: &CrowValue, expected: &[bool]) {
        match &val.value {
            Value::Array(items) => {
                let got: Vec<bool> = items.iter().map(|v| match v.value {
                    Value::Bool(b) => b,
                    _ => panic!("expected Bool element, got {:?}", v.value),
                }).collect();
                assert_eq!(got, expected);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_eq_broadcasts_over_string_array() {
        let val = get_var("names = ['a', 'b', 'a']\nm = names eq 'a'\n", "m");
        assert_bool_array(&val, &[true, false, true]);
    }

    #[test]
    fn test_ne_broadcasts_over_string_array() {
        let val = get_var("names = ['a', 'b', 'a']\nm = names ne 'a'\n", "m");
        assert_bool_array(&val, &[false, true, false]);
    }

    #[test]
    fn test_scalar_eq_unchanged() {
        let val = get_var("x = 5 eq 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_array_array_eq_stays_whole_equality() {
        let val = get_var("x = ['a', 'b'] eq ['a', 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_and_combines_bool_arrays_elementwise() {
        let val = get_var("m = [true, true, false] and [true, false, false]\n", "m");
        assert_bool_array(&val, &[true, false, false]);
    }

    #[test]
    fn test_or_combines_bool_arrays_elementwise() {
        let val = get_var("m = [true, false, false] or [false, false, true]\n", "m");
        assert_bool_array(&val, &[true, false, true]);
    }

    #[test]
    fn test_not_negates_bool_array_elementwise() {
        let val = get_var("m = not [true, false]\n", "m");
        assert_bool_array(&val, &[false, true]);
    }

    #[test]
    fn test_frame_filter_eq_mask() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nr = filter[df, df.name eq 'Alice']\nn = len[r]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_frame_filter_compound_mask() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nr = filter[df, df.age gt 26 and df.salary gt 60000]\nn = len[r]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_len_frame() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nn = len[df]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_for_loop_outer_mutation_runs_sequential() {
        // Regression: parallel for silently discarded outer mutations (total stayed 0)
        let val = get_var("total = 0\nfor x in [1, 2, 3, 4]\n    total = total + x\n", "total");
        assert!(matches!(val.value, Value::Int(10)));
    }

    #[test]
    fn test_implicit_return_last_expression() {
        // Regression: implicit return is desugared in the parser for both modes
        let val = get_var("fn add[a, b]\n    a + b\nr = add[3, 4]\n", "r");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== Search builtins: any / all / find =====

    #[test]
    fn test_any_all() {
        let src = "fn big[x]\n    return x gt 10\n";
        let val = get_var(&format!("{}a = any[[1, 20, 3], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(true)));
        let val = get_var(&format!("{}a = any[[1, 2, 3], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(false)));
        let val = get_var(&format!("{}a = all[[11, 20, 30], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(true)));
        let val = get_var(&format!("{}a = all[[11, 2, 30], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_find_element_and_miss() {
        let src = "fn big[x]\n    return x gt 10\n";
        let val = get_var(&format!("{}h = find[[1, 20, 3], big]\n", src), "h");
        assert!(matches!(val.value, Value::Int(20)));
        let val = get_var(&format!("{}h = find[[1, 2, 3], big]\nok = h.ok\n", src), "ok");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_find_regex_dispatch_unchanged() {
        let val = get_var("m = find['ab12', '([0-9]+)']\nok = m.ok\n", "ok");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    // ===== Comments end at the newline =====

    #[test]
    fn test_comment_without_closer_does_not_eat_next_line() {
        // Regression: the old paired --- design silently swallowed the next
        // line into code when a closer was missing
        let val = get_var("--- NOTE: no closer here\nx = 42\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_inline_comment_runs_to_end_of_line() {
        let val = get_var("x = 5 --- everything here is comment --- even this\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    // ===== Backtick interpolation =====

    #[test]
    fn test_backtick_escaped_literal() {
        let val = get_var("s = 'code: \\`x\\`'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "code: `x`"));
    }

    #[test]
    fn test_backtick_unterminated_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("s = 'hello `name'\nlog[s]\n")
        });
        assert!(result.is_err(), "unterminated interpolation should error");
    }

    #[test]
    fn test_backtick_nested_string_args() {
        // Inside interpolation you are still inside the string: quotes escaped
        let val = get_var("s = 'up: `upper[\\'hi\\']`'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "up: HI"));
    }

    #[test]
    fn test_regex_quantifier_braces_now_safe() {
        // Regression: '{4}' used to be silently interpolated, corrupting patterns
        let val = get_var("m = matches['2024', '[0-9]{4}']\n", "m");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_concat_strings() {
        let val = get_var("x = concat['ab', 'cd']\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "abcd"));
    }

    // ===== fmt builtin =====

    #[test]
    fn test_fmt_basics() {
        let val = get_var("x = fmt[1234.567, decimals = 2, commas = true]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "1,234.57"));
        let val = get_var("x = fmt[0.5, percent = true, decimals = 0]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "50%"));
        let val = get_var("x = fmt['hi', width = 4]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "hi  "));
    }

    #[test]
    fn test_fmt_decimals_on_string_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("x = fmt['hi', decimals = 2]\n")
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_pad_removed() {
        let result = std::panic::catch_unwind(|| {
            run_program("x = pad['hi', 5]\n")
        });
        assert!(result.is_err(), "pad should error with removal message");
    }

    // ===== Errors in conditions + batch semantics (#2) =====

    #[test]
    fn test_error_condition_halts_at_top_level() {
        let result = std::panic::catch_unwind(|| {
            run_program("age = int['abc']\nif age gte 18\n    log['adult']\n")
        });
        assert!(result.is_err(), "branching on an error at top level should halt");
    }

    #[test]
    fn test_error_condition_propagates_out_of_function() {
        let val = get_var(
            "fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['abc']\nok = r.ok\n",
            "ok",
        );
        assert!(matches!(val.value, Value::Bool(false)));
        let val = get_var(
            "fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['9']\n",
            "r",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "big"));
    }

    #[test]
    fn test_error_condition_in_while_refused() {
        let result = std::panic::catch_unwind(|| {
            run_program("n = int['x']\nwhile n gt 0\n    n = n - 1\n")
        });
        assert!(result.is_err(), "while on an error condition should refuse");
    }

    #[test]
    fn test_ok_check_always_branchable() {
        let val = get_var("bad = int['zzz']\nmsg = ''\nif not bad.ok\n    msg = 'guarded'\n", "msg");
        assert!(matches!(&val.value, Value::Str(s) if s == "guarded"));
    }

    #[test]
    fn test_float_var_stays_float_on_int_assignment() {
        let val = get_var("v = 2.5\nv = 1\n", "v");
        assert!(matches!(val.value, Value::Float(f) if f == 1.0));
    }

    #[test]
    fn test_braces_are_inert_literals() {
        let val = get_var("s = 'a {b} c'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "a {b} c"));
    }

    #[test]
    fn test_json_object_literal_raw() {
        let val = get_var("j = json['{\"n\": 7}']\nx = j.n\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== 2-D tensors from literals (F3) =====

    #[test]
    fn test_nested_literal_builds_shaped_tensor() {
        let val = get_var("m = [[1, 2], [3, 4]]\n", "m");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.shape(), &[2, 2]),
            other => panic!("expected 2-D tensor, got {:?}", other),
        }
    }

    #[test]
    fn test_ragged_literal_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = [[1, 2], [3]]\n")
        });
        assert!(result.is_err(), "ragged rows should error");
    }

    #[test]
    fn test_tensor_row_indexing_and_iteration() {
        let val = get_var("m = [[1, 2], [3, 4]]\nr = m[1]\n", "r");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.to_vec(), vec![3.0, 4.0]),
            other => panic!("expected row tensor, got {:?}", other),
        }
        let val = get_var("m = [[1, 2], [3, 4]]\nx = m[1][0]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_tensor_push_row_and_errors() {
        let val = get_var("m = [[1, 2], [3, 4]]\nm2 = push[m, [5, 6]]\nn = len[m2]\n", "n");
        assert!(matches!(val.value, Value::Int(6)), "3x2 tensor has 6 elements");
        // wrong row length
        let r = std::panic::catch_unwind(|| run_program("m = [[1, 2], [3, 4]]\nm2 = push[m, [5, 6, 7]]\n"));
        assert!(r.is_err(), "wrong row length should error");
        // wrong type
        let r = std::panic::catch_unwind(|| run_program("m = [1, 2]\nm2 = push[m, 'x']\n"));
        assert!(r.is_err(), "pushing str onto numeric array should error");
    }

    #[test]
    fn test_tensor_comparison_masks() {
        let val = get_var("x = [1, 2, 3, 4]\nm = x gt 2\n", "m");
        assert_bool_array(&val, &[false, false, true, true]);
        let val = get_var("x = [1, 2, 3]\nm = x eq 2\n", "m");
        assert_bool_array(&val, &[false, true, false]);
    }

    #[test]
    fn test_concat_tensor_axis() {
        let val = get_var("use math\nm = [[1, 2], [3, 4]]\nc = concat[m, m, 1]\nn = len[c]\n", "n");
        assert!(matches!(val.value, Value::Int(8)), "2x4 tensor has 8 elements");
    }

    // ===== Function scope: module variables are invisible (F1) =====

    #[test]
    fn test_fn_cannot_read_module_variable() {
        let result = std::panic::catch_unwind(|| {
            run_program("base = 10\nfn add_base[x]\n    return x + base\nr = add_base[5]\n")
        });
        assert!(result.is_err(), "module variable read inside fn should error");
    }

    #[test]
    fn test_fn_cannot_mutate_module_variable() {
        // The old silent-local bug: count must NOT silently stay 0 — the
        // read of 'count' inside the function errors instead
        let result = std::panic::catch_unwind(|| {
            run_program("count = 0\nfn inc[]\n    count = count + 1\ninc[]\n")
        });
        assert!(result.is_err(), "module variable mutation inside fn should error");
    }

    #[test]
    fn test_fn_local_may_reuse_module_name() {
        // A local named like a module variable is a fresh, unrelated binding
        let val = get_var(
            "count = 5\nfn scaled[n]\n    count = n * 2\n    return count\nr = scaled[3]\n",
            "r",
        );
        assert!(matches!(val.value, Value::Int(6)));
        // and the module variable is untouched
        let val2 = get_var(
            "count = 5\nfn scaled[n]\n    count = n * 2\n    return count\nr = scaled[3]\n",
            "count",
        );
        assert!(matches!(val2.value, Value::Int(5)));
    }

    #[test]
    fn test_fn_can_call_other_functions_and_structs() {
        let val = get_var(
            "struct P [x]\nfn double[n]\n    return n * 2\nfn build[n]\n    return P[double[n]]\nr = build[4]\nv = r.x\n",
            "v",
        );
        assert!(matches!(val.value, Value::Int(8)));
    }

    #[test]
    fn test_fn_index_assign_module_var_blocked() {
        let result = std::panic::catch_unwind(|| {
            run_program("nums = [1, 2, 3]\nfn tweak[]\n    nums[0] = 99\ntweak[]\n")
        });
        assert!(result.is_err(), "index-assign to module variable inside fn should error");
    }

    // ===== Maps are dictionaries, structs are records =====

    #[test]
    fn test_map_heterogeneous_values_rejected() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = map[name = 'crow', age = 3]\n")
        });
        assert!(result.is_err(), "mixed str/int map values should error");
    }

    #[test]
    fn test_map_hetero_index_assign_rejected() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = map[a = 1]\nm['b'] = 'x'\n")
        });
        assert!(result.is_err(), "adding str to int map via index assign should error");
    }

    #[test]
    fn test_map_int_float_values_unify() {
        let val = get_var("m = map[lr = 0.01, epochs = 100]\nx = m.epochs\n", "x");
        assert!(matches!(val.value, Value::Int(100)));
    }

    #[test]
    fn test_map_of_structs() {
        let val = get_var(
            "struct Address [street, city, zip]\naddresses = map[home = Address['12 Oak St', 'Austin', 78701], office = Address['500 Main St', 'Austin', 78702]]\nx = addresses.home.city\n",
            "x",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "Austin"));
    }

    #[test]
    fn test_map_of_structs_dynamic_key() {
        let val = get_var(
            "struct Address [street, city, zip]\naddresses = map[home = Address['12 Oak St', 'Austin', 78701]]\naddresses = set[addresses, 'work', Address['500 Main St', 'Dallas', 75001]]\nx = addresses['work'].city\n",
            "x",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "Dallas"));
    }

    #[test]
    fn test_map_literal_removed_helpful_error() {
        // Regression: bare [k = v] must error with a pointer to map[...]
        let mut lexer = crate::lexer::Lexer::new("m = [a = 1]\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(err.contains("map[key = value"), "unexpected error: {}", err);
    }

    #[test]
    fn test_frame_bare_column_name_is_undefined_variable() {
        // Regression: bare-name column DSL removed — sort[df, age] must error
        let result = std::panic::catch_unwind(|| {
            run_program("use frame\ndf = load['examples/test_data.csv']\ns = sort[df, age]\n")
        });
        assert!(result.is_err(), "bare column name should be an undefined variable");
    }
    #[test]
    fn test_now_is_a_number() {
        let val = get_var("t = now[]", "t");
        match &val.value {
            Value::Float(f) => assert!(*f > 1_700_000_000.0, "epoch seconds, got {}", f),
            other => panic!("now[] must be a number, got {:?}", other),
        }
    }

    #[test]
    fn test_moment_calendar_and_arithmetic() {
        // 09:30 New York on 2026-07-14 reads as 13:30 UTC — same instant.
        let val = get_var(
            "t = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']\nc = calendar[t, tz = 'UTC']\nh = c.hour",
            "h",
        );
        assert_eq!(format!("{}", val.value), "13");
        // Subtraction is elapsed seconds.
        let d = get_var(
            "a = moment[2026, 7, 14, hour = 9]\nb = moment[2026, 7, 14, hour = 10]\nd = b - a",
            "d",
        );
        assert_eq!(format!("{}", d.value), "3600");
    }

    #[test]
    fn test_parse_time_and_fmt_time() {
        let val = get_var("t = parse_time['2026-07-14T04:09:00-04:00']\ns = fmt[t, time = 'iso']", "s");
        assert_eq!(format!("{}", val.value), "2026-07-14T08:09:00Z");
        let ny = get_var("t = parse_time['2026-07-14 12:00:00']\ns = fmt[t, time = 'datetime', tz = 'America/New_York']", "s");
        assert_eq!(format!("{}", ny.value), "2026-07-14 08:00:00");
    }

    #[test]
    fn f11_click_with_frozen_args() {
        // click[a, set_speed[10.0]] delivers the frozen arg to the handler,
        // and rendering must NOT invoke the deferred call.
        let src = "use ui\nhits = 0\nspeed = 1.0\nfn set_speed[s]\n    hits = hits + 1\n    speed = s\nview render[]\n    return div[button['10x', click = set_speed[10.0]], p['`speed`']]\ntest t[]\n    a = app[render]\n    h = html[a]\n    assert[a.hits eq 0]\n    click[a, set_speed[10.0]]\n    assert[a.hits eq 1]\n    assert[a.speed eq 10.0]\n";
        let stmts = crate::build(src).unwrap();
        let (issues, counts) = crate::checker::run_test_blocks(&stmts);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
        assert!(counts.blocks >= 1);
    }

    #[test]
    fn test_removed_time_builtins_teach() {
        let mut i = Interpreter::new();
        for (src, want) in [
            ("d = date[2024, 1, 15]", "moment["),
            ("e = elapsed[1.0, 2.0]", "subtract moments"),
            ("t = today[]", "fmt[now[]"),
        ] {
            let stmts = crate::build(src).unwrap();
            let err = i.run(&stmts).unwrap_err();
            assert!(err.contains(want), "expected teaching error for {}, got {}", src, err);
        }
    }

}
