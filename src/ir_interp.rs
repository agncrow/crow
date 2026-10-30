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

/// Scalar and tensor names both core and math answer: scalars go to the
/// core builtin, as the AST walker routes them.

/// A call of the core's `len`: by the row the IR names, or by the name its
/// values pick a row for.
fn is_len(target: &Target) -> bool {
    match target {
        Target::Row(row) => row.module.is_empty() && row.name == "len",
        Target::Runtime(name) => name == "len",
        _ => false,
    }
}

enum Flow {
    Next,
    Return(CrowValue),
    Break,
    Continue,
}

/// How the walker's own maps hash a name: cheaply (FxHash's rotate and
/// multiply), since their keys are the program's names, not outside input.
#[derive(Default, Clone, Copy)]
pub(crate) struct NameHasher(u64);

impl std::hash::Hasher for NameHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

impl NameHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub(crate) type Names = std::hash::BuildHasherDefault<NameHasher>;

/// A scope is shared until written (a parallel loop clones the walker per
/// chunk of items; the clone costs one reference per scope).
type Scope = Arc<HashMap<String, CrowValue, Names>>;

mod prepared;

#[derive(Clone)]
struct Frame {
    scopes: Vec<Scope>,
    /// A function frame's block bindings, flat: a name bound in a block is
    /// pushed, the block's end truncates to its mark, and a lookup scans a
    /// handful of short names from the end — a scope per block had been a
    /// hash map, and a lookup a hash per block of the chain (the main
    /// frame keeps its maps: its first scope is the facts, shared with the
    /// sessions until written).
    locals: Vec<(String, CrowValue)>,
    marks: Vec<usize>,
    /// The function the frame runs, named as it was called (a trace and a
    /// refusal say it). Its own copy: a name shared between the threads of
    /// a parallel loop is a reference count every call of every thread
    /// writes, slower than the copy.
    fn_name: Arc<str>,
    /// A prepared body's locals (prepared.rs), each binding its slot,
    /// decided before the function runs; empty for a frame the walker runs.
    slots: Vec<CrowValue>,
    /// While the walker runs an IR node of a prepared body: the names in
    /// scope there, each to its slot.
    env: Option<prepared::Env>,
    /// What is known of the start of each text a prepared body reads by
    /// position (prepared.rs, `text_slots`): one per such text, none yet.
    texts: Vec<crow_runtime::text::Known>,
}

#[derive(Clone)]
pub struct IrInterp {
    /// How many recursive functions' calls are nested now (the limit is
    /// crow_runtime::stops::DEPTH).
    depth: usize,
    fns: HashMap<String, Arc<Fn>, Names>,
    /// Each function's parameters a call may lend the caller's value
    /// (`ir::lendable_params`), decided over the program as it is loaded.
    lendable: Arc<HashMap<String, Vec<bool>>>,
    /// Each function's body prepared for the walker (prepared.rs), made on
    /// its first call.
    prepared: HashMap<String, Arc<prepared::Prepared>, Names>,
    /// Register files a returned call left, for the next call to take
    /// (prepared.rs): a call had allocated four and freed them.
    reg_pool: prepared::Pool,
    derives: HashMap<String, Arc<Derive>>,
    structs: HashMap<String, Arc<crate::ir::StructDef>>,
    /// Each record declaration's layout, shared by its values, and where
    /// each field in declaration order lies in it (`value::Layout`).
    layouts: HashMap<String, Shape>,
    registry: ModuleRegistry,
    frames: Vec<Frame>,
    /// Scopes blocks left behind, for the next block (see `push_scope`).
    scope_pool: Vec<Scope>,
    line: usize,
    file: String,
    /// The value of the last expression statement, what a collecting loop yields.
    last_value: Option<CrowValue>,
    /// Inside a view being rendered: bound inputs render, elements are markup.
    in_view: usize,
    /// While the walker runs a loop: what is known of the texts its rounds
    /// read by position (`slice[s, a, b]`, `s[i]`), by frame and name —
    /// let go when the name is written, and when the loop ends, however it
    /// ends. A fresh `Known` at every read had walked a long text from its
    /// start each time: revcomp's 60-character pieces of a 50 MB sequence
    /// took 11 trillion instructions.
    texts_known: Vec<(usize, String, Arc<crow_runtime::text::Known>)>,
    walker_loops: usize,
    /// Inside a function `grad` differentiates over tensors: its loops run
    /// in order, since one tape records the run.
    tracing: bool,
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
    /// The page's data sources as the template met them: the fact, its
    /// URL, its period, and what it is held to (`Fact::held`) — what a
    /// served app registers, as the compiled program does.
    pub(crate) sources: Vec<(String, String, Option<u64>, Vec<(String, String, String)>)>,
    /// The source the fact being set declares: `fetch`/`poll` just evaluated.
    pending_source: Option<(String, Option<u64>)>,
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
    /// The column each sorted table of the served session is sorted by, and
    /// whether descending (by its place among the render's sorted tables); a
    /// test sees the frame's own order.
    pub(crate) session_sorts: HashMap<usize, (String, bool)>,
    /// Sorted tables drawn so far in the render under way.
    sorted_tables: usize,
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
    /// What each `page[…]` asked for, in order: the path, the view, the url
    /// facts. `crow run -i` serves every one.
    pub(crate) pages: Vec<(String, String, Vec<String>)>,
    /// What each `route[…]` asked for, in order: the method, the path, the
    /// handler. `crow run -i` answers every one.
    /// method, path, handler and the requests a minute each client may
    /// make (0: no limit)
    pub(crate) routes: Vec<(String, String, String, u64)>,
    /// The program uses web: `text`, `html`, `json`, `respond`, `redirect`
    /// and `bytes` make a route's response.
    web: bool,
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
    /// A route's request under way (`begin_route`): the derivations it has
    /// read, from the app's values — each once for the request.
    route_reads: Option<HashMap<String, CrowValue>>,
    /// The app's own state, which a served program's routes read its
    /// derivations through (crow_web::live), and the program's tag among
    /// the app's values — two programs a process serves share none.
    pub(crate) app_state: Option<Arc<crow_web::live::CrowLiveState>>,
    pub(crate) app_tag: u64,
    /// Whether the effect being run has its answer used (`Effect.answered`):
    /// `sent = mail[…]` sends at once, `mail[…]` waits for the commit.
    answering: bool,
    /// A test block's run: a message is recorded, never sent.
    pub(crate) testing: bool,
    /// The messages each session sent, as a test reads them (`mails[a]`),
    /// each with its mailer's place in `MAILERS`.
    pub(crate) session_mails: HashMap<u64, Vec<(crow_web::live::SentMail, CrowValue)>>,
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
    Mail(crow_web::live::SentMail, CrowValue),
}

/// The mailers `smtp[…]` made. A mailer value names its place here, so its
/// password is never part of a value a program can print.

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

/// A holder of ints as one of floats, at any depth (`Convert::IntsToFloats`;
/// an error stays as it is): a list's items, a map's values, a list of lists'
/// lists. (An int-keyed map had been left as it was.)
fn floats_held(v: CrowValue) -> CrowValue {
    if !v.is_ok() {
        return v;
    }
    match &v.value {
        Value::Int(n) => CrowValue::ok(Value::Float(*n as f64)),
        Value::Big(n) => CrowValue::ok(Value::Float(n.to_f64().value())),
        Value::Array(items) => CrowValue::ok(Value::Array(items.iter().map(|i| floats_held(i.clone())).collect::<Vec<_>>().into())),
        Value::Map(m) => CrowValue::ok(Value::map(m.iter().map(|(k, i)| (k.clone(), floats_held(i.clone()))).collect())),
        Value::IntMap(m) => CrowValue::ok(Value::int_map(m.iter().map(|(k, i)| (*k, floats_held(i.clone()))).collect())),
        _ => v,
    }
}

fn placeholder(cell: CrowValue, ty: &Ty) -> CrowValue {
    match (cell.is_ok() || cell.kind() != "missing", ty) {
        (true, _) => cell,
        // a column that can have missing cells (a nullable database column,
        // a join's unmatched side): a missing one is an error
        (false, Ty::Result(_)) => {
            let mut missing = CrowValue::error_with_kind("missing", "missing", "");
            missing.status_mut().trace.clear();
            missing
        }
        (false, Ty::Float) => CrowValue::ok(Value::Float(f64::NAN)),
        (false, Ty::Int) => CrowValue::ok(Value::Int(0)),
        (false, Ty::Bool) => CrowValue::ok(Value::Bool(false)),
        // (a plain text column's placeholder is empty text, as the
        // column's cell read answers it; it had been the text "nan")
        (false, Ty::Str) => CrowValue::ok(Value::Str(String::new())),
        // a column nothing typed: its missing cell is JSON's null, an error
        // value of reason 'null', as `json[]` reads one
        (false, Ty::Json) => {
            let mut null = CrowValue::error_with_kind("null", "parse", "");
            null.status_mut().trace.clear();
            null
        }
        (false, _) => cell,
    }
}

/// How a mailer sends one message, from wherever it is asked.

/// How a stop travels out of a handler, through every helper, to the
/// harness verb that ran it: an error the walker never shows.
const STOP: &str = "\u{1}stop\u{1}";

/// How a return from inside a collecting loop, an expression, travels to
/// the function it leaves; the value waits in `returning`.
const RETURN: &str = "\u{1}return\u{1}";

/// The function `stream[feed]` hands its feed, as a value: a name no
/// program's function can have.
const PUT: &str = "<put>";

/// The pipeline every runner shares: the front end's lowering, inference,
/// then the lowering to the IR.
/// Whether the interpreter walks every body (`CROW_WALK=1`), its prepared
/// form and parallel loops off: a second interpreter for `crow check
/// --self`, which every program must agree with.
fn walk_only() -> bool {
    static WALK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WALK.get_or_init(|| std::env::var("CROW_WALK").is_ok_and(|v| !v.is_empty() && v != "0"))
}

pub fn prepare(stmts: &[crate::ast::Stmt], name: &str) -> Result<(Program, crate::type_infer::TypeContext), String> {
    prepare_with(stmts, name, &std::collections::BTreeSet::new())
}

/// `prepare`, with the program's top-level names the statements leave out
/// (a test lowered over the declarations alone): a read of one is refused
/// by name (lower_ir/checks.rs, `check_ident`).
pub fn prepare_with(stmts: &[crate::ast::Stmt], name: &str, left_out: &std::collections::BTreeSet<String>) -> Result<(Program, crate::type_infer::TypeContext), String> {
    let stmts = crate::lower::lower_program(stmts)?;
    let mut ctx = crate::type_infer::TypeContext::new();
    ctx.left_out = left_out.clone();
    // a test's program types its queries' columns as the program does
    // (crate::describing): with them off, every cell of a test read as
    // dynamic, or as text before that, while `crow run` typed it
    ctx.describe_queries = crate::describing();
    ctx.infer_program(&stmts);
    // the gate every pipeline passes: the lowering refuses what `crow run`
    // refuses
    let program = crate::lower_ir::lower(&stmts, &mut ctx, name)?;
    Ok((program, ctx))
}

/// A script run for its variables.
pub fn run_script(stmts: &[crate::ast::Stmt], name: &str) -> Result<IrInterp, String> {
    let (program, ctx) = prepare(stmts, name)?;
    let mut it = IrInterp::new(&ctx.registry, name);
    it.load(&program);
    it.guard(|it| it.run_main(&program))?;
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

    /// A fragment fed after `flag` is set stops at its next loop turn
    /// (a notebook's interrupt), as a served session's cancelled work does.
    pub fn cancel_with(&mut self, flag: Arc<AtomicBool>) {
        self.it.cancel = Some(flag);
    }

    /// A name the fragments so far bound (a crowfile's key).
    pub fn var(&self, name: &str) -> Option<&CrowValue> {
        self.it.var(name)
    }

    pub fn feed(&mut self, stmts: &[crate::ast::Stmt]) -> Result<Option<CrowValue>, String> {
        let stmts = crate::lower::lower_program(stmts)?;
        self.ctx.infer_program(&stmts);
        let known: HashSet<String> = self.it.frames[0].scopes[0].keys().cloned().collect();
        // the functions the fragments before defined, which this one may
        // call: the lowering knows them by the context, and the check of
        // what it made by these (a call of one had been "a function the
        // program does not define", an internal error, in every cell after)
        let earlier: Vec<&Fn> = self.it.fns.values().map(|f| f.as_ref()).collect();
        let program = crate::lower_ir::lower_fragment(&stmts, &mut self.ctx, "repl", &known, &earlier)?;
        // a `use` in the fragment loaded its module into the context's registry
        self.it.registry = self.ctx.registry.clone();
        self.it.load(&program);
        self.it.last_value = None;
        self.it.guard(|it| it.run_main(&program))?;
        let value = self.it.last().cloned();
        self.it.last_value = None;
        Ok(value)
    }
}

/// Run a program: its facts get their defaults, then its main block runs.
pub fn run(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<(), String> {
    let mut it = IrInterp::new(registry, source_file);
    it.load(program);
    it.guard(|it| it.run_main(program))
}

/// Run a program that serves pages: its facts get their defaults and its
/// main runs, recording what `page` and `serve` ask for; the interpreter
/// that ran it is the template every session's render and handler copy.
pub fn run_to_serve(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<IrInterp, String> {
    let mut it = IrInterp::new(registry, source_file);
    it.load(program);
    it.guard(|it| it.run_main(program))?;
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
    if let Err(e) = it.guard(|it| it.run_main(&program)) {
        crow_runtime::moments::hold(None);
        return (Err(format!("a declaration failed before the test ran: {}", e)), vec![], vec![]);
    }
    let before = it.fact_values();
    let Some(test) = program.parts.iter().flat_map(|p| p.tests.iter()).last() else {
        crow_runtime::moments::hold(None);
        return (Err("lowering: the test block is missing".into()), vec![], vec![]);
    };
    it.frames[0].scopes.push(Arc::new(HashMap::default()));
    let result = it.guard(|it| it.exec_stmts(&test.body).map(|_| ())).map_err(|e| it.with_line(e));
    crow_runtime::moments::hold(None);
    let after = it.fact_values();
    let mut changed: Vec<String> = after
        .iter()
        .filter(|(n, v)| before.get(*n).map_or(true, |b| format!("{}", b.value) != format!("{}", v.value) || b.is_ok() != v.is_ok()))
        .map(|(n, v)| {
            let text = if v.is_ok() { format!("{}", v.value) } else { crow_runtime::display::failed(v.reason(), v.trace()) };
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

/// A test block lowered with the program's declarations: its inputs typed
/// by their use and its truths asserted (the lowering's `lower_test`), the
/// last of the program's tests, which the runner reads for both.
pub fn prepare_test(decls: &[crate::ast::Stmt], left_out: &std::collections::BTreeSet<String>, label: &str, params: &[String], body: &[crate::ast::Stmt]) -> Result<(Program, crate::type_infer::TypeContext), String> {
    let mut all = decls.to_vec();
    // named, so the gate reads it as the block it was in its file (an
    // unnamed block is one attached to the function before it)
    all.push(crate::ast::Stmt::TestBlock { name: Some(label.to_string()), params: params.to_vec(), body: body.to_vec() });
    let (program, ctx) = prepare_with(&all, "test", left_out)?;
    // CROW_IR_SHOW=1 prints the lowered test program, as `crow build --ir` does for a file
    if std::env::var("CROW_IR_SHOW").is_ok() {
        eprintln!("{}", crate::ir::print(&program));
    }
    Ok((program, ctx))
}

/// A prepared test run with these inputs: the declarations first, then the
/// block, each run in an interpreter of its own.
pub fn run_prepared_test(program: &Program, ctx: &crate::type_infer::TypeContext, params: &[(String, CrowValue)]) -> Result<(), String> {
    let mut it = IrInterp::new(&ctx.registry, "test");
    // a test records what the program would send, and sends nothing
    it.testing = true;
    it.load(program);
    it.guard(|it| it.run_main(program)).map_err(|e| format!("a declaration failed before the test ran: {}", e))?;
    let test = program.parts.iter().flat_map(|p| p.tests.iter()).last().ok_or("lowering: the test block is missing")?;
    // the block runs at the program's top level, in a scope of its own: it
    // sees the declarations' variables as a caller would, and its inputs
    let mut scope: HashMap<String, CrowValue, Names> = HashMap::default();
    for (p, v) in params {
        scope.insert(p.clone(), v.clone());
    }
    it.frames[0].scopes.push(Arc::new(scope));
    let result = it.guard(|it| it.exec_stmts(&test.body));
    // the test's clock ends with the test
    crow_runtime::moments::hold(None);
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(it.with_line(e)),
    }
}

impl IrInterp {
    pub fn new(registry: &ModuleRegistry, source_file: &str) -> Self {
        // the program's names, from its top level on (a function entered
        // sets its own scope and resumes this one)
        crate::modules::ModuleRegistry::reset_scope();
        let file = std::path::Path::new(source_file)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| source_file.to_string());
        IrInterp {
            depth: 0,
            fns: HashMap::default(),
            lendable: Arc::new(HashMap::new()),
            prepared: HashMap::default(),
            reg_pool: prepared::Pool::default(),
            derives: HashMap::new(),
            structs: HashMap::new(),
            layouts: HashMap::new(),
            registry: registry.clone(),
            frames: vec![Frame { scopes: vec![Arc::new(HashMap::default())], locals: Vec::new(), marks: Vec::new(), fn_name: Arc::from(""), slots: Vec::new(), env: None, texts: Vec::new() }],
            scope_pool: Vec::new(),
            line: 0,
            file,
            last_value: None,
            in_view: 0,
            texts_known: Vec::new(),
            walker_loops: 0,
            // (`CROW_WALK=1`: every body walked and every loop run in order,
            // as a traced run is — the plain path `crow check --self` holds
            // the prepared form and the parallel loops to)
            tracing: walk_only(),
            tick: 0,
            engine_loaded: false,
            cancel: None,
            fact_names: Vec::new(),
            fact_types: HashMap::new(),
            sources: Vec::new(),
            pending_source: None,
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
            session_sorts: HashMap::new(),
            sorted_tables: 0,
            session_downloads: HashMap::new(),
            remote: None,
            remote_sessions: HashMap::new(),
            route_reads: None,
            app_state: None,
            app_tag: 0,
            answering: true,
            testing: false,
            session_mails: HashMap::new(),
            held: None,
            notice_read: false,
            pages: Vec::new(),
            routes: Vec::new(),
            web: false,
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
            // cpu[n] bounds the parallel loops
            if part.gpu.is_some() {
                crow_runtime::gpu::set_gpu_enabled(true);
            }
            if let Some(n) = part.cpu {
                if n >= 1 {
                    crow_runtime::cores::set_default(n as usize);
                }
            }
            // a page's draws seeded before its declarations are set
            for e in &part.seeds {
                self.eval(e).map_err(|e| self.with_line(e))?;
            }
            for fact in &part.facts {
                if fact.line > 0 {
                    self.line = fact.line;
                }
                let v = self.eval(&fact.default).map_err(|e| self.with_line(e))?;
                if fact.refuses && v.is_error() {
                    return Err(self.with_line(format!("{} — {}", Decision::Assign(fact.name.clone()).refusal(), v.reason())));
                }
                self.set_fact(&fact.name, v);
                if let Some((url, secs)) = self.pending_source.take() {
                    // the table with no rows yet, in the columns the program
                    // reads, until the session loads it
                    let names: Vec<String> = fact.held.iter().map(|(n, _, _)| n.clone()).collect();
                    let kinds: Vec<String> = fact.held.iter().map(|(_, k, _)| k.clone()).collect();
                    self.set_fact(&fact.name, CrowValue::ok(Value::Frame(Box::new(crow_runtime::frames::empty_with(&names, &kinds)))));
                    self.sources.push((fact.name.clone(), url, secs, fact.held.clone()));
                }
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
        self.web = program.parts.iter().any(|p| p.uses.iter().any(|u| u == "web" || u.starts_with("web.")));
        for part in &program.parts {
            for f in &part.fns {
                self.fns.insert(f.name.clone(), Arc::new(f.clone()));
                // (a session's cell that defines a function again: the body
                // prepared for the earlier one is let go, or it ran on)
                self.prepared.remove(&f.name);
            }
            for d in &part.derives {
                self.derives.insert(d.name.clone(), Arc::new(d.clone()));
            }
            for s in &part.structs {
                self.structs.insert(s.name.clone(), Arc::new(s.clone()));
                self.layouts.insert(s.name.clone(), layout_of(s));
            }
        }
        let fns: Vec<&Fn> = self.fns.values().map(|f| f.as_ref()).collect();
        self.lendable = Arc::new(crate::ir::lendable_params(&fns));
        // a program that carries a database driver written in Crow: what
        // a child interpreter is made from, for the driver's calls
        crate::driver::set_template(crate::driver::Template { fns: self.fns.clone(), structs: self.structs.clone(), registry: self.registry.clone(), file: self.file.clone() });
    }

    /// A child over a program's functions: the driver's calls run in it.
    pub(crate) fn from_template(t: &crate::driver::Template) -> Self {
        let mut it = IrInterp::new(&t.registry, &t.file);
        it.fns = t.fns.clone();
        it.structs = t.structs.clone();
        it.layouts = t.structs.iter().map(|(n, s)| (n.clone(), layout_of(s))).collect();
        it
    }

    /// A struct's fields and their types, as the program declared it.
    /// The program's functions that neither wait on anything nor loop
    /// (`ir::quick_fns`): a route's handler among them is answered on the
    /// server's own thread, as the compiled program's is.
    pub(crate) fn quick_fns(&self) -> std::collections::BTreeSet<String> {
        crate::ir::quick_fns(self.fns.values().map(|f| f.as_ref()))
    }

    pub(crate) fn struct_def(&self, name: &str) -> Option<Arc<crate::ir::StructDef>> {
        self.structs.get(name).cloned()
    }

    /// Whether the program declares a derivation.
    pub(crate) fn has_derivations(&self) -> bool {
        !self.derives.is_empty()
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
            Value::Array(items) => items.as_ref().clone(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for b in list {
            if let Value::StructInstance(si) = &b.value {
                let s = |k: &str| si.get(k).map(|v| format!("{}", v.value)).unwrap_or_default();
                let f = |k: &str| {
                    si.get(k).and_then(|v| match &v.value {
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
                    overflow: matches!(si.get("overflow").map(|v| &v.value), Some(Value::Bool(true))),
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

    /// Run part of the program so that a panic inside it — a check Crow is
    /// missing — is a stop on the line it happened on, never a crash: the
    /// same words as the compiled program's (crow_runtime::stops).
    pub(crate) fn guard<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        match crow_runtime::stops::guarded(|| f(self)) {
            Ok(r) => r,
            Err(words) => Err(self.with_line(words)),
        }
    }

    /// A stop's reason with the line it happened on, once: the innermost
    /// statement names it, as the compiled program's stop does. A control
    /// signal (a handler's stop, a return) passes as it is.
    fn with_line(&self, e: String) -> String {
        if self.line > 0 && !e.starts_with("line ") && !e.starts_with('\u{1}') {
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

    /// A block's scope: one from the pool where a block left one behind
    /// (a loop's body entered a scope every round, and each round had
    /// allocated it), else a new one.
    /// The end of a loop's round: what the round bound is dropped, the
    /// loop's variable kept (the first binding of the loop's scope).
    fn end_round(&mut self, var: &str) {
        if self.in_fn_frame() {
            let frame = self.frames.last_mut().unwrap();
            let mark = frame.marks.last().copied().unwrap_or(0);
            frame.locals.truncate(mark + 1);
            return;
        }
        Arc::make_mut(self.frames.last_mut().unwrap().scopes.last_mut().unwrap()).retain(|k, _| k == var);
    }

    /// Inside a function (a view, a handler): the frame's flat bindings;
    /// the main frame keeps its maps.
    fn in_fn_frame(&self) -> bool {
        self.frames.len() > 1
    }

    fn push_scope(&mut self) {
        if self.in_fn_frame() {
            let frame = self.frames.last_mut().unwrap();
            frame.marks.push(frame.locals.len());
            return;
        }
        let scope = self.scope_pool.pop().unwrap_or_else(|| Arc::new(HashMap::default()));
        self.frames.last_mut().unwrap().scopes.push(scope);
    }

    fn pop_scope(&mut self) {
        if self.in_fn_frame() {
            let frame = self.frames.last_mut().unwrap();
            if let Some(mark) = frame.marks.pop() {
                frame.locals.truncate(mark);
            }
            return;
        }
        if let Some(mut scope) = self.frames.last_mut().unwrap().scopes.pop() {
            // (a scope nothing else holds goes back to the pool, emptied
            // but keeping its table)
            if let Some(m) = Arc::get_mut(&mut scope) {
                m.clear();
                if self.scope_pool.len() < 64 {
                    self.scope_pool.push(scope);
                }
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<&CrowValue> {
        let frame = self.frames.last().unwrap();
        if let Some((_, v)) = frame.locals.iter().rev().find(|(n, _)| n == name) {
            return Some(v);
        }
        // a prepared body's names, for the IR node the walker runs for it
        if let Some(env) = &frame.env
            && let Some(s) = env.get(name)
        {
            return frame.slots.get(*s as usize);
        }
        for scope in frame.scopes.iter().rev() {
            // (a block that bound nothing is skipped without hashing)
            if scope.is_empty() {
                continue;
            }
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn slot(&mut self, name: &str) -> Option<&mut CrowValue> {
        self.forget_text(name);
        let frame = self.frames.last_mut().unwrap();
        if let Some(at) = frame.locals.iter().rposition(|(n, _)| n == name) {
            return Some(&mut frame.locals[at].1);
        }
        if let Some(s) = frame.env.as_ref().and_then(|e| e.get(name).copied()) {
            return frame.slots.get_mut(s as usize);
        }
        for scope in frame.scopes.iter_mut().rev() {
            if scope.is_empty() {
                continue;
            }
            if scope.contains_key(name) {
                return Arc::make_mut(scope).get_mut(name);
            }
        }
        None
    }

    fn bind(&mut self, name: &str, v: CrowValue) {
        self.forget_text(name);
        if self.in_fn_frame() {
            let frame = self.frames.last_mut().unwrap();
            // (a name bound again in the same block — a loop's variable each
            // round — keeps its place and its key)
            let since = frame.marks.last().copied().unwrap_or(0);
            if let Some(at) = frame.locals[since..].iter().rposition(|(n, _)| n == name) {
                frame.locals[since + at].1 = v;
            } else {
                frame.locals.push((name.to_string(), v));
            }
            return;
        }
        let scope = Arc::make_mut(self.frames.last_mut().unwrap().scopes.last_mut().unwrap());
        // (a name bound again — a loop's variable each round — keeps its key)
        if let Some(slot) = scope.get_mut(name) {
            *slot = v;
        } else {
            scope.insert(name.to_string(), v);
        }
    }

    /// A name written: what was known of the text it held is let go.
    fn forget_text(&mut self, name: &str) {
        if !self.texts_known.is_empty() {
            let frame = self.frames.len() - 1;
            self.texts_known.retain(|(f, n, _)| !(*f == frame && n == name));
        }
    }

    /// What is known of the text a local holds, kept while a loop runs
    /// (none outside one: a read by position there is not repeated).
    fn loop_text(&mut self, name: &str) -> Option<Arc<crow_runtime::text::Known>> {
        if self.walker_loops == 0 {
            return None;
        }
        let frame = self.frames.len() - 1;
        if let Some((_, _, k)) = self.texts_known.iter().find(|(f, n, _)| *f == frame && n == name) {
            return Some(k.clone());
        }
        let k = Arc::new(crow_runtime::text::Known::new());
        self.texts_known.push((frame, name.to_string(), k.clone()));
        Some(k)
    }

    /// A loop the walker runs: what it learns of the texts its rounds read
    /// by position is kept for them, and let go when it ends — prepared
    /// code may write those names once the walker hands back.
    fn walker_loop<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let frame = self.frames.len() - 1;
        self.walker_loops += 1;
        let r = f(self);
        self.walker_loops -= 1;
        if !self.texts_known.is_empty() {
            self.texts_known.retain(|(f, _, _)| *f < frame);
        }
        r
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
        let name = self.frames.last().map(|f| f.fn_name.to_string()).unwrap_or_default();
        // a handler, or a helper acting for it, meets an error at a decision:
        // the whole click stops and nothing it did lands (core-design.md §2)
        if self.held.is_some() {
            let acting = self.fns.get(&name).map_or(false, |f| matches!(f.kind, FnKind::Handler | FnKind::Helper));
            if acting {
                return Err(format!("{}{}\u{1}{}\u{1}{}", STOP, name, err.reason(), err.kind()));
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
            Err(format!("{} — {}", at.refusal(), err.reason()))
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
            Stmt::SetPath { var, path, expr, taken } => {
                if *taken && self.grown_in_place(var, path, expr)? {
                    return Ok(Flow::Next);
                }
                let new = self.eval(expr)?;
                // the common case — `xs[i] = v` of a local list — set where it
                // lies, with no list of indexes
                if let [PathStep::Index(i)] = path.as_slice() {
                    let at = self.eval(i)?;
                    let root = self.slot(var).ok_or_else(|| format!("undefined variable '{}'", var))?;
                    if root.is_error() {
                        return Err(format!("cannot set an item of an error value — {}", root.reason()));
                    }
                    if let (Value::Array(items), Value::Int(n)) = (&mut root.value, &at.value) {
                        let k = crow_runtime::stops::item(*n, items.len())?;
                        std::sync::Arc::make_mut(items)[k] = new;
                        return Ok(Flow::Next);
                    }
                    set_path(&mut root.value, path, &[at], new)?;
                    return Ok(Flow::Next);
                }
                let mut idx = Vec::new();
                for p in path {
                    if let PathStep::Index(i) = p {
                        idx.push(self.eval(i)?);
                    }
                }
                let root = self
                    .slot(var)
                    .ok_or_else(|| format!("undefined variable '{}'", var))?;
                // a value that failed has no items to set (spec, Errors: no
                // fields or items to read), in the words both modes use
                if root.is_error() {
                    return Err(format!("cannot set an item of an error value — {}", root.reason()));
                }
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
            Stmt::While { cond, body } => self.walker_loop(|me| {
                // (the condition is the `while` line's, every round: a stop
                // in it had been placed on the body's last line, where the
                // compiled program and the prepared form name the `while`)
                let at = me.line;
                loop {
                    me.check_cancel()?;
                    me.line = at;
                    let c = me.eval(cond)?;
                    if c.is_error() {
                        return me.refuse(c, &Decision::Branch);
                    }
                    if !crate::ops::is_truthy(&c) {
                        break;
                    }
                    match me.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }),
            Stmt::For { var, iter, body, parallel } => self.walker_loop(|me| me.exec_for(var, iter, body, *parallel)),
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
                // each value read where it lies, and copied only when it
                // failed: a top-level loop reading the lines of a file had
                // copied every line at every round (k-nucleotide's search
                // for its section: 86 s for 42,000 rounds)
                for v in values {
                    match self.lookup(v) {
                        None => return Err(format!("undefined variable '{}'", v)),
                        Some(val) if val.is_error() => {
                            let val = val.clone();
                            return self.refuse(val, at);
                        }
                        Some(_) => {}
                    }
                }
                Ok(Flow::Next)
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
                CrowValue::ok(Value::Array(std::sync::Arc::new(items)))
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
                // a sorted table: which one it is in this render, and its order
                if element == "table" && matches!(map.get("sort").map(|v| &v.value), Some(Value::Bool(true))) {
                    let index = self.sorted_tables;
                    self.sorted_tables += 1;
                    map.insert("__sort_index".to_string(), CrowValue::ok(Value::Int(index as i64)));
                    if let Some((column, desc)) = self.session_sorts.get(&index) {
                        map.insert("__sort_by".to_string(), CrowValue::ok(Value::Str(column.clone())));
                        map.insert("__sort_desc".to_string(), CrowValue::ok(Value::Bool(*desc)));
                    }
                }
                let mut vals = vec![CrowValue::ok(Value::map(map))];
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
                // (each side read once, as the compiled program reads them:
                // a failed assertion had read both again for its message,
                // and a side that moved a local, or drew a random number,
                // then said something else)
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                let full = crate::ops::binop(&lv, &binop_of(op)?, &rv)?;
                if !full.is_ok() {
                    return Err(format!("cannot branch on an error value — {}", full.reason()));
                }
                if matches!(full.value, Value::Bool(false)) {
                    if crate::rehearsal::unheld(format!("{}: {} {} {}", self.trace_entry(), lv.value, op, rv.value)) {
                        return Ok(CrowValue::done());
                    }
                    return Err(crow_runtime::display::assert_failed(op, &lv.value.to_string(), &rv.value.to_string()));
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
                    return Ok(CrowValue::markup(crate::harness::html_escape_text(&crow_web::core::response::failed_in_view(&bad.reason()))));
                }
                CrowValue::markup(crate::ops::render_input(name, &key, &vals)?)
            }
            // the length of a local, read where it lies: a list is not
            // copied to be measured (a loop over a growing list was quadratic)
            Expr::Call { target, args, .. }
                if is_len(target) && args.len() == 1 && matches!(unwrapped(&args[0]), Expr::Local(..)) && !self.registry.has_function("len") =>
            {
                let Expr::Local(v, _) = unwrapped(&args[0]) else { unreachable!() };
                let trace = self.trace_entry();
                let b = self.lookup(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                if b.is_error() {
                    return Ok(b.clone());
                }
                Self::len_of(target, b, &trace)?
            }
            // ... and of a local's field: a struct's list, a frame's column
            Expr::Call { target, args, .. }
                if is_len(target) && args.len() == 1 && !self.registry.has_function("len") && matches!(unwrapped(&args[0]), Expr::Field { base, name, .. } if self.field_in_place(base, name)) =>
            {
                let Expr::Field { base: owner, name: field, .. } = unwrapped(&args[0]) else { unreachable!() };
                let trace = self.trace_entry();
                let o = self.place(owner).ok_or_else(|| format!("no field '{}'", field))?;
                let item = match &o.value {
                    Value::Frame(f) => return Ok(CrowValue::ok(Value::Int(f.nrows() as i64))),
                    Value::StructInstance(si) => si.get(field.as_str()).ok_or_else(|| format!("struct '{}' has no field '{}'", si.shown(), field))?,
                    Value::Map(m) => &m[field.as_str()],
                    _ => unreachable!(),
                };
                Self::len_of(target, item, &trace)?
            }
            Expr::Call { target: Target::Row(row), args, propagates, .. }
                if row.id == "core.slice_str" && args.len() == 3 && matches!(&args[0], Expr::Local(..)) && self.walker_loops > 0 =>
            {
                // `slice[s, a, b]` of a local text in a loop: read where it
                // lies, with what the loop knows of it (`loop_text`)
                let Expr::Local(name, _) = &args[0] else { unreachable!() };
                let (a, b) = (self.eval(&args[1])?, self.eval(&args[2])?);
                let known = self.loop_text(name);
                let s = self.lookup(name).ok_or_else(|| format!("undefined variable '{}'", name))?;
                match (&s.value, a.plain_int(), b.plain_int(), known) {
                    (Value::Str(text), Some(x), Some(y), Some(k)) if s.is_ok() => CrowValue::ok(Value::Str(crow_runtime::text::slice_known(text, &k, x, y))),
                    _ => {
                        let vals = vec![s.clone(), a, b];
                        if *propagates && let Some(err) = self.first_error(&vals) {
                            return Ok(err);
                        }
                        self.row_call(row, vals)?
                    }
                }
            }
            Expr::Call { target, args, propagates, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                match target {
                    Target::Row(row) => self.row_call(row, vals)?,
                    Target::Runtime(path) => self.runtime_call(path, vals)?,
                    Target::User(name) => self.call_user(name, vals)?,
                    Target::Module(m, f) => self.module_call(m, f, vals)?,
                }
            }
            Expr::Effect { name, row, args, propagates, answered, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                let asked = std::mem::replace(&mut self.answering, *answered);
                let r = self.effect(name, *row, vals);
                self.answering = asked;
                r?
            }
            Expr::Convert { how, expr, ty } => {
                let v = self.eval(expr)?;
                match how {
                    Convert::IntToFloat => match &v.value {
                        Value::Int(n) if v.is_ok() => CrowValue::ok(Value::Float(*n as f64)),
                        Value::Big(n) if v.is_ok() => CrowValue::ok(Value::Float(n.to_f64().value())),
                        _ => v,
                    },
                    // an int held as a growing one is the int (the interpreter's
                    // grow as they run); one handed where 64 bits are taken is
                    // refused past them
                    Convert::ToBig | Convert::Proven => v,
                    // an int, or a float literal, as the decimal it is; an int
                    // or a decimal as the fraction it is (an error stays)
                    Convert::ToDecimal if v.is_ok() => match &v.value {
                        Value::Float(x) => crate::table::write_decimal(crow_runtime::decimal::of_literal(*x)),
                        _ => crate::table::write_decimal(crate::table::read_decimal(&v)?),
                    },
                    Convert::ToFraction if v.is_ok() => crate::table::write_fraction(crate::table::read_fraction(&v)?),
                    Convert::ToDecimal | Convert::ToFraction => v,
                    Convert::ToSmall(who) => crate::ops::small_value(v, *who)?,
                    // a holder of ints as one of floats (an error stays)
                    Convert::IntsToFloats => floats_held(v),
                    // a tensor's numbers as a list's items, ints where the
                    // list holds ints
                    Convert::ToList => match &v.value {
                        Value::NumArray(t) if v.is_ok() => {
                            let ints = matches!(ty, Ty::List(e) if **e == Ty::Int);
                            CrowValue::ok(Value::Array(
                                t.to_vec().into_iter().map(|x| CrowValue::ok(if ints { Value::Int(x as i64) } else { Value::Float(x) })).collect::<Vec<_>>().into(),
                            ))
                        }
                        _ => v,
                    },
                    // a list's numbers as a tensor of one axis; a frame's
                    // column's, read as the compiled program's
                    // CrowColumn::numbers reads them: a missing cell or a
                    // cell of text or a bool is the reason it has none
                    Convert::ToTensor => match &v.value {
                        // a mask of a matrix's shape (its rows of bools, deeper
                        // for more axes) is a matrix of 1 and 0, or the reason
                        // its rows are not one length — the words of the
                        // compiled program's `crow_runtime::mask::tensor_of`
                        Value::Array(items) if v.is_ok() && matches!(expr.ty().payload(), t if t.truths_depth() >= 2) => {
                            let mut flat = Vec::new();
                            let shape = crate::ops::truths_flat(items, &mut flat)?;
                            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_vec(flat, shape))))
                        }
                        // (a list of no rows where the type states a row's
                        // shape is a matrix of none, `[0, 2]`, as the compiled
                        // program's `from_rows_shaped` makes it)
                        Value::Array(items)
                            if v.is_ok()
                                && items.is_empty()
                                && matches!(expr.ty().payload(), Ty::List(item) if matches!(item.as_ref(), Ty::Tensor(_, Some(d)) if !d.is_empty() && d.iter().all(Option::is_some))) =>
                        {
                            let row: Vec<usize> = match expr.ty().payload() {
                                Ty::List(item) => match item.as_ref() {
                                    Ty::Tensor(_, Some(d)) => d.iter().flatten().copied().collect(),
                                    _ => Vec::new(),
                                },
                                _ => Vec::new(),
                            };
                            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_rows_shaped(&[], &row)?)))
                        }
                        // a list of rows is a matrix: rows of one shape, or
                        // the reason (the compiled program's words)
                        Value::Array(items) if v.is_ok() && !items.is_empty() && items.iter().all(|i| i.is_ok() && matches!(i.value, Value::NumArray(_))) => {
                            let rows: Vec<crow_runtime::tensor::CrowTensor> = items.iter().filter_map(|i| if let Value::NumArray(t) = &i.value { Some(t.as_ref().clone()) } else { None }).collect();
                            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_elements(&rows)?)))
                        }
                        // a list of lists of numbers is a matrix as a list
                        // of rows is: each list a row, every row one length,
                        // or the reason (the compiled program's words)
                        Value::Array(items) if v.is_ok() && !items.is_empty() && items.iter().all(|i| i.is_ok() && matches!(i.value, Value::Array(_))) => {
                            let mut rows = Vec::with_capacity(items.len());
                            for row in items.iter() {
                                let Value::Array(xs) = &row.value else { continue };
                                let mut data = Vec::with_capacity(xs.len());
                                for x in xs.iter() {
                                    match &x.value {
                                        Value::Int(n) if x.is_ok() => data.push(*n as f64),
                                        Value::Float(f) if x.is_ok() => data.push(*f),
                                        Value::Big(b) if x.is_ok() => data.push(crow_runtime::int::Int::Big(b.clone()).to_f64()),
                                        _ => return Err(format!("a list of {} is not a list of numbers", x.value.type_name())),
                                    }
                                }
                                let n = data.len();
                                rows.push(crow_runtime::tensor::CrowTensor::from_vec(data, vec![n]));
                            }
                            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_elements(&rows)?)))
                        }
                        Value::Array(items) if v.is_ok() => {
                            let column = matches!(expr.ty(), Ty::Column(_));
                            let mut data = Vec::with_capacity(items.len());
                            for (i, item) in items.iter().enumerate() {
                                match item.value {
                                    _ if column && item.is_error() => return Err(crow_runtime::stops::missing_cell(i)),
                                    // an item that failed: its reason, the words
                                    // the compiled program stops with
                                    _ if item.is_error() => {
                                        return Err(if item.kind() == "missing" { crow_runtime::stops::missing_cell(i) } else { format!("item {} of the list failed: {}", i, item.reason()) })
                                    }
                                    Value::Int(n) => data.push(n as f64),
                                    Value::Float(x) => data.push(x),
                                    // an int past 64 bits is its nearest float, as
                                    // `float[n]` and the compiled program read it
                                    // (it had been "a list of int is not a list of
                                    // numbers"; Rosetta Code, CSV data manipulation)
                                    Value::Big(ref b) => data.push(crow_runtime::int::Int::Big(b.clone()).to_f64()),
                                    Value::Bool(_) if column => return Err(crow_runtime::stops::not_numbers("bools")),
                                    // a mask where numbers are taken: true 1, false 0
                                    Value::Bool(b) => data.push(if b { 1.0 } else { 0.0 }),
                                    _ if column => return Err(crow_runtime::stops::not_numbers("text")),
                                    _ => return Err(format!("a list of {} is not a list of numbers", item.value.type_name())),
                                }
                            }
                            let n = data.len();
                            CrowValue::ok(Value::NumArray(Box::new(crow_runtime::tensor::CrowTensor::from_vec(data, vec![n]))))
                        }
                        _ => v,
                    },
                    // a CrowValue is its own payload; an error reached here is
                    // the caller's to propagate
                    Convert::Unwrap | Convert::Known => v,
                }
            }
            // `and`/`or` of truths: the right side only where the left does
            // not decide (spec, Operators) — the left's error is the value
            Expr::Binary { op, l, r, .. } if (op == "and" || op == "or") && crate::ir::short_circuits(l, r) => {
                let lv = self.eval(l)?;
                let and = op == "and";
                if lv.is_error() || lv.plain_bool() == Some(!and) {
                    return Ok(lv);
                }
                let rv = self.eval(r)?;
                crate::ops::binop(&lv, &binop_of(op)?, &rv)?
            }
            Expr::Binary { op, l, r, .. } => {
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                let bop = binop_of(op)?;
                // two ints: computed as the compiled program computes them
                // (crow_runtime::stops, a stop on overflow), without the
                // general dispatch — a tight loop's arithmetic and compares
                if lv.is_ok()
                    && rv.is_ok()
                    && let (Value::Int(a), Value::Int(b)) = (&lv.value, &rv.value)
                {
                    let (a, b) = (*a, *b);
                    let fast = match bop {
                        // (past 64 bits, or a zero divisor: the general way,
                        // which grows the int or stops)
                        BinOp::Add => a.checked_add(b).map(Value::Int),
                        BinOp::Sub => a.checked_sub(b).map(Value::Int),
                        BinOp::Mul => a.checked_mul(b).map(Value::Int),
                        BinOp::Mod => a.checked_rem(b).map(Value::Int),
                        BinOp::Eq => Some(Value::Bool(a == b)),
                        BinOp::Ne => Some(Value::Bool(a != b)),
                        BinOp::Gt => Some(Value::Bool(a > b)),
                        BinOp::Lt => Some(Value::Bool(a < b)),
                        BinOp::Gte => Some(Value::Bool(a >= b)),
                        BinOp::Lte => Some(Value::Bool(a <= b)),
                        _ => None,
                    };
                    if let Some(v) = fast {
                        return Ok(CrowValue::ok(v));
                    }
                }
                crate::ops::binop(&lv, &bop, &rv)?
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
                // the common case — `xs[i]` of a local by a local or a
                // constant — read where it lies, with no walk over the index,
                // no list for it and one lookup (the interpreted fannkuch
                // spent its time here)
                if let [index] = indices.as_slice()
                    && let Expr::Local(v, _) = unwrapped(base)
                    && matches!(index, Expr::Local(..) | Expr::Const(..))
                {
                    let at = self.eval(index)?;
                    // (a text read by position in a loop: with what the loop
                    // knows of it, not from its start each time)
                    let known = if self.walker_loops > 0 && matches!(self.lookup(v), Some(b) if matches!(b.value, Value::Str(_))) { self.loop_text(v) } else { None };
                    if let Some(b) = self.lookup(v)
                        && b.is_ok()
                    {
                        let column = matches!(base.ty_ref(), Some(Ty::Column(_)));
                        if let (Value::Str(s), Value::Int(n), Some(k)) = (&b.value, &at.value, &known) {
                            return Ok(CrowValue::ok(Value::Str(crow_runtime::text::char_at_known(s, k, *n)?)));
                        }
                        if let (Value::Array(items), Value::Int(n)) = (&b.value, &at.value) {
                            let k = crow_runtime::stops::item(*n, items.len())?;
                            let cell = items[k].clone();
                            return Ok(if column { placeholder(cell, ty) } else { cell });
                        }
                        if matches!(b.value, Value::NumArray(_) | Value::Map(_) | Value::IntMap(_) | Value::Str(_) | Value::Frame(_)) {
                            let cell = Self::index_value(b, vec![at], ty)?;
                            return Ok(if column { placeholder(cell, ty) } else { cell });
                        }
                    }
                }
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
                    let in_place = !mentioned && matches!(self.lookup(v), Some(b) if b.is_ok() && matches!(b.value, Value::Array(_) | Value::NumArray(_) | Value::Map(_) | Value::Str(_) | Value::Frame(_)));
                    if in_place {
                        let mut idx = Vec::with_capacity(indices.len());
                        for i in indices {
                            idx.push(self.eval(i)?);
                        }
                        let b = self.lookup(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                        let cell = Self::index_value(b, idx, ty)?;
                        return Ok(if matches!(base.ty_ref(), Some(Ty::Column(_))) { placeholder(cell, ty) } else { cell });
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
                                let col = f.col_index(name).ok_or_else(|| crow_runtime::stops::no_column(name))?;
                                crate::ops::frame_cell(f, col, crate::ops::list_position(&at, f.nrows())?)
                            }
                            Value::StructInstance(si) => Self::index_value(si.get(name).ok_or_else(|| format!("struct '{}' has no field '{}'", si.shown(), name))?, vec![at], ty)?,
                            Value::Map(m) => Self::index_value(&m[name], vec![at], ty)?,
                            _ => unreachable!(),
                        };
                        return Ok(if matches!(base.ty_ref(), Some(Ty::Column(_))) { placeholder(cell, ty) } else { cell });
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
                        // (an argument that failed is the call's value, the
                        // first such, and the gradient never runs — as a
                        // function value's, and as the IR types the call: it
                        // had been handed in, "takes numbers or tensors, got
                        // done")
                        if let Some(err) = self.first_error(&idx) {
                            return Ok(err);
                        }
                        let name = fv.name.clone();
                        return self.grad_call(&name, idx);
                    }
                    _ => {}
                }
                Self::index_value(&b, idx, ty)?
            }
            Expr::Field { base, name, .. } => {
                let b = self.eval(base)?;
                // a value that failed has no fields: where the lowering
                // could not refuse it first (a fact's declared value), the
                // read refuses it, as the compiled read does
                if base.ty().is_result() && b.is_error() {
                    return Err(format!("{} — {}", Decision::Read(name.clone()).refusal(), b.reason()));
                }
                // an app session reads its own reactive state: the globals
                // while it is the active one, its waiting copy otherwise
                if let Value::App(a) = &b.value {
                    let id = &a.id;
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
                // a value its type says cannot fail did not: the lowering
                // decides, and what was read for it has been read
                if *which != Meta::Valid && crate::lower_ir::meta_decided(&base.ty()) {
                    return Ok(match which {
                        Meta::Ok => CrowValue::ok(Value::Bool(true)),
                        Meta::Trace => CrowValue::ok(Value::Array(std::sync::Arc::new(Vec::new()))),
                        _ => CrowValue::ok(Value::Str(String::new())),
                    });
                }
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
            Expr::ListLit(items, ty) => {
                let mut vals = Vec::with_capacity(items.len());
                for i in items {
                    vals.push(self.eval(i)?);
                }
                // held as the IR types it: a list the program grows as one
                // is a list, the compiled program's `Vec` (numbers written
                // as a list literal had been a tensor however the IR typed
                // them, whose own rows the values then picked)
                // (a vector of whole numbers holds each exactly, or stops)
                if let Ty::Tensor(e, _) = ty.payload()
                    && matches!(e.as_ref(), Ty::Int)
                    && let Some(n) = vals.iter().find_map(|v| match v.value {
                        Value::Int(n) if n.unsigned_abs() > 1 << 53 => Some(n),
                        _ => None,
                    })
                {
                    return Err(crow_runtime::core::past_vector(n));
                }
                match ty.payload() {
                    Ty::List(_) => CrowValue::ok(Value::Array(std::sync::Arc::new(vals))),
                    _ => crate::ops::array_literal(vals)?,
                }
            }
            Expr::MapLit(pairs, ty) => {
                // (keyed by ints where the literal's keys were ints)
                if matches!(ty, Ty::Map(crate::ir::Key::Int, _)) {
                    let mut map = std::collections::BTreeMap::new();
                    for (k, v) in pairs {
                        map.insert(k.parse::<i64>().unwrap_or(0), self.eval(v)?);
                    }
                    return Ok(CrowValue::ok(Value::int_map(map)));
                }
                let mut map = std::collections::BTreeMap::new();
                for (k, v) in pairs {
                    map.insert(k.clone(), self.eval(v)?);
                }
                CrowValue::ok(Value::map(map))
            }
            Expr::RecordLit { name, fields } => {
                let def = self.structs.get(name).cloned().ok_or_else(|| format!("unknown struct '{}'", name))?;
                if def.fields.len() != fields.len() {
                    return Err(format!("{}[] expects {} fields, got {}", name, def.fields.len(), fields.len()));
                }
                // the fields evaluated in their declaration's order, each into
                // its place in the layout every value of the record shares
                let shape: Shape = self.layouts.get(name).copied().ok_or_else(|| format!("unknown struct '{}'", name))?;
                let mut vals = vec![CrowValue::done(); fields.len()];
                for (i, e) in fields.iter().enumerate() {
                    vals[shape.1[i]] = self.eval(e)?;
                }
                CrowValue::ok(Value::StructInstance(Arc::new(StructInstance::with_layout(shape.0, vals))))
            }
            Expr::ForExpr { var, iter, body, parallel, ty } => self.walker_loop(|me| me.collect_for(var, iter, body, *parallel, ty))?,
        })
    }

    /// `x = push[move x, v]` and the other in-place builtins on a list or a
    /// map (`Prop::InPlace`): the value is taken out of its slot and
    /// moved into the row its values pick, whose function grows it, as the
    /// compiled mode does. Only a row that makes its answer of the
    /// collection takes this path; anything else (a frame, a tensor, a
    /// module's function of the same name, a kind no row takes) goes the
    /// ordinary way.
    /// `xs[i] = push[xs[i], v]` (ir::Stmt::SetPath, `taken`): the place's
    /// value taken out of its slot — the lists and maps along the path
    /// copied first where another value shares them, as a write does —
    /// handed to the row moved (its take path, as a local's `xs = push[xs,
    /// v]` is) and the answer written back, so the list under the key grows
    /// where it lies. A failed argument goes the ordinary way (spec,
    /// Auto-Propagation), the place untouched.
    fn grown_in_place(&mut self, var: &str, path: &[PathStep], expr: &Expr) -> Result<bool, String> {
        let Expr::Call { target: Target::Row(row), args, .. } = expr else { return Ok(false) };
        if !row.grows_in_place() || args.is_empty() {
            return Ok(false);
        }
        let mut idx = Vec::new();
        for p in path {
            if let PathStep::Index(i) = p {
                idx.push(self.eval(i)?);
            }
        }
        // (the other arguments are plain reads — the lowering computed any
        // other first — so a failed one goes the ordinary way, which reads
        // them again and hands the failure on as any call does)
        let rest = self.eval_args(&args[1..])?;
        if rest.iter().any(|r| r.is_error()) {
            return Ok(false);
        }
        let trace = self.trace_entry();
        let root = self.slot(var).ok_or_else(|| format!("undefined variable '{}'", var))?;
        if root.is_error() {
            return Err(format!("cannot set an item of an error value — {}", root.reason()));
        }
        let slot = path_slot(&mut root.value, path, &idx)?;
        let taken = std::mem::replace(slot, CrowValue::done());
        let mut vals = Vec::with_capacity(1 + rest.len());
        vals.push(taken);
        vals.extend(rest);
        // (the answer is of the place's kind — the row makes it of what it
        // was given — so it goes straight back into the slot; a stop in the
        // row ends the program there, as the compiled one's does)
        *slot = (row.take)(vals, &trace)?;
        Ok(true)
    }

    fn assign_in_place(&mut self, var: &str, expr: &Expr) -> Result<bool, String> {
        let (name, args, named) = match expr {
            Expr::Call { target: Target::Runtime(name), args, .. } => (name.rsplit('.').next().unwrap_or(name), args, None),
            Expr::Call { target: Target::Row(row), args, .. } if row.module.is_empty() => (row.name, args, Some(*row)),
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
        if !crate::table::has(name, crate::table::Prop::InPlace) || !matches!(list, Some(Expr::Move(v, _)) if v == var) {
            return Ok(false);
        }
        if named.is_none() && self.registry.has_function(name) && name != "sort" {
            return Ok(false);
        }
        // (a named row decides before any argument is evaluated: one whose
        // collection is no list or map goes the ordinary way, where a
        // moved argument is its take path — evaluating the arguments here
        // first had read a moved one twice: `m[1] = r` at r's last read,
        // "expected numbers, got done")
        if named.is_some_and(|row| !row.takes_in_place()) {
            return Ok(false);
        }
        let rest = self.eval_args(&args[1..])?;
        if rest.iter().any(|r| r.is_error()) {
            return Ok(false);
        }
        // the row the values pick (the collection's kind read where it
        // lies), when it makes its answer of the collection: taken out of
        // its slot and moved into the call
        let row = {
            let Some(cur) = self.lookup(var).filter(|c| !c.is_error()) else {
                return Ok(false);
            };
            let mut kinds = vec![crate::table::Arg::of_value(cur)];
            kinds.extend(rest.iter().map(crate::table::Arg::of_value));
            match named.or_else(|| crate::table::core_row(name, &kinds)) {
                Some(row) if row.takes_in_place() => row,
                _ => return Ok(false),
            }
        };
        let taken = match self.slot(var) {
            Some(slot) => std::mem::replace(slot, CrowValue::done()),
            None => return Ok(false),
        };
        let mut vals = Vec::with_capacity(1 + rest.len());
        vals.push(taken);
        vals.extend(rest);
        let out = (row.take)(vals, &self.trace_entry())?;
        self.assign(var, out);
        Ok(true)
    }

    /// A parallel loop: one clone of the walker per chunk of items, its
    /// scopes shared until written, each iteration in a scope of its own.
    /// The lowering marked the loop parallel because its body writes no
    /// outer variable or fact and reaches no effect, so nothing is lost
    /// when the clones are dropped; a `return` inside is refused.
    /// The local a chain of fields starts from: `bars` in `bars.c`.
    /// What a loop over a tensor walks, as the IR's type decided
    /// (crow_runtime::core::walk_rows, walk_numbers — the compiled program
    /// reads the value through the same two): a matrix's rows, a vector's
    /// numbers, ints of a tensor of ints and floats of one of floats. The
    /// value had decided — rows by its rank, and a whole number as an int —
    /// where the compiled program went by the type, so `type[x]` and a loop
    /// over a matrix typed by nothing differed between the modes. Any other
    /// value walks its items.
    /// `for i in range[lo, hi]` counted: the rounds of the list path
    /// (`core::range` walks `lo..hi` by one) without the list, the
    /// variable rebound in place each round.
    fn counted_for(&mut self, var: &str, lo: i64, hi: i64, body: &[Stmt]) -> Result<Flow, String> {
        self.push_scope();
        let mut flow = Flow::Next;
        let mut i = lo;
        while i < hi {
            if let Err(e) = self.check_cancel() {
                self.pop_scope();
                return Err(e);
            }
            self.bind(var, CrowValue::ok(Value::Int(i)));
            let r = self.exec_stmts(body);
            self.end_round(var);
            match r {
                Err(e) => {
                    self.pop_scope();
                    return Err(e);
                }
                Ok(Flow::Break) => break,
                Ok(Flow::Return(v)) => {
                    flow = Flow::Return(v);
                    break;
                }
                Ok(Flow::Next | Flow::Continue) => {}
            }
            i += 1;
        }
        self.pop_scope();
        Ok(flow)
    }

    /// A collecting `for`, in the walker: each round's value, gathered.
    fn collect_for(&mut self, var: &str, iter: &Expr, body: &[Stmt], parallel: bool, ty: &Ty) -> Result<CrowValue, String> {
        let as_tensor = matches!(ty.payload(), Ty::Tensor(..));
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
        let items = Self::walked_items(&it, iter)?;
        if parallel && items.len() > 1 && !self.tracing {
            let results = self.run_parallel(&items, var, body, true)?;
            return Ok(crate::ops::collect_loop(results, as_tensor));
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
        Ok(crate::ops::collect_loop(results, as_tensor))
    }

    /// `for var in iter` of a statement, in the walker.
    fn exec_for(&mut self, var: &str, iter: &Expr, body: &[Stmt], parallel: bool) -> Result<Flow, String> {
        // `for i in range[n]` and `range[a, b]`: a counted loop, no
        // list built — the emitter's twin (stmts.rs emit_for): the
        // interpreted fannkuch built one per permutation too. A
        // parallel loop keeps its list (its rounds are handed out as
        // items), as does a stepped range.
        let it = if !(parallel && !self.tracing)
            && let Expr::Call { target: Target::Row(row), args, propagates, ty, .. } = iter
            && matches!(row.id, "core.range_to" | "core.range_from")
            && !ty.is_result()
        {
            let vals = self.eval_args(args)?;
            let bound = |v: &CrowValue| if v.is_ok() && let Value::Int(n) = &v.value { Some(*n) } else { None };
            let bounds = match vals.as_slice() {
                [n] => bound(n).map(|hi| (0, hi)),
                [a, b] => bound(a).zip(bound(b)),
                _ => None,
            };
            if let Some((lo, hi)) = bounds {
                return self.counted_for(var, lo, hi, body);
            }
            // what the call would have answered from these values
            match (*propagates, self.first_error(&vals)) {
                (true, Some(err)) => err,
                _ => self.row_call(row, vals)?,
            }
        } else {
            self.eval(iter)?
        };
        // a list that failed is refused like a condition (the
        // lowering refuses a typed one first; this is one it could
        // not see)
        if it.is_error() {
            return self.refuse(it, &Decision::Loop);
        }
        let items = Self::walked_items(&it, iter)?;
        if parallel && items.len() > 1 && !self.tracing {
            self.run_parallel(&items, var, body, false)?;
            return Ok(Flow::Next);
        }
        // one scope for every round: the variable rebound in place,
        // and what a round bound dropped after it (a scope per round
        // had allocated the map and the name each time)
        self.push_scope();
        let mut flow = Flow::Next;
        for item in items {
            if let Err(e) = self.check_cancel() {
                self.pop_scope();
                return Err(e);
            }
            self.bind(var, item);
            let r = self.exec_stmts(body);
            self.end_round(var);
            match r {
                Err(e) => {
                    self.pop_scope();
                    return Err(e);
                }
                Ok(Flow::Break) => break,
                Ok(Flow::Return(v)) => {
                    flow = Flow::Return(v);
                    break;
                }
                Ok(Flow::Next | Flow::Continue) => {}
            }
        }
        self.pop_scope();
        Ok(flow)
    
    }

    fn walked_items(it: &CrowValue, iter: &Expr) -> Result<Vec<CrowValue>, String> {
        Self::walked_items_of(it, &iter.ty())
    }

    /// The items a loop walks, by the iterable's type: a tensor's rows or
    /// numbers (ints where the tensor holds ints), else a value's items.
    fn walked_items_of(it: &CrowValue, ty: &Ty) -> Result<Vec<CrowValue>, String> {
        if let (Value::NumArray(t), Ty::Tensor(e, dims)) = (&it.value, ty.payload()) {
            return Ok(match Ty::tensor_item(e, dims) {
                Ty::Tensor(..) => crow_runtime::core::walk_rows(t)?.into_iter().map(|r| CrowValue::ok(Value::NumArray(Box::new(r)))).collect(),
                Ty::Int => crow_runtime::core::walk_numbers(t)?.into_iter().map(|v| CrowValue::ok(Value::Int(v as i64))).collect(),
                _ => crow_runtime::core::walk_numbers(t)?.into_iter().map(|v| CrowValue::ok(Value::Float(v))).collect(),
            });
        }
        crate::ops::iter_items(it)
    }

    /// `t[i]` of a tensor, as the IR's type decided (crow_runtime::core::
    /// row_at, number_at): a matrix's row, a vector's number — an int of a
    /// tensor of ints, else a float. A gather by a tensor of indices, and
    /// any value that is no tensor, read by their own rule.
    fn index_value(b: &CrowValue, idx: Vec<CrowValue>, ty: &Ty) -> Result<CrowValue, String> {
        if let (Value::NumArray(t), [i]) = (&b.value, idx.as_slice())
            && let Value::Int(i) = &i.value
        {
            match ty.payload() {
                Ty::Tensor(..) => return crow_runtime::core::row_at(t, *i).map(|r| CrowValue::ok(Value::NumArray(Box::new(r)))),
                Ty::Int => return crow_runtime::core::number_at(t, *i).map(|v| CrowValue::ok(Value::Int(v as i64))),
                Ty::Float => return crow_runtime::core::number_at(t, *i).map(|v| CrowValue::ok(Value::Float(v))),
                _ => {}
            }
        }
        crate::ops::index(b, idx)
    }

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
            Expr::Local(v, _) => self.lookup(v).filter(|b| b.is_ok()),
            Expr::Field { base, name, .. } => {
                if matches!(name.as_str(), "ok" | "reason" | "kind" | "trace" | "len") {
                    return None;
                }
                match &self.place(base)?.value {
                    Value::StructInstance(si) => si.get(name.as_str()).filter(|b| b.is_ok()),
                    Value::Map(m) => m.get(name.as_str()).filter(|b| b.is_ok()),
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
        let indexable = |v: &CrowValue| v.is_ok() && matches!(v.value, Value::Array(_) | Value::NumArray(_) | Value::Map(_) | Value::Str(_));
        match self.place(owner).map(|o| &o.value) {
            Some(Value::Frame(f)) => !matches!(name, "shape" | "columns" | "dtypes") && f.col_index(name).is_some(),
            Some(Value::StructInstance(si)) => si.get(name).is_some_and(indexable),
            Some(Value::Map(m)) => m.get(name).is_some_and(indexable),
            _ => false,
        }
    }

    fn run_parallel(&mut self, items: &[CrowValue], var: &str, body: &[Stmt], collect: bool) -> Result<Vec<CrowValue>, String> {
        self.check_cancel()?;
        // (the first item worked here and timed: where the rest would cost
        // less than a trip to the pool, it runs here too — a collecting
        // loop over ten flags, called 20,000 times, had cloned the walker
        // for each item and waited on the pool at every call, 1 to 4 s
        // where the loop in order took 0.02; Rosetta Code, Average loop
        // length. The compiled loop decides by the same rule:
        // crow_runtime::cores::worth_sharing)
        let start = std::time::Instant::now();
        let mut first = self.run_part(&items[..1], var, body, collect)?;
        if !crow_runtime::cores::worth_sharing(start.elapsed(), items.len() - 1) {
            first.extend(self.run_part(&items[1..], var, body, collect)?);
            return Ok(first);
        }
        let items = &items[1..];
        // the functions the body reaches, prepared once here: each chunk's
        // clone had prepared them again, and dropped what it made
        let reached = self.prepare_reached(body);
        let base = &*self;
        type Chunk = (Result<Vec<CrowValue>, String>, HashMap<String, Arc<prepared::Prepared>, Names>);
        // on the cores `cpu[…]` allows where the loop starts
        // (crow_runtime::cores, as the compiled loop's)
        let chunks: Vec<Chunk> = crow_runtime::cores::run(|| {
            let threads = rayon::current_num_threads().max(1);
            let chunk = (items.len() / (threads * 4)).max(1);
            items
            .par_chunks(chunk)
            .map(|part| {
                let mut local = base.clone();
                // each chunk its own handle on each body the loop reaches
                // (prepared.rs, `handle`)
                for name in &reached {
                    if let Some(p) = base.prepared.get(name.as_str()) {
                        local.prepared.insert(name.clone(), Arc::new(p.handle(&p.name)));
                    }
                }
                let r = local.run_part(part, var, body, collect);
                (r, std::mem::take(&mut local.prepared))
            })
            .collect()
        });
        let mut results = first;
        results.reserve(if collect { items.len() } else { 0 });
        let mut failed = None;
        for (c, made) in chunks {
            // (what a clone prepared — a function reached only as a value — kept)
            for (name, p) in made {
                self.prepared.entry(name).or_insert(p);
            }
            match c {
                Ok(part) if failed.is_none() => results.extend(part),
                Ok(_) => {}
                Err(e) => {
                    if failed.is_none() {
                        failed = Some(e);
                    }
                }
            }
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(results),
        }
    }

    /// A parallel loop's items worked in order by this walker: each bound
    /// to the loop's variable in a scope of its own, what each yields where
    /// the loop collects.
    fn run_part(&mut self, part: &[CrowValue], var: &str, body: &[Stmt], collect: bool) -> Result<Vec<CrowValue>, String> {
        let mut out = Vec::with_capacity(if collect { part.len() } else { 0 });
        for item in part {
            self.push_scope();
            self.bind(var, item.clone());
            self.last_value = None;
            // a stop names the body's line, as the compiled loop's does
            let r = self.exec_stmts(body);
            let r = r.map_err(|e| self.with_line(e));
            let yielded = self.last_value.take();
            self.pop_scope();
            if let Flow::Return(_) = r? {
                return Err("cannot return from inside a for loop".to_string());
            }
            if collect {
                out.push(yielded.unwrap_or_else(CrowValue::done));
            }
        }
        Ok(out)
    }

    /// Every function a body calls by name, and every function those call,
    /// prepared under the names the calls write (a traced run prepares
    /// none: it walks): the names.
    fn prepare_reached(&mut self, body: &[Stmt]) -> Vec<String> {
        if self.tracing {
            return Vec::new();
        }
        let mut todo: Vec<String> = Vec::new();
        crate::ir::visit_block(body, &mut |e| {
            if let Expr::Call { target: Target::User(name), .. } = e {
                todo.push(name.clone());
            }
        });
        let mut seen: HashSet<String> = HashSet::new();
        let mut reached = Vec::new();
        while let Some(name) = todo.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Ok(f) = self.function_named(&name) else { continue };
            if !self.prepared.contains_key(name.as_str()) && self.prepare_called(&name).is_err() {
                continue;
            }
            crate::ir::visit_block(&f.body, &mut |e| {
                if let Expr::Call { target: Target::User(n), .. } = e {
                    todo.push(n.clone());
                }
            });
            reached.push(name);
        }
        reached
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
        if crate::table::stamps(name) && v.is_ok() {
            v.stamp_captured()
        } else {
            v
        }
    }

    /// `stamp`, for a call whose row is known: the row's flag, read by its
    /// place in the table, with no lookup by name (asked after every
    /// builtin call).
    fn stamp_row(row: &'static crate::table::Row, v: CrowValue) -> CrowValue {
        if crate::table::row_stamps(row) && v.is_ok() {
            v.stamp_captured()
        } else {
            v
        }
    }

    /// A builtin's row, as the IR names it: its runtime function, the call
    /// the compiled program prints (a dual number a function grad
    /// differentiates takes the derivative's own form).
    fn row_call(&mut self, row: &'static crate::table::Row, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        // a builtin written in Crow (an effect's row reaches here): its function
        if row.is_crow() {
            // a message is held to its handler's commit by the session layer,
            // in both modes: the row's function is what sends it
            if row.id == "web.mail.mail" {
                return self.mail_effect(vals);
            }
            let handed = crate::table::crow_arguments(row, &vals)?;
            // (a reader's value carries the moment it was read, whichever
            // language the reader is written in)
            return self.call_user(&crate::table::crow_fn_name(row), handed).map(|v| Self::stamp_row(row, v));
        }
        // (by the name the program wrote: `floor[x]` of a float of any
        // size is the growing form's row)
        if vals.iter().any(|v| matches!(v.value, Value::Dual(..))) {
            return self.bare_call(crate::table::written_name(row), vals);
        }
        if crate::autograd_bridge::any_traced(&vals) {
            return crate::autograd_bridge::call(crate::table::written_name(row), &vals);
        }
        if row.module.is_empty() {
            // (only a row whose failure is a value reads the trace: for the
            // others it is not built — a string formatted at every call)
            let trace = if matches!(row.fails, crate::table::Fails::Value(_)) { self.trace_entry() } else { String::new() };
            (row.take)(vals, &trace).map(|v| Self::stamp_row(row, v))
        } else {
            // a module's error value carries no trace, in either mode
            (row.take)(vals, "").map(|v| Self::stamp_row(row, v))
        }
    }

    /// The length of a value read where it lies, by the row the IR names
    /// (or, when the values pick it, the row they pick).
    fn len_of(target: &Target, v: &CrowValue, trace: &str) -> Result<CrowValue, String> {
        // a traced value's length is read through the kernel's side, as
        // every call on one is (a function grad differentiates had read
        // its tensor's length through the row, which does not take one)
        if matches!(v.value, Value::Traced(_)) {
            return crate::autograd_bridge::call("len", std::slice::from_ref(v));
        }
        match target {
            Target::Row(row) => (row.call)(std::slice::from_ref(v), trace),
            _ => Self::core_rows("len", std::slice::from_ref(v), trace, false).unwrap_or_else(|| Err("len[] is not a builtin".into())),
        }
    }

    /// A builtin the IR leaves by name (`core.push`, or its own name): a
    /// form of its own, or a row its values pick — an argument whose kind
    /// the lowering did not know, a parsed value.
    fn runtime_call(&mut self, path: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        // a builtin with a row is its runtime function, read and dispatched
        // as the row says: the row the values' kinds pick
        let name = RUNTIME_TABLE.iter().find(|(_, r)| *r == path).map(|(b, _)| *b).unwrap_or(path).to_string();
        if name != "log" && crate::autograd_bridge::any_traced(&vals) {
            return crate::autograd_bridge::call(&name, &vals);
        }
        if let Some(answer) = self.web_response(&name, &vals) {
            return answer;
        }
        // a dual number a function grad differentiates takes the
        // derivative's own form (a plain number's math is its row)
        if vals.iter().any(|v| matches!(v.value, Value::Dual(..))) {
            return self.bare_call(&name, vals);
        }
        match Self::core_row(&name, &vals, self.registry.has_function(&name)) {
            Some(Ok(row)) => (row.take)(vals, &self.trace_entry()),
            Some(Err(refused)) => Err(refused),
            None => self.bare_call(&name, vals),
        }
    }

    /// A core builtin's row for these values; or, when none takes them and
    /// no module answers the name (math's min of a tensor, frame's join of
    /// frames), the refusal the lowering gives when it knows the kinds.
    /// None when the name has no rows, or a module may take them.
    fn core_row(name: &str, vals: &[CrowValue], module_too: bool) -> Option<Result<&'static crate::table::Row, String>> {
        let kinds: Vec<crate::table::Arg> = vals.iter().map(crate::table::Arg::of_value).collect();
        if let Some(row) = crate::table::core_row(name, &kinds) {
            return Some(Ok(row));
        }
        if crate::table::has_core_rows(name) && !module_too {
            return crate::table::refusal(&[""], name, &kinds).map(Err);
        }
        None
    }

    /// A core builtin's row called on values read where they lie (a list
    /// measured is not copied).
    fn core_rows(name: &str, vals: &[CrowValue], trace: &str, module_too: bool) -> Option<Result<CrowValue, String>> {
        Self::core_row(name, vals, module_too).map(|row| row.and_then(|row| (row.call)(vals, trace)))
    }

    /// A bare name, resolved as the interpreter resolves it: a loaded
    /// module's function wins over a builtin of the same name, except that
    /// a frame function given something other than a frame is the builtin
    /// (sort[xs], join[parts, sep]); then the builtins; then what only a
    /// walker can do.
    /// A builtin written in Crow (crate::table, `crow!`) whose row the
    /// values pick as the program runs — a call the lowering could not
    /// decide, an argument's kind unknown to it: the row's function, handed
    /// the values as its parameters take them. None where no Crow row takes
    /// them (the registry answers, or refuses, as for any name).
    fn crow_row_call(&mut self, module: &str, name: &str, vals: &[CrowValue]) -> Option<Result<CrowValue, String>> {
        let kinds: Vec<crate::table::Arg> = vals.iter().map(crate::table::Arg::of_value).collect();
        let row = crate::table::crow_row(module, name, &kinds)?;
        Some(crate::table::crow_arguments(row, vals).and_then(|handed| self.call_user(&crate::table::crow_fn_name(row), handed)).map(|v| Self::stamp_row(row, v)))
    }

    /// `mail[mailer, to, subject, body]`: held to a handler's commit, or
    /// sent when its answer is used; in a test, recorded and never sent.
    /// The sending is the row written in Crow (src/modules/mail.co): the
    /// SMTP conversation, called with the mailer and the message.
    fn mail_effect(&mut self, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if vals.len() != 4 {
            return Err("mail[] takes a mailer, to, subject and body: mail[mailer, 'ada@example.com', 'Hello', 'Welcome']".into());
        }
        if !matches!(vals[0].value, Value::Mailer(_)) {
            return Err("mail[] sends through a mailer: mailer = smtp[host, port, user, pass]".into());
        }
        let mailer = vals[0].clone();
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
        match self.send_mail(&mailer, &mail) {
            Ok(()) => Ok(CrowValue::ok(Value::Bool(true))),
            Err(e) => Ok(CrowValue::error_with_kind(&e, "mail", &self.trace_entry())),
        }
    }

    /// A message sent through its mailer by the row written in Crow: whether
    /// the server took it, or why not (the reason already names the message).
    pub(crate) fn send_mail(&mut self, mailer: &CrowValue, mail: &crow_web::live::SentMail) -> Result<(), String> {
        let args = vec![mailer.clone(), CrowValue::ok(Value::Str(mail.to.clone())), CrowValue::ok(Value::Str(mail.subject.clone())), CrowValue::ok(Value::Str(mail.body.clone()))];
        let sent = self.call_user("_web_mail_mail", args)?;
        if sent.is_ok() {
            Ok(())
        } else {
            Err(sent.reason().to_string())
        }
    }

    fn bare_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some(stop) = self.effect_refused(name, &vals) {
            return stop;
        }
        // (a message is the session's to hold, whichever way the row is reached)
        if name == "mail" && matches!(vals.first().map(|v| &v.value), Some(Value::Mailer(_))) {
            return self.mail_effect(vals);
        }
        // a value a function grad differentiates computes with: the
        // autograd kernel's builtin, recorded on the tape
        if name != "log" && crate::autograd_bridge::any_traced(&vals) {
            return crate::autograd_bridge::call(name, &vals);
        }
        // a dual number a function grad differentiates carries takes the
        // derivative's own form — before a module's row would take a
        // number (math's `cos` of a dual had reached its row, which takes
        // no dual: `expected a number, got float`)
        if vals.iter().any(|v| matches!(v.value, Value::Dual(..))) {
            if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
                return r.map(|v| Self::stamp(name, v));
            }
        }
        // in a rehearsal what reaches past the program is recorded, not run
        if let Some(v) = crate::rehearsal::outside(name, &vals) {
            return Ok(v);
        }
        if name == "transact" {
            return self.transact(vals);
        }
        if let Some(answer) = self.web_response(name, &vals) {
            return answer;
        }
        if matches!(name, "serve" | "page" | "route" | "fetch" | "poll" | "app" | "html" | "elements" | "change" | "click" | "box" | "fits" | "style" | "downloads" | "mails") {
            return self.harness(name, vals);
        }
        // a name a module answers reaches the module the values' kinds pick,
        // by the one rule inference and the lowering decide by
        // (table::module_of_bare); none is the core's row, below. (The
        // interpreter had its own two rules here — a scalar name math and
        // the core share, a frame name given no frame — and the emitter
        // another copy of each.)
        let kinds: Vec<crate::table::Arg> = vals.iter().map(crate::table::Arg::of_value).collect();
        let providers: Vec<String> = self.registry.providers_of(name).into_iter().map(str::to_string).collect();
        let provider_names: Vec<&str> = providers.iter().map(String::as_str).collect();
        if let Some(m) = crate::table::module_of_bare(name, &provider_names, &kinds) {
            if let Some(answer) = self.crow_row_call(&m, name, &vals) {
                return answer;
            }
            return self.registry.call_in(&m, name, &vals).map(|v| Self::stamp(name, v));
        }
        // an effect with a row (the file system's): the row, inside what the
        // session layer does around it (a dual number takes grad's own form)
        let dual = vals.iter().any(|v| matches!(v.value, Value::Dual(..)));
        if !dual {
            // a core row written in Crow the lowering left to the values
            // (`read[upload.path]` from a map's value): its function
            if let Some(answer) = self.crow_row_call("", name, &vals) {
                return answer;
            }
            if let Some(r) = Self::core_rows(name, &vals, &self.trace_entry(), false) {
                return r.map(|v| Self::stamp(name, v));
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
                            map.insert("groups".to_string(), CrowValue::ok(Value::Array(std::sync::Arc::new(groups.into_iter().map(|m| CrowValue::ok(Value::Str(m))).collect()))));
                            CrowValue::ok(Value::map(map))
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
                Ok(match vals[0].captured() {
                    Some(t) => CrowValue::ok(Value::Moment(crow_runtime::moments::Moment::of_seconds(t))),
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
            n if crate::table::revises(n) => {
                // revise[name, default, f, …]: f run under the name's lock
                // with the record held (the default where none is) and the
                // rest; what it answers held — nothing changed where its
                // answer failed (crow_runtime::memory, as compiled)
                if vals.len() < 3 {
                    return Err("revise[] takes a name, a record and a function: revise[name, default, f, …]".into());
                }
                let name = match &vals[0].value {
                    Value::Str(s) => s.to_string(),
                    other => return Err(format!("memory: a name is text, not {}", other.type_name())),
                };
                let (kind, shown) = crate::table::held::kind_of(&vals[1])?;
                let default = vals[1].clone();
                let f = vals[2].clone();
                let rest: Vec<CrowValue> = vals[3..].to_vec();
                // (the record handed over; copied first only where the
                // function's answer can fail, to be put back then)
                let can_fail = match &f.value {
                    Value::Fn(fv) => self.function_named(&fv.name).map_or(true, |x| matches!(x.ret, Ty::Result(_))),
                    _ => true,
                };
                crow_runtime::memory::revise_as(&name, &kind, &shown, default, can_fail, |held| {
                    let mut args = vec![held];
                    args.extend(rest);
                    match self.call_value(&f, args) {
                        Ok(v) if v.is_error() => (None, Ok(v)),
                        Ok(v) => match crate::table::held::kind_of(&v) {
                            Ok((k, _)) if k == kind => (Some(v.clone()), Ok(v)),
                            Ok((_, given)) => (None, Err(crow_runtime::memory::other_kind(&name, &shown, &given))),
                            Err(e) => (None, Err(e)),
                        },
                        Err(e) => (None, Err(e)),
                    }
                })?
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
                } else if let Err(e) = self.call_value(&f, rest) {
                    // a stop inside the work ends the work, not the
                    // program, and says why, as the compiled program's
                    // thread does (spec §15); a control signal goes on
                    if e.starts_with('\u{1}') {
                        return Err(e);
                    }
                    crow_runtime::display::flush_output();
                    eprintln!("runtime error: {}", e);
                }
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            // the page's signed-in reader: the session's, a map of strings
            "__reader" => {
                let id = self.active_session.unwrap_or(0);
                Ok(CrowValue::ok(Value::map(self.session_reader.get(&id).cloned().unwrap_or_default())))
            }
            // sign_in[record] / sign_out[]: held to the handler's commit
            "sign_in" | "sign_out" => {
                let record = if name == "sign_in" {
                    let m = match vals.first().map(|v| &v.value) {
                        Some(Value::Map(m)) => m.as_ref().clone(),
                        _ => return Err("sign_in[] takes the reader as a map of strings: sign_in[map[id = '42', name = 'Ada']]".into()),
                    };
                    let mut strings = std::collections::BTreeMap::new();
                    for (k, v) in m.iter() {
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
            "__same_fn" => match (&vals[0].value, &vals[1].value) {
                (Value::Fn(a), Value::Fn(b)) => Ok(CrowValue::ok(Value::Bool(a.name == b.name))),
                (a, b) => Err(format!("__same_fn compares functions, got {} and {}", a.type_name(), b.type_name())),
            },
            "running" => Ok(CrowValue::ok(Value::Bool(false))),
            // the resources as calls: gpu[n] turns the device on, cpu[n]
            // bounds the loops
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
            "cpu" => {
                if vals.len() != 1 {
                    return Err(format!("{}[] takes exactly 1 argument", name));
                }
                let n = match vals[0].unwrap_or_propagate()? {
                    Value::Int(n) if *n >= 1 => *n as usize,
                    Value::Int(_) => return Err(format!("{}[] requires a positive integer", name)),
                    _ => return Err(format!("{}[] requires an integer argument", name)),
                };
                // the loops that start after it, where it runs
                // (crow_runtime::cores, as the compiled program's)
                crow_runtime::cores::set(n as i64)?;
                Ok(CrowValue::done())
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
                        return Ok(CrowValue::markup(crate::harness::html_escape_text(&crow_web::core::response::failed_in_view(&vals[0].reason()))));
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
                    points.map(|v| &**v),
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

    /// A web program's response forms, as the compiled program makes them
    /// (emit_rs/builtins.rs), through the same crow_web functions: none for
    /// another name, or for a form given what it does not answer (`json` of
    /// text parses it; `html` of a test's app session is the harness's).
    fn web_response(&self, name: &str, vals: &[CrowValue]) -> Option<Result<CrowValue, String>> {
        use crow_web::core::CrowResponse;
        if !self.web {
            return None;
        }
        use crow_web::core::response as said;
        let text = |v: &CrowValue| format!("{}", v.value);
        let is_text = |v: &CrowValue| matches!(v.value, Value::Str(_));
        let what = |v: &CrowValue| crate::table::Arg::of_value(v).words();
        // (a body of another kind is refused in the lowering's words: one
        // that reached the run had been answered as its text; `html` of a
        // test's app session is the harness's)
        let body_at = match name {
            "text" | "html" | "redirect" | "bytes" => Some(0),
            "respond" => Some(1),
            _ => None,
        };
        if let Some(body) = body_at.and_then(|i| vals.get(i)) {
            let session = name == "html" && matches!(body.value, Value::App(_));
            if !is_text(body) && !body.is_error() && !session {
                return Some(Err(said::takes_text(name, what(body))));
            }
        }
        let response = match (name, vals) {
            ("text", [body]) if is_text(body) => CrowResponse::text(text(body)),
            ("html", [body]) if is_text(body) => CrowResponse::html(text(body)),
            // a frame answers as its records (`frame::records`, the one form)
            ("json", [frame]) if matches!(frame.value, Value::Frame(_)) => match &frame.value {
                Value::Frame(f) => CrowResponse::json(crow_runtime::frame::records(f)),
                other => return Some(Err(format!("json[] takes a map or a frame, got {}", other.type_name()))),
            },
            ("json", [value]) if !is_text(value) && !value.is_error() => match crate::builtins::crow_to_json("json", value) {
                Ok(j) => CrowResponse::json(j),
                Err(e) => return Some(Err(e)),
            },
            ("respond", [status, body]) => match status.value {
                Value::Int(n) => match said::status(n) {
                    Ok(s) => CrowResponse::respond(s, text(body)),
                    Err(why) => return Some(Err(why)),
                },
                _ => return Some(Err(said::takes_status(what(status)))),
            },
            ("redirect", [url]) => CrowResponse::redirect(text(url)),
            ("bytes", [data]) => CrowResponse::binary(200, text(data).into_bytes(), "application/octet-stream"),
            ("arrow", [frame]) => match &frame.value {
                Value::Frame(f) => CrowResponse::arrow_of(f),
                _ => return Some(Err(said::takes_frame(what(frame)))),
            },
            ("stream", [feed, rest @ ..]) => match &feed.value {
                Value::Fn(f) => self.stream_response(&f.name, rest),
                other => return Some(Err(format!("stream[] takes a named function, got {}", other.type_name()))),
            },
            _ => return None,
        };
        Some(Ok(CrowValue::ok(Value::Response(Box::new(response)))))
    }

    /// `stream[feed, a, …]`: a response whose body is what `feed[put, a, …]`
    /// puts, each chunk sent as it is put. The feed runs on a copy of the
    /// program when the response is sent; one that fails or stops ends the
    /// body there, and crow_web's log says why, as for the compiled feed.
    fn stream_response(&self, feed: &str, rest: &[CrowValue]) -> crow_web::core::CrowResponse {
        let mut it = self.clone();
        let name = feed.to_string();
        let mut args = vec![CrowValue::ok(Value::Fn(FnValue { name: PUT.to_string() }))];
        args.extend(rest.iter().cloned());
        crow_web::core::CrowResponse::stream(feed, move |_put| match it.guard(|it| it.call_user(&name, args)) {
            Ok(v) if v.is_error() => Err(v.reason().to_string()),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        })
    }

    /// The runtime's put, as a feed calls it: the chunk goes to the reader
    /// (crow_web's put, which the compiled feed is handed), and the answer
    /// is whether the reader still listens.
    fn put(vals: &[CrowValue]) -> Result<CrowValue, String> {
        match vals {
            [chunk] => Ok(CrowValue::ok(Value::Bool(crow_web::core::response::put(format!("{}", chunk.value))))),
            _ => Err("put[] takes one chunk of text: put['a line\\n']".to_string()),
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
        // the function's statements come through the handle it is handed:
        // crow_db routes them to the transaction (one connection; inside a
        // handler, a savepoint on its own). What the function answers is
        // transact's value; a function that fails rolls back, and its
        // error is the value; a stop stops.
        let tx_val = CrowValue::ok(Value::DbConn(conn.clone()));
        let mut answer: Option<CrowValue> = None;
        let mut stopped: Option<String> = None;
        let done = conn.transact_calling(&mut || match self.call_value(&f, vec![tx_val.clone()]) {
            Ok(v) if v.is_error() => {
                let why = v.reason().to_string();
                answer = Some(v);
                Err(why)
            }
            Ok(v) => {
                answer = Some(v);
                Ok(())
            }
            Err(e) => {
                stopped = Some(e.clone());
                Err(e)
            }
        });
        if let Some(e) = stopped {
            return Err(format!("transact[]: rolled back: {}", e));
        }
        match (done, answer) {
            (Ok(()), Some(v)) => Ok(v),
            (Ok(()), None) => Ok(CrowValue::ok(Value::Done)),
            (Err(_), Some(v)) if v.is_error() => Ok(v),
            (Err(e), _) => Ok(CrowValue::error_with_kind(&format!("transaction rolled back: {}", e), "db", &self.trace_entry())),
        }
    }

    /// Forward-mode differentiation: the function once per parameter with
    /// that parameter a dual (x, 1) and the others (x, 0).
    fn grad_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let nparams = self.fns.get(name).map(|f| f.params.len()).ok_or_else(|| format!("unknown function '{}'", name))?;
        if nparams != vals.len() {
            return Err(format!("grad[{}][] expects {} args, got {}", name, nparams, vals.len()));
        }
        // handed tensors, the function is one of tensors: run once on
        // traced leaves, its number differentiated backwards over the tape
        if vals.iter().any(|a| matches!(a.value, Value::NumArray(_) | Value::Traced(_))) {
            let inputs: Vec<crow_runtime::tensor::CrowTensor> = vals
                .iter()
                .map(|a| match &a.value {
                    Value::NumArray(t) => Ok(t.as_ref().clone()),
                    Value::Traced(t) => Ok(t.value.clone()),
                    Value::Int(n) => Ok(crow_runtime::tensor::CrowTensor::scalar(*n as f64)),
                    Value::Float(x) => Ok(crow_runtime::tensor::CrowTensor::scalar(*x)),
                    other => Err(format!("grad[{}][] takes numbers or tensors, got {}", name, other.type_name())),
                })
                .collect::<Result<_, _>>()?;
            let fname = name.to_string();
            let was = std::mem::replace(&mut self.tracing, true);
            let answer = crate::autograd_bridge::gradient(inputs, |leaves| {
                let args: Vec<CrowValue> = leaves.iter().map(|l| crate::autograd_bridge::wrap(l.clone())).collect();
                let out = self.call_user(&fname, args)?;
                if out.is_error() {
                    return Err(format!("grad[{}][]: the function answered an error — {}", fname, out.reason()));
                }
                match out.value {
                    Value::Traced(t) => Ok(*t),
                    Value::Float(x) => Ok(crow_runtime::autograd::Traced::number(x)),
                    Value::Int(n) => Ok(crow_runtime::autograd::Traced::number(n as f64)),
                    Value::NumArray(t) => Ok(crow_runtime::autograd::Traced::constant(*t)),
                    other => Err(format!("grad[{}][] requires function to return a number, got {}", fname, other.type_name())),
                }
            });
            self.tracing = was;
            return answer;
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
            Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(derivatives.into_iter().map(|d| CrowValue::ok(Value::Float(d))).collect()))))
        }
    }

    fn module_call(&mut self, module: &str, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some(stop) = self.effect_refused(name, &vals) {
            return stop;
        }
        if name == "transact" && module == "db" {
            return self.transact(vals);
        }
        // math.min[1, 2]: a name the core answers for these kinds is the
        // core's row, as the bare call's is (the one rule, table::module_of_bare)
        if module == "math" && crate::table::has_core_rows(name) {
            let kinds: Vec<crate::table::Arg> = vals.iter().map(crate::table::Arg::of_value).collect();
            if crate::table::module_of_bare(name, &[module], &kinds).is_none() {
                if let Some(row) = crate::table::core_row(name, &kinds) {
                    return (row.call)(&vals, &self.trace_entry());
                }
                if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
                    return r;
                }
            }
        }
        if self.registry.is_loaded(module) {
            if let Some(answer) = self.crow_row_call(module, name, &vals) {
                return answer;
            }
            return self.registry.call_in(module, name, &vals).map(|v| Self::stamp(name, v));
        }
        Err(format!("the IR interpreter does not run '{}.{}' yet", module, name))
    }

    /// An effect by name: qualified (`frame.load`) through its module,
    /// bare as any bare call.
    fn effect(&mut self, name: &str, row: Option<&'static crate::table::Row>, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        // the row the IR names, inside what the session layer does around
        // it: an effect held for a handler's commit given a failed value
        // stops the handler, and a rehearsal records what would reach past
        // the program
        if let Some(row) = row {
            let short = name.rsplit('.').next().unwrap_or(name);
            if let Some(stop) = self.effect_refused(short, &vals) {
                return stop;
            }
            if !name.contains('.') {
                if let Some(v) = crate::rehearsal::outside(name, &vals) {
                    return Ok(v);
                }
            }
            return self.row_call(row, vals);
        }
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

    pub(crate) fn call_user(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        self.check_cancel()?;
        if name == PUT {
            return Self::put(&vals);
        }
        // the body prepared (prepared.rs): instructions over registers,
        // typed where the IR fixes the operands, found by the name the call
        // writes; a traced run keeps the walker
        if !self.tracing {
            let p = match self.prepared.get(name) {
                Some(p) => p.clone(),
                None => self.prepare_called(name)?,
            };
            // (a standard module's function resolves the names its row left
            // to the runtime in its own scope)
            let scope = self.registry.enter_scope(p.scope);
            let r = self.call_prepared(name, &p, vals);
            self.registry.resume_call(scope);
            return r;
        }
        let scope = self.registry.enter_call(name);
        let r = self.call_walked(name, vals);
        self.registry.resume_call(scope);
        r
    }

    /// The function a call names: a specialised name (first__1) is the one
    /// body to the interpreter.
    fn function_named(&self, name: &str) -> Result<Arc<Fn>, String> {
        match self.fns.get(name) {
            Some(f) => Ok(f.clone()),
            None => {
                let base = crate::ir::copy_base(name);
                self.fns.get(base).cloned().ok_or_else(|| format!("unknown function '{}'", name))
            }
        }
    }

    /// The prepared body a name calls, prepared now, kept under the name the
    /// call writes (a specialised name gets its own handle on the body).
    fn prepare_called(&mut self, name: &str) -> Result<Arc<prepared::Prepared>, String> {
        let f = self.function_named(name)?;
        let p = self.prepared_for(&f);
        if &*p.name == name {
            return Ok(p);
        }
        let h = Arc::new(p.handle(name));
        self.prepared.insert(name.to_string(), h.clone());
        Ok(h)
    }

    /// What every call checks of its arguments before the body runs: their
    /// number; a view's failed argument, which is its text; an int handed
    /// to a float parameter, a float — where the call is written (the
    /// lowering converts it) and where a function value is called (`any`,
    /// `find`, `later`) alike. A value here is the call's answer.
    fn arguments_checked(name: &str, floats: impl ExactSizeIterator<Item = bool>, view: bool, vals: &mut [CrowValue]) -> Result<Option<CrowValue>, String> {
        if floats.len() != vals.len() {
            return Err(format!("{}[] expects {} arguments, got {}", name, floats.len(), vals.len()));
        }
        if view && let Some(bad) = vals.iter().find(|v| v.is_error()) {
            return Ok(Some(CrowValue::markup(crate::harness::html_escape_text(&crow_web::core::response::failed_in_view(&bad.reason())))));
        }
        for (float, v) in floats.zip(vals.iter_mut()) {
            if float && let Some(n) = v.plain_int() {
                *v = CrowValue::ok(Value::Float(n as f64));
            }
        }
        Ok(None)
    }

    /// A recursive function's calls nest to the limit and no deeper, as the
    /// compiled program's guard counts them; the stop names the body's first
    /// line in both modes.
    fn deeper(&mut self, first_line: usize) -> Result<(), String> {
        if self.depth >= crow_runtime::stops::DEPTH {
            self.line = first_line;
            return Err(self.with_line(crow_runtime::stops::too_deep()));
        }
        self.depth += 1;
        Ok(())
    }

    /// A call of a prepared body. The handle is the caller's own (a thread
    /// of a parallel loop has its own, `run_parallel`): the call writes no
    /// count another thread writes.
    fn call_prepared(&mut self, name: &str, p: &prepared::Prepared, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let n = vals.len();
        let mut given = vals.into_iter();
        self.call_prepared_with(name, p, n, &mut prepared::Given(&mut given))
    }

    /// A call the walker runs (a traced run).
    fn call_walked(&mut self, name: &str, mut vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let f = self.function_named(name)?;
        // (a function that calls cpu[…] puts its caller's count back)
        let _cores = crate::ir::sets_cores(&f.body).then(crow_runtime::cores::Restore::new);
        let view = f.kind == FnKind::View;
        if let Some(v) = Self::arguments_checked(name, f.params.iter().map(|(_, t)| matches!(t, Ty::Float)), view, &mut vals)? {
            return Ok(v);
        }
        if f.recursive {
            self.deeper(crate::ir::first_line(&f.body))?;
        }
        let saved_line = self.line;
        if view {
            self.in_view += 1;
        }
        let mut scope = HashMap::with_capacity_and_hasher(f.params.len(), Names::default());
        for ((p, _), v) in f.params.iter().zip(vals) {
            scope.insert(p.clone(), v);
        }
        self.frames.push(Frame { scopes: vec![Arc::new(scope)], locals: Vec::new(), marks: Vec::new(), fn_name: Arc::from(name), slots: Vec::new(), env: None, texts: Vec::new() });
        let r = self.exec_stmts(&f.body);
        self.call_ended(r, view, f.recursive, saved_line)
    }

    /// A call's end: a stop names the line in the body it happened on, not
    /// the call's; the frame goes, and the caller's line is current again.
    fn call_ended(&mut self, r: Result<Flow, String>, view: bool, recursive: bool, saved_line: usize) -> Result<CrowValue, String> {
        let r = self.returned(r).map_err(|e| self.with_line(e));
        self.frames.pop();
        if view {
            self.in_view -= 1;
        }
        if recursive {
            self.depth -= 1;
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

    /// A derivation read: a route's request reads the app's values — the
    /// ones its handler read as it began, and any other that computing one
    /// reads; anywhere else it is computed where it is read.
    fn derived(&mut self, name: &str) -> Result<CrowValue, String> {
        if let Some(rid) = self.active_session.and_then(|id| self.remote_sessions.get(&id).copied()) {
            return self.remote_read(rid, name);
        }
        if let Some(reads) = &self.route_reads {
            if let Some(v) = reads.get(name) {
                return Ok(v.clone());
            }
            let v = self.app_read(name)?;
            if let Some(reads) = self.route_reads.as_mut() {
                reads.insert(name.to_string(), v.clone());
            }
            return Ok(v);
        }
        self.compute_derived(name)
    }

    /// A route's request begins: the derivations its handler reads (bound
    /// as it starts, by the lowering) are read from the app's values, once
    /// each for the request (spec, Derivations: a route reads what the app
    /// holds).
    pub(crate) fn begin_route(&mut self) {
        self.route_reads = Some(HashMap::new());
    }

    /// The request ended: its reads are let go.
    pub(crate) fn end_route(&mut self) {
        self.route_reads = None;
    }

    /// A derivation the app holds, from the app's values: its inputs that are
    /// derivations first, so their versions are current, then its own — the
    /// value kept while nothing it read moved, else computed here, once for
    /// the readers arriving together (crow_web::live, as the compiled
    /// program reads it).
    fn app_read(&mut self, name: &str) -> Result<CrowValue, String> {
        let Some(app) = self.app_state.clone() else {
            return self.compute_derived(name);
        };
        let d = self.derives.get(name).cloned().ok_or_else(|| format!("unknown derivation '{}'", name))?;
        for other in &d.spec.derives {
            if other != name {
                self.app_read(other)?;
            }
        }
        let spec = app_spec(self.app_tag, &d);
        let scalar = matches!(crate::lower_ir::as_inferred(&d.ty), Ty::Int | Ty::Float | Ty::Str | Ty::Bool);
        app.derive_with(spec, |_| self.compute_derived(name), move |a: &CrowValue, b: &CrowValue| scalar && same_scalar(a, b))
    }

    /// A derivation's body run now, stamped with its moment.
    fn compute_derived(&mut self, name: &str) -> Result<CrowValue, String> {
        let d = self.derives.get(name).cloned().ok_or_else(|| format!("unknown derivation '{}'", name))?;
        // the reader's line is still the line reading it
        let saved_line = self.line;
        self.frames.push(Frame { scopes: vec![Arc::new(HashMap::default())], locals: Vec::new(), marks: Vec::new(), fn_name: Arc::from(name), slots: Vec::new(), env: None, texts: Vec::new() });
        let r = self.exec_stmts(&d.body);
        let r = self.returned(r).map_err(|e| self.with_line(e));
        self.frames.pop();
        self.line = saved_line;
        let mut v = match r? {
            Flow::Return(v) => v,
            _ => CrowValue::done(),
        };
        v.set_captured(Some(crow_runtime::moments::now()));
        Ok(v)
    }

    /// An effect a handler calls with an error among its arguments — a
    /// parameter list included — stops the handler: an error never reaches
    /// the world outside the session (core-design.md §2.1).
    fn effect_refused(&mut self, name: &str, vals: &[CrowValue]) -> Option<Result<CrowValue, String>> {
        if self.held.is_none() || !crate::table::held(name) {
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
        // an event that does not fit the program — no handler of the name,
        // or not the arguments it takes — stops, as a handler that stops
        // does, in a test's click as on a served page (a click failed the
        // test; the compiled page dropped the event without a word)
        let fits = match self.fns.get(name).or_else(|| self.fns.get(crate::ir::copy_base(name))) {
            None => Err(format!("unknown function '{}'", name)),
            Some(f) if f.params.len() != args.len() => Err(format!("{}[] expects {} arguments, got {}", name, f.params.len(), args.len())),
            Some(_) => Ok(()),
        };
        if let Err(reason) = fits {
            let told = format!("{} stopped: {}", name, reason);
            self.last_stop = Some(told.clone());
            // the reader is told it stopped, and an id; the reason is the
            // click's value and, served, the server's log (crow_web::live)
            self.session_notice.insert(id, (crow_web::live::Stopped::new(told).told(), crow_runtime::moments::now()));
            let mut stopped = CrowValue::error_with_kind(&reason, "stop", "");
            stopped.status_mut().trace.clear();
            return Ok(stopped);
        }
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
                self.session_notice.insert(id, (crow_web::live::Stopped::new(told).told(), crow_runtime::moments::now()));
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
                Ok(f) => CrowValue::ok(Value::Frame(Box::new(f))),
                Err(_) => crate::builtins::json_to_crow("", &value),
            },
            // by the type the IR gives it, as a fact is read
            ty => crate::served::json_to_typed(&value, &ty, self, &CrowValue::ok(Value::Done)),
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
        self.sorted_tables = 0;
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
            if let Value::App(a) = &v.value { Some((a.view.clone(), a.width, a.wide, a.id)) } else { None }
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
                self.pages.push((path, view, url));
                CrowValue::done()
            }
            "serve" => {
                self.serve_config = Some(vals);
                CrowValue::done()
            }
            // route[path, handler] answers GET, route[path, method, handler]
            // the method named; a limit, last, is the requests a minute each
            // client may make
            "route" => {
                let (path, method, handler, limit) = match vals.as_slice() {
                    [path, handler] => (path, "GET".to_string(), handler, 0),
                    [path, method, handler] => (path, format!("{}", method.value).to_uppercase(), handler, 0),
                    [path, method, handler, limit] => (path, format!("{}", method.value).to_uppercase(), handler, match limit.value {
                        Value::Int(n) => n.max(0),
                        _ => 0,
                    }),
                    _ => return Err("route[] takes a path and a handler: route['/health', health] (or a method between them)".into()),
                };
                let handler = match &handler.value {
                    Value::Fn(f) => f.name.clone(),
                    _ => return Err("route[] takes a handler, a function of the request: route['/health', health]".into()),
                };
                self.routes.push((method, format!("{}", path.value), handler, limit as u64));
                CrowValue::done()
            }
            // a page's data source: a frame the session loads — empty until
            // it arrives, held to the columns the program reads there
            "fetch" | "poll" => {
                let url = vals.first().map(|v| format!("{}", v.value)).unwrap_or_default();
                let secs = (name == "poll").then(|| match vals.get(1).map(|v| &v.value) {
                    Some(Value::Int(n)) => (*n).max(0) as u64,
                    Some(Value::Float(x)) => x.max(0.0) as u64,
                    _ => 0,
                });
                self.pending_source = Some((url, secs));
                CrowValue::ok(Value::Frame(Box::new(crow_runtime::frame::CrowFrame::new(Vec::new(), Vec::new(), Vec::new()))))
            }
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
                CrowValue::ok(Value::App(Box::new(crate::value::AppSession { view: f, width, wide, id })))
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
                        CrowValue::ok(Value::map(m))
                    })
                    .collect::<Vec<CrowValue>>();
                CrowValue::ok(Value::Array(std::sync::Arc::new(items)))
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
                        CrowValue::ok(Value::map(map))
                    })
                    .collect::<Vec<CrowValue>>();
                CrowValue::ok(Value::Array(std::sync::Arc::new(items)))
            }
            "html" => {
                let (f, _, _, id) = vals.first().and_then(app_view).ok_or("html[] takes an app session: html[a]")?;
                // the page as the browser builds it, as the compiled harness
                // hands it back: a table's rows read the same in both modes
                let built = crow_web::view::CrowVNode::raw(&self.session_html(&f.name, id)?).as_built().to_html();
                CrowValue::ok(Value::Str(built))
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
                CrowValue::ok(Value::Array(std::sync::Arc::new(hits)))
            }
            "change" => {
                let id = match (vals.len(), &vals.first().map(|v| &v.value)) {
                    (3, Some(Value::App(a))) => a.id,
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
                        Value::Map(m) if m.values().all(|v| matches!(v.value, Value::Str(_))) => m.as_ref().clone(),
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
                        Value::Moment(m) => m.seconds(),
                        other => return Err(format!("change[a, 'now', t] takes a moment, not {}", other.type_name())),
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
                    (2, Some(Value::App(a))) => a.id,
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
                    return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out))));
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
                    Value::Array(items) => items.as_ref().clone(),
                    _ => Vec::new(),
                };
                if name == "fits" {
                    let any_overflow = list.iter().any(|b| match &b.value {
                        Value::StructInstance(si) => matches!(si.get("overflow").map(|v| &v.value), Some(Value::Bool(true))),
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
                            let path = si.get("path").map(|v| format!("{}", v.value)).unwrap_or_default();
                            crate::harness::harness_path_matches(&path, &steps)
                        } else {
                            false
                        }
                    })
                    .collect::<Vec<CrowValue>>();
                CrowValue::ok(Value::Array(std::sync::Arc::new(hits)))
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
    CrowValue::ok(Value::map(r))
}

fn binop_of(op: &str) -> Result<BinOp, String> {
    // (the common operators by length and first bytes: the chain of text
    // compares had shown in the interpreter's profile)
    let b = op.as_bytes();
    match (b.len(), b.first().copied(), b.get(1).copied()) {
        (3, Some(b'a'), Some(b'd')) => return Ok(BinOp::Add),
        (3, Some(b's'), _) => return Ok(BinOp::Sub),
        (3, Some(b'm'), Some(b'u')) => return Ok(BinOp::Mul),
        (3, Some(b'd'), _) => return Ok(BinOp::Div),
        (3, Some(b'm'), Some(b'o')) => return Ok(BinOp::Mod),
        (2, Some(b'e'), _) => return Ok(BinOp::Eq),
        (2, Some(b'n'), _) => return Ok(BinOp::Ne),
        (2, Some(b'g'), _) => return Ok(BinOp::Gt),
        (2, Some(b'l'), _) => return Ok(BinOp::Lt),
        (3, Some(b'g'), _) => return Ok(BinOp::Gte),
        (3, Some(b'l'), _) => return Ok(BinOp::Lte),
        (3, Some(b'a'), Some(b'n')) => return Ok(BinOp::And),
        (2, Some(b'o'), _) => return Ok(BinOp::Or),
        _ => {}
    }
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
/// The slot a path reaches, to take a value out of (`grown_in_place`): each
/// step as a write makes it, a missing key or a position outside stopping
/// as a read does.
fn path_slot<'v>(root: &'v mut Value, path: &[PathStep], idx: &[CrowValue]) -> Result<&'v mut CrowValue, String> {
    let mut it = idx.iter();
    let (first, rest) = path.split_first().ok_or("empty path")?;
    let mut cur: &mut CrowValue = path_step(root, first, &mut it)?;
    for p in rest {
        cur = path_step(&mut cur.value, p, &mut it)?;
    }
    Ok(cur)
}

/// One step of a path write: the field or the item, where the value
/// lies, the record, list or map copied first where another value shares
/// it (values stay values).
fn path_step<'v>(cur: &'v mut Value, p: &PathStep, idx: &mut std::slice::Iter<'_, CrowValue>) -> Result<&'v mut CrowValue, String> {
    match p {
        PathStep::Field(f) => match cur {
            Value::StructInstance(si) => {
                // (a record another value shares is copied first: values stay values)
                let si = Arc::make_mut(si);
                let shown = si.shown().to_string();
                si.get_mut(f).ok_or_else(|| format!("struct '{}' has no field '{}'", shown, f))
            }
            // (a map another value shares is copied first, as a record is)
            Value::Map(m) => {
                crow_runtime::stops::key(m, &f.to_string())?;
                std::sync::Arc::make_mut(m).get_mut(f).ok_or_else(|| format!("map has no key '{}'", f))
            }
            other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
        },
        PathStep::Index(_) => {
            let i = idx.next().ok_or("missing index")?;
            match cur {
                Value::Array(items) => match &i.value {
                    Value::Int(n) => {
                        let at = crow_runtime::stops::item(*n, items.len())?;
                        Ok(&mut std::sync::Arc::make_mut(items)[at])
                    }
                    _ => Err("array index must be an integer".to_string()),
                },
                Value::Map(m) => match &i.value {
                    Value::Str(k) => {
                        crow_runtime::stops::key(m, k)?;
                        std::sync::Arc::make_mut(m).get_mut(k).ok_or_else(|| format!("map has no key '{}'", k))
                    }
                    Value::Int(n) => Err(format!("the map's keys are text, not ints: {}", n)),
                    _ => Err("a map's key is text or an int".to_string()),
                },
                Value::IntMap(m) => match &i.value {
                    Value::Int(n) => {
                        crow_runtime::stops::key(m, n)?;
                        std::sync::Arc::make_mut(m).get_mut(n).ok_or_else(|| format!("map has no key '{}'", n))
                    }
                    Value::Str(k) => Err(format!("the map's keys are ints, not text: '{}'", k)),
                    _ => Err("a map's key is text or an int".to_string()),
                },
                other => Err(format!("cannot index-assign into {}", other.type_name())),
            }
        }
    }
}

fn set_path(root: &mut Value, path: &[PathStep], idx: &[CrowValue], new: CrowValue) -> Result<(), String> {
    let mut it = idx.iter();
    let (last, front) = path.split_last().ok_or("empty path")?;
    let mut cur: &mut Value = root;
    for p in front {
        cur = &mut path_step(cur, p, &mut it)?.value;
    }
    match last {
        PathStep::Field(f) => match cur {
            Value::StructInstance(si) => {
                let si = Arc::make_mut(si);
                let shown = si.shown().to_string();
                match si.get_mut(f) {
                    Some(slot) => {
                        *slot = new;
                        Ok(())
                    }
                    None => Err(format!("struct '{}' has no field '{}'", shown, f)),
                }
            }
            Value::Map(m) => {
                crate::builtins::check_map_homogeneous(m, &new)?;
                std::sync::Arc::make_mut(m).insert(f.clone(), new);
                Ok(())
            }
            other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
        },
        PathStep::Index(_) => {
            let i = it.next().ok_or("missing index")?;
            match cur {
                Value::Array(items) => match &i.value {
                    Value::Int(n) => {
                        let at = crow_runtime::stops::item(*n, items.len())?;
                        std::sync::Arc::make_mut(items)[at] = new;
                        Ok(())
                    }
                    _ => Err("array index must be an integer".to_string()),
                },
                Value::NumArray(t) => match (&i.value, &new.value) {
                    // the number set where the tensor lies (it had been
                    // copied out and built again at every write)
                    (Value::Int(n), Value::Int(x)) => crow_runtime::core::set_flat_at(t, *n, *x as f64),
                    (Value::Int(n), Value::Float(x)) => crow_runtime::core::set_flat_at(t, *n, *x),
                    _ => Err("a numeric array takes a number at an integer index".to_string()),
                },
                Value::Map(m) => match &i.value {
                    Value::Str(k) => {
                        crate::builtins::check_map_homogeneous(m, &new)?;
                        std::sync::Arc::make_mut(m).insert(k.clone(), new);
                        Ok(())
                    }
                    // an open map takes its first key's kind, as `set` does
                    // (`m = map[]`, then `m[1] = 'one'`, had stopped here
                    // while the compiled program wrote the key; Rosetta Code,
                    // Achilles numbers)
                    Value::Int(n) if m.is_empty() => {
                        let mut first = std::collections::BTreeMap::new();
                        first.insert(*n, new);
                        *cur = Value::int_map(first);
                        Ok(())
                    }
                    Value::Int(n) => Err(format!("the map's keys are text, not ints: {}", n)),
                    _ => Err("a map's key is text or an int".to_string()),
                },
                Value::IntMap(m) => match &i.value {
                    Value::Int(n) => {
                        std::sync::Arc::make_mut(m).insert(*n, new);
                        Ok(())
                    }
                    Value::Str(k) => Err(format!("the map's keys are ints, not text: '{}'", k)),
                    _ => Err("a map's key is text or an int".to_string()),
                },
                other => Err(format!("cannot index-assign into {}", other.type_name())),
            }
        }
    }
}

/// A derivation as the app's values know it (crow_web::live::DeriveSpec),
/// made once per served program: its names carry the program's tag, so two
/// programs one process serves share no value.
fn app_spec(tag: u64, d: &Derive) -> &'static crow_web::live::DeriveSpec {
    type Specs = HashMap<(u64, String), &'static crow_web::live::DeriveSpec>;
    static SPECS: std::sync::LazyLock<std::sync::Mutex<Specs>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }
    fn list(xs: impl Iterator<Item = String>) -> &'static [&'static str] {
        Box::leak(xs.map(leak).collect::<Vec<_>>().into_boxed_slice())
    }
    let mut specs = SPECS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(spec) = specs.get(&(tag, d.name.clone())) {
        return spec;
    }
    let rs = &d.spec;
    let spec: &'static crow_web::live::DeriveSpec = Box::leak(Box::new(crow_web::live::DeriveSpec {
        name: leak(format!("{}#{}", tag, d.name)),
        facts: list(rs.facts.iter().cloned()),
        derives: list(rs.derives.iter().map(|n| format!("{}#{}", tag, n))),
        tables: list(rs.tables.iter().cloned()),
        now: rs.now,
        external: rs.external,
        fresh: d.fresh,
        shared: rs.shared,
        keys: Box::leak(rs.keys.iter().map(|(t, (c, f))| (leak(t.clone()), leak(c.clone()), leak(f.clone()))).collect::<Vec<_>>().into_boxed_slice()),
    }));
    specs.insert((tag, d.name.clone()), spec);
    spec
}

/// Whether two values of a derivation that holds a number, a text or a
/// bool are the same: its readers keep their values (early cutoff), as the
/// compiled program's `==` decides.
fn same_scalar(a: &CrowValue, b: &CrowValue) -> bool {
    match (&a.value, &b.value) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
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
    /// cell (a missing one of a join's unmatched side an error of reason
    /// `missing`), a struct's or map's item, from the end with a negative
    /// index, and out of range refused.
    #[test]
    fn a_field_indexed_in_place_reads_what_the_copy_read() {
        let src = "use frame\nstruct Bars [c]\ndf = frame.new[k = [0, 1, 2], p = [1.5, 2.5, 3.5]]\nother = frame.new[k = [0, 1], q = [7.5, 8.5]]\ndf = join[df, other, 'k', 'left']\nb = Bars[[4.0, 5.0, 6.0]]\nm = map[xs = [1, 2, 3]]\na = df.p[1]\ng = df.q[2]\nz = b.c[-1]\nw = m.xs[0]\nn = b.c[1] + df.p[0]\n";
        let r = run_script(&parse(src), "t").unwrap();
        assert!(matches!(r.var("a").unwrap().value, Value::Float(x) if x == 2.5));
        assert!(r.var("g").is_some_and(|g| !g.is_ok() && g.reason() == "missing"), "{:?}", r.var("g"));
        assert_eq!(format!("{}", r.var("z").unwrap().value), "6");
        assert!(matches!(r.var("w").unwrap().value, Value::Int(1)));
        assert!(matches!(r.var("n").unwrap().value, Value::Float(x) if x == 6.5));
        let far = run_script(&parse("struct Bars [c]\nb = Bars[[4.0, 5.0]]\nx = b.c[2]\n"), "t");
        assert!(far.is_err_and(|e| e.contains("index 2 is out of range for an array of 2 items")));
    }

    /// A top-level loop reading an item of a list that can fail checks the
    /// list where it lies at every round: the check had copied the whole
    /// list each round (k-nucleotide's search for its section took 86 s,
    /// 42,000 rounds over 83,000 lines)
    #[test]
    fn a_list_that_can_fail_is_checked_where_it_lies() {
        let dir = std::env::temp_dir().join(format!("crow_propagate_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rows.txt");
        let text: String = (0..20000).map(|i| format!("row {}\n", i)).collect();
        std::fs::write(&path, text).unwrap();
        let src = format!("use io\nrows = lines['{}']\nfirst = 0\nwhile rows[first] ne 'row 19999'\n    first = first + 1\n", path.display());
        let start = std::time::Instant::now();
        let r = run_script(&parse(&src), "t").unwrap();
        let took = start.elapsed();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(r.var("first").unwrap().value, Value::Int(19999)));
        // (20,000 copies of 20,000 lines took minutes in a test build; the
        // loop itself runs in a tenth of a second, instrumented too, so the
        // bound is wall time's under a loaded run — the coverage run's
        // parallel tests build programs on every core, and this test had
        // failed there past ten seconds three runs in four, 2026-10-20)
        assert!(took.as_secs() < 60, "{:?}", took);
    }

    /// `cpu[n]` holds from where it runs, in the thread that runs it, and a
    /// function's count is put back as the function returns — as the
    /// compiled program's (crow_runtime::cores); one below 1 is refused
    #[test]
    fn a_core_count_holds_from_where_it_runs() {
        let src = "fn inner[]\n    cpu[2]\n    xs = for i in range[4]\n        i * 2\n    return len[xs]\ncpu[3]\nn = inner[]\n";
        let r = run_script(&parse(src), "t").unwrap();
        assert!(matches!(r.var("n").unwrap().value, Value::Int(4)));
        assert_eq!(crow_runtime::cores::get(), 3, "the top level's count holds after the function's is put back");
        let refused = run_script(&parse("cpu[0]\n"), "t");
        assert!(refused.is_err_and(|e| e.contains("cpu[] requires a positive integer")));
    }
}

/// A record declaration's layout and where each field, in declaration
/// order, lies in it.
/// A record declaration's layout and where each field, in declaration order,
/// lies in it: one per declaration for as long as the process runs, read
/// with no count to keep (`value::Layout::intern`).
pub(crate) type Shape = &'static (&'static crate::value::Layout, Vec<usize>);

fn layout_of(s: &crate::ir::StructDef) -> Shape {
    static SHAPES: std::sync::Mutex<std::collections::BTreeMap<(String, String, Vec<String>), Shape>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
    let declared: Vec<String> = s.fields.iter().map(|(f, _)| f.clone()).collect();
    let key = (s.name.clone(), s.shown.clone(), declared.clone());
    let mut shapes = SHAPES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = shapes.get(&key) {
        return found;
    }
    let layout = crate::value::Layout::intern(s.name.clone(), s.shown.clone(), declared.clone());
    let slots = declared.iter().map(|f| layout.slot(f).unwrap_or(0)).collect();
    let shape: Shape = Box::leak(Box::new((layout, slots)));
    shapes.insert(key, shape);
    shape
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn feed(s: &mut Session, src: &str) -> Result<Option<String>, String> {
        s.feed(&crate::build(src)?).map(|v| v.map(|v| format!("{}", v)))
    }

    /// A notebook's cells are whole fragments: a name one cell defines
    /// among several statements is the next cell's, its type with it
    #[test]
    fn a_cells_names_are_the_next_cells() {
        let mut s = Session::new();
        assert_eq!(feed(&mut s, "use frame, math\nprices = [187.5, 402.25, 121.0]\nlog['mean: `mean[tensor[prices]]`']\nlen[prices]"), Ok(Some("3".to_string())));
        // (a loop's last value is its log's `done`, which shows nothing)
        assert_eq!(feed(&mut s, "for p in prices\n    log[p * 2.0]"), Ok(Some("done".to_string())));
        assert_eq!(feed(&mut s, "df = frame.new[price = prices]\nlen[df]"), Ok(Some("3".to_string())));
        assert_eq!(feed(&mut s, "df.price[0]"), Ok(Some("187.5".to_string())));
        assert!(feed(&mut s, "df").map(|v| v.unwrap_or_default().contains("price")).unwrap_or(false));
    }

    /// A function one cell defines is the next cells' to call, as a name
    /// is — the lowering knew it, and the check of the IR it made had
    /// refused the call as one of a function the program does not define,
    /// an internal error (it ran only with CROW_IR_VERIFY=off); a record's
    /// shape and a function calling an earlier one cross cells too, and a
    /// cell that defines a function again calls its own
    #[test]
    fn a_cells_functions_are_the_next_cells() {
        let mut s = Session::new();
        assert_eq!(feed(&mut s, "fn double[n]\n    n * 2"), Ok(None));
        assert_eq!(feed(&mut s, "double[3]"), Ok(Some("6".to_string())));
        assert_eq!(feed(&mut s, "fn quad[n]\n    double[double[n]]"), Ok(None));
        assert_eq!(feed(&mut s, "quad[2]"), Ok(Some("8".to_string())));
        assert_eq!(feed(&mut s, "struct Point [x, y]"), Ok(None));
        assert_eq!(feed(&mut s, "fn norm[p]\n    p.x * p.x + p.y * p.y"), Ok(None));
        assert_eq!(feed(&mut s, "norm[Point[3, 4]]"), Ok(Some("25".to_string())));
        assert_eq!(feed(&mut s, "fn double[n]\n    n * 3\ndouble[3]"), Ok(Some("9".to_string())));
        assert_eq!(feed(&mut s, "double[2]"), Ok(Some("6".to_string())));
    }
}
