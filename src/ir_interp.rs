//! The interpreter (docs/ir-design.md §5): walks the IR and decides
//! nothing. Every operation goes through `builtins`, `ops` and the module
//! registry, the same code the compiled mode's runtime is built from;
//! values are `CrowValue` for now, and the typed, compact values of the
//! design are the step after the Rust backend. `crow run -i file.co` runs
//! a program here; `crow test --parity` holds it to the compiled mode.

use crate::ast::{BinOp, UnaryOp};
use crate::harness::LayoutBox;
use crate::ir::*;
use crate::modules::ModuleRegistry;
use crate::value::*;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const READER_FNS: &[&str] = &["query", "read", "fetch", "recv", "input", "load", "env", "shell", "ls", "csv", "lines"];
/// Scalar and tensor names both core and math answer: scalars go to the
/// core builtin, as the AST walker routes them.
const OVERLAPPING: &[&str] = &["abs", "sqrt", "floor", "ceil", "round", "min", "max"];

enum Flow {
    Next,
    Return(CrowValue),
    Break,
    Continue,
}

/// A scope is shared until written (a parallel loop clones the walker per
/// chunk of items; the clone costs one reference per scope).
type Scope = Arc<HashMap<String, CrowValue>>;

#[derive(Clone)]
struct Frame {
    scopes: Vec<Scope>,
    fn_name: String,
}

#[derive(Clone)]
pub struct IrInterp {
    fns: HashMap<String, Arc<Fn>>,
    derives: HashMap<String, Arc<Derive>>,
    structs: HashMap<String, Arc<crate::ir::StructDef>>,
    registry: ModuleRegistry,
    frames: Vec<Frame>,
    line: usize,
    file: String,
    /// The value of the last expression statement, what a collecting loop yields.
    last_value: Option<CrowValue>,
    /// Inside a view being rendered: bound inputs render, elements are markup.
    in_view: usize,
    /// One per render, for derivations that read the clock.
    tick: u64,
    engine_loaded: bool,
    /// Set by a caller that gives up on a run (the checker's timeouts):
    /// loops and calls stop at their next step.
    cancel: Option<Arc<AtomicBool>>,
    /// Every fact's name: what a harness session holds.
    pub(crate) fact_names: Vec<String>,
    /// Every fact's declared type: how a session's facts come back from
    /// the JSON they cross the loop as.
    pub(crate) fact_types: HashMap<String, Ty>,
    /// The harness's sessions, each with its own facts, as the server gives
    /// each page. The active session's facts are the globals; the others
    /// wait here until a verb acts on them.
    sessions: HashMap<u64, HashMap<String, CrowValue>>,
    /// The session the test acted on last, whose facts the globals hold.
    active_session: Option<u64>,
    /// The facts as the first session found them: where every later
    /// session starts, as a new page starts from the declared values.
    session_start: HashMap<String, CrowValue>,
    next_session: u64,
    /// Each harness session's clock: it starts when the test's first
    /// session opens and moves only when the test moves it.
    session_clock: HashMap<u64, f64>,
    /// Each harness session's notice and the moment it landed (0: no session).
    pub(crate) session_notice: HashMap<u64, (String, f64)>,
    /// Each session's signed-in reader (empty: nobody signed in).
    pub(crate) session_reader: HashMap<u64, std::collections::BTreeMap<String, CrowValue>>,
    /// The sign-in or sign-out the last committed handler made — the served
    /// page hands it to the runtime, which keeps the cookie.
    pub(crate) session_sign: Option<Option<std::collections::BTreeMap<String, String>>>,
    /// The page each paged table of the served session shows (by its place
    /// among the render's paged tables); a test sees the first pages.
    pub(crate) session_pages: HashMap<usize, usize>,
    /// Paged tables drawn so far in the render under way.
    paged_tables: usize,
    /// The files each session's handlers handed its reader, in order.
    pub(crate) session_downloads: HashMap<u64, Vec<(String, Vec<u8>)>>,
    /// While the harness runs a handler: the effects held to its commit.
    held: Option<Vec<HeldEffect>>,
    /// Whether the view being rendered read `notice` (a page that does not
    /// gets the runtime's toast, as the served page does).
    pub(crate) notice_read: bool,
    /// Whether a render adds the harness's own toast for a page that shows
    /// no notice — the test's page; a served page gets the runtime's.
    pub(crate) harness_toast: bool,
    /// What `page[…]` asked for: the path, the view, the url facts.
    pub(crate) page_config: Option<(String, String, Vec<String>)>,
    /// What `serve[…]` asked for, as its lowered arguments.
    pub(crate) serve_config: Option<Vec<CrowValue>>,
    /// Why the last transaction stopped, when it did.
    pub(crate) last_stop: Option<String>,
    /// A collecting loop's body handed the function its value (a return, an
    /// error handed back from a condition): what the function returns.
    returning: Option<CrowValue>,
    /// `crow test --compiled`: the compiled program this run drives,
    /// started at its first `app[]`, and each harness session's session
    /// there (compiled_harness.rs).
    remote: Option<Arc<crate::compiled_harness::RemoteApp>>,
    remote_sessions: HashMap<u64, u64>,
    /// Whether the effect being run has its answer used (`Effect.answered`):
    /// `sent = mail[…]` sends at once, `mail[…]` waits for the commit.
    answering: bool,
    /// A test block's run: a message is recorded, never sent.
    pub(crate) testing: bool,
    /// The messages each session sent, as a test reads them (`mails[a]`),
    /// each with its mailer's place in `MAILERS`.
    pub(crate) session_mails: HashMap<u64, Vec<(crow_web::live::SentMail, usize)>>,
}

/// An effect a handler held to its commit (core-design.md §1.2).
#[derive(Clone)]
enum HeldEffect {
    Notice(String, f64),
    Later(CrowValue, Vec<CrowValue>),
    /// sign_in[record] (Some) or sign_out[] (None)
    Sign(Option<std::collections::BTreeMap<String, CrowValue>>),
    /// download[value, name]: the file's name and bytes
    Download(String, Vec<u8>),
    /// mail[mailer, to, subject, body]: the message, and its mailer's place
    /// in `MAILERS`
    Mail(crow_web::live::SentMail, usize),
}

/// The mailers `smtp[…]` made. A mailer value names its place here, so its
/// password is never part of a value a program can print.
static MAILERS: std::sync::LazyLock<std::sync::Mutex<Vec<crow_web::mail::CrowMailer>>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// A frame column's cell as read: a missing one is its column's
/// placeholder, as the runtime's getters give it and the IR types it.
/// What an unwrap passes on: the value itself (a failed one is the
/// caller's to propagate), so a read in place sees through it.
fn unwrapped(e: &Expr) -> &Expr {
    match e {
        Expr::Convert { how: Convert::Unwrap, expr, .. } => expr,
        other => other,
    }
}

fn placeholder(cell: CrowValue, ty: &Ty) -> CrowValue {
    match (cell.ok || cell.kind != "missing", ty) {
        (true, _) => cell,
        (false, Ty::Float) => CrowValue::ok(Value::Float(f64::NAN)),
        (false, Ty::Int) => CrowValue::ok(Value::Int(0)),
        (false, Ty::Bool) => CrowValue::ok(Value::Bool(false)),
        (false, Ty::Str) => CrowValue::ok(Value::Str("nan".to_string())),
        // a column nothing typed: its missing cell is JSON's null, an error
        // value of reason 'null', as `json[]` reads one
        (false, Ty::Json) => {
            let mut null = CrowValue::error_with_kind("null", "parse", "");
            null.trace.clear();
            null
        }
        (false, _) => cell,
    }
}

/// How a mailer sends one message, from wherever it is asked.
pub(crate) fn mail_sender(mailer: usize, mail: &crow_web::live::SentMail) -> crow_web::live::MailSender {
    let m = mail.clone();
    std::sync::Arc::new(move || {
        let mailer = MAILERS.lock().unwrap_or_else(|p| p.into_inner()).get(mailer).cloned().ok_or("no such mailer")?;
        mailer.send_blocking(&m.to, &m.subject, &m.body)
    })
}

/// How a stop travels out of a handler, through every helper, to the
/// harness verb that ran it: an error the walker never shows.
const STOP: &str = "\u{1}stop\u{1}";

/// How a return from inside a collecting loop, an expression, travels to
/// the function it leaves; the value waits in `returning`.
const RETURN: &str = "\u{1}return\u{1}";

/// The pipeline every runner shares: the front end's lowering, inference,
/// then the lowering to the IR.
pub fn prepare(stmts: &[crate::ast::Stmt], name: &str) -> Result<(Program, crate::type_infer::TypeContext), String> {
    let stmts = crate::lower::lower_program(stmts)?;
    let mut ctx = crate::type_infer::TypeContext::new();
    ctx.describe_queries = false;
    ctx.infer_program(&stmts);
    // the gate every pipeline passes: the lowering refuses what `crow run`
    // refuses
    let program = crate::lower_ir::lower(&stmts, &mut ctx, name)?;
    Ok((program, ctx))
}

/// A script run for its variables: the crowfile.
pub fn run_script(stmts: &[crate::ast::Stmt], name: &str) -> Result<IrInterp, String> {
    let (program, ctx) = prepare(stmts, name)?;
    let mut it = IrInterp::new(&ctx.registry, name);
    it.load(&program);
    it.run_main(&program)?;
    Ok(it)
}

/// A program fed a fragment at a time, each lowered against what came
/// before: the REPL. A fragment's expression statement is its value.
pub struct Session {
    it: IrInterp,
    ctx: crate::type_infer::TypeContext,
}

impl Session {
    pub fn new() -> Self {
        let ctx = crate::type_infer::TypeContext::new();
        Session { it: IrInterp::new(&ctx.registry, "repl"), ctx }
    }

    pub fn feed(&mut self, stmts: &[crate::ast::Stmt]) -> Result<Option<CrowValue>, String> {
        let stmts = crate::lower::lower_program(stmts)?;
        self.ctx.infer_program(&stmts);
        let known: HashSet<String> = self.it.frames[0].scopes[0].keys().cloned().collect();
        let program = crate::lower_ir::lower_with_known(&stmts, &mut self.ctx, "repl", &known)?;
        // a `use` in the fragment loaded its module into the context's registry
        self.it.registry = self.ctx.registry.clone();
        self.it.load(&program);
        self.it.last_value = None;
        self.it.run_main(&program)?;
        let value = self.it.last().cloned();
        self.it.last_value = None;
        Ok(value)
    }
}

/// Run a program: its facts get their defaults, then its main block runs.
pub fn run(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<(), String> {
    let mut it = IrInterp::new(registry, source_file);
    it.load(program);
    it.run_main(program)
}

/// Run a program that serves pages: its facts get their defaults and its
/// main runs, recording what `page` and `serve` ask for; the interpreter
/// that ran it is the template every session's render and handler copy.
pub fn run_to_serve(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<IrInterp, String> {
    let mut it = IrInterp::new(registry, source_file);
    it.load(program);
    it.run_main(program)?;
    Ok(it)
}

/// A test block's one input, run as the checker runs it: the program's
/// declarations first (a failure there is named as such), then the block
/// with its inputs bound. The block is lowered as the test it is, so its
/// statements keep their order — lowered at the top of a ui program they
/// would become facts, evaluated before main.
/// The moment a rehearsal's clock stands at: 2026-01-01 00:00 UTC, the
/// same on every run, so what a scenario stamps is the same.
pub const REHEARSAL_EPOCH: f64 = 1767225600.0;

/// A named test run as a scenario (B3): in a rehearsal, with the clock at
/// `REHEARSAL_EPOCH` until the test moves it. Whether it ran to its end
/// (or why it stopped), each fact whose value it changed, as
/// `name = value`, sorted, and the mail its sessions would have sent.
pub fn run_scenario(decls: &[crate::ast::Stmt], label: &str, body: &[crate::ast::Stmt]) -> (Result<(), String>, Vec<String>, Vec<String>) {
    let mut all = decls.to_vec();
    all.push(crate::ast::Stmt::TestBlock { name: Some(label.to_string()), params: vec![], body: body.to_vec() });
    let (program, ctx) = match prepare(&all, "test") {
        Ok(p) => p,
        Err(e) => return (Err(e), vec![], vec![]),
    };
    let mut it = IrInterp::new(&ctx.registry, "test");
    it.testing = true;
    it.load(&program);
    crow_runtime::moments::hold(Some(REHEARSAL_EPOCH));
    if let Err(e) = it.run_main(&program) {
        crow_runtime::moments::hold(None);
        return (Err(format!("a declaration failed before the test ran: {}", e)), vec![], vec![]);
    }
    let before = it.fact_values();
    let Some(test) = program.parts.iter().flat_map(|p| p.tests.iter()).last() else {
        crow_runtime::moments::hold(None);
        return (Err("lowering: the test block is missing".into()), vec![], vec![]);
    };
    it.frames[0].scopes.push(Arc::new(HashMap::new()));
    let result = it.exec_stmts(&test.body).map(|_| ()).map_err(|e| it.with_line(e));
    crow_runtime::moments::hold(None);
    let after = it.fact_values();
    let mut changed: Vec<String> = after
        .iter()
        .filter(|(n, v)| before.get(*n).map_or(true, |b| format!("{}", b.value) != format!("{}", v.value) || b.ok != v.ok))
        .map(|(n, v)| {
            let text = if v.ok { format!("{}", v.value) } else { format!("error: {}", v.reason) };
            let text: String = if text.chars().count() > 160 { format!("{}…", text.chars().take(160).collect::<String>()) } else { text };
            format!("{} = {}", n, text)
        })
        .collect();
    changed.sort();
    let mut ids: Vec<&u64> = it.session_mails.keys().collect();
    ids.sort();
    let mailed: Vec<String> = ids
        .into_iter()
        .flat_map(|id| it.session_mails[id].iter().map(|(m, _)| format!("to {}: {}", m.to, m.subject)))
        .collect();
    (result, changed, mailed)
}

pub fn run_test(decls: &[crate::ast::Stmt], label: &str, params: &[(String, CrowValue)], body: &[crate::ast::Stmt]) -> Result<(), String> {
    let mut all = decls.to_vec();
    // named, so the gate reads it as the block it was in its file (an
    // unnamed block is one attached to the function before it)
    all.push(crate::ast::Stmt::TestBlock {
        name: Some(label.to_string()),
        params: params.iter().map(|(p, _)| p.clone()).collect(),
        body: body.to_vec(),
    });
    let (program, ctx) = prepare(&all, "test")?;
    // CROW_IR_SHOW=1 prints the lowered test program, as `crow build --ir` does for a file
    if std::env::var("CROW_IR_SHOW").is_ok() {
        eprintln!("{}", crate::ir::print(&program));
    }
    let mut it = IrInterp::new(&ctx.registry, "test");
    // a test records what the program would send, and sends nothing
    it.testing = true;
    it.load(&program);
    it.run_main(&program).map_err(|e| format!("a declaration failed before the test ran: {}", e))?;
    let test = program.parts.iter().flat_map(|p| p.tests.iter()).last().ok_or("lowering: the test block is missing")?;
    // the block runs at the program's top level, in a scope of its own: it
    // sees the declarations' variables as a caller would, and its inputs
    let mut scope = HashMap::new();
    for (p, v) in params {
        scope.insert(p.clone(), v.clone());
    }
    it.frames[0].scopes.push(Arc::new(scope));
    let result = it.exec_stmts(&test.body);
    // the test's clock ends with the test
    crow_runtime::moments::hold(None);
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(it.with_line(e)),
    }
}

impl IrInterp {
    pub fn new(registry: &ModuleRegistry, source_file: &str) -> Self {
        let file = std::path::Path::new(source_file)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| source_file.to_string());
        IrInterp {
            fns: HashMap::new(),
            derives: HashMap::new(),
            structs: HashMap::new(),
            registry: registry.clone(),
            frames: vec![Frame { scopes: vec![Arc::new(HashMap::new())], fn_name: String::new() }],
            line: 0,
            file,
            last_value: None,
            in_view: 0,
            tick: 0,
            engine_loaded: false,
            cancel: None,
            fact_names: Vec::new(),
            fact_types: HashMap::new(),
            sessions: HashMap::new(),
            active_session: None,
            session_start: HashMap::new(),
            next_session: 0,
            session_clock: HashMap::new(),
            session_notice: HashMap::new(),
            session_reader: HashMap::new(),
            session_sign: None,
            session_pages: HashMap::new(),
            paged_tables: 0,
            session_downloads: HashMap::new(),
            remote: None,
            remote_sessions: HashMap::new(),
            answering: true,
            testing: false,
            session_mails: HashMap::new(),
            held: None,
            notice_read: false,
            page_config: None,
            serve_config: None,
            last_stop: None,
            returning: None,
            harness_toast: true,
        }
    }

    /// The facts' defaults, then main, part by part.
    pub fn run_main(&mut self, program: &Program) -> Result<(), String> {
        for part in &program.parts {
            // the part's resources: gpu[n] turns the kernels' device on,
            // nodes[n] runs as the one rank of a cluster, cpu[n] bounds the
            // parallel loops
            if part.gpu.is_some() {
                crow_runtime::gpu::set_gpu_enabled(true);
            }
            if let Some(n) = part.nodes {
                if n >= 1 {
                    crow_runtime::distributed::init(1);
                }
            }
            if let Some(n) = part.cpu {
                if n >= 1 {
                    let _ = rayon::ThreadPoolBuilder::new().num_threads(n as usize).build_global();
                }
            }
            for fact in &part.facts {
                let v = self.eval(&fact.default).map_err(|e| self.with_line(e))?;
                self.set_fact(&fact.name, v);
                if !self.fact_names.contains(&fact.name) {
                    self.fact_names.push(fact.name.clone());
                }
                self.fact_types.insert(fact.name.clone(), fact.ty.clone());
            }
            if let Err(e) = self.exec_stmts(&part.main) {
                return Err(self.with_line(e));
            }
        }
        Ok(())
    }

    pub fn load(&mut self, program: &Program) {
        for part in &program.parts {
            for f in &part.fns {
                self.fns.insert(f.name.clone(), Arc::new(f.clone()));
            }
            for d in &part.derives {
                self.derives.insert(d.name.clone(), Arc::new(d.clone()));
            }
            for s in &part.structs {
                self.structs.insert(s.name.clone(), Arc::new(s.clone()));
            }
        }
    }

    /// A struct's fields and their types, as the program declared it.
    pub(crate) fn struct_def(&self, name: &str) -> Option<Arc<crate::ir::StructDef>> {
        self.structs.get(name).cloned()
    }

    /// A top-level variable or fact, as the crowfile reader and the tools ask.
    pub fn var(&self, name: &str) -> Option<&CrowValue> {
        self.frames[0].scopes[0].get(name)
    }

    /// The value of the last expression statement run, as the REPL shows it.
    pub fn last(&self) -> Option<&CrowValue> {
        self.last_value.as_ref()
    }

    /// A function called by name with values, as the checker probes one.
    pub fn call(&mut self, name: &str, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        self.call_user(name, args)
    }

    pub fn set_cancel_flag(&mut self, flag: Arc<AtomicBool>) {
        self.cancel = Some(flag);
    }

    fn check_cancel(&self) -> Result<(), String> {
        if self.cancel.as_ref().map_or(false, |f| f.load(Ordering::Relaxed)) {
            return Err("execution cancelled".to_string());
        }
        Ok(())
    }

    pub(crate) fn set_fact(&mut self, name: &str, v: CrowValue) {
        Arc::make_mut(&mut self.frames[0].scopes[0]).insert(name.to_string(), v);
    }

    /// The facts as the globals hold them now.
    fn fact_values(&self) -> HashMap<String, CrowValue> {
        self.fact_names.iter()
            .filter_map(|n| self.frames[0].scopes[0].get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }

    /// Make a harness session the one the globals hold: the session that
    /// held them keeps its facts, and this one's come back.
    fn activate(&mut self, id: u64) {
        if self.active_session == Some(id) {
            return;
        }
        let Some(facts) = self.sessions.remove(&id) else { return };
        if let Some(prev) = self.active_session {
            let held = self.fact_values();
            self.sessions.insert(prev, held);
        }
        for (name, v) in facts {
            self.set_fact(&name, v);
        }
        self.active_session = Some(id);
        crow_runtime::moments::hold(self.session_clock.get(&id).copied());
    }

    /// Lay out an HTML fragment as the harness does — the same engine, the
    /// same conversion — and return every element's box. The wireframes in
    /// the app's book come from here with nothing rendered from state.
    pub fn layout_html(&mut self, html: &str, viewport: f64, wide: bool) -> Result<Vec<LayoutBox>, String> {
        self.ensure_engine()?;
        let tree = crate::harness::harness_parse_html(html);
        let root = crate::harness::harness_to_lnode(&tree);
        let boxes = self.call_user("crow_layout", vec![root, CrowValue::ok(Value::Float(viewport)), CrowValue::ok(Value::Bool(wide))])?;
        let list = match &boxes.value {
            Value::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for b in list {
            if let Value::StructInstance(si) = &b.value {
                let s = |k: &str| si.fields.get(k).map(|v| format!("{}", v.value)).unwrap_or_default();
                let f = |k: &str| {
                    si.fields.get(k).and_then(|v| match &v.value {
                        Value::Float(x) => Some(*x),
                        Value::Int(n) => Some(*n as f64),
                        _ => None,
                    }).unwrap_or(0.0)
                };
                out.push(LayoutBox {
                    tag: s("tag"),
                    class: s("class"),
                    path: s("path"),
                    x: f("x"),
                    y: f("y"),
                    width: f("width"),
                    height: f("height"),
                    overflow: matches!(si.fields.get("overflow").map(|v| &v.value), Some(Value::Bool(true))),
                });
            }
        }
        Ok(out)
    }

    /// The layout engine, Crow code lowered through the same pipeline and
    /// added to this walker's functions on first use by box, fits or style.
    fn ensure_engine(&mut self) -> Result<(), String> {
        if self.engine_loaded {
            return Ok(());
        }
        let stmts = crate::build(crate::harness::LAYOUT_ENGINE_SRC).map_err(|e| format!("layout engine: {}", e))?;
        let stmts = crate::lower::lower_program(&stmts).map_err(|e| format!("layout engine: {}", e))?;
        let mut ctx = crate::type_infer::TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "layout").map_err(|e| format!("layout engine: {}", e))?;
        self.load(&program);
        self.engine_loaded = true;
        Ok(())
    }

    fn with_line(&self, e: String) -> String {
        if self.line > 0 && !e.starts_with("line ") {
            format!("line {}: {}", self.line, e)
        } else {
            e
        }
    }

    fn trace_entry(&self) -> String {
        let f = &self.frames.last().unwrap().fn_name;
        if f.is_empty() {
            format!("{}:{}", self.file, self.line)
        } else {
            format!("{}.{}:{}", self.file, f, self.line)
        }
    }

    fn in_function(&self) -> bool {
        self.frames.len() > 1
    }

    // ── scopes ──────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.frames.last_mut().unwrap().scopes.push(Arc::new(HashMap::new()));
    }

    fn pop_scope(&mut self) {
        self.frames.last_mut().unwrap().scopes.pop();
    }

    fn lookup(&self, name: &str) -> Option<&CrowValue> {
        let frame = self.frames.last().unwrap();
        for scope in frame.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn slot(&mut self, name: &str) -> Option<&mut CrowValue> {
        let frame = self.frames.last_mut().unwrap();
        for scope in frame.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                return Arc::make_mut(scope).get_mut(name);
            }
        }
        None
    }

    fn bind(&mut self, name: &str, v: CrowValue) {
        Arc::make_mut(self.frames.last_mut().unwrap().scopes.last_mut().unwrap()).insert(name.to_string(), v);
    }

    fn assign(&mut self, name: &str, v: CrowValue) {
        if let Some(s) = self.slot(name) {
            *s = v;
        } else {
            self.bind(name, v);
        }
    }

    fn fact(&self, name: &str) -> Result<CrowValue, String> {
        self.frames[0].scopes[0].get(name).cloned().ok_or_else(|| format!("undefined variable '{}'", name))
    }

    // ── statements ──────────────────────────────────────────────────

    fn exec_stmts(&mut self, b: &[Stmt]) -> Result<Flow, String> {
        for s in b {
            match self.exec_stmt(s)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn exec_block(&mut self, b: &[Stmt]) -> Result<Flow, String> {
        self.push_scope();
        let r = self.exec_stmts(b);
        self.pop_scope();
        r
    }

    /// An error where a decision is needed, or assigned to a variable that
    /// holds no error: a function hands it to its caller, the top level
    /// halts with the reason (spec §10).
    fn refuse(&mut self, err: CrowValue, at: &Decision) -> Result<Flow, String> {
        let name = self.frames.last().map(|f| f.fn_name.clone()).unwrap_or_default();
        // a handler, or a helper acting for it, meets an error at a decision:
        // the whole click stops and nothing it did lands (core-design.md §2)
        if self.held.is_some() {
            let acting = self.fns.get(&name).map_or(false, |f| matches!(f.kind, FnKind::Handler | FnKind::Helper));
            if acting {
                return Err(format!("{}{}\u{1}{}\u{1}{}", STOP, name, err.reason, err.kind));
            }
        }
        // a function whose value cannot fail has no way to hand the error
        // back: the program halts with the reason, as the top level does
        let hands_back = self.in_function()
            && self.fns.get(&name).map_or(true, |f| f.kind == FnKind::View || matches!(f.ret, Ty::Result(_) | Ty::Unit | Ty::Unknown));
        if hands_back {
            let mut e = err;
            e.append_trace(&self.trace_entry());
            Ok(Flow::Return(e))
        } else {
            Err(format!("{} — {}", at.refusal(), err.reason))
        }
    }

    fn exec_stmt(&mut self, s: &Stmt) -> Result<Flow, String> {
        match s {
            Stmt::Line(n) => {
                self.line = *n;
                Ok(Flow::Next)
            }
            Stmt::Let { var, expr, .. } => {
                let v = self.eval(expr)?;
                self.bind(var, v);
                Ok(Flow::Next)
            }
            Stmt::Set { var, expr } => {
                if self.assign_in_place(var, expr)? {
                    return Ok(Flow::Next);
                }
                let v = self.eval(expr)?;
                self.assign(var, v);
                Ok(Flow::Next)
            }
            Stmt::Store { fact, expr } => {
                let v = self.eval(expr)?;
                // a fact never holds an error: the handler stops instead,
                // and the fact keeps its last value (core-design.md §2.3)
                if v.is_error() && self.held.is_some() {
                    if let Err(e) = self.refuse(v.clone(), &Decision::Branch) {
                        return Err(e);
                    }
                }
                self.set_fact(fact, v);
                Ok(Flow::Next)
            }
            Stmt::SetPath { var, path, expr } => {
                let new = self.eval(expr)?;
                let mut idx = Vec::new();
                for p in path {
                    if let PathStep::Index(i) = p {
                        idx.push(self.eval(i)?);
                    }
                }
                let root = self
                    .slot(var)
                    .ok_or_else(|| format!("undefined variable '{}'", var))?;
                set_path(&mut root.value, path, &idx, new)?;
                Ok(Flow::Next)
            }
            Stmt::If { cond, then, els } => {
                let c = self.eval(cond)?;
                if c.is_error() {
                    return self.refuse(c, &Decision::Branch);
                }
                if crate::ops::is_truthy(&c) {
                    self.exec_block(then)
                } else if let Some(e) = els {
                    self.exec_block(e)
                } else {
                    Ok(Flow::Next)
                }
            }
            Stmt::While { cond, body } => {
                loop {
                    self.check_cancel()?;
                    let c = self.eval(cond)?;
                    if c.is_error() {
                        return self.refuse(c, &Decision::Branch);
                    }
                    if !crate::ops::is_truthy(&c) {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::For { var, iter, body, parallel } => {
                let it = self.eval(iter)?;
                // a list that failed is refused like a condition (the
                // lowering refuses a typed one first; this is one it could
                // not see)
                if it.is_error() {
                    return self.refuse(it, &Decision::Loop);
                }
                let items = crate::ops::iter_items(&it)?;
                if *parallel && items.len() > 1 {
                    self.run_parallel(&items, var, body, false)?;
                    return Ok(Flow::Next);
                }
                for item in items {
                    self.check_cancel()?;
                    self.push_scope();
                    self.bind(var, item);
                    let r = self.exec_stmts(body);
                    self.pop_scope();
                    match r? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::Return(None) => Ok(Flow::Return(CrowValue::done())),
            Stmt::Return(Some(e)) => {
                let v = self.eval(e)?;
                Ok(Flow::Return(v))
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Do(e) => {
                let v = self.eval(e)?;
                self.last_value = Some(v);
                Ok(Flow::Next)
            }
            Stmt::Propagate { values, at } => {
                for v in values {
                    let val = self.lookup(v).cloned().ok_or_else(|| format!("undefined variable '{}'", v))?;
                    if val.is_error() {
                        return self.refuse(val, at);
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::Stop { value } => {
                let val = self.lookup(value).cloned().ok_or_else(|| format!("undefined variable '{}'", value))?;
                Err(format!("stopped: {}", val.reason))
            }
        }
    }

    // ── expressions ─────────────────────────────────────────────────

    fn eval(&mut self, e: &Expr) -> Result<CrowValue, String> {
        Ok(match e {
            Expr::Const(c, _) => match c {
                Const::Int(n) => CrowValue::ok(Value::Int(*n)),
                Const::Float(f) => CrowValue::ok(Value::Float(*f)),
                Const::Bool(b) => CrowValue::ok(Value::Bool(*b)),
                Const::Str(s) => CrowValue::ok(Value::Str(s.clone())),
                Const::Unit => CrowValue::done(),
            },
            Expr::Local(v, _) => self.lookup(v).cloned().ok_or_else(|| format!("undefined variable '{}'", v))?,
            Expr::Move(v, _) => {
                let slot = self.slot(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                std::mem::replace(slot, CrowValue::done())
            }
            Expr::Load(f, _) => self.fact(f)?,
            Expr::Derived(d, _) => self.derived(d)?,
            Expr::Constant(name, _) => {
                let bare = name.rsplit('.').next().unwrap_or(name);
                self.registry.get_constant(bare).ok_or_else(|| format!("unknown constant '{}'", name))?
            }
            Expr::FnRef(name, _) => CrowValue::ok(Value::Fn(FnValue { name: name.clone() })),
            Expr::Deferred { name, args } => {
                let mut items = vec![self.eval(&Expr::FnRef(name.clone(), Ty::Unknown))?];
                items.extend(self.eval_args(args)?);
                CrowValue::ok(Value::Array(items))
            }
            Expr::Tree { element, attrs, children } => {
                let mut map = std::collections::BTreeMap::new();
                for (k, v) in attrs {
                    map.insert(k.clone(), self.eval(v)?);
                }
                // a paged table: which one it is in this render, and its page
                if element == "table" && map.contains_key("page") {
                    let index = self.paged_tables;
                    self.paged_tables += 1;
                    let at = self.session_pages.get(&index).copied().unwrap_or(0);
                    map.insert("__page_index".to_string(), CrowValue::ok(Value::Int(index as i64)));
                    map.insert("__page_at".to_string(), CrowValue::ok(Value::Int(at as i64)));
                }
                let mut vals = vec![CrowValue::ok(Value::Map(map))];
                for c in children {
                    vals.push(self.eval(c)?);
                }
                CrowValue::markup(crate::ops::render_element(element, &vals)?)
            }
            // assert[a op b]: a failed comparison reports both operands
            Expr::Call { target: Target::Runtime(name), args, .. }
                if name == "assert" && args.len() == 1 && matches!(&args[0], Expr::Binary { op, .. } if matches!(op.as_str(), "eq" | "ne" | "gt" | "lt" | "gte" | "lte")) =>
            {
                let (op, l, r) = match &args[0] {
                    Expr::Binary { op, l, r, .. } => (op, l, r),
                    _ => unreachable!(),
                };
                let full = self.eval(&args[0])?;
                if !full.ok {
                    return Err(format!("cannot branch on an error value — {}", full.reason));
                }
                if matches!(full.value, Value::Bool(false)) {
                    let lv = self.eval(l)?;
                    let rv = self.eval(r)?;
                    if crate::rehearsal::unheld(format!("{}: {} {} {}", self.trace_entry(), lv.value, op, rv.value)) {
                        return Ok(CrowValue::done());
                    }
                    return Err(format!("assert failed: {}\n    left:  {}\n    right: {}", op, lv.value, rv.value));
                }
                CrowValue::done()
            }
            Expr::Call { target: Target::Runtime(name), args, .. }
                if self.in_view > 0
                    && (matches!(name.as_str(), "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                        || (name == "select" && args.len() == 2 && !matches!(args.first(), Some(Expr::MapLit(..)))))
                    && args.len() == match name.as_str() { "field" => 4, "slider" => 5, _ => 2 } =>
            {
                // a bound input: the variable's name is the binding's key
                let key = match args.get(1) {
                    Some(Expr::Load(n, _)) | Some(Expr::Local(n, _)) | Some(Expr::Move(n, _)) => n.clone(),
                    Some(Expr::Field { name, .. }) => name.clone(),
                    _ => "unknown".to_string(),
                };
                let vals = self.eval_args(args)?;
                // a control renders what it is given: a failed argument is
                // its reason in place of the control, as for a view
                if let Some(bad) = vals.iter().find(|v| v.is_error()) {
                    return Ok(CrowValue::markup(format!("error: {}", crate::harness::html_escape_text(&bad.reason))));
                }
                CrowValue::markup(crate::ops::render_input(name, &key, &vals)?)
            }
            // the length of a local, read where it lies: a list is not
            // copied to be measured (a loop over a growing list was quadratic)
            Expr::Call { target: Target::Runtime(name), args, .. }
                if name == "len" && args.len() == 1 && matches!(unwrapped(&args[0]), Expr::Local(..)) && !self.registry.has_function("len") =>
            {
                let Expr::Local(v, _) = unwrapped(&args[0]) else { unreachable!() };
                let trace = self.trace_entry();
                let b = self.lookup(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                if b.is_error() {
                    return Ok(b.clone());
                }
                crate::builtins::call("len", std::slice::from_ref(b), &trace).unwrap_or_else(|| Err("len[] is not a builtin".into()))?
            }
            // ... and of a local's field: a struct's list, a frame's column
            Expr::Call { target: Target::Runtime(name), args, .. }
                if name == "len" && args.len() == 1 && !self.registry.has_function("len") && matches!(unwrapped(&args[0]), Expr::Field { base, name, .. } if self.field_in_place(base, name)) =>
            {
                let Expr::Field { base: owner, name: field, .. } = unwrapped(&args[0]) else { unreachable!() };
                let trace = self.trace_entry();
                let o = self.place(owner).ok_or_else(|| format!("no field '{}'", field))?;
                let item = match &o.value {
                    Value::Frame(f) => return Ok(CrowValue::ok(Value::Int(f.nrows() as i64))),
                    Value::StructInstance(si) => &si.fields[field.as_str()],
                    Value::Map(m) => &m[field.as_str()],
                    _ => unreachable!(),
                };
                crate::builtins::call("len", std::slice::from_ref(item), &trace).unwrap_or_else(|| Err("len[] is not a builtin".into()))?
            }
            Expr::Call { target, args, propagates, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                match target {
                    Target::Runtime(path) => self.runtime_call(path, vals)?,
                    Target::User(name) => self.call_user(name, vals)?,
                    Target::Module(m, f) => self.module_call(m, f, vals)?,
                }
            }
            Expr::Effect { name, args, propagates, answered, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                let asked = std::mem::replace(&mut self.answering, *answered);
                let r = self.effect(name, vals);
                self.answering = asked;
                r?
            }
            Expr::Convert { how, expr, ty } => {
                let v = self.eval(expr)?;
                match how {
                    Convert::IntToFloat => match &v.value {
                        Value::Int(n) if v.ok => CrowValue::ok(Value::Float(*n as f64)),
                        _ => v,
                    },
                    // a tensor's numbers as a list's items, ints where the
                    // list holds ints
                    Convert::ToList => match &v.value {
                        Value::NumArray(t) if v.ok => {
                            let ints = matches!(ty, Ty::List(e) if **e == Ty::Int);
                            CrowValue::ok(Value::Array(
                                t.to_vec().into_iter().map(|x| CrowValue::ok(if ints { Value::Int(x as i64) } else { Value::Float(x) })).collect(),
                            ))
                        }
                        _ => v,
                    },
                    // a list's numbers as a tensor of one axis
                    Convert::ToTensor => match &v.value {
                        Value::Array(items) if v.ok => {
                            let mut data = Vec::with_capacity(items.len());
                            for item in items.iter() {
                                match item.value {
                                    Value::Int(n) => data.push(n as f64),
                                    Value::Float(x) => data.push(x),
                                    _ => return Err(format!("a list of {} is not a list of numbers", item.value.type_name())),
                                }
                            }
                            let n = data.len();
                            CrowValue::ok(Value::NumArray(crow_runtime::tensor::CrowTensor::from_vec(data, vec![n])))
                        }
                        _ => v,
                    },
                    // a CrowValue is its own payload; an error reached here is
                    // the caller's to propagate
                    Convert::Unwrap | Convert::Known | Convert::ToStr => v,
                }
            }
            Expr::Binary { op, l, r, .. } => {
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                crate::ops::binop(&lv, &binop_of(op)?, &rv, &self.trace_entry())?
            }
            Expr::Unary { op, expr, .. } => {
                let v = self.eval(expr)?;
                let o = match op.as_str() {
                    "neg" => UnaryOp::Neg,
                    "not" => UnaryOp::Not,
                    other => return Err(format!("unknown unary operator '{}'", other)),
                };
                crate::ops::unaryop(&o, &v)?
            }
            Expr::Index { base, indices, ty } => {
                // a local list, map, text or frame is indexed where it lies:
                // reading it copied it first, and a loop over a growing list
                // was quadratic (its indices cannot touch it, so they go first)
                let place = unwrapped(base);
                if let Expr::Local(v, _) = place {
                    // (not when an index moves the list itself — its last read
                    // there, `r[len(move r) - 1]`; a plain read is fine)
                    let mut mentioned = false;
                    for i in indices {
                        crate::ir::visit_expr(i, &mut |e| mentioned |= matches!(e, Expr::Move(n, _) if n == v));
                    }
                    let in_place = !mentioned && matches!(self.lookup(v), Some(b) if b.ok && matches!(b.value, Value::Array(_) | Value::NumArray(_) | Value::Map(_) | Value::Str(_) | Value::Frame(_)));
                    if in_place {
                        let mut idx = Vec::with_capacity(indices.len());
                        for i in indices {
                            idx.push(self.eval(i)?);
                        }
                        let b = self.lookup(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                        let cell = crate::ops::index(b, idx)?;
                        return Ok(if matches!(base.ty(), Ty::Column(_)) { placeholder(cell, ty) } else { cell });
                    }
                }
                // a local's field indexed where it lies too — a frame's column
                // (`df.price[i]`), a struct's list (`bars.c[i]`): reading the
                // field copied the whole column or list for every cell
                if let Expr::Field { base: owner, name, .. } = place {
                    let root = Self::root_local(owner);
                    let mut moved = root.is_none();
                    for i in indices {
                        crate::ir::visit_expr(i, &mut |e| moved |= matches!(e, Expr::Move(n, _) if Some(n.as_str()) == root));
                    }
                    if !moved && indices.len() == 1 && self.field_in_place(owner, name) {
                        let at = self.eval(&indices[0])?;
                        let o = self.place(owner).ok_or_else(|| format!("no field '{}'", name))?;
                        let cell = match &o.value {
                            Value::Frame(f) => {
                                let col = f.col_index(name).ok_or_else(|| format!("frame has no property or column '{}'", name))?;
                                crate::ops::frame_cell(f, col, crate::ops::list_position(&at, f.nrows())?)
                            }
                            Value::StructInstance(si) => crate::ops::index(&si.fields[name], vec![at])?,
                            Value::Map(m) => crate::ops::index(&m[name], vec![at])?,
                            _ => unreachable!(),
                        };
                        return Ok(if matches!(base.ty(), Ty::Column(_)) { placeholder(cell, ty) } else { cell });
                    }
                }
                let b = self.eval(base)?;
                if b.is_error() {
                    return Ok(b);
                }
                // a frame column's cell: a missing one reads as its column's
                // placeholder, as the runtime's getters give it and the IR
                // types it (a number, not an error)
                if matches!(base.ty(), Ty::Column(_)) {
                    let mut idx = Vec::with_capacity(indices.len());
                    for i in indices {
                        idx.push(self.eval(i)?);
                    }
                    let cell = crate::ops::index(&b, idx)?;
                    return Ok(placeholder(cell, ty));
                }
                let mut idx = Vec::with_capacity(indices.len());
                for i in indices {
                    idx.push(self.eval(i)?);
                }
                // a function value applied: f[3]; a gradient applied: df[3.0]
                match &b.value {
                    Value::Fn(fv) => {
                        let name = fv.name.clone();
                        if let Some(err) = self.first_error(&idx) {
                            return Ok(err);
                        }
                        return self.call_user(&name, idx);
                    }
                    Value::GradFn(fv) => {
                        let name = fv.name.clone();
                        return self.grad_call(&name, idx);
                    }
                    _ => {}
                }
                crate::ops::index(&b, idx)?
            }
            Expr::Field { base, name, .. } => {
                let b = self.eval(base)?;
                // an app session reads its own reactive state: the globals
                // while it is the active one, its waiting copy otherwise
                if let Value::App(_, _, _, id) = &b.value {
                    if let Some(rid) = self.remote_sessions.get(id).copied() {
                        return self.remote_read(rid, name);
                    }
                    // a derivation, as that session sees it
                    if self.derives.contains_key(name.as_str()) {
                        self.activate(*id);
                        return self.derived(name);
                    }
                    let waiting = if self.active_session == Some(*id) { None } else { self.sessions.get(id) };
                    return waiting
                        .and_then(|facts| facts.get(name))
                        .or_else(|| self.frames[0].scopes[0].get(name))
                        .cloned()
                        .ok_or_else(|| format!("no reactive state named '{}'", name));
                }
                crate::ops::field(b, name)?
            }
            Expr::Meta { base, which } => {
                let b = self.eval(base)?;
                let name = match which {
                    Meta::Ok => "ok",
                    Meta::Reason => "reason",
                    Meta::Kind => "kind",
                    Meta::Trace => "trace",
                    Meta::Valid => return crate::ops::valid(b),
                };
                crate::ops::field(b, name)?
            }
            Expr::Interp(pieces) => {
                let mut out = String::new();
                for p in pieces {
                    match p {
                        Piece::Text(t) => out.push_str(t),
                        Piece::Expr(x) => {
                            let v = self.eval(x)?;
                            if v.is_error() {
                                let mut err = v;
                                err.append_trace(&self.trace_entry());
                                return Ok(err);
                            }
                            out.push_str(&format!("{}", v.value));
                        }
                    }
                }
                CrowValue::ok(Value::Str(out))
            }
            Expr::ListLit(items, _) => {
                let mut vals = Vec::with_capacity(items.len());
                for i in items {
                    vals.push(self.eval(i)?);
                }
                crate::ops::array_literal(vals)?
            }
            Expr::MapLit(pairs, _) => {
                let mut map = std::collections::BTreeMap::new();
                for (k, v) in pairs {
                    map.insert(k.clone(), self.eval(v)?);
                }
                CrowValue::ok(Value::Map(map))
            }
            Expr::RecordLit { name, fields } => {
                let def = self.structs.get(name).cloned().ok_or_else(|| format!("unknown struct '{}'", name))?;
                if def.fields.len() != fields.len() {
                    return Err(format!("{}[] expects {} fields, got {}", name, def.fields.len(), fields.len()));
                }
                let mut vals = std::collections::BTreeMap::new();
                for ((fname, _), e) in def.fields.iter().zip(fields) {
                    vals.insert(fname.clone(), self.eval(e)?);
                }
                CrowValue::ok(Value::StructInstance(StructInstance { name: name.clone(), fields: vals }))
            }
            Expr::ForExpr { var, iter, body, parallel, .. } => {
                let it = self.eval(iter)?;
                // refused like the statement form's; a function hands the
                // error back from here, as from a return inside the loop
                if it.is_error() {
                    return match self.refuse(it, &Decision::Loop)? {
                        Flow::Return(v) => {
                            self.returning = Some(v);
                            Err(RETURN.to_string())
                        }
                        _ => Ok(CrowValue::done()),
                    };
                }
                let items = crate::ops::iter_items(&it)?;
                if *parallel && items.len() > 1 {
                    let results = self.run_parallel(&items, var, body, true)?;
                    return Ok(crate::ops::collect_loop(results));
                }
                let mut results = Vec::with_capacity(items.len());
                for item in items {
                    self.check_cancel()?;
                    self.push_scope();
                    self.bind(var, item);
                    self.last_value = None;
                    let r = self.exec_stmts(body);
                    let yielded = self.last_value.take();
                    self.pop_scope();
                    match r? {
                        // the function returns from here: an expression
                        // cannot, so the value travels to its call
                        Flow::Return(v) => {
                            self.returning = Some(v);
                            return Err(RETURN.to_string());
                        }
                        Flow::Break => break,
                        _ => {}
                    }
                    results.push(yielded.unwrap_or_else(CrowValue::done));
                }
                crate::ops::collect_loop(results)
            }
        })
    }

    /// `x = push[move x, v]` and the other in-place builtins on a list or a
    /// map (ast::IN_PLACE_BUILTINS): the value is taken out of its slot and
    /// the runtime's own function grows it, as the compiled mode does —
    /// through the shared builtin table it would be copied first. Only a
    /// shape the runtime handles totally takes this path; anything else
    /// (a frame, a tensor, a module's function of the same name, a wrong
    /// key type) goes the ordinary way.
    fn assign_in_place(&mut self, var: &str, expr: &Expr) -> Result<bool, String> {
        let (name, args) = match expr {
            Expr::Call { target: Target::Runtime(name), args, .. } => (name.rsplit('.').next().unwrap_or(name), args),
            _ => return Ok(false),
        };
        // the runtime table's names for the map builtins (ir::RUNTIME_TABLE)
        let name = match name {
            "map_set" => "set",
            "map_del" => "del",
            other => other,
        };
        // (a list that could hold a failed item is unwrapped where it is
        // passed: `push(unwrap move xs, …)` grows it in place too — through
        // the table a loop of pushes copied the list every time)
        let list = match args.first() {
            Some(Expr::Convert { how: Convert::Unwrap, expr, .. }) => Some(expr.as_ref()),
            other => other,
        };
        if !crate::ast::IN_PLACE_BUILTINS.contains(&name) || !matches!(list, Some(Expr::Move(v, _)) if v == var) {
            return Ok(false);
        }
        if self.registry.has_function(name) && name != "sort" {
            return Ok(false);
        }
        let rest = self.eval_args(&args[1..])?;
        if rest.iter().any(|r| r.is_error()) {
            return Ok(false);
        }
        let ok = match self.lookup(var) {
            Some(cur) if !cur.is_error() => match (&cur.value, name) {
                (Value::Array(_), "push") => rest.len() == 1,
                (Value::Array(_), "pop") | (Value::Array(_), "reverse") | (Value::Array(_), "sort") => rest.is_empty(),
                (Value::Array(_), "concat") => rest.len() == 1 && matches!(rest[0].value, Value::Array(_)),
                (Value::Map(m), "set") => {
                    rest.len() == 2 && matches!(rest[0].value, Value::Str(_)) && crate::builtins::check_map_homogeneous(m, &rest[1]).is_ok()
                }
                (Value::Map(_), "del") => rest.len() == 1 && matches!(rest[0].value, Value::Str(_)),
                _ => false,
            },
            _ => false,
        };
        if !ok {
            return Ok(false);
        }
        let taken = match self.slot(var) {
            Some(slot) => std::mem::replace(slot, CrowValue::done()),
            None => return Ok(false),
        };
        let mut rest = rest.into_iter();
        let out = match (taken.value, name) {
            (Value::Array(items), "push") => Value::Array(crow_runtime::core::push(items, rest.next().unwrap())),
            (Value::Array(items), "pop") => Value::Array(crow_runtime::core::pop(items)),
            (Value::Array(items), "reverse") => Value::Array(crow_runtime::core::reverse(items)),
            (Value::Array(items), "sort") => Value::Array(crow_runtime::core::sort_by(items, crate::builtins::value_cmp)),
            (Value::Array(items), "concat") => match rest.next().unwrap().value {
                Value::Array(other) => Value::Array(crow_runtime::core::concat(items, &other)),
                _ => unreachable!("checked above"),
            },
            (Value::Map(m), "set") => {
                let key = match rest.next().unwrap().value {
                    Value::Str(s) => s,
                    _ => unreachable!("checked above"),
                };
                Value::Map(crow_runtime::core::map_set(m, key, rest.next().unwrap()))
            }
            (Value::Map(m), "del") => {
                let key = match rest.next().unwrap().value {
                    Value::Str(s) => s,
                    _ => unreachable!("checked above"),
                };
                Value::Map(crow_runtime::core::map_del(m, &key))
            }
            _ => unreachable!("checked above"),
        };
        self.assign(var, CrowValue::ok(out));
        Ok(true)
    }

    /// A parallel loop: one clone of the walker per chunk of items, its
    /// scopes shared until written, each iteration in a scope of its own.
    /// The lowering marked the loop parallel because its body writes no
    /// outer variable or fact and reaches no effect, so nothing is lost
    /// when the clones are dropped; a `return` inside is refused.
    /// The local a chain of fields starts from: `bars` in `bars.c`.
    fn root_local(e: &Expr) -> Option<&str> {
        match e {
            Expr::Local(v, _) => Some(v),
            Expr::Field { base, .. } => Self::root_local(base),
            _ => None,
        }
    }

    /// A local, or a field of one (a struct's, a map's), where it lies;
    /// `None` for anything else (a session's state, a failed value).
    fn place(&self, e: &Expr) -> Option<&CrowValue> {
        match e {
            Expr::Local(v, _) => self.lookup(v).filter(|b| b.ok),
            Expr::Field { base, name, .. } => {
                if matches!(name.as_str(), "ok" | "reason" | "kind" | "trace" | "len") {
                    return None;
                }
                match &self.place(base)?.value {
                    Value::StructInstance(si) => si.fields.get(name.as_str()).filter(|b| b.ok),
                    Value::Map(m) => m.get(name.as_str()).filter(|b| b.ok),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `owner.name[i]` can be read without copying `owner.name`: a frame's
    /// column, or a struct's or map's list, map or text.
    fn field_in_place(&self, owner: &Expr, name: &str) -> bool {
        if matches!(name, "ok" | "reason" | "kind" | "trace" | "len") {
            return false;
        }
        let indexable = |v: &CrowValue| v.ok && matches!(v.value, Value::Array(_) | Value::NumArray(_) | Value::Map(_) | Value::Str(_));
        match self.place(owner).map(|o| &o.value) {
            Some(Value::Frame(f)) => !matches!(name, "shape" | "columns" | "dtypes") && f.col_index(name).is_some(),
            Some(Value::StructInstance(si)) => si.fields.get(name).is_some_and(indexable),
            Some(Value::Map(m)) => m.get(name).is_some_and(indexable),
            _ => false,
        }
    }

    fn run_parallel(&mut self, items: &[CrowValue], var: &str, body: &[Stmt], collect: bool) -> Result<Vec<CrowValue>, String> {
        self.check_cancel()?;
        let threads = rayon::current_num_threads().max(1);
        let chunk = (items.len() / (threads * 4)).max(1);
        let base = &*self;
        let chunks: Vec<Result<Vec<CrowValue>, String>> = items
            .par_chunks(chunk)
            .map(|part| {
                let mut local = base.clone();
                let mut out = Vec::with_capacity(if collect { part.len() } else { 0 });
                for item in part {
                    local.push_scope();
                    local.bind(var, item.clone());
                    local.last_value = None;
                    let r = local.exec_stmts(body);
                    let yielded = local.last_value.take();
                    local.pop_scope();
                    if let Flow::Return(_) = r? {
                        return Err("cannot return from inside a for loop".to_string());
                    }
                    if collect {
                        out.push(yielded.unwrap_or_else(CrowValue::done));
                    }
                }
                Ok(out)
            })
            .collect();
        let mut results = Vec::with_capacity(if collect { items.len() } else { 0 });
        for c in chunks {
            results.extend(c?);
        }
        Ok(results)
    }

    fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<CrowValue>, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a)?);
        }
        Ok(vals)
    }

    fn first_error(&self, vals: &[CrowValue]) -> Option<CrowValue> {
        for v in vals {
            if v.is_error() {
                let mut err = v.clone();
                err.append_trace(&self.trace_entry());
                return Some(err);
            }
        }
        None
    }

    fn stamp(name: &str, v: CrowValue) -> CrowValue {
        if READER_FNS.contains(&name) && v.is_ok() {
            v.stamp_captured()
        } else {
            v
        }
    }

    /// A runtime builtin by its table name (`core.push`) or its own name.
    fn runtime_call(&mut self, path: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let name = RUNTIME_TABLE.iter().find(|(_, r)| *r == path).map(|(b, _)| *b).unwrap_or(path).to_string();
        self.bare_call(&name, vals)
    }

    /// A bare name, resolved as the interpreter resolves it: a loaded
    /// module's function wins over a builtin of the same name, except that
    /// a frame function given something other than a frame is the builtin
    /// (sort[xs], join[parts, sep]); then the builtins; then what only a
    /// walker can do.
    fn bare_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some(stop) = self.effect_refused(name, &vals) {
            return stop;
        }
        // in a rehearsal what reaches past the program is recorded, not run
        if let Some(v) = crate::rehearsal::outside(name, &vals) {
            return Ok(v);
        }
        if name == "transact" {
            return self.transact(vals);
        }
        if matches!(name, "serve" | "page" | "fetch" | "poll" | "app" | "html" | "elements" | "change" | "click" | "box" | "fits" | "style" | "downloads" | "mails") {
            return self.harness(name, vals);
        }
        // a scalar name both core and math answer goes to the core builtin
        // unless a tensor is among the arguments
        let scalar_overlap = OVERLAPPING.contains(&name) && !vals.iter().any(|v| matches!(v.value, Value::NumArray(_)));
        if self.registry.has_function(name) && !scalar_overlap {
            let frame_fn = self.registry.module_name_for(name) == Some("frame");
            let first_is_frame = vals.first().map_or(false, |a| matches!(a.value, Value::Frame(_)));
            let core_too = crate::checker::BUILTINS.contains(&name);
            if !(frame_fn && core_too && !first_is_frame) {
                return self.registry.call(name, &vals).map(|v| Self::stamp(name, v));
            }
        }
        if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
            return r.map(|v| Self::stamp(name, v));
        }
        match name {
            "any" | "all" => {
                let want = name == "any";
                let items = crate::ops::iter_items(&vals[0])?;
                for item in items {
                    let hit = self.call_value(&vals[1], vec![item])?;
                    if crate::ops::is_truthy(&hit) == want {
                        return Ok(CrowValue::ok(Value::Bool(want)));
                    }
                }
                Ok(CrowValue::ok(Value::Bool(!want)))
            }
            "find" => {
                if vals.len() != 2 {
                    return Err("find[] takes exactly 2 arguments".into());
                }
                // find[text, pattern]: a record with the groups, or an error value
                if let Value::Str(s) = &vals[0].value {
                    let pattern = match &vals[1].value {
                        Value::Str(p) => p.clone(),
                        _ => return Err("find[] second argument must be a string pattern".into()),
                    };
                    return Ok(match crow_runtime::text::find(s, &pattern) {
                        Ok(Some(groups)) => {
                            let mut map = std::collections::BTreeMap::new();
                            map.insert("groups".to_string(), CrowValue::ok(Value::Array(groups.into_iter().map(|m| CrowValue::ok(Value::Str(m))).collect())));
                            CrowValue::ok(Value::Map(map))
                        }
                        Ok(None) => CrowValue::error("no match", &self.trace_entry()),
                        Err(e) => CrowValue::error_with_kind(&e, "regex", &self.trace_entry()),
                    });
                }
                let items = crate::ops::iter_items(&vals[0])?;
                for item in items {
                    let hit = self.call_value(&vals[1], vec![item.clone()])?;
                    if crate::ops::is_truthy(&hit) {
                        return Ok(item);
                    }
                }
                Ok(CrowValue::error("no match", "find"))
            }
            "captured" => {
                if vals.len() != 1 {
                    return Err("captured[] takes exactly 1 argument".into());
                }
                if vals[0].is_error() {
                    let mut err = vals[0].clone();
                    err.append_trace(&self.trace_entry());
                    return Ok(err);
                }
                Ok(match vals[0].captured {
                    Some(t) => CrowValue::ok(Value::Float(t)),
                    None => CrowValue::error_with_kind(
                        "this value was computed here, not read from outside - it has no capture moment",
                        "no_capture",
                        &self.trace_entry(),
                    ),
                })
            }
            "grad" => {
                if vals.len() != 1 {
                    return Err("grad[] takes exactly 1 argument".into());
                }
                match &vals[0].value {
                    Value::Fn(fv) => Ok(CrowValue::ok(Value::GradFn(fv.clone()))),
                    other => Err(format!("grad[] takes a function, got {}", other.type_name())),
                }
            }
            "later" => {
                // the interpreter runs the work inline: immediate is a legal
                // schedule (spec §15) — after the handler that asked commits
                if vals.is_empty() {
                    return Err("later[] needs a function: later[work] or later[work, args…]".into());
                }
                let f = vals[0].clone();
                let rest: Vec<CrowValue> = vals[1..].to_vec();
                if let Some(held) = self.held.as_mut() {
                    held.push(HeldEffect::Later(f, rest));
                } else {
                    let _ = self.call_value(&f, rest)?;
                }
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            // the page's signed-in reader: the session's, a map of strings
            "__reader" => {
                let id = self.active_session.unwrap_or(0);
                Ok(CrowValue::ok(Value::Map(self.session_reader.get(&id).cloned().unwrap_or_default())))
            }
            // sign_in[record] / sign_out[]: held to the handler's commit
            "sign_in" | "sign_out" => {
                let record = if name == "sign_in" {
                    let m = match vals.first().map(|v| &v.value) {
                        Some(Value::Map(m)) => m.clone(),
                        _ => return Err("sign_in[] takes the reader as a map of strings: sign_in[map[id = '42', name = 'Ada']]".into()),
                    };
                    let mut strings = std::collections::BTreeMap::new();
                    for (k, v) in &m {
                        match &v.value {
                            Value::Str(s) => {
                                strings.insert(k.clone(), s.clone());
                            }
                            other => return Err(format!("sign_in[]: the reader's '{}' is {}; the reader is a map of strings", k, other.type_name())),
                        }
                    }
                    crow_runtime::signin::check(&strings)?;
                    Some(m)
                } else {
                    None
                };
                match self.held.as_mut() {
                    Some(held) => held.push(HeldEffect::Sign(record)),
                    None => {
                        let id = self.active_session.unwrap_or(0);
                        self.session_reader.insert(id, record.unwrap_or_default());
                    }
                }
                Ok(CrowValue::done())
            }
            // download[value, name]: the value's bytes, held to the commit
            "download" => {
                let name = match vals.get(1).map(|v| &v.value) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err("download[] takes the value and the name to save it under: download[trades, 'trades.csv']".into()),
                };
                let bytes = match vals.first().map(|v| &v.value) {
                    Some(Value::Frame(f)) => f.to_csv().map_err(|e| format!("download: {}", e))?.into_bytes(),
                    Some(Value::Str(s)) => s.clone().into_bytes(),
                    Some(Value::Map(m)) => match m.get("path").map(|p| &p.value) {
                        Some(Value::Str(p)) => std::fs::read(p).map_err(|e| format!("download: {}", e))?,
                        _ => return Err("download: a file to hand on is a picked file, with a path".into()),
                    },
                    Some(other) => format!("{}", other).into_bytes(),
                    None => Vec::new(),
                };
                match self.held.as_mut() {
                    Some(held) => held.push(HeldEffect::Download(name, bytes)),
                    None => {
                        let id = self.active_session.unwrap_or(0);
                        self.session_downloads.entry(id).or_default().push((name, bytes));
                    }
                }
                Ok(CrowValue::done())
            }
            "notice" => {
                let text = match vals.first().map(|v| &v.value) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(other) => format!("{}", other),
                    None => return Err("notice[] takes the text to tell the reader".into()),
                };
                let at = crow_runtime::moments::now();
                match self.held.as_mut() {
                    Some(held) => held.push(HeldEffect::Notice(text, at)),
                    None => {
                        let id = self.active_session.unwrap_or(0);
                        self.session_notice.insert(id, (text, at));
                    }
                }
                Ok(CrowValue::done())
            }
            "__notice_text" | "__notice_at" => {
                self.notice_read = true;
                let id = self.active_session.unwrap_or(0);
                let (text, at) = self.session_notice.get(&id).cloned().unwrap_or((String::new(), 0.0));
                Ok(if name == "__notice_text" { CrowValue::ok(Value::Str(text)) } else { CrowValue::ok(Value::Float(at)) })
            }
            "running" => Ok(CrowValue::ok(Value::Bool(false))),
            // the resources as calls: gpu[n] turns the device on, nodes[n]
            // runs as the one rank of a cluster, cpu[n] bounds the loops
            "gpu" => {
                if vals.len() > 1 {
                    return Err("gpu[] takes 0 or 1 arguments".to_string());
                }
                if let Some(v) = vals.first() {
                    match v.unwrap_or_propagate()? {
                        Value::Int(n) if *n >= 1 => {}
                        Value::Int(_) => return Err("gpu[] requires a positive integer".to_string()),
                        _ => return Err("gpu[] requires an integer argument".to_string()),
                    }
                }
                crow_runtime::gpu::set_gpu_enabled(true);
                Ok(CrowValue::done())
            }
            "nodes" | "cpu" => {
                if vals.len() != 1 {
                    return Err(format!("{}[] takes exactly 1 argument", name));
                }
                let n = match vals[0].unwrap_or_propagate()? {
                    Value::Int(n) if *n >= 1 => *n as usize,
                    Value::Int(_) => return Err(format!("{}[] requires a positive integer", name)),
                    _ => return Err(format!("{}[] requires an integer argument", name)),
                };
                if name == "nodes" {
                    // one process, rank 0: no connections in the interpreter
                    crow_runtime::distributed::init(1);
                } else {
                    let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
                }
                Ok(CrowValue::done())
            }
            // the web module's builtins answer with stand-ins here: the
            // served program is the compiled one, and a script that builds
            // an app still runs to its end
            "cors" | "sse" | "ws" | "static_dir" => {
                let want = if name == "cors" { 1 } else { 3 };
                if vals.len() < want {
                    return Err(format!("{}[] takes {} arguments", name, want));
                }
                Ok(vals[0].clone())
            }
            "sessions" | "cache" => {
                if name == "cache" && vals.is_empty() {
                    return Err("cache[] requires at least 1 argument".into());
                }
                Ok(CrowValue::ok(Value::Map(std::collections::BTreeMap::new())))
            }
            "templates" => {
                if vals.len() != 1 {
                    return Err("templates[] takes 1 argument".into());
                }
                let glob = match vals[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => String::new(),
                };
                let mut map = std::collections::BTreeMap::new();
                map.insert("glob".to_string(), CrowValue::ok(Value::Str(glob)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "render" => {
                if vals.len() != 3 {
                    return Err("render[] takes 3 arguments (templates, name, data)".into());
                }
                let tname = match vals[1].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("render[] name must be a string".into()),
                };
                Ok(CrowValue::ok(Value::Str(format!("[rendered: {}]", tname))))
            }
            "limit" => {
                if vals.len() != 2 {
                    return Err("limit[] takes 2 arguments (max, window)".into());
                }
                let mut map = std::collections::BTreeMap::new();
                map.insert("max".to_string(), vals[0].clone());
                map.insert("window".to_string(), vals[1].clone());
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "uploads" => {
                if vals.len() != 1 {
                    return Err("uploads[] takes 1 argument".into());
                }
                Ok(CrowValue::ok(Value::Array(vec![])))
            }
            "connect" => {
                if vals.len() != 1 {
                    return Err("connect[] takes 1 argument".into());
                }
                let url = match vals[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("connect[] url must be a string".into()),
                };
                let mut map = std::collections::BTreeMap::new();
                map.insert("url".to_string(), CrowValue::ok(Value::Str(url)));
                map.insert("ok".to_string(), CrowValue::ok(Value::Bool(false)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            "send" | "close" => {
                let want = if name == "send" { 2 } else { 1 };
                if vals.len() != want {
                    return Err(format!("{}[] takes {} argument{}", name, want, if want == 1 { "" } else { "s" }));
                }
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            "recv" => {
                if vals.len() != 1 {
                    return Err("recv[] takes 1 argument".into());
                }
                Ok(CrowValue::ok(Value::Str(String::new())))
            }
            "smtp" => {
                if vals.len() != 4 {
                    return Err("smtp[] takes 4 arguments".into());
                }
                let text = |i: usize, what: &str| -> Result<String, String> {
                    match vals[i].unwrap_or_propagate()? {
                        Value::Str(s) => Ok(s.clone()),
                        _ => Err(format!("smtp[] {} must be a string", what)),
                    }
                };
                let (host, user, pass) = (text(0, "host")?, text(2, "user")?, text(3, "password")?);
                let port = match vals[1].unwrap_or_propagate()? {
                    Value::Int(n) => *n as u16,
                    _ => return Err("smtp[] port must be an int".into()),
                };
                let mailer = crow_web::mail::CrowMailer::new(&host, port, &user, &pass)?;
                let id = {
                    let mut all = MAILERS.lock().unwrap_or_else(|p| p.into_inner());
                    all.push(mailer);
                    all.len() - 1
                };
                Ok(CrowValue::ok(Value::Mailer(id, host)))
            }
            // mail[mailer, to, subject, body]: held to a handler's commit, or
            // sent when its answer is used; in a test, recorded and never sent
            "mail" => {
                if vals.len() != 4 {
                    return Err("mail[] takes a mailer, to, subject and body: mail[mailer, 'ada@example.com', 'Hello', 'Welcome']".into());
                }
                let mailer = match &vals[0].value {
                    Value::Mailer(id, _) => *id,
                    _ => return Err("mail[] sends through a mailer: mailer = smtp[host, port, user, pass]".into()),
                };
                let text = |i: usize| format!("{}", vals[i].value);
                let mail = crow_web::live::SentMail { to: text(1), subject: text(2), body: text(3) };
                if self.testing || self.held.is_some() && !self.answering {
                    match self.held.as_mut() {
                        Some(held) if !self.answering => held.push(HeldEffect::Mail(mail, mailer)),
                        _ => {
                            let id = self.active_session.unwrap_or(0);
                            self.session_mails.entry(id).or_default().push((mail, mailer));
                        }
                    }
                    return Ok(CrowValue::ok(Value::Bool(true)));
                }
                match (mail_sender(mailer, &mail))() {
                    Ok(()) => Ok(CrowValue::ok(Value::Bool(true))),
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("mail to {} failed: {}", mail.to, e), "mail", &self.trace_entry())),
                }
            }
            "msgpack" | "arrow" | "bytes" | "stream" => {
                if vals.len() != 1 {
                    return Err(format!("{}[] takes 1 argument", name));
                }
                if name != "stream" {
                    let _ = vals[0].unwrap_or_propagate()?;
                }
                let kind = match name {
                    "msgpack" => "application/msgpack",
                    "arrow" => "application/vnd.apache.arrow.stream",
                    _ => "application/octet-stream",
                };
                let mut map = std::collections::BTreeMap::new();
                map.insert("status".to_string(), CrowValue::ok(Value::Int(200)));
                map.insert("type".to_string(), CrowValue::ok(Value::Str(kind.to_string())));
                map.insert(if name == "stream" { "streaming" } else { "binary" }.to_string(), CrowValue::ok(Value::Bool(true)));
                Ok(CrowValue::ok(Value::Map(map)))
            }
            // progress[value, max] (lowered)
            "progress" if vals.len() == 2 => {
                if self.in_view == 0 {
                    return Ok(CrowValue::done());
                }
                let num = |v: &CrowValue| match &v.value {
                    Value::Int(n) => *n as f64,
                    Value::Float(f) => *f,
                    _ => 0.0,
                };
                let fmt_num = |x: f64| if (x - x.round()).abs() < 1e-9 { format!("{}", x.round() as i64) } else { format!("{}", x) };
                let max = num(&vals[1]);
                Ok(CrowValue::markup(format!(
                    "<progress class=\"crow-progress\" max=\"{}\" value=\"{}\"></progress>",
                    fmt_num(if max > 0.0 { max } else { 1.0 }),
                    fmt_num(num(&vals[0]))
                )))
            }
            // chart[frame, x, candles, lines, bars_y, marker, tz, height,
            // panes, levels, points, seek] (lowered): the chart's markup
            // from the kernel's renderer, as the compiled view draws it
            "chart" if self.in_view > 0 && vals.len() == 12 => {
                let Value::Frame(f) = &vals[0].value else {
                    if vals[0].is_error() {
                        // the reason is text, as the compiled chart writes it
                        return Ok(CrowValue::markup(crate::harness::html_escape_text(&format!("chart error: {}", vals[0].reason))));
                    }
                    return Err("chart[] takes a frame first".into());
                };
                let as_str = |v: &CrowValue| -> String { format!("{}", v.value) };
                let str_list = |v: &CrowValue| -> Vec<String> {
                    match &v.value {
                        Value::Array(items) => items.iter().map(|i| format!("{}", i.value)).collect(),
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
                // levels = map[rsi = [30, 70], cvd = [0]]: reference lines,
                // keyed by the column whose pane draws them
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
                Ok(CrowValue::markup(node.to_html()))
            }
            // icon[name, size, color] (lowered): inline SVG from the kernel's set
            "icon" if vals.len() == 3 => {
                if self.in_view == 0 {
                    return Ok(CrowValue::done());
                }
                let s = |v: &CrowValue| format!("{}", v.value);
                crow_runtime::layout::icon_html(&s(&vals[0]), &s(&vals[1]), &s(&vals[2]))
                    .map(CrowValue::markup)
                    .map_err(|e| format!("icon: {}", e))
            }
            _ => Err(format!("the IR interpreter does not run '{}' yet", name)),
        }
    }

    /// `transact[conn, f]`: BEGIN, the function on the same connection,
    /// COMMIT, or ROLLBACK when it fails, as the interpreter does.
    fn transact(&mut self, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if vals.len() != 2 {
            return Err("transact[] takes 2 arguments: transact[conn, fn_name]".into());
        }
        let conn = match &vals[0].value {
            Value::DbConn(h) => h.clone(),
            other => return Err(format!("transact[] first argument must be a db connection, got {}", other.type_name())),
        };
        let f = vals[1].clone();
        conn.exec("BEGIN", &[]).map_err(|e| format!("transact[]: {}", e))?;
        let tx_val = CrowValue::ok(Value::DbConn(conn.clone()));
        match self.call_value(&f, vec![tx_val]) {
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

    /// Forward-mode differentiation: the function once per parameter with
    /// that parameter a dual (x, 1) and the others (x, 0).
    fn grad_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let nparams = self.fns.get(name).map(|f| f.params.len()).ok_or_else(|| format!("unknown function '{}'", name))?;
        if nparams != vals.len() {
            return Err(format!("grad[{}][] expects {} args, got {}", name, nparams, vals.len()));
        }
        let xs: Vec<f64> = vals
            .iter()
            .map(|a| match &a.value {
                Value::Int(n) => Ok(*n as f64),
                Value::Float(n) => Ok(*n),
                Value::Dual(n, _) => Ok(*n),
                _ => Err(format!("grad[{}][] requires numeric arguments", name)),
            })
            .collect::<Result<_, _>>()?;
        let mut derivatives = Vec::with_capacity(xs.len());
        for i in 0..xs.len() {
            let args: Vec<CrowValue> = xs.iter().enumerate().map(|(j, &x)| CrowValue::ok(Value::Dual(x, if j == i { 1.0 } else { 0.0 }))).collect();
            let result = self.call_user(name, args)?;
            let d = match &result.value {
                Value::Dual(_, d) => *d,
                Value::Float(_) | Value::Int(_) => 0.0,
                other => return Err(format!("grad[{}][] requires function to return a number, got {}", name, other.type_name())),
            };
            derivatives.push(d);
        }
        if derivatives.len() == 1 {
            Ok(CrowValue::ok(Value::Float(derivatives[0])))
        } else {
            Ok(CrowValue::ok(Value::Array(derivatives.into_iter().map(|d| CrowValue::ok(Value::Float(d))).collect())))
        }
    }

    fn module_call(&mut self, module: &str, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some(stop) = self.effect_refused(name, &vals) {
            return stop;
        }
        if name == "transact" && module == "db" {
            return self.transact(vals);
        }
        if OVERLAPPING.contains(&name) && !vals.iter().any(|v| matches!(v.value, Value::NumArray(_))) {
            if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
                return r;
            }
        }
        if self.registry.is_loaded(module) {
            return self.registry.call_in(module, name, &vals).map(|v| Self::stamp(name, v));
        }
        Err(format!("the IR interpreter does not run '{}.{}' yet", module, name))
    }

    /// An effect by name: qualified (`frame.load`) through its module,
    /// bare as any bare call.
    fn effect(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some((m, f)) = name.split_once('.') {
            let (m, f) = (m.to_string(), f.to_string());
            return self.module_call(&m, &f, vals);
        }
        self.bare_call(name, vals)
    }

    fn call_value(&mut self, f: &CrowValue, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        match &f.value {
            Value::Fn(fv) => self.call_user(&fv.name.clone(), args),
            other => Err(format!("expected a function, got {}", other.type_name())),
        }
    }

    fn call_user(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        self.check_cancel()?;
        // a specialised name (first__1) is the one body to this walker
        let f = match self.fns.get(name) {
            Some(f) => f.clone(),
            None => {
                let base = name.split("__").next().unwrap_or(name);
                self.fns.get(base).cloned().ok_or_else(|| format!("unknown function '{}'", name))?
            }
        };
        if f.params.len() != vals.len() {
            return Err(format!("{}[] expects {} arguments, got {}", name, f.params.len(), vals.len()));
        }
        // a view renders what it is given: a failed argument is its text
        if f.kind == FnKind::View {
            if let Some(bad) = vals.iter().find(|v| v.is_error()) {
                return Ok(CrowValue::markup(format!("error: {}", crate::harness::html_escape_text(&bad.reason))));
            }
        }
        let mut scope = HashMap::new();
        for ((p, _), v) in f.params.iter().zip(vals) {
            scope.insert(p.clone(), v);
        }
        let saved_line = self.line;
        let is_view = f.kind == FnKind::View;
        if is_view {
            self.in_view += 1;
        }
        self.frames.push(Frame { scopes: vec![Arc::new(scope)], fn_name: name.to_string() });
        let r = self.exec_stmts(&f.body);
        let r = self.returned(r);
        self.frames.pop();
        if is_view {
            self.in_view -= 1;
        }
        self.line = saved_line;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(CrowValue::done()),
        }
    }

    /// A body's outcome, with a return from inside one of its collecting
    /// loops taken as the body's own.
    fn returned(&mut self, r: Result<Flow, String>) -> Result<Flow, String> {
        match r {
            Err(e) if e == RETURN => Ok(Flow::Return(self.returning.take().unwrap_or_else(CrowValue::done))),
            other => other,
        }
    }

    /// A derivation, recomputed on every read for now (the memoized,
    /// versioned form comes with the session layer), stamped with its moment.
    fn derived(&mut self, name: &str) -> Result<CrowValue, String> {
        if let Some(rid) = self.active_session.and_then(|id| self.remote_sessions.get(&id).copied()) {
            return self.remote_read(rid, name);
        }
        let d = self.derives.get(name).cloned().ok_or_else(|| format!("unknown derivation '{}'", name))?;
        // the reader's line is still the line reading it
        let saved_line = self.line;
        self.frames.push(Frame { scopes: vec![Arc::new(HashMap::new())], fn_name: name.to_string() });
        let r = self.exec_stmts(&d.body);
        let r = self.returned(r);
        self.frames.pop();
        self.line = saved_line;
        let mut v = match r? {
            Flow::Return(v) => v,
            _ => CrowValue::done(),
        };
        v.captured = Some(crow_runtime::moments::now());
        Ok(v)
    }

    /// An effect a handler calls with an error among its arguments — a
    /// parameter list included — stops the handler: an error never reaches
    /// the world outside the session (core-design.md §2.1).
    fn effect_refused(&mut self, name: &str, vals: &[CrowValue]) -> Option<Result<CrowValue, String>> {
        const EFFECTS: &[&str] = &["query", "exec", "insert", "transact", "read", "write", "fetch", "shell", "mail", "later", "notice", "sign_in", "download"];
        if self.held.is_none() || !EFFECTS.contains(&name) {
            return None;
        }
        let err = vals.iter().find_map(|v| {
            if v.is_error() {
                return Some(v.clone());
            }
            match &v.value {
                Value::Array(items) => items.iter().find(|i| i.is_error()).cloned(),
                _ => None,
            }
        })?;
        match self.refuse(err, &Decision::Branch) {
            Err(e) => Some(Err(e)),
            _ => None,
        }
    }

    /// One handler as one transaction on the active session (core-design.md
    /// §1): its writes land as one when it ends, then its held effects in
    /// program order; when it stops, nothing it did lands and the reason is
    /// the session's notice. Returns what `click` evaluates to: the
    /// handler's value, or the error it stopped on.
    pub(crate) fn transaction(&mut self, name: &str, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        let id = self.active_session.unwrap_or(0);
        self.last_stop = None;
        let before = self.fact_values();
        let outer = self.held.replace(Vec::new());
        let saved_view = self.in_view;
        self.in_view = 0;
        crow_runtime::txn::begin();
        let mut r = self.call_user(name, args);
        self.in_view = saved_view;
        // its database writes commit with it, or roll back when it stopped;
        // a commit the database refuses is a stop too
        let stopped = matches!(&r, Err(e) if e.starts_with(STOP));
        if let Err(e) = crow_runtime::txn::finish(!stopped && r.is_ok()) {
            if !stopped {
                r = Err(format!("{}{}\u{1}{}\u{1}{}", STOP, name, e, "db"));
            }
        }
        let held = std::mem::replace(&mut self.held, outer).unwrap_or_default();
        match r {
            Err(e) if e.starts_with(STOP) => {
                let mut parts = e[STOP.len()..].splitn(3, '\u{1}');
                let who = parts.next().unwrap_or(name).to_string();
                let reason = parts.next().unwrap_or("").to_string();
                let kind = parts.next().unwrap_or("").to_string();
                for (n, v) in before {
                    self.set_fact(&n, v);
                }
                let told = format!("{} stopped: {}", who, reason);
                self.last_stop = Some(told.clone());
                self.session_notice.insert(id, (told, crow_runtime::moments::now()));
                Ok(CrowValue::error_with_kind(&reason, &kind, &self.trace_entry()))
            }
            Err(e) => Err(e),
            Ok(v) => {
                self.session_sign = None;
                for effect in held {
                    match effect {
                        HeldEffect::Notice(text, at) => {
                            self.session_notice.insert(id, (text, at));
                        }
                        HeldEffect::Download(name, bytes) => {
                            self.session_downloads.entry(id).or_default().push((name, bytes));
                        }
                        HeldEffect::Mail(mail, mailer) => {
                            self.session_mails.entry(id).or_default().push((mail, mailer));
                        }
                        HeldEffect::Sign(record) => {
                            let strings = record.as_ref().map(|m| {
                                m.iter().map(|(k, v)| (k.clone(), match &v.value { Value::Str(s) => s.clone(), other => format!("{}", other) })).collect()
                            });
                            self.session_reader.insert(id, record.unwrap_or_default());
                            self.session_sign = Some(strings);
                        }
                        HeldEffect::Later(f, rest) => {
                            // a scheduled function is a transaction of its own
                            if let Value::Fn(fv) = &f.value {
                                let fname = fv.name.clone();
                                let _ = self.transaction(&fname, rest)?;
                            }
                        }
                    }
                }
                Ok(v)
            }
        }
    }

    // ── the harness against the compiled program ───────────────────

    fn remote(&self) -> Result<Arc<crate::compiled_harness::RemoteApp>, String> {
        self.remote.clone().ok_or_else(|| "no compiled program is running".to_string())
    }

    /// A harness session opened on the compiled program too: it starts from
    /// the facts the test holds now, at the session's moment.
    fn remote_open(&mut self, view: &str, id: u64) -> Result<(), String> {
        if self.remote.is_none() {
            self.remote = Some(Arc::new(crate::compiled_harness::RemoteApp::start()?));
        }
        let facts: serde_json::Map<String, serde_json::Value> = self
            .fact_values()
            .into_iter()
            .filter(|(_, v)| crate::served::is_data(v))
            .map(|(k, v)| (k, crate::served::fact_to_json(&v)))
            .collect();
        let now = self.session_clock.get(&id).copied();
        let answer = self.remote()?.post("open", serde_json::json!({ "view": view, "facts": facts, "now": now }))?;
        let rid = answer.get("session").and_then(|v| v.as_u64()).ok_or("the compiled program opened no session")?;
        self.remote_sessions.insert(id, rid);
        self.activate(id);
        self.remote_pull(id)
    }

    /// The compiled session's facts, copied into the globals, and its notice
    /// and reader into the session's: its session is the active one, as a
    /// local session's would be.
    fn remote_pull(&mut self, id: u64) -> Result<(), String> {
        let rid = self.remote_sessions.get(&id).copied().ok_or("not a compiled session")?;
        self.activate(id);
        let answer = self.remote()?.post("facts", serde_json::json!({ "session": rid }))?;
        match answer.get("notice").and_then(|n| n.get("text").and_then(|t| t.as_str()).map(|t| (t.to_string(), n.get("at").and_then(|a| a.as_f64()).unwrap_or(0.0)))) {
            Some(notice) => {
                self.session_notice.insert(id, notice);
            }
            None => {
                self.session_notice.remove(&id);
            }
        }
        if let Some(serde_json::Value::Object(reader)) = answer.get("reader") {
            let record: std::collections::BTreeMap<String, CrowValue> =
                reader.iter().map(|(k, v)| (k.clone(), CrowValue::ok(Value::Str(v.as_str().unwrap_or_default().to_string())))).collect();
            self.session_reader.insert(id, record);
        }
        if let Some(serde_json::Value::Object(facts)) = answer.get("facts") {
            for name in self.fact_names.clone() {
                if let (Some(j), Some(declared)) = (facts.get(&name), self.frames[0].scopes[0].get(&name).cloned()) {
                    if crate::served::is_data(&declared) {
                        self.set_fact(&name, crate::served::json_to_fact(j, &declared));
                    }
                }
            }
        }
        Ok(())
    }

    /// A handler run by the compiled program as one event: a stop is the
    /// click's error value, as the interpreter's transaction answers it.
    fn remote_click(&mut self, id: u64, handler: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        let rid = self.remote_sessions.get(&id).copied().ok_or("not a compiled session")?;
        let args: Vec<serde_json::Value> = args.iter().map(crate::served::fact_to_json).collect();
        let answer = self.remote()?.post("event", serde_json::json!({ "session": rid, "event": handler, "args": args }))?;
        self.remote_pull(id)?;
        if answer.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let told = answer.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let reason = told.strip_prefix(&format!("{} stopped: ", handler)).unwrap_or(&told).to_string();
            self.last_stop = Some(told);
            return Ok(CrowValue::error_with_kind(&reason, "", &self.trace_entry()));
        }
        Ok(CrowValue::done())
    }

    /// A fact or a derivation as the compiled session holds it.
    fn remote_read(&mut self, rid: u64, name: &str) -> Result<CrowValue, String> {
        let answer = self.remote()?.post("read", serde_json::json!({ "session": rid, "name": name }))?;
        if let Some(j) = answer.get("fact") {
            return Ok(match (self.frames[0].scopes[0].get(name).cloned(), self.fact_types.get(name)) {
                (Some(declared), Some(ty)) => crate::served::json_to_typed(j, ty, self, &declared),
                (Some(declared), None) => crate::served::json_to_fact(j, &declared),
                (None, _) => crate::builtins::json_to_crow("", j),
            });
        }
        let d = answer.get("derived").ok_or_else(|| format!("the compiled program has no '{}'", name))?;
        if d.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let reason = d.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let kind = d.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            return Ok(CrowValue::error_with_kind(reason, kind, &self.trace_entry()));
        }
        let value = d.get("value").cloned().unwrap_or(serde_json::Value::Null);
        let ty = self.derives.get(name).map(|d| d.ty.payload().clone()).unwrap_or(Ty::Unknown);
        Ok(match ty {
            Ty::Frame(_) => match serde_json::from_value::<crow_runtime::frame::CrowFrame>(value.clone()) {
                Ok(f) => CrowValue::ok(Value::Frame(f)),
                Err(_) => crate::builtins::json_to_crow("", &value),
            },
            Ty::Float => CrowValue::ok(Value::Float(value.as_f64().unwrap_or(0.0))),
            _ => crate::builtins::json_to_crow("", &value),
        })
    }

    fn remote_downloads(&mut self, rid: u64) -> Result<Vec<(String, Vec<u8>)>, String> {
        let answer = self.remote()?.post("downloads", serde_json::json!({ "session": rid }))?;
        Ok(answer
            .get("files")
            .and_then(|f| f.as_array())
            .map(|files| {
                files
                    .iter()
                    .map(|f| {
                        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let text = f.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        (name, text.into_bytes())
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn remote_mails(&mut self, rid: u64) -> Result<Vec<crow_web::live::SentMail>, String> {
        let answer = self.remote()?.post("mails", serde_json::json!({ "session": rid }))?;
        let text = |m: &serde_json::Value, k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok(answer
            .get("mails")
            .and_then(|m| m.as_array())
            .map(|ms| ms.iter().map(|m| crow_web::live::SentMail { to: text(m, "to"), subject: text(m, "subject"), body: text(m, "body") }).collect())
            .unwrap_or_default())
    }

    /// A session's page: rendered here, or by the compiled program.
    fn session_html(&mut self, view: &str, id: u64) -> Result<String, String> {
        if let Some(rid) = self.remote_sessions.get(&id).copied() {
            let answer = self.remote()?.post("html", serde_json::json!({ "session": rid }))?;
            return Ok(answer.get("html").and_then(|h| h.as_str()).unwrap_or("").to_string());
        }
        self.activate(id);
        self.render_view(view)
    }

    // ── the harness and the live layer's mocks ─────────────────────

    /// A session's view rendered to markup.
    pub(crate) fn render_view(&mut self, view: &str) -> Result<String, String> {
        self.tick += 1;
        self.notice_read = false;
        self.paged_tables = 0;
        let r = self.call_user(view, Vec::new())?;
        let html = match &r.value {
            Value::Str(s) => s.clone(),
            other => format!("{}", other),
        };
        // a page that shows no notice of its own gets the runtime's toast,
        // as the served page does, while the notice is fresh
        if !self.notice_read && self.harness_toast {
            let id = self.active_session.unwrap_or(0);
            if let Some((text, at)) = self.session_notice.get(&id) {
                if !text.is_empty() && crow_runtime::moments::now() - at < 4.0 {
                    let toast = format!("<div class=\"crow-notice\" role=\"status\">{}</div>", crate::harness::html_escape_text(text));
                    return Ok(match html.rfind("</") {
                        Some(i) if html.ends_with('>') => format!("{}{}{}", &html[..i], toast, &html[i..]),
                        _ => format!("<div>{}{}</div>", html, toast),
                    });
                }
            }
        }
        Ok(html)
    }

    fn harness(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let app_view = |v: &CrowValue| -> Option<(FnValue, f64, bool, u64)> {
            if let Value::App(f, w, wide, id) = &v.value { Some((f.clone(), *w, *wide, *id)) } else { None }
        };
        Ok(match name {
            // recorded for `crow run -i`, which serves what they ask for
            "page" => {
                let path = vals.first().map(|v| format!("{}", v.value)).unwrap_or_else(|| "/".to_string());
                let view = match vals.get(1).map(|v| &v.value) {
                    Some(Value::Fn(f)) => f.name.clone(),
                    _ => return Err("page[] takes a path and a view: page['/', render]".into()),
                };
                let url = match vals.get(2).map(|v| &v.value) {
                    Some(Value::Array(items)) => items.iter().map(|i| format!("{}", i.value)).collect(),
                    _ => Vec::new(),
                };
                self.page_config = Some((path, view, url));
                CrowValue::done()
            }
            "serve" => {
                self.serve_config = Some(vals);
                CrowValue::done()
            }
            "fetch" | "poll" => CrowValue::ok(Value::Map(std::collections::BTreeMap::new())),
            "app" => {
                let f = match vals.first().map(|v| &v.value) {
                    Some(Value::Fn(f)) => f.clone(),
                    _ => return Err("app[] takes a view name: app[render] (and an optional width = 390, wide = true)".into()),
                };
                let mut width = 1280.0;
                let mut wide = false;
                for a in &vals[1..] {
                    match &a.value {
                        Value::Int(n) => width = *n as f64,
                        Value::Float(x) => width = *x,
                        Value::Bool(b) => wide = *b,
                        _ => return Err("app[] takes a view name, then width = 390 and wide = true".into()),
                    }
                }
                self.next_session += 1;
                let id = self.next_session;
                let opens_remote = crate::compiled_harness::active();
                if self.active_session.is_none() {
                    // the first session is the test's own state: it takes the
                    // facts as they are, and every later session starts where
                    // this one started; the test's clock starts now and moves
                    // only when the test moves it
                    self.session_start = self.fact_values();
                    self.active_session = Some(id);
                    // a rehearsal's clock stands still at its epoch until
                    // the test moves it, so a scenario stamps the same
                    let start = if crate::rehearsal::active().is_some() { REHEARSAL_EPOCH } else { crow_runtime::moments::clock() };
                    self.session_clock.insert(id, start);
                    crow_runtime::moments::hold(Some(start));
                } else {
                    self.sessions.insert(id, self.session_start.clone());
                    let start = self.session_clock.values().copied().fold(f64::INFINITY, f64::min);
                    self.session_clock.insert(id, start);
                }
                if opens_remote {
                    self.remote_open(&f.name, id)?;
                }
                CrowValue::ok(Value::App(f, width, wide, id))
            }
            // downloads[a]: the files the session was handed, as a reader's
            // browser saved them
            "downloads" => {
                let (_, _, _, id) = vals.first().and_then(app_view).ok_or("downloads[] takes an app session: downloads[a]")?;
                let list = match self.remote_sessions.get(&id).copied() {
                    Some(rid) => self.remote_downloads(rid)?,
                    None => self.session_downloads.get(&id).cloned().unwrap_or_default(),
                };
                let items = list
                    .into_iter()
                    .map(|(name, bytes)| {
                        let mut m = std::collections::BTreeMap::new();
                        m.insert("name".to_string(), CrowValue::ok(Value::Str(name)));
                        m.insert("text".to_string(), CrowValue::ok(Value::Str(String::from_utf8_lossy(&bytes).to_string())));
                        CrowValue::ok(Value::Map(m))
                    })
                    .collect();
                CrowValue::ok(Value::Array(items))
            }
            // mails[a]: the messages the session sent, which a test never sends
            "mails" => {
                let (_, _, _, id) = vals.first().and_then(app_view).ok_or("mails[] takes an app session: mails[a]")?;
                let list = match self.remote_sessions.get(&id).copied() {
                    Some(rid) => self.remote_mails(rid)?,
                    None => self.session_mails.get(&id).cloned().unwrap_or_default().into_iter().map(|(m, _)| m).collect(),
                };
                let items = list
                    .into_iter()
                    .map(|m| {
                        let mut map = std::collections::BTreeMap::new();
                        map.insert("to".to_string(), CrowValue::ok(Value::Str(m.to)));
                        map.insert("subject".to_string(), CrowValue::ok(Value::Str(m.subject)));
                        map.insert("body".to_string(), CrowValue::ok(Value::Str(m.body)));
                        CrowValue::ok(Value::Map(map))
                    })
                    .collect();
                CrowValue::ok(Value::Array(items))
            }
            "html" => {
                let (f, _, _, id) = vals.first().and_then(app_view).ok_or("html[] takes an app session: html[a]")?;
                CrowValue::ok(Value::Str(self.session_html(&f.name, id)?))
            }
            "elements" => {
                let (f, _, _, id) = vals.first().and_then(app_view).ok_or("elements[] takes an app session and a selector: elements[a, 'button']")?;
                let selector = match vals.get(1).map(|v| &v.value) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err("elements[] selector must be a string like 'button', '.card', or 'div.stats .card'".into()),
                };
                let html = self.session_html(&f.name, id)?;
                let tree = crate::harness::harness_parse_html(&html);
                let steps = selector_steps(&selector);
                if steps.is_empty() {
                    return Err("elements[] selector is empty".into());
                }
                let mut hits = Vec::new();
                crate::harness::harness_select(&tree, &steps, 0, &mut hits);
                CrowValue::ok(Value::Array(hits))
            }
            "change" => {
                let id = match (vals.len(), &vals.first().map(|v| &v.value)) {
                    (3, Some(Value::App(_, _, _, id))) => *id,
                    _ => return Err("change[] first argument must be an app session: change[a, 'min_qty', 5]".into()),
                };
                self.activate(id);
                let var = match &vals[1].value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("change[] names the reactive variable as a string: change[a, 'min_qty', 5]".into()),
                };
                let value = vals[2].clone();
                // a reader who signed in, as a handler's sign_in would say
                if var == "reader" {
                    let m = match &value.value {
                        Value::Map(m) if m.values().all(|v| matches!(v.value, Value::Str(_))) => m.clone(),
                        other => return Err(format!("change[a, 'reader', record] takes the reader as a map of strings, not {}", other.type_name())),
                    };
                    if let Some(rid) = self.remote_sessions.get(&id).copied() {
                        let record: serde_json::Map<String, serde_json::Value> =
                            m.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(format!("{}", v.value)))).collect();
                        self.remote()?.post("reader", serde_json::json!({ "session": rid, "record": record }))?;
                    }
                    self.session_reader.insert(id, m);
                    return Ok(CrowValue::done());
                }
                // time is an input: the session's clock is set as a field is
                if var == "now" {
                    let t = match &value.value {
                        Value::Float(f) => *f,
                        Value::Int(n) => *n as f64,
                        other => return Err(format!("change[a, 'now', t] takes a moment — a number of seconds — not {}", other.type_name())),
                    };
                    self.session_clock.insert(id, t);
                    crow_runtime::moments::hold(Some(t));
                    if let Some(rid) = self.remote_sessions.get(&id).copied() {
                        self.remote()?.post("clock", serde_json::json!({ "session": rid, "now": t }))?;
                    }
                    return Ok(CrowValue::done());
                }
                let current = self.frames[0].scopes[0].get(&var).cloned().ok_or_else(|| format!("change[]: no reactive variable named '{}' (reactive state is a top-level assignment)", var))?;
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
                if let Some(rid) = self.remote_sessions.get(&id).copied() {
                    self.remote()?.post("field", serde_json::json!({ "session": rid, "name": var, "value": crate::served::fact_to_json(&value) }))?;
                    self.remote_pull(id)?;
                    return Ok(CrowValue::done());
                }
                self.set_fact(&var, value);
                CrowValue::done()
            }
            "click" => {
                let id = match (vals.len(), &vals.first().map(|v| &v.value)) {
                    (2, Some(Value::App(_, _, _, id))) => *id,
                    _ => return Err("click[] first argument must be an app session: click[a, go]".into()),
                };
                self.activate(id);
                let (name, args) = match &vals[1].value {
                    Value::Fn(f) => (f.name.clone(), Vec::new()),
                    Value::Array(items) if matches!(items.first().map(|v| &v.value), Some(Value::Fn(_))) => {
                        let name = if let Value::Fn(f) = &items[0].value { f.name.clone() } else { unreachable!() };
                        (name, items[1..].to_vec())
                    }
                    _ => return Err("click[] second argument must be a handler name or call: click[a, go] or click[a, go[x]]".to_string()),
                };
                if self.remote_sessions.contains_key(&id) {
                    return self.remote_click(id, &name, &args);
                }
                self.transaction(&name, args)?
            }
            "box" | "fits" | "style" => {
                if vals.len() != 2 {
                    return Err(format!("{}[] takes an app session and a selector or width", name));
                }
                let (f, session_width, session_wide, id) = app_view(&vals[0]).ok_or_else(|| format!("{}[] takes an app session first", name))?;
                let arg1 = vals[1].clone();
                let html = self.session_html(&f.name, id)?;
                let tree = crate::harness::harness_parse_html(&html);
                if name == "style" {
                    let selector = match &arg1.value {
                        Value::Str(s) => s.clone(),
                        _ => return Err("style[] selector must be a string".into()),
                    };
                    let steps = selector_steps(&selector);
                    let mut hits: Vec<CrowValue> = Vec::new();
                    crate::harness::harness_select(&tree, &steps, 0, &mut hits);
                    let mut out = Vec::new();
                    for h in hits {
                        if let Value::Map(m) = &h.value {
                            out.push(style_tokens(m));
                        }
                    }
                    return Ok(CrowValue::ok(Value::Array(out)));
                }
                self.ensure_engine()?;
                let viewport = if name == "fits" {
                    match &arg1.value {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => return Err("fits[] takes a width: fits[s, 390]".into()),
                    }
                } else {
                    session_width
                };
                let root = crate::harness::harness_to_lnode(&tree);
                let boxes = self.call_user("crow_layout", vec![root, CrowValue::ok(Value::Float(viewport)), CrowValue::ok(Value::Bool(session_wide))])?;
                let list = match &boxes.value {
                    Value::Array(items) => items.clone(),
                    _ => Vec::new(),
                };
                if name == "fits" {
                    let any_overflow = list.iter().any(|b| match &b.value {
                        Value::StructInstance(si) => matches!(si.fields.get("overflow").map(|v| &v.value), Some(Value::Bool(true))),
                        _ => false,
                    });
                    return Ok(CrowValue::ok(Value::Bool(!any_overflow)));
                }
                let selector = match &arg1.value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("box[] selector must be a string like 'button' or 'div.stats .card'".into()),
                };
                let steps = selector_steps(&selector);
                let hits: Vec<CrowValue> = list
                    .into_iter()
                    .filter(|b| {
                        if let Value::StructInstance(si) = &b.value {
                            let path = si.fields.get("path").map(|v| format!("{}", v.value)).unwrap_or_default();
                            crate::harness::harness_path_matches(&path, &steps)
                        } else {
                            false
                        }
                    })
                    .collect();
                CrowValue::ok(Value::Array(hits))
            }
            _ => return Err(format!("the IR interpreter does not run '{}' yet", name)),
        })
    }
}

/// A selector's steps: `tag`, `.class`, `tag.class`, space-separated.
fn selector_steps(selector: &str) -> Vec<(String, String)> {
    selector
        .split_whitespace()
        .map(|step| match step.find('.') {
            Some(i) => (step[..i].to_string(), step[i + 1..].to_string()),
            None => (step.to_string(), String::new()),
        })
        .collect()
}

/// The resolved tokens of a rendered element, from its class and style.
fn style_tokens(m: &std::collections::BTreeMap<String, CrowValue>) -> CrowValue {
    let cls = m.get("class").map(|v| format!("{}", v.value)).unwrap_or_default();
    let style = m.get("style").map(|v| format!("{}", v.value)).unwrap_or_default();
    let mut background = String::new();
    let mut color = String::from("--crow-text");
    let mut size = String::from("m");
    let mut weight = String::from("regular");
    for c in cls.split_whitespace() {
        match c {
            "card" => background = "--crow-surface".into(),
            "primary" => {
                background = "--crow-accent".into();
                color = "--crow-bg".into();
            }
            "muted" => color = "--crow-muted".into(),
            "title" => {
                size = "xl".into();
                weight = "bold".into();
            }
            "label" => {
                size = "s".into();
                color = "--crow-muted".into();
            }
            _ => {}
        }
    }
    for decl in style.split(';') {
        if let Some((k, v)) = decl.split_once(':') {
            let v = v.trim();
            match k.trim() {
                "color" => color = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                "background" => background = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                "font-size" => {
                    size = match v {
                        "12px" => "xs",
                        "14px" => "s",
                        "16px" => "m",
                        "20px" => "l",
                        "24px" => "xl",
                        "32px" => "xxl",
                        _ => "m",
                    }
                    .to_string()
                }
                "font-weight" => weight = match v { "700" => "bold", "500" => "medium", _ => "regular" }.to_string(),
                _ => {}
            }
        }
    }
    let mut r = std::collections::BTreeMap::new();
    r.insert("background".to_string(), CrowValue::ok(Value::Str(background)));
    r.insert("color".to_string(), CrowValue::ok(Value::Str(color)));
    r.insert("size".to_string(), CrowValue::ok(Value::Str(size)));
    r.insert("weight".to_string(), CrowValue::ok(Value::Str(weight)));
    CrowValue::ok(Value::Map(r))
}

fn binop_of(op: &str) -> Result<BinOp, String> {
    Ok(match op {
        "add" => BinOp::Add,
        "sub" => BinOp::Sub,
        "mul" => BinOp::Mul,
        "div" => BinOp::Div,
        "mod" => BinOp::Mod,
        "pow" => BinOp::Pow,
        "elemmul" => BinOp::ElemMul,
        "elemdiv" => BinOp::ElemDiv,
        "elempow" => BinOp::ElemPow,
        "eq" => BinOp::Eq,
        "ne" => BinOp::Ne,
        "gt" => BinOp::Gt,
        "lt" => BinOp::Lt,
        "gte" => BinOp::Gte,
        "lte" => BinOp::Lte,
        "and" => BinOp::And,
        "or" => BinOp::Or,
        other => return Err(format!("unknown operator '{}'", other)),
    })
}

/// `x.f = v`, `xs[i] = v`, and their nestings, applied to a value in place.
fn set_path(root: &mut Value, path: &[PathStep], idx: &[CrowValue], new: CrowValue) -> Result<(), String> {
    fn step<'v>(cur: &'v mut Value, p: &PathStep, idx: &mut std::slice::Iter<'_, CrowValue>) -> Result<&'v mut CrowValue, String> {
        match p {
            PathStep::Field(f) => match cur {
                Value::StructInstance(si) => si.fields.get_mut(f).ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, f)),
                Value::Map(m) => m.get_mut(f).ok_or_else(|| format!("map has no key '{}'", f)),
                other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
            },
            PathStep::Index(_) => {
                let i = idx.next().ok_or("missing index")?;
                match cur {
                    Value::Array(items) => match &i.value {
                        Value::Int(n) => {
                            let len = items.len() as i64;
                            let r = if *n < 0 { len + n } else { *n };
                            if r < 0 || r >= len {
                                return Err(format!("index {} out of bounds (len {})", n, items.len()));
                            }
                            Ok(&mut items[r as usize])
                        }
                        _ => Err("array index must be an integer".to_string()),
                    },
                    Value::Map(m) => match &i.value {
                        Value::Str(k) => m.get_mut(k).ok_or_else(|| format!("map has no key '{}'", k)),
                        _ => Err("map key must be a string".to_string()),
                    },
                    other => Err(format!("cannot index-assign into {}", other.type_name())),
                }
            }
        }
    }
    let mut it = idx.iter();
    let (last, front) = path.split_last().ok_or("empty path")?;
    let mut cur: &mut Value = root;
    for p in front {
        cur = &mut step(cur, p, &mut it)?.value;
    }
    match last {
        PathStep::Field(f) => match cur {
            Value::StructInstance(si) => {
                if !si.fields.contains_key(f) {
                    return Err(format!("struct '{}' has no field '{}'", si.name, f));
                }
                si.fields.insert(f.clone(), new);
                Ok(())
            }
            Value::Map(m) => {
                crate::builtins::check_map_homogeneous(m, &new)?;
                m.insert(f.clone(), new);
                Ok(())
            }
            other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
        },
        PathStep::Index(_) => {
            let i = it.next().ok_or("missing index")?;
            match cur {
                Value::Array(items) => match &i.value {
                    Value::Int(n) => {
                        let len = items.len() as i64;
                        let r = if *n < 0 { len + n } else { *n };
                        if r < 0 || r >= len {
                            return Err(format!("index {} out of bounds (len {})", n, items.len()));
                        }
                        items[r as usize] = new;
                        Ok(())
                    }
                    _ => Err("array index must be an integer".to_string()),
                },
                Value::NumArray(t) => match (&i.value, &new.value) {
                    (Value::Int(n), Value::Int(_) | Value::Float(_)) => {
                        let mut data = t.to_vec();
                        let len = data.len() as i64;
                        let r = if *n < 0 { len + n } else { *n };
                        if r < 0 || r >= len {
                            return Err(format!("index {} out of bounds (len {})", n, data.len()));
                        }
                        data[r as usize] = match &new.value {
                            Value::Int(x) => *x as f64,
                            Value::Float(x) => *x,
                            _ => unreachable!(),
                        };
                        let shape = t.shape().to_vec();
                        *t = crow_runtime::tensor::CrowTensor::from_vec(data, shape);
                        Ok(())
                    }
                    _ => Err("a numeric array takes a number at an integer index".to_string()),
                },
                Value::Map(m) => match &i.value {
                    Value::Str(k) => {
                        crate::builtins::check_map_homogeneous(m, &new)?;
                        m.insert(k.clone(), new);
                        Ok(())
                    }
                    _ => Err("map key must be a string".to_string()),
                },
                other => Err(format!("cannot index-assign into {}", other.type_name())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Vec<crate::ast::Stmt> {
        crate::build(src).unwrap()
    }

    /// A parallel loop clones the walker per chunk; the clone must share
    /// the scopes (a per-item copy of every value was the benchmark's
    /// finding against the AST walker) and a write must copy only the
    /// scope it touches.
    #[test]
    fn a_clone_shares_its_scopes_until_written() {
        let a = run_script(&parse("xs = for i in range[1000]\n    'item`i`'\ny = 1\n"), "t").unwrap();
        let mut b = a.clone();
        assert!(Arc::ptr_eq(&a.frames[0].scopes[0], &b.frames[0].scopes[0]));
        b.bind("y", CrowValue::ok(Value::Int(2)));
        assert!(!Arc::ptr_eq(&a.frames[0].scopes[0], &b.frames[0].scopes[0]));
        assert!(matches!(a.var("y").unwrap().value, Value::Int(1)));
        assert!(matches!(b.var("y").unwrap().value, Value::Int(2)));
    }

    /// A local's field indexed in place reads what the copy read: a frame's
    /// cell (a missing one its column's placeholder), a struct's or map's
    /// item, from the end with a negative index, and out of range refused.
    #[test]
    fn a_field_indexed_in_place_reads_what_the_copy_read() {
        let src = "use frame\nstruct Bars [c]\ndf = with[with[frame.new[], 'k', [0, 1, 2]], 'p', [1.5, 2.5, 3.5]]\nother = with[with[frame.new[], 'k', [0, 1]], 'q', [7.5, 8.5]]\ndf = join[df, other, 'k', 'left']\nb = Bars[[4.0, 5.0, 6.0]]\nm = map[xs = [1, 2, 3]]\na = df.p[1]\ng = df.q[2]\nz = b.c[-1]\nw = m.xs[0]\nn = b.c[1] + df.p[0]\n";
        let r = run_script(&parse(src), "t").unwrap();
        assert!(matches!(r.var("a").unwrap().value, Value::Float(x) if x == 2.5));
        assert!(matches!(r.var("g").unwrap().value, Value::Float(x) if x.is_nan()), "{:?}", r.var("g"));
        assert_eq!(format!("{}", r.var("z").unwrap().value), "6");
        assert!(matches!(r.var("w").unwrap().value, Value::Int(1)));
        assert!(matches!(r.var("n").unwrap().value, Value::Float(x) if x == 6.5));
        let far = run_script(&parse("struct Bars [c]\nb = Bars[[4.0, 5.0]]\nx = b.c[2]\n"), "t");
        assert!(far.is_err_and(|e| e.contains("index 2 out of bounds for array of length 2")));
    }
}
