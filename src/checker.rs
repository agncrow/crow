/// Automatic verification for Crow programs.
/// Implements `crow check` — purity detection, static checks,
/// edge case generation, and regression snapshots.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::ast::*;
use crate::type_infer::{Ty, FnSig, TypeContext};
use crate::value::{CrowValue, Value};

// --- Impure builtins ---
/// The builtins a pure function cannot call: the effects (crate::ir::EFFECTS),
/// the draws from the random generator (crate::ir::DRAWS), and what reads
/// the file system or the session's state without being an effect.
fn impure_builtin(name: &str) -> bool {
    crate::ir::is_effect(name) || crate::ir::DRAWS.contains(&name) || matches!(name, "exists" | "running" | "captured")
}


// --- Issue types ---

#[derive(Debug, Clone)]
pub enum Issue {
    UnusedVariable { func: String, var: String, line: usize },
    UnreachableCode { func: String, line: usize, detail: String },
    UndefinedVariable { func: String, var: String, line: usize },
    EdgeCaseCrash { func: String, input_desc: String, error: String },
    RegressionChange { func: String, input_desc: String, was: String, now: String },
    /// A view or event handler reaches a blocking builtin — it runs on the
    /// live loop, so every connected client freezes while it runs.
    BlockingInView { view: String, builtin: String, path: Vec<String>, line: usize },
    /// A view/handler reads a variable captured from the database at startup —
    /// every render serves launch-time data, forever.
    StaleSnapshot { view: String, var: String, source_line: usize, line: usize },
    /// A view/handler queries a remote database directly — a network round
    /// trip per render, and every connected client waits on it.
    RemoteQueryInView { view: String, conn: String, url: String, line: usize },
    /// A while+sleep poll loop reads a query result captured before the loop —
    /// every iteration decides on launch-time data.
    StaleLoopRead { var: String, source_line: usize, line: usize },
    /// A truth in a test block failed (or its body errored).
    TestFailure { block: String, line: usize, detail: String },
    /// css/app.css overrides a --crow-* token that doesn't exist.
    UnknownStyleToken { name: String, suggestion: String },
    /// The app baseline moved: a declaration's derived behaviour or
    /// composition differs from what was accepted (app-baseline-design.md).
    BaselineChange { key: String, file: String, was: String, now: String, reach: Vec<String>, expected: bool },
    /// A kernel rule over the wiring: a modal nobody can close, a control
    /// that does nothing, a handler nobody can reach.
    UiRule { subject: String, file: String, detail: String, fix: String },
    /// A value read from the outside world flows into a decision with no
    /// freshness guard — the stale-data class.
    UnguardedStaleness { var: String, root: String, read_line: usize, decision_line: usize },
    /// Layer-0 contract: a pure fn returned different results for the same
    /// input — hidden state or an impure builtin the classifier missed.
    Nondeterministic { func: String, input_desc: String },
}

impl Issue {
    pub fn summary(&self) -> String {
        match self {
            Issue::UnusedVariable { func, var, .. } =>
                format!("{} has unused variable '{}'", func, var),
            Issue::UnreachableCode { func, detail, .. } =>
                format!("{} has unreachable code — {}", func, detail),
            Issue::UndefinedVariable { func, var, .. } =>
                format!("{} uses undefined variable '{}'", func, var),
            Issue::EdgeCaseCrash { func, input_desc, error } =>
                format!("{} crashes on {} — {}", func, input_desc, error),
            Issue::RegressionChange { func, input_desc, was, now } =>
                format!("{} behavior changed: {} was {}, now {}", func, input_desc, was, now),
            Issue::BaselineChange { key, file, was, now, expected, .. } => {
                if now.is_empty() {
                    return format!("{} was removed (it was in {})", key, file);
                }
                let scope = if *expected { "in what you edited" } else { "reached from elsewhere" };
                let lines = crate::baseline::compact_diff(was, now);
                let shown: Vec<String> = lines.iter().take(8).map(|l| format!("          {}", l)).collect();
                let more = if lines.len() > 8 { format!("\n          … {} more", lines.len() - 8) } else { String::new() };
                format!("{} changed ({}, {}):\n{}{}", key, file, scope, shown.join("\n"), more)
            }
            Issue::UiRule { subject, file, detail, .. } =>
                format!("{} ({}): {}", subject, file, detail),
            Issue::BlockingInView { view, builtin, path, .. } =>
                format!(
                    "'{}' holds the session's other clicks until it ends — {}[] blocks it ({})",
                    view, builtin, path.join(" → ")
                ),
            Issue::StaleSnapshot { view, var, source_line, .. } =>
                format!(
                    "'{}' serves launch-time data — '{}' was queried once at startup (line {}) and never refreshed",
                    view, var, source_line
                ),
            Issue::RemoteQueryInView { view, conn, url, .. } =>
                format!(
                    "'{}' waits on the network every render — query['{}', …] where '{}' is {}",
                    view, conn, conn, url
                ),
            Issue::StaleLoopRead { var, source_line, .. } =>
                format!(
                    "the poll loop decides on launch-time data — '{}' was queried once (line {}) and read every iteration",
                    var, source_line
                ),
            Issue::TestFailure { block, detail, .. } =>
                format!("{} — {}", block, detail),
            Issue::UnknownStyleToken { name, suggestion } => {
                if suggestion.is_empty() {
                    format!("css/app.css overrides '{}', which is not a style token", name)
                } else {
                    format!("css/app.css overrides '{}', which is not a style token — did you mean '{}'?", name, suggestion)
                }
            }
            Issue::UnguardedStaleness { var, root, read_line, .. } => {
                if var == root {
                    format!(
                        "'{}' was read from outside (line {}) and decides this branch — nothing checks how old it is",
                        var, read_line
                    )
                } else {
                    format!(
                        "'{}' comes from '{}', read from outside (line {}), and decides this branch — nothing checks how old it is",
                        var, root, read_line
                    )
                }
            }
            Issue::Nondeterministic { func, input_desc } =>
                format!(
                    "'{}' is pure but returned different results for the same input ({}) — hidden state or an impure call the classifier missed",
                    func, input_desc
                ),
        }
    }

    pub fn line(&self) -> usize {
        match self {
            Issue::UnusedVariable { line, .. } => *line,
            Issue::UnreachableCode { line, .. } => *line,
            Issue::UndefinedVariable { line, .. } => *line,
            Issue::EdgeCaseCrash { .. } => 0,
            Issue::RegressionChange { .. } => 0,
            Issue::BaselineChange { .. } | Issue::UiRule { .. } => 0,
            Issue::BlockingInView { line, .. } => *line,
            Issue::StaleSnapshot { line, .. } => *line,
            Issue::RemoteQueryInView { line, .. } => *line,
            Issue::StaleLoopRead { line, .. } => *line,
            Issue::TestFailure { line, .. } => *line,
            Issue::UnknownStyleToken { .. } => 0,
            Issue::UnguardedStaleness { decision_line, .. } => *decision_line,
            Issue::Nondeterministic { .. } => 0,
        }
    }

    /// Get the function name this issue belongs to.
    #[allow(dead_code)]
    pub fn func(&self) -> &str {
        match self {
            Issue::UnusedVariable { func, .. } => func,
            Issue::UnreachableCode { func, .. } => func,
            Issue::UndefinedVariable { func, .. } => func,
            Issue::BlockingInView { view, .. } => view,
            Issue::StaleSnapshot { view, .. } => view,
            Issue::RemoteQueryInView { view, .. } => view,
            Issue::StaleLoopRead { .. } => "<top-level>",
            Issue::TestFailure { block, .. } => block,
            Issue::UnknownStyleToken { .. } => "<css/app.css>",
            Issue::UnguardedStaleness { .. } => "<top-level>",
            Issue::Nondeterministic { func, .. } => func,
            Issue::EdgeCaseCrash { func, .. } => func,
            Issue::RegressionChange { func, .. } => func,
            Issue::BaselineChange { key, .. } => key.split(' ').nth(1).unwrap_or(key.as_str()),
            Issue::UiRule { subject, .. } => subject,
        }
    }

    /// A finding whose cure the language has as a form is an error, not a
    /// warning: a launch-time snapshot or a network query in the render
    /// path is what `derive` exists for, and `d` in the check loop applies
    /// it (crow-check.md, tier 2).
    pub fn is_error(&self) -> bool {
        matches!(self, Issue::StaleSnapshot { .. } | Issue::RemoteQueryInView { .. })
    }

    /// Get a "try" command the user can paste, if applicable.
    pub fn try_input(&self) -> Option<String> {
        match self {
            Issue::EdgeCaseCrash { input_desc, .. } => Some(input_desc.clone()),
            Issue::RegressionChange { input_desc, .. } => Some(input_desc.clone()),
            _ => None,
        }
    }

    /// Suggest a fix for this issue.
    pub fn suggest(&self) -> String {
        match self {
            Issue::BaselineChange { now, reach, .. } => {
                let pages: Vec<&String> = reach.iter().filter(|r| r.starts_with("page ")).collect();
                let views = reach.len() - pages.len();
                let mut named: Vec<String> = pages.iter().map(|p| p.to_string()).collect();
                if views > 0 && views <= 4 { named.extend(reach.iter().filter(|r| !r.starts_with("page ")).cloned()); }
                else if views > 4 { named.push(format!("{} views", views)); }
                let where_ = if named.is_empty() { String::new() } else { format!(" — reaches {}", named.join(", ")) };
                if now.is_empty() { "accept with a to drop it from the baseline, or restore it".to_string() }
                else { format!("if intended, accept with a and the baseline moves; otherwise fix the cause{}", where_) }
            }
            Issue::UiRule { fix, .. } => fix.clone(),
            Issue::UnusedVariable { var, line, .. } =>
                if *line == 0 {
                    format!("remove parameter '{}'", var)
                } else {
                    format!("remove '{}' or use _ for destructuring", var)
                },
            Issue::UnreachableCode { .. } =>
                "remove the dead code or move it before the return".to_string(),
            Issue::BlockingInView { builtin, path, .. } =>
                if path.len() >= 3 {
                    format!(
                        "schedule it off the loop: later[{}] — the work runs in the background and its state writes re-render every client",
                        path[1]
                    )
                } else {
                    format!(
                        "extract the {}[] work into a fn and schedule it with later[fn] — results land in state, every client stays live",
                        builtin
                    )
                },
            Issue::StaleSnapshot { var, .. } =>
                format!(
                    "make '{}' a derivation: `derive {}` with the query as its body is read like a fact and recomputed when a fact it reads or a table this program writes changes — press d to rewrite",
                    var, var
                ),
            Issue::RemoteQueryInView { .. } =>
                "move the query into a derivation with a freshness window: `derive rows[fresh = 60]` runs the network hop once per window, not per render, and the view reads `rows` — press d to rewrite the `name = query[…]` line, or a if this app means to query on every render (records --- crow: remote-render)".to_string(),
            Issue::StaleLoopRead { var, .. } =>
                format!("re-query '{}' inside the loop so each poll sees the current database", var),
            Issue::TestFailure { .. } =>
                "fix the code — or the truth, if the decision changed".to_string(),
            Issue::UnknownStyleToken { .. } =>
                "a typo'd token renders fine with the default value — silently unthemed. Fix the name; the blessed list is in style-module-design.md".to_string(),
            Issue::UnguardedStaleness { root, .. } =>
                format!(
                    "guard the read: if now[] - captured[{}] gt 10.0 → refuse. Data captured at one moment and trusted at another is the stale-data class",
                    root
                ),
            Issue::Nondeterministic { .. } =>
                "find the hidden state: shell/time/rand reached through a path the purity scan missed".to_string(),
            Issue::UndefinedVariable { var, .. } =>
                format!("'{}' is not defined — check for typos or add an assignment", var),
            Issue::EdgeCaseCrash { error, .. } => {
                if error.contains("overflow") {
                    "add a bounds check or use saturating arithmetic".to_string()
                } else if error.contains("out of range") || error.contains("index") {
                    "check length before indexing, e.g. if len[lst] > 0".to_string()
                } else if error.contains("divide by zero") || error.contains("division by zero") {
                    "check for zero before dividing, e.g. if b != 0".to_string()
                } else {
                    "add a guard to handle this edge case".to_string()
                }
            }
            Issue::RegressionChange { was, now, .. } =>
                format!("was {} — verify the new result {} is correct, then press 'a' to accept", was, now),
        }
    }
}

// --- Test blocks (the `test` keyword): authored truths and laws ---

#[derive(Default, Clone, Copy)]
pub struct TestCounts {
    pub blocks: usize,
    pub truths: usize,
    pub laws: usize,
    pub inputs_run: usize,
}

/// How many assertions a test block reaches: the `assert` calls in its
/// body (a bare truth is lowered as one) and in every function it calls,
/// transitively.
fn asserts_reached(program: &crate::ir::Program, test: &crate::ir::Test) -> usize {
    use crate::ir::{Expr, Target};
    let fns: std::collections::HashMap<&str, &crate::ir::Fn> = program.parts.iter().flat_map(|p| p.fns.iter()).map(|f| (f.name.as_str(), f)).collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut todo: Vec<&[crate::ir::Stmt]> = vec![&test.body];
    let mut count = 0;
    while let Some(body) = todo.pop() {
        let mut called = Vec::new();
        crate::ir::visit_block(body, &mut |e| {
            if let Expr::Call { target, .. } = e {
                match target {
                    Target::Runtime(n) if n == "assert" => count += 1,
                    Target::User(n) => called.push(n.clone()),
                    _ => {}
                }
            }
        });
        for n in called {
            if seen.insert(n.clone()) {
                if let Some(f) = fns.get(n.as_str()) {
                    todo.push(&f.body);
                }
            }
        }
    }
    count
}

/// Execute every test block: attached (unnamed — the fn above's examples and
/// laws) and named (standalone scenarios). Params are generated inputs,
/// boundaries first; bodies run in the interpreter against declarations only
/// (the verification boundary: tests see what callers see). The lowering
/// types the params by their use and lowers each bare boolean line as an
/// assertion, so a failed truth reports its operands; each block is lowered
/// once and run for each of its inputs.
pub fn run_test_blocks(stmts: &[Stmt]) -> (Vec<Issue>, TestCounts) {
    let mut issues = Vec::new();
    let mut counts = TestCounts::default();

    // Collect blocks with attachment labels.
    struct Block {
        label: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        line: usize,
    }
    let mut blocks = Vec::new();
    let mut line = 0usize;
    let mut last_fn: Option<String> = None;
    for s in stmts {
        match s {
            Stmt::Line(n) => line = *n,
            Stmt::FnDecl { name, .. } => last_fn = Some(name.clone()),
            Stmt::TestBlock { name, params, body } => {
                let label = match name {
                    Some(n) => format!("test {}", n),
                    None => format!(
                        "{} tests",
                        last_fn.clone().unwrap_or_else(|| "<top-level>".into())
                    ),
                };
                blocks.push(Block {
                    label,
                    params: params.clone(),
                    body: body.clone(),
                    line,
                });
            }
            _ => {}
        }
    }
    if blocks.is_empty() {
        return (issues, counts);
    }
    counts.blocks = blocks.len();

    let decls = declarations(stmts);

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    for block in &blocks {
        // the block lowered with the declarations, once: its inputs typed by
        // their use and its bare boolean lines asserted (lower_ir's
        // lower_test); what the lowering refuses is the block's failure
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::ir_interp::prepare_test(&decls, &block.label, &block.params, &block.body)));
        let (program, pctx) = match prepared {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                issues.push(Issue::TestFailure { block: block.label.clone(), line: block.line, detail: e });
                continue;
            }
            Err(_) => {
                issues.push(Issue::TestFailure { block: block.label.clone(), line: block.line, detail: "panicked".into() });
                continue;
            }
        };
        let Some(test) = program.parts.iter().flat_map(|p| p.tests.iter()).last() else {
            continue;
        };
        counts.truths += test.truths;
        let sig = FnSig {
            params: test.params.iter().map(|(p, t)| (p.clone(), crate::lower_ir::as_inferred(t))).collect(),
            return_type: Ty::Unit,
            body: Vec::new(),
        };

        // Input tuples: none → run once; params → generated, boundaries first.
        let combos: Vec<Vec<CrowValue>> = if block.params.is_empty() {
            vec![Vec::new()]
        } else {
            if !can_generate_edge_cases(&sig)
                || sig
                    .params
                    .iter()
                    .any(|(_, ty)| matches!(ty, Ty::Unknown))
            {
                issues.push(Issue::TestFailure {
                    block: block.label.clone(),
                    line: block.line,
                    detail: "cannot generate inputs for these param types yet — give the params simpler types or drop them".into(),
                });
                continue;
            }
            counts.laws += 1;
            generate_input_combos(&sig)
        };
        // a block that reaches no assertion — no bare truth, no assert[],
        // in its body or in a function it calls — cannot fail, and is a
        // failure itself (a block whose one truth was never a truth had
        // passed for weeks; a law whose truths stood under its
        // precondition had asserted nothing at all)
        if asserts_reached(&program, test) == 0 {
            issues.push(Issue::TestFailure {
                block: block.label.clone(),
                line: block.line,
                detail: "asserts nothing: no bare truth and no assert[…] is reached by the block or the functions it calls".into(),
            });
            continue;
        }

        'combos: for combo in &combos {
            counts.inputs_run += 1;
            // a replay runs each test as the recording ran it: its draws
            // seeded afresh, as a rehearsal's are
            let replaying = crow_runtime::fixtures::replaying().is_some();
            if replaying {
                crow_runtime::rng::set_seed(0);
                let _ = crow_runtime::fixtures::take_misses();
            }
            // the declarations run first; one that fails (a database that is
            // down, a top-level query that errors) is the finding, named as
            // such — not the "undefined variable" every later test would see
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let inputs: Vec<(String, CrowValue)> = block.params.iter().cloned().zip(combo.iter().cloned()).collect();
                crate::ir_interp::run_prepared_test(&program, &pctx, &inputs)
            }));
            let failure = match result {
                Ok(Ok(_)) => None,
                Ok(Err(e)) => Some(e),
                Err(_) => Some("panicked".to_string()),
            };
            if let Some(msg) = failure {
                let at = if block.params.is_empty() {
                    String::new()
                } else {
                    let vals: Vec<String> = block
                        .params
                        .iter()
                        .zip(combo.iter())
                        .map(|(p, v)| format!("{} = {}", p, v.value))
                        .collect();
                    format!("  (at {})", vals.join(", "))
                };
                // what the replay could not answer, which is likely why
                let missed: String = if replaying {
                    crow_runtime::fixtures::take_misses()
                        .iter()
                        .map(|m| format!("\n     {}", m))
                        .collect()
                } else {
                    String::new()
                };
                issues.push(Issue::TestFailure {
                    block: block.label.clone(),
                    line: block.line,
                    detail: format!("{}{}{}", msg, at, missed),
                });
                break 'combos; // one failure per block keeps reports readable
            }
        }
    }

    std::panic::set_hook(prev_hook);
    (issues, counts)
}

// --- Layer 0: determinism contract ---

/// A pure fn must return identical results for identical inputs. This can
/// only fail when the purity classification or the runtime is wrong — which
/// is exactly why it's checked: it audits the machinery, free.
/// (The error-propagation contract is runtime-enforced in the interpreter,
/// so probing it here is vacuous — it becomes meaningful as a compiled-mode
/// contract, queued with the compiled test-runner milestone.)
pub fn run_determinism_probe(
    ctx: &TypeContext,
    stmts: &[Stmt],
    pure_fns: &BTreeSet<String>,
    recursive_fns: &BTreeSet<String>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let decls: Vec<Stmt> = declarations(stmts);
    // the declarations lowered once; every probe runs on a fresh walker
    let Ok((program, pctx)) = crate::ir_interp::prepare(&decls, "check") else { return issues };
    for name in pure_fns {
        if recursive_fns.contains(name) {
            continue;
        }
        let sig = match ctx.functions.get(name) {
            Some(s) => s,
            None => continue,
        };
        if !can_generate_edge_cases(sig)
            || sig.params.iter().any(|(_, ty)| matches!(ty, Ty::Unknown))
        {
            continue;
        }
        for combo in generate_input_combos(sig).iter().take(2) {
            let mut it = crate::ir_interp::IrInterp::new(&pctx.registry, "check");
            it.load(&program);
            let _ = it.run_main(&program);
            let a = it.call(name, combo.clone());
            let b = it.call(name, combo.clone());
            let (Ok(a), Ok(b)) = (a, b) else { continue };
            let (da, db) = (format!("{}", a.value), format!("{}", b.value));
            if da != db {
                issues.push(Issue::Nondeterministic {
                    func: name.clone(),
                    input_desc: format!("{} → {} then {}", format_call(name, combo), da, db),
                });
                break;
            }
        }
    }
    issues
}

// --- Blocking-in-view analysis ---

/// Builtins that stall the caller for arbitrary time. A view renders for
/// every connected client and a handler runs on the live loop — reaching one
/// of these freezes the whole app for everyone (the asyncio-event-loop bug
/// class: slow work executed directly on the loop). Deliberately excludes
/// `query`/`exec`: render-time database reads are the blessed derive idiom.
const BLOCKING_BUILTINS: &[&str] = &["sleep", "input", "shell"];

/// For each view and event handler, find a call path to a blocking builtin.
pub fn find_blocking_in_views(
    ctx: &TypeContext,
    fn_lines: &BTreeMap<String, usize>,
) -> Vec<Issue> {
    let mut call_graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, sig) in &ctx.functions {
        let mut callees = BTreeSet::new();
        collect_callees_from_stmts(&sig.body, &mut callees);
        call_graph.insert(name.clone(), callees);
    }

    // Fns scheduled via later[] are the sanctioned home for slow work —
    // they run off the loop, so they are never blocking roots even when
    // they are also referenced as handlers.
    let mut later_targets: BTreeSet<String> = BTreeSet::new();
    for sig in ctx.functions.values() {
        collect_later_target_names(&sig.body, &mut later_targets);
    }

    let roots: BTreeSet<&String> = ctx
        .views
        .iter()
        .chain(ctx.view_handlers.iter())
        .filter(|r| !later_targets.contains(*r))
        .collect();
    let mut issues = Vec::new();
    for root in roots {
        let mut path = vec![root.clone()];
        let mut visited = BTreeSet::new();
        if let Some((builtin, chain)) =
            reach_blocking(root, &call_graph, &mut path, &mut visited)
        {
            issues.push(Issue::BlockingInView {
                view: root.clone(),
                builtin,
                path: chain,
                line: fn_lines.get(root).copied().unwrap_or(0),
            });
        }
    }
    issues
}

/// Fn names appearing as later[f]/running[f] targets anywhere in a body.
fn collect_later_target_names(body: &[Stmt], out: &mut BTreeSet<String>) {
    fn walk(e: &Expr, out: &mut BTreeSet<String>) {
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
    for stmt in body {
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
                collect_later_target_names(then_body, out);
                if let Some(eb) = else_body {
                    collect_later_target_names(eb, out);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                collect_later_target_names(body, out);
            }
            _ => {}
        }
    }
}

fn reach_blocking(
    fn_name: &str,
    call_graph: &BTreeMap<String, BTreeSet<String>>,
    path: &mut Vec<String>,
    visited: &mut BTreeSet<String>,
) -> Option<(String, Vec<String>)> {
    if !visited.insert(fn_name.to_string()) {
        return None;
    }
    let callees = call_graph.get(fn_name)?;
    for c in callees {
        if BLOCKING_BUILTINS.contains(&c.as_str()) {
            let mut chain = path.clone();
            chain.push(c.clone());
            return Some((c.clone(), chain));
        }
    }
    for c in callees {
        if call_graph.contains_key(c) {
            path.push(c.clone());
            if let Some(found) = reach_blocking(c, call_graph, path, visited) {
                return Some(found);
            }
            path.pop();
        }
    }
    None
}

// --- Stale-snapshot analysis ---

/// Top-level variables holding query results are a snapshot of the database
/// at launch. A view or handler that reads one serves that snapshot forever —
/// the "stale coverage / stale prints" class. Refreshing the variable in any
/// handler clears the hazard; the blessed idiom is querying inside the view.
pub fn find_stale_snapshots(
    ctx: &TypeContext,
    top_level_stmts: &[Stmt],
    fn_lines: &BTreeMap<String, usize>,
) -> Vec<Issue> {
    // 1. Startup snapshots: top-level `x = …query[…]…`.
    let mut snapshots: BTreeMap<String, usize> = BTreeMap::new();
    let mut line = 0usize;
    for stmt in top_level_stmts {
        match stmt {
            Stmt::Line(n) => line = *n,
            Stmt::Assign { name, value } if expr_calls_query(value) => {
                snapshots.insert(name.clone(), line);
            }
            _ => {}
        }
    }
    if snapshots.is_empty() {
        return Vec::new();
    }

    // 2. A handler that re-queries the variable keeps it fresh — not a hazard.
    for handler in &ctx.view_handlers {
        if let Some(sig) = ctx.functions.get(handler) {
            let mut refreshed = BTreeSet::new();
            collect_query_assigns(&sig.body, &mut refreshed);
            for name in refreshed {
                snapshots.remove(&name);
            }
        }
    }

    // 3. Views/handlers reading a snapshot serve launch-time data. A view
    // registered in page[] is also collected as a handler — one root, one
    // finding.
    let mut issues = Vec::new();
    let roots: BTreeSet<&String> = ctx.views.iter().chain(ctx.view_handlers.iter()).collect();
    for root in roots {
        let Some(sig) = ctx.functions.get(root) else { continue };
        let mut reads = BTreeSet::new();
        collect_reads_from_stmts(&sig.body, &mut reads);
        for (var, source_line) in &snapshots {
            if reads.contains(var) {
                issues.push(Issue::StaleSnapshot {
                    view: root.clone(),
                    var: var.clone(),
                    source_line: *source_line,
                    line: fn_lines.get(root).copied().unwrap_or(0),
                });
            }
        }
    }
    issues
}

fn expr_calls_query(e: &Expr) -> bool {
    match e {
        Expr::Call { callee, args } => {
            let is_query = match callee.as_ref() {
                Expr::Ident(n) => n == "query",
                Expr::FieldAccess { object, field } => {
                    matches!(object.as_ref(), Expr::Ident(m) if m == "db") && field == "query"
                }
                _ => false,
            };
            is_query
                || args.iter().any(expr_calls_query)
                || expr_calls_query(callee)
        }
        Expr::BinOp { left, right, .. } => expr_calls_query(left) || expr_calls_query(right),
        Expr::UnaryOp { expr, .. } => expr_calls_query(expr),
        Expr::FieldAccess { object, .. } => expr_calls_query(object),
        Expr::Array(items) => items.iter().any(expr_calls_query),
        Expr::Named { value, .. } => expr_calls_query(value),
        _ => false,
    }
}

fn collect_query_assigns(body: &[Stmt], out: &mut BTreeSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Assign { name, value } if expr_calls_query(value) => {
                out.insert(name.clone());
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_query_assigns(then_body, out);
                if let Some(eb) = else_body {
                    collect_query_assigns(eb, out);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                collect_query_assigns(body, out);
            }
            _ => {}
        }
    }
}

// --- Remote-query-in-view analysis ---

/// Connections opened at top level with a literal url: conn name → url.
fn top_level_connections(top_level_stmts: &[Stmt]) -> BTreeMap<String, String> {
    let mut conns = BTreeMap::new();
    for stmt in top_level_stmts {
        if let Stmt::Assign { name, value } = stmt {
            if let Expr::Call { callee, args } = value {
                let is_connect = match callee.as_ref() {
                    Expr::Ident(n) => n == "connect",
                    Expr::FieldAccess { object, field } => {
                        matches!(object.as_ref(), Expr::Ident(m) if m == "db")
                            && field == "connect"
                    }
                    _ => false,
                };
                if is_connect {
                    if let Some(Expr::Str(url)) = args.first() {
                        conns.insert(name.clone(), url.clone());
                    }
                }
            }
        }
    }
    conns
}

/// A render-path query against a remote database is a network round trip
/// per render, serialized on the live loop — every connected client waits.
/// Local sqlite is the blessed render-time source and is not flagged.
pub fn find_remote_queries_in_views(
    ctx: &TypeContext,
    top_level_stmts: &[Stmt],
    fn_lines: &BTreeMap<String, usize>,
) -> Vec<Issue> {
    let remote: BTreeMap<String, String> = top_level_connections(top_level_stmts)
        .into_iter()
        .filter(|(_, url)| !url.starts_with("sqlite"))
        .collect();
    if remote.is_empty() {
        return Vec::new();
    }
    let mut issues = Vec::new();
    // The render path only: a handler that asks a remote database on a
    // click is on-demand work, the blessed home for it (crow-check.md's
    // matrix); a view that asks on every render is the hazard.
    for root in ctx.views.iter() {
        let Some(sig) = ctx.functions.get(root) else { continue };
        let mut flagged: BTreeSet<String> = BTreeSet::new();
        collect_query_conns(&sig.body, &mut flagged);
        for conn in flagged {
            if let Some(url) = remote.get(&conn) {
                issues.push(Issue::RemoteQueryInView {
                    view: root.clone(),
                    conn: conn.clone(),
                    url: url.clone(),
                    line: fn_lines.get(root).copied().unwrap_or(0),
                });
            }
        }
    }
    issues
}

/// Connection variables passed as first arg to query/exec anywhere in a body.
fn collect_query_conns(body: &[Stmt], out: &mut BTreeSet<String>) {
    fn walk_expr(e: &Expr, out: &mut BTreeSet<String>) {
        if let Expr::Call { callee, args } = e {
            let name = match callee.as_ref() {
                Expr::Ident(n) => Some(n.as_str()),
                Expr::FieldAccess { object, field } => {
                    if matches!(object.as_ref(), Expr::Ident(m) if m == "db") {
                        Some(field.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if matches!(name, Some("query") | Some("exec")) {
                if let Some(Expr::Ident(conn)) = args.first() {
                    out.insert(conn.clone());
                }
            }
        }
        match e {
            Expr::Call { callee, args } => {
                walk_expr(callee, out);
                for a in args {
                    walk_expr(a, out);
                }
            }
            Expr::BinOp { left, right, .. } => {
                walk_expr(left, out);
                walk_expr(right, out);
            }
            Expr::UnaryOp { expr, .. } => walk_expr(expr, out),
            Expr::FieldAccess { object, .. } => walk_expr(object, out),
            Expr::Array(items) => items.iter().for_each(|i| walk_expr(i, out)),
            Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                walk_expr(k, out);
                walk_expr(v, out);
            }),
            Expr::Named { value, .. } => walk_expr(value, out),
            Expr::ForExpr { iter, .. } => walk_expr(iter, out),
            _ => {}
        }
    }
    for stmt in body {
        match stmt {
            Stmt::Assign { value, .. } => walk_expr(value, out),
            Stmt::ExprStmt(e) => walk_expr(e, out),
            Stmt::Return(Some(e)) => walk_expr(e, out),
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                walk_expr(condition, out);
                collect_query_conns(then_body, out);
                if let Some(eb) = else_body {
                    collect_query_conns(eb, out);
                }
            }
            Stmt::For { iter, body, .. } => {
                walk_expr(iter, out);
                collect_query_conns(body, out);
            }
            Stmt::While { condition, body } => {
                walk_expr(condition, out);
                collect_query_conns(body, out);
            }
            _ => {}
        }
    }
}

/// Drop findings the code has explicitly accepted with a pragma comment
/// on the line above the flagged declaration.
pub fn suppress_acknowledged(issues: Vec<Issue>, source: &str) -> Vec<Issue> {
    let lines: Vec<&str> = source.lines().collect();
    issues
        .into_iter()
        .filter(|i| match i {
            Issue::RemoteQueryInView { line, .. } => {
                let above = line.checked_sub(2).and_then(|n| lines.get(n));
                !matches!(above, Some(l) if l.trim().starts_with("--- crow: remote-render"))
            }
            _ => true,
        })
        .collect()
}

// --- Style-token check: css/app.css may override --crow-* tokens; a
// typo'd token fails silently (everything renders, slightly wrong), so
// unknown names get a did-you-mean. ---

pub const STYLE_TOKENS: &[&str] = &[
    "--crow-bg", "--crow-surface", "--crow-text", "--crow-muted",
    "--crow-border", "--crow-accent", "--crow-up", "--crow-down",
    "--crow-series-2", "--crow-series-3", "--crow-series-4", "--crow-series-5", "--crow-series-6",
];

pub fn check_style_tokens(css_path: &str) -> Vec<(String, String)> {
    let Ok(css) = std::fs::read_to_string(css_path) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let bytes = css.as_bytes();
    let mut i = 0;
    while let Some(pos) = css[i..].find("--crow-") {
        let start = i + pos;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-')
        {
            end += 1;
        }
        let name = &css[start..end];
        i = end;
        if STYLE_TOKENS.contains(&name) || !seen.insert(name.to_string()) {
            continue;
        }
        let suggestion = STYLE_TOKENS
            .iter()
            .min_by_key(|t| crate::db_check::catalog::levenshtein(name, t))
            .filter(|t| crate::db_check::catalog::levenshtein(name, t) <= 3)
            .map(|t| t.to_string())
            .unwrap_or_default();
        out.push((name.to_string(), suggestion));
    }
    out
}

// --- Unguarded-staleness analysis (F-2's static half) ---

/// A value read from the outside world (query, read, fetch…) that flows —
/// directly or through computation — into a decision, with no freshness
/// guard anywhere. This is the propagation the runtime deliberately does
/// not carry: capture lives on the read, the checker traces the flow.
pub fn find_unguarded_staleness(stmts: &[Stmt]) -> Vec<Issue> {
    const READERS: &[&str] = &["query", "read", "fetch", "recv", "load", "csv", "lines", "input", "ls"];

    fn calls_reader(e: &Expr) -> bool {
        match e {
            Expr::Call { callee, args } => {
                let named = match callee.as_ref() {
                    Expr::Ident(n) => READERS.contains(&n.as_str()),
                    Expr::FieldAccess { field, .. } => READERS.contains(&field.as_str()),
                    _ => false,
                };
                named || args.iter().any(calls_reader)
            }
            Expr::BinOp { left, right, .. } => calls_reader(left) || calls_reader(right),
            Expr::FieldAccess { object, .. } => calls_reader(object),
            _ => false,
        }
    }

    // Guarded once anywhere in the file = guarded (conservative: one guard
    // is evidence the author is thinking about freshness).
    let mut guarded: BTreeSet<String> = BTreeSet::new();
    fn collect_guards(stmts: &[Stmt], out: &mut BTreeSet<String>) {
        fn walk(e: &Expr, out: &mut BTreeSet<String>) {
            if let Expr::Call { callee, args } = e {
                if matches!(callee.as_ref(), Expr::Ident(n) if n == "captured") {
                    if let Some(Expr::Ident(v)) = args.first() {
                        out.insert(v.clone());
                    }
                }
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
            }
        }
        for st in stmts {
            match st {
                Stmt::Assign { value, .. } => walk(value, out),
                Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk(e, out),
                Stmt::If { condition, then_body, else_body, .. } => {
                    walk(condition, out);
                    collect_guards(then_body, out);
                    if let Some(eb) = else_body {
                        collect_guards(eb, out);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => collect_guards(body, out),
                Stmt::FnDecl { body, .. } => collect_guards(body, out),
                _ => {}
            }
        }
    }
    collect_guards(stmts, &mut guarded);

    // Reader-derived variables, and the decisions that use them.
    // name → (line of the read, root reader variable)
    let mut derived: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut issues = Vec::new();
    let mut line = 0usize;
    let mut reported: BTreeSet<String> = BTreeSet::new();
    fn scan(
        stmts: &[Stmt],
        line: &mut usize,
        derived: &mut BTreeMap<String, (usize, String)>,
        guarded: &BTreeSet<String>,
        reported: &mut BTreeSet<String>,
        issues: &mut Vec<Issue>,
    ) {
        for st in stmts {
            match st {
                Stmt::Line(n) => *line = *n,
                Stmt::Assign { name, value } => {
                    if calls_reader(value) {
                        derived.insert(name.clone(), (*line, name.clone()));
                    } else {
                        // Flows onward: a value computed from a reader value
                        // is still reader-derived (what the runtime drops).
                        let mut reads = BTreeSet::new();
                        collect_reads_from_stmts(
                            std::slice::from_ref(&Stmt::ExprStmt(value.clone())),
                            &mut reads,
                        );
                        if let Some(src) = reads.iter().find_map(|r| derived.get(r).cloned()) {
                            derived.insert(name.clone(), src);
                        }
                    }
                }
                Stmt::If { condition, then_body, else_body, .. } => {
                    let mut reads = BTreeSet::new();
                    collect_reads_from_stmts(
                        std::slice::from_ref(&Stmt::ExprStmt(condition.clone())),
                        &mut reads,
                    );
                    for r in &reads {
                        if let Some((read_line, root)) = derived.get(r) {
                            // A guard anywhere on the root read covers every
                            // value derived from it.
                            if !guarded.contains(r)
                                && !guarded.contains(root)
                                && reported.insert(root.clone())
                            {
                                issues.push(Issue::UnguardedStaleness {
                                    var: r.clone(),
                                    root: root.clone(),
                                    read_line: *read_line,
                                    decision_line: *line,
                                });
                            }
                        }
                    }
                    scan(then_body, line, derived, guarded, reported, issues);
                    if let Some(eb) = else_body {
                        scan(eb, line, derived, guarded, reported, issues);
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    scan(body, line, derived, guarded, reported, issues)
                }
                _ => {}
            }
        }
    }
    scan(stmts, &mut line, &mut derived, &guarded, &mut reported, &mut issues);
    issues
}

// --- Stale-loop-read analysis ---

/// A top-level `while` loop that sleeps is a poll loop. Reading a query
/// result captured before the loop means every iteration decides on
/// launch-time data (the stale-prints class).
pub fn find_stale_loop_reads(top_level_stmts: &[Stmt]) -> Vec<Issue> {
    // Startup snapshots, as in find_stale_snapshots.
    let mut snapshots: BTreeMap<String, usize> = BTreeMap::new();
    let mut line = 0usize;
    let mut issues = Vec::new();
    for stmt in top_level_stmts {
        match stmt {
            Stmt::Line(n) => line = *n,
            Stmt::Assign { name, value } if expr_calls_query(value) => {
                snapshots.insert(name.clone(), line);
            }
            Stmt::While { body, .. } => {
                let mut callees = BTreeSet::new();
                collect_callees_from_stmts(body, &mut callees);
                if !callees.contains("sleep") {
                    continue; // not a poll loop
                }
                let mut refreshed = BTreeSet::new();
                collect_query_assigns(body, &mut refreshed);
                let mut reads = BTreeSet::new();
                collect_reads_from_stmts(body, &mut reads);
                for (var, source_line) in &snapshots {
                    if reads.contains(var) && !refreshed.contains(var) {
                        issues.push(Issue::StaleLoopRead {
                            var: var.clone(),
                            source_line: *source_line,
                            line,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    issues
}

// --- Purity analysis ---

/// Returns the set of function names that are pure.
/// Functions that read reactive state: views, handlers and derivations,
/// and every function they call — the closure the validator exempts from
/// function-scope isolation. Their fact reads are not undefined names,
/// their fact writes are the point, and they are exercised by rendering,
/// never by synthetic inputs.
pub fn reactive_readers(ctx: &TypeContext) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = ctx.views.iter()
        .chain(ctx.view_handlers.iter())
        .chain(ctx.derives.iter())
        .cloned()
        .collect();
    let fns: BTreeMap<String, Vec<Stmt>> = ctx.functions.iter().map(|(n, s)| (n.clone(), s.body.clone())).collect();
    for root in out.clone() {
        if let Some(body) = fns.get(&root) {
            out.extend(crate::derive::called_fns(body, &fns));
        }
    }
    // a specialisation of a reactive function (`jump__1`, typed for one
    // call site) is the same function
    let clones: Vec<String> = fns.keys()
        .filter(|n| n.rsplit_once("__").map_or(false, |(base, sfx)| !sfx.is_empty() && sfx.chars().all(|c| c.is_ascii_digit()) && out.contains(base)))
        .cloned()
        .collect();
    out.extend(clones);
    out
}

pub fn find_pure_functions(ctx: &TypeContext) -> BTreeSet<String> {
    let mut impure: BTreeSet<String> = BTreeSet::new();
    let mut call_graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, sig) in &ctx.functions {
        let mut callees = BTreeSet::new();
        collect_callees_from_stmts(&sig.body, &mut callees);
        call_graph.insert(name.clone(), callees);
    }
    for (name, callees) in &call_graph {
        for c in callees {
            if impure_builtin(c.as_str()) {
                impure.insert(name.clone());
                break;
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (name, callees) in &call_graph {
            if impure.contains(name) { continue; }
            for c in callees {
                if impure.contains(c) {
                    impure.insert(name.clone());
                    changed = true;
                    break;
                }
            }
        }
    }
    let all: BTreeSet<String> = ctx.functions.keys().cloned().collect();
    all.difference(&impure).cloned().collect()
}

/// Detect functions that are recursive (directly or indirectly via the call graph).
pub fn find_recursive_functions(ctx: &TypeContext) -> BTreeSet<String> {
    let mut call_graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, sig) in &ctx.functions {
        let mut callees = BTreeSet::new();
        collect_callees_from_stmts(&sig.body, &mut callees);
        call_graph.insert(name.clone(), callees);
    }
    let mut recursive = BTreeSet::new();
    for name in ctx.functions.keys() {
        if is_reachable(name, name, &call_graph, &mut BTreeSet::new()) {
            recursive.insert(name.clone());
        }
    }
    recursive
}

/// Check if `target` is reachable from `current` in the call graph.
fn is_reachable(
    target: &str,
    current: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if !visited.insert(current.to_string()) { return false; }
    if let Some(callees) = graph.get(current) {
        for callee in callees {
            if callee == target { return true; }
            if is_reachable(target, callee, graph, visited) { return true; }
        }
    }
    false
}

fn collect_callees_from_stmts(stmts: &[Stmt], callees: &mut BTreeSet<String>) {
    for stmt in stmts { collect_callees_from_stmt(stmt, callees); }
}

fn collect_callees_from_stmt(stmt: &Stmt, callees: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Assign { value, .. } => collect_callees_from_expr(value, callees),
        Stmt::MultiAssign { values, .. } => {
            for v in values { collect_callees_from_expr(v, callees); }
        }
        Stmt::FnDecl { body, .. } => collect_callees_from_stmts(body, callees),
        Stmt::Return(Some(e)) => collect_callees_from_expr(e, callees),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::ExprStmt(e) => collect_callees_from_expr(e, callees),
        Stmt::If { condition, then_body, else_body } => {
            collect_callees_from_expr(condition, callees);
            collect_callees_from_stmts(then_body, callees);
            if let Some(eb) = else_body { collect_callees_from_stmts(eb, callees); }
        }
        Stmt::For { iter, body, .. } => {
            collect_callees_from_expr(iter, callees);
            collect_callees_from_stmts(body, callees);
        }
        Stmt::While { condition, body } => {
            collect_callees_from_expr(condition, callees);
            collect_callees_from_stmts(body, callees);
        }
        Stmt::FieldAssign { object, value, .. } => {
            collect_callees_from_expr(object, callees);
            collect_callees_from_expr(value, callees);
        }
        Stmt::IndexAssign { object, index, value } => {
            collect_callees_from_expr(object, callees);
            collect_callees_from_expr(index, callees);
            collect_callees_from_expr(value, callees);
        }
        _ => {}
    }
}

fn collect_callees_from_expr(expr: &Expr, callees: &mut BTreeSet<String>) {
    match expr {
        Expr::Call { callee, args } => {
            if let Expr::Ident(name) = callee.as_ref() {
                callees.insert(name.clone());
                // a password hash is salted with fresh random bytes, so
                // `hash[x, 'password']` — or an algorithm only named at run
                // time — draws as rand does
                if name == "hash" && !matches!(args.get(1), Some(Expr::Str(a)) if a != "password") {
                    callees.insert("rand".to_string());
                }
            }
            collect_callees_from_expr(callee, callees);
            for a in args { collect_callees_from_expr(a, callees); }
        }
        Expr::BinOp { left, right, .. } => {
            collect_callees_from_expr(left, callees);
            collect_callees_from_expr(right, callees);
        }
        Expr::UnaryOp { expr: e, .. } => collect_callees_from_expr(e, callees),
        Expr::FieldAccess { object, .. } => collect_callees_from_expr(object, callees),
        Expr::Array(items) => { for i in items { collect_callees_from_expr(i, callees); } }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_callees_from_expr(k, callees);
                collect_callees_from_expr(v, callees);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            collect_callees_from_expr(iter, callees);
            collect_callees_from_stmts(body, callees);
        }
        _ => {}
    }
}

// --- Static checks ---

/// Find unused variables within a function body.
fn find_unused_vars_in_fn(
    name: &str, params: &[String], body: &[Stmt], extra_reads: &BTreeSet<String>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut assigned: Vec<(String, usize)> = Vec::new();
    let mut read: BTreeSet<String> = extra_reads.clone();
    collect_reads_from_stmts(body, &mut read);
    for p in params {
        if !read.contains(p) {
            issues.push(Issue::UnusedVariable {
                func: name.to_string(), var: p.clone(), line: 0,
            });
        }
    }
    collect_assigns_from_stmts(body, &mut assigned);
    for (var, line) in &assigned {
        if !read.contains(var) && var != "_" {
            issues.push(Issue::UnusedVariable {
                func: name.to_string(), var: var.clone(), line: *line,
            });
        }
    }
    issues
}

fn collect_reads_from_stmts(stmts: &[Stmt], reads: &mut BTreeSet<String>) {
    for s in stmts { collect_reads_from_stmt(s, reads); }
}

fn collect_reads_from_stmt(stmt: &Stmt, reads: &mut BTreeSet<String>) {
    match stmt {
        Stmt::Assign { value, .. } => collect_reads_from_expr(value, reads),
        Stmt::MultiAssign { values, .. } => {
            for v in values { collect_reads_from_expr(v, reads); }
        }
        Stmt::FnDecl { body, .. } => collect_reads_from_stmts(body, reads),
        Stmt::Return(Some(e)) => collect_reads_from_expr(e, reads),
        Stmt::ExprStmt(e) => collect_reads_from_expr(e, reads),
        Stmt::If { condition, then_body, else_body } => {
            collect_reads_from_expr(condition, reads);
            collect_reads_from_stmts(then_body, reads);
            if let Some(eb) = else_body { collect_reads_from_stmts(eb, reads); }
        }
        Stmt::For { iter, body, .. } => {
            collect_reads_from_expr(iter, reads);
            collect_reads_from_stmts(body, reads);
        }
        Stmt::While { condition, body } => {
            collect_reads_from_expr(condition, reads);
            collect_reads_from_stmts(body, reads);
        }
        Stmt::FieldAssign { object, value, .. } => {
            collect_reads_from_expr(object, reads);
            collect_reads_from_expr(value, reads);
        }
        Stmt::IndexAssign { object, index, value } => {
            collect_reads_from_expr(object, reads);
            collect_reads_from_expr(index, reads);
            collect_reads_from_expr(value, reads);
        }
        _ => {}
    }
}

fn collect_reads_from_expr(expr: &Expr, reads: &mut BTreeSet<String>) {
    match expr {
        Expr::Ident(name) => { reads.insert(name.clone()); }
        Expr::Str(s) => {
            // Extract reads from string interpolation: '{x.ok}' reads x
            for name in crate::parallel::str_interpolation_reads(s) {
                reads.insert(name);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_reads_from_expr(left, reads);
            collect_reads_from_expr(right, reads);
        }
        Expr::UnaryOp { expr: e, .. } => collect_reads_from_expr(e, reads),
        Expr::Call { callee, args } => {
            collect_reads_from_expr(callee, reads);
            for a in args { collect_reads_from_expr(a, reads); }
        }
        Expr::FieldAccess { object, .. } => collect_reads_from_expr(object, reads),
        Expr::Array(items) => { for i in items { collect_reads_from_expr(i, reads); } }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_reads_from_expr(k, reads);
                collect_reads_from_expr(v, reads);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            collect_reads_from_expr(iter, reads);
            collect_reads_from_stmts(body, reads);
        }
        // a named argument reads its value: disabled = starting, open = ending
        Expr::Named { value, .. } => collect_reads_from_expr(value, reads),
        _ => {}
    }
}

fn collect_assigns_from_stmts(stmts: &[Stmt], assigns: &mut Vec<(String, usize)>) {
    let mut current_line = 0;
    for s in stmts {
        if let Stmt::Line(n) = s { current_line = *n; }
        if let Stmt::Assign { name, .. } = s { assigns.push((name.clone(), current_line)); }
        if let Stmt::MultiAssign { names, .. } = s {
            for n in names { assigns.push((n.clone(), current_line)); }
        }
    }
}

/// Find unreachable code after return statements.
fn find_unreachable_code(name: &str, body: &[Stmt]) -> Vec<Issue> {
    let mut issues = Vec::new();
    check_unreachable(name, body, &mut issues);
    issues
}

fn check_unreachable(name: &str, stmts: &[Stmt], issues: &mut Vec<Issue>) {
    let mut current_line = 0;
    let mut seen_return = false;
    for stmt in stmts {
        if let Stmt::Line(n) = stmt { current_line = *n; continue; }
        if seen_return {
            if matches!(stmt, Stmt::Line(_)) { continue; }
            issues.push(Issue::UnreachableCode {
                func: name.to_string(), line: current_line,
                detail: "code after return can never execute".to_string(),
            });
            return;
        }
        if matches!(stmt, Stmt::Return(_)) { seen_return = true; }
        match stmt {
            Stmt::If { then_body, else_body, .. } => {
                check_unreachable(name, then_body, issues);
                if let Some(eb) = else_body { check_unreachable(name, eb, issues); }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                check_unreachable(name, body, issues);
            }
            _ => {}
        }
    }
}

/// Find undefined variable references in a function body.
/// A variable is undefined if it is read but not a parameter, not assigned before use,
/// not a builtin, and not a user-defined function or struct name.
/// A program's declarations — the boundary tests and probes see: its
/// functions, structs, imports and derivations, and the top-level values
/// that are declarations too (a literal default, a connection, a mailer);
/// never top-level code, which could do something.
pub(crate) fn declarations(stmts: &[Stmt]) -> Vec<Stmt> {
    // Declarations only — the boundary: tests see what callers see. Literal
    // top-level assigns ride along: they are reactive-state defaults, which
    // app[]-harness scenarios read and handlers write.
    fn is_literal(e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) => true,
            // a negative number is a literal too (`low = -1.0`): a fact
            // with one had been left out of what the harness sets, and a
            // derivation reading it found no such name
            Expr::UnaryOp { op: UnaryOp::Neg, expr } => matches!(expr.as_ref(), Expr::Int(_) | Expr::Float(_)),
            Expr::Array(items) => items.iter().all(is_literal),
            // a default built from literals by a constructor — map[...] (a
            // set[] chain after lowering), frame.new[], with[...], and a
            // moment of literals (`moment[0.0]`, `moment['2026-01-01']`,
            // `moment[2026, 7, 14, tz = …]`) — is a literal too
            Expr::Call { callee, args } => {
                let name = match callee.as_ref() {
                    Expr::Ident(n) => n.as_str(),
                    Expr::FieldAccess { object, field } if matches!(object.as_ref(), Expr::Ident(m) if m == "frame") => field.as_str(),
                    _ => return false,
                };
                matches!(name, "map" | "set" | "new" | "with" | "moment") && args.iter().all(is_literal)
            }
            // `map[launch = 'VWAP Launch']` as it is written, before its
            // named arguments become a set[] chain
            Expr::Named { value, .. } => is_literal(value),
            _ => false,
        }
    }
    // A connection is a declaration of a resource, not a computation: a
    // view that reads at render (--- crow: remote-render) needs it to exist
    // for app[] to render at all, exactly as it does for the served page.
    // A mailer is one too: making it sends nothing.
    fn is_connect(e: &Expr) -> bool {
        match e {
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(n) => n == "connect" || n == "smtp",
                Expr::FieldAccess { object, field } => field == "connect" && matches!(object.as_ref(), Expr::Ident(m) if m == "db"),
                _ => false,
            },
            _ => false,
        }
    }
    stmts
        .iter()
        .filter(|s| match s {
            Stmt::FnDecl { .. } | Stmt::StructDecl { .. } | Stmt::Use { .. } => true,
            // a derivation is a declaration: what a value is, from what
            Stmt::Derive { .. } => true,
            Stmt::Assign { value, .. } => is_literal(value) || is_connect(value),
            _ => false,
        })
        .cloned()
        .collect()
}

/// Full stdlib export vocabulary, built once — keeps the undefined-variable
/// check in agreement with what the registry actually provides (db, frame,
/// web, …) instead of the frozen BUILTINS list.
fn stdlib_names() -> &'static BTreeSet<String> {
    static NAMES: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        crate::modules::ModuleRegistry::with_stdlib()
            .all_function_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    })
}

fn find_undefined_vars_in_fn(
    name: &str, params: &[String], body: &[Stmt], ctx: &TypeContext, top_names: &BTreeSet<String>,
) -> Vec<Issue> {
    let mut defined: BTreeSet<String> = BTreeSet::new();
    // Parameters are defined
    for p in params { defined.insert(p.clone()); }
    // Builtins are defined — the static list plus everything the module
    // registry provides (db/web/frame/... exports) and, in views/handlers,
    // the HTML element vocabulary.
    for b in crate::lower_ir::BUILTINS.iter() { defined.insert(b.to_string()); }
    for b in stdlib_names() { defined.insert(b.clone()); }
    if reactive_readers(ctx).contains(name) {
        // the signed-in reader, where the gate lets a web program read it
        if ctx.bare_modules.iter().any(|m| m == "web" || m.starts_with("web.")) {
            defined.insert("reader".to_string());
        }
        for el in crate::lower::UI_ELEMENTS {
            defined.insert(el.to_string());
        }
        // Live elements the Rust backend handles outside the core list.
        for live in ["field", "slider", "checkbox", "toggle", "radio", "tabs", "select", "progress", "icon"] {
            defined.insert(live.to_string());
        }
        defined.insert("chart".to_string());
        // Reactive state is the sanctioned channel: views and handlers read
        // top-level variables (the function-scope isolation exemption).
        for v in ctx.vars.keys() {
            defined.insert(v.clone());
        }
        // (a top-level assignment is one whatever inference made of its
        // value: a map literal of several entries had no entry in `vars`,
        // and a handler reading it was told it used an undefined name)
        for v in top_names {
            defined.insert(v.clone());
        }
    }
    // Imported module prefixes and aliases are readable names (db.connect)
    for m in &ctx.bare_modules { defined.insert(m.clone()); }
    for a in ctx.module_aliases.keys() { defined.insert(a.clone()); }
    // User-defined functions and structs are defined (global scope)
    for fname in ctx.functions.keys() { defined.insert(fname.clone()); }
    for sname in ctx.structs.keys() { defined.insert(sname.clone()); }
    // Boolean literals that parse as identifiers
    defined.insert("true".to_string());
    defined.insert("false".to_string());

    let mut issues = Vec::new();
    collect_undefined_in_stmts(name, body, &defined, &mut issues);
    issues
}

/// Walk statements, tracking local assignments and flagging undefined reads.
fn collect_undefined_in_stmts(
    func: &str, stmts: &[Stmt], outer_defined: &BTreeSet<String>, issues: &mut Vec<Issue>,
) {
    let mut defined = outer_defined.clone();
    let mut current_line = 0usize;
    for stmt in stmts {
        if let Stmt::Line(n) = stmt { current_line = *n; continue; }
        match stmt {
            Stmt::Assign { name, value, .. } => {
                // Check value for undefined reads before adding name to scope
                collect_undefined_in_expr(func, value, &defined, current_line, issues);
                defined.insert(name.clone());
            }
            Stmt::MultiAssign { names, values, .. } => {
                for v in values {
                    collect_undefined_in_expr(func, v, &defined, current_line, issues);
                }
                for n in names { defined.insert(n.clone()); }
            }
            Stmt::ExprStmt(e) => {
                collect_undefined_in_expr(func, e, &defined, current_line, issues);
            }
            Stmt::Return(Some(e)) => {
                collect_undefined_in_expr(func, e, &defined, current_line, issues);
            }
            Stmt::If { condition, then_body, else_body } => {
                collect_undefined_in_expr(func, condition, &defined, current_line, issues);
                collect_undefined_in_stmts(func, then_body, &defined, issues);
                if let Some(eb) = else_body {
                    collect_undefined_in_stmts(func, eb, &defined, issues);
                }
            }
            Stmt::For { var, iter, body, .. } => {
                collect_undefined_in_expr(func, iter, &defined, current_line, issues);
                let mut inner = defined.clone();
                inner.insert(var.clone());
                collect_undefined_in_stmts(func, body, &inner, issues);
            }
            Stmt::While { condition, body } => {
                collect_undefined_in_expr(func, condition, &defined, current_line, issues);
                collect_undefined_in_stmts(func, body, &defined, issues);
            }
            Stmt::IndexAssign { object, index, value } => {
                collect_undefined_in_expr(func, object, &defined, current_line, issues);
                collect_undefined_in_expr(func, index, &defined, current_line, issues);
                collect_undefined_in_expr(func, value, &defined, current_line, issues);
            }
            Stmt::FnDecl { .. } | Stmt::StructDecl { .. } => {
                // Already tracked globally via ctx
            }
            _ => {}
        }
    }
}

/// Check a single expression for undefined variable reads.
fn collect_undefined_in_expr(
    func: &str, expr: &Expr, defined: &BTreeSet<String>, line: usize, issues: &mut Vec<Issue>,
) {
    match expr {
        Expr::Ident(name) => {
            if !defined.contains(name) {
                issues.push(Issue::UndefinedVariable {
                    func: func.to_string(), var: name.clone(), line,
                });
            }
        }
        Expr::Str(s) => {
            for name in crate::parallel::str_interpolation_reads(s) {
                if !defined.contains(&name) {
                    issues.push(Issue::UndefinedVariable {
                        func: func.to_string(), var: name, line,
                    });
                }
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_undefined_in_expr(func, left, defined, line, issues);
            collect_undefined_in_expr(func, right, defined, line, issues);
        }
        Expr::UnaryOp { expr: e, .. } => {
            collect_undefined_in_expr(func, e, defined, line, issues);
        }
        Expr::Call { callee, args } => {
            collect_undefined_in_expr(func, callee, defined, line, issues);
            for a in args { collect_undefined_in_expr(func, a, defined, line, issues); }
        }
        Expr::FieldAccess { object, .. } => {
            collect_undefined_in_expr(func, object, defined, line, issues);
        }
        Expr::Array(items) => {
            for item in items { collect_undefined_in_expr(func, item, defined, line, issues); }
        }
        Expr::Map(entries) => {
            for (k, v) in entries {
                collect_undefined_in_expr(func, k, defined, line, issues);
                collect_undefined_in_expr(func, v, defined, line, issues);
            }
        }
        Expr::ForExpr { var, iter, body } => {
            collect_undefined_in_expr(func, iter, defined, line, issues);
            let mut inner = defined.clone();
            inner.insert(var.clone());
            collect_undefined_in_stmts(func, body, &inner, issues);
        }
        _ => {}
    }
}

/// Run all static checks on functions and top-level code.
pub fn run_static_checks(ctx: &TypeContext, top_level_stmts: &[Stmt]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let reactive = reactive_readers(ctx);
    // a route calls its handler with the request whether or not the
    // handler reads it, so an unread request is no finding (removing it,
    // as the finding had advised, makes a handler no route can call)
    let route_handlers: BTreeSet<String> = top_level_stmts
        .iter()
        .filter_map(|s| match s {
            // (the handler is the last name after the path: a method's text
            // may come before it, and a limit after)
            Stmt::ExprStmt(Expr::Call { callee, args }) if matches!(callee.as_ref(), Expr::Ident(n) if n == "route") => {
                args.iter().skip(1).rev().find_map(|a| if let Expr::Ident(h) = a { Some(h.clone()) } else { None })
            }
            _ => None,
        })
        .collect();
    // the names the top level assigns: reactive state a view, a handler
    // or a derivation reads
    let top_names: BTreeSet<String> = top_level_stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Assign { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    // Check user-defined functions
    for (name, sig) in &ctx.functions {
        let param_names: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
        let checked_params: &[String] = if route_handlers.contains(name) { &[] } else { &param_names };
        // Views, handlers, derivations and their helpers write reactive
        // state; those assignments are the point of the fn, never "unused
        // locals".
        let state_writes: BTreeSet<String> =
            if reactive.contains(name) {
                top_level_stmts
                    .iter()
                    .filter_map(|s| match s {
                        Stmt::Assign { name, .. } => Some(name.clone()),
                        _ => None,
                    })
                    .collect()
            } else {
                BTreeSet::new()
            };
        issues.extend(find_unused_vars_in_fn(name, checked_params, &sig.body, &state_writes));
        issues.extend(find_unreachable_code(name, &sig.body));
        issues.extend(find_undefined_vars_in_fn(name, &param_names, &sig.body, ctx, &top_names));
    }
    // Check top-level code
    let top_stmts: Vec<&Stmt> = top_level_stmts.iter()
        .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. }))
        .collect();
    if !top_stmts.is_empty() {
        let top_owned: Vec<Stmt> = top_stmts.into_iter().cloned().collect();
        // Views, handlers and derivations read top-level reactive state —
        // their reads keep those variables "used".
        let mut view_reads: BTreeSet<String> = BTreeSet::new();
        for name in reactive.iter() {
            if let Some(sig) = ctx.functions.get(name) {
                collect_reads_from_stmts(&sig.body, &mut view_reads);
            }
        }
        issues.extend(find_unused_vars_in_fn("<top-level>", &[], &top_owned, &view_reads));
        issues.extend(find_unreachable_code("<top-level>", &top_owned));
        issues.extend(find_undefined_vars_in_fn("<top-level>", &[], &top_owned, ctx, &top_names));
    }
    issues
}

/// Build a map of function name → declaration line number by scanning for
/// Line markers preceding FnDecl statements.
pub fn find_fn_lines(stmts: &[Stmt]) -> BTreeMap<String, usize> {
    let mut map = BTreeMap::new();
    let mut last_line = 0usize;
    for stmt in stmts {
        match stmt {
            Stmt::Line(n) => { last_line = *n; }
            Stmt::FnDecl { name, .. } => {
                map.insert(name.clone(), last_line);
            }
            _ => {}
        }
    }
    map
}

// --- Edge case generation ---

/// Generate boundary values for a Ty.
fn boundary_values(ty: &Ty) -> Option<Vec<CrowValue>> {
    match ty {
        Ty::Int => Some(vec![
            CrowValue::ok(Value::Int(0)),
            CrowValue::ok(Value::Int(1)),
            CrowValue::ok(Value::Int(-1)),
            CrowValue::ok(Value::Int(i64::MAX)),
            CrowValue::ok(Value::Int(i64::MIN)),
        ]),
        Ty::Float => Some(vec![
            CrowValue::ok(Value::Float(0.0)),
            CrowValue::ok(Value::Float(1.0)),
            CrowValue::ok(Value::Float(-1.0)),
            CrowValue::ok(Value::Float(0.5)),
            CrowValue::ok(Value::Float(-0.5)),
        ]),
        Ty::Str => Some(vec![
            CrowValue::ok(Value::Str(String::new())),
            CrowValue::ok(Value::Str("a".into())),
            CrowValue::ok(Value::Str("hello".into())),
        ]),
        Ty::Bool => Some(vec![
            CrowValue::ok(Value::Bool(true)),
            CrowValue::ok(Value::Bool(false)),
        ]),
        Ty::List(inner) => {
            match inner.as_ref() {
                Ty::Int => Some(vec![
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![CrowValue::ok(Value::Int(0))]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![
                        CrowValue::ok(Value::Int(1)),
                        CrowValue::ok(Value::Int(2)),
                        CrowValue::ok(Value::Int(3)),
                    ]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![CrowValue::ok(Value::Int(1))]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![
                        CrowValue::ok(Value::Int(-1)),
                        CrowValue::ok(Value::Int(0)),
                        CrowValue::ok(Value::Int(1)),
                    ]))),
                ]),
                Ty::Str => Some(vec![
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![CrowValue::ok(Value::Str(String::new()))]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![
                        CrowValue::ok(Value::Str("a".into())),
                        CrowValue::ok(Value::Str("b".into())),
                    ]))),
                ]),
                _ => None, // unsupported inner type
            }
        }
        Ty::Tensor(inner, _) => {
            // Same boundary values as Array but for numeric arrays
            match inner.as_ref() {
                Ty::Int | Ty::Float => Some(vec![
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![CrowValue::ok(Value::Int(0))]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![
                        CrowValue::ok(Value::Int(1)),
                        CrowValue::ok(Value::Int(2)),
                        CrowValue::ok(Value::Int(3)),
                    ]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![CrowValue::ok(Value::Int(1))]))),
                    CrowValue::ok(Value::Array(std::sync::Arc::new(vec![
                        CrowValue::ok(Value::Int(-1)),
                        CrowValue::ok(Value::Int(0)),
                        CrowValue::ok(Value::Int(1)),
                    ]))),
                ]),
                _ => None,
            }
        }
        _ => None, // map, struct, etc. — skip
    }
}

/// Check if all params of a function have supported types for edge case generation.
fn can_generate_edge_cases(sig: &FnSig) -> bool {
    sig.params.iter().all(|(_, ty)| {
        boundary_values(ty).is_some()
    })
}

/// Generate all combinations of boundary inputs for a function (capped at 50).
fn generate_input_combos(sig: &FnSig) -> Vec<Vec<CrowValue>> {
    // All params or none: silently dropping an ungeneratable param would
    // call the fn with the wrong arity and report phantom crashes.
    let mut param_values: Vec<Vec<CrowValue>> = Vec::new();
    for (_, ty) in &sig.params {
        match boundary_values(ty) {
            Some(vals) => param_values.push(vals),
            None => return vec![],
        }
    }

    if param_values.is_empty() { return vec![vec![]]; }

    let mut combos: Vec<Vec<CrowValue>> = vec![vec![]];
    for vals in &param_values {
        let mut new_combos = Vec::new();
        for combo in &combos {
            for v in vals {
                let mut c = combo.clone();
                c.push(v.clone());
                new_combos.push(c);
            }
        }
        combos = new_combos;
        // Cap the count, never the arity: breaking out mid-product would
        // emit short tuples and report phantom wrong-arity crashes.
        if combos.len() > 50 { combos.truncate(50); }
    }
    combos
}

/// Format a function call with arguments for display.
fn format_call(name: &str, args: &[CrowValue]) -> String {
    let arg_strs: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
    format!("{}[{}]", name, arg_strs.join(", "))
}

/// Run edge case generation on pure functions.
/// Returns (issues, snapshots) where snapshots are input→output pairs.
pub fn run_edge_cases(
    ctx: &TypeContext,
    stmts: &[Stmt],
    pure_fns: &BTreeSet<String>,
    recursive_fns: &BTreeSet<String>,
) -> (Vec<Issue>, BTreeMap<String, Vec<(String, String)>>) {
    let mut crash_seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut issues = Vec::new();
    let mut snapshots: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    // The declarations as tests see them — skip top-level code to avoid
    // side effects. Without the imports, derivations and facts a function
    // or a view reads, the program did not lower and every probe was
    // skipped (a ui app's views read its facts and derivations).
    let decls_only: Vec<Stmt> = declarations(stmts);

    // Suppress panic output during edge case testing
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    // the declarations lowered once, shared by every probe's thread
    let prepared: Option<(Arc<crate::ir::Program>, crate::modules::ModuleRegistry)> =
        crate::ir_interp::prepare(&decls_only, "check").ok().map(|(p, c)| (Arc::new(p), c.registry));

    for name in pure_fns {
        // Skip recursive functions — edge cases with boundary values cause stack overflow
        if recursive_fns.contains(name) { continue; }
        let sig = match ctx.functions.get(name) {
            Some(s) => s,
            None => continue,
        };
        if !can_generate_edge_cases(sig) { continue; }
        if sig.params.iter().any(|(_, ty)| matches!(ty, Ty::Unknown)) { continue; }

        let combos = generate_input_combos(sig);
        let mut fn_snapshots = Vec::new();

        for combo in &combos {
            let call_desc = format_call(name, combo);
            let fn_name = name.clone();
            let prepared_clone = prepared.clone();
            let combo_clone = combo.clone();

            // Run in a thread with timeout to handle infinite loops
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let cancel_clone = cancel_flag.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let (program, registry) = prepared_clone.ok_or_else(|| "the declarations do not lower".to_string())?;
                    let mut it = crate::ir_interp::IrInterp::new(&registry, "check");
                    it.set_cancel_flag(cancel_clone);
                    it.load(&program);
                    let _ = it.run_main(&program);
                    it.call(&fn_name, combo_clone)
                }));
                let _ = tx.send(result);
            });

            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(Ok(val))) => {
                    let output = format!("{}", val);
                    fn_snapshots.push((call_desc, output));
                }
                Ok(Ok(Err(e))) => {
                    // One report per distinct failure — fifty combos hitting
                    // the same error is one finding, not fifty.
                    if crash_seen.insert((name.clone(), e.clone())) {
                        issues.push(Issue::EdgeCaseCrash {
                            func: name.clone(),
                            input_desc: call_desc,
                            error: e,
                        });
                    }
                }
                Ok(Err(panic_info)) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "panic during execution".to_string()
                    };
                    issues.push(Issue::EdgeCaseCrash {
                        func: name.clone(),
                        input_desc: call_desc,
                        error: msg,
                    });
                }
                Err(_) => {
                    // Timeout — signal the thread to stop, then wait briefly for cleanup
                    cancel_flag.store(true, Ordering::Relaxed);
                    issues.push(Issue::EdgeCaseCrash {
                        func: name.clone(),
                        input_desc: call_desc,
                        error: "timed out (possible infinite loop)".to_string(),
                    });
                    // Give the thread a moment to notice the cancel flag and exit
                    let _ = handle.join().ok();
                }
            }
        }

        if !fn_snapshots.is_empty() {
            snapshots.insert(name.clone(), fn_snapshots);
        }
    }

    // Restore the previous panic hook
    std::panic::set_hook(prev_hook);

    (issues, snapshots)
}

// --- Regression snapshots ---

const SNAPSHOT_DIR: &str = ".crow/snapshots";

/// Load existing snapshots from disk.
fn load_snapshots() -> BTreeMap<String, Vec<(String, String)>> {
    let mut all = BTreeMap::new();
    let dir = std::path::Path::new(SNAPSHOT_DIR);
    if !dir.exists() { return all; }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "snap").unwrap_or(false) {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut pairs = Vec::new();
                    for line in content.lines() {
                        if let Some((input, output)) = line.split_once(" = ") {
                            pairs.push((input.to_string(), output.to_string()));
                        }
                    }
                    all.insert(name, pairs);
                }
            }
        }
    }
    all
}

/// Save snapshots to disk.
fn save_snapshots(snapshots: &BTreeMap<String, Vec<(String, String)>>) {
    // the store is the project's, version-controlled; a unit test running
    // the check pipeline from the crate root must not write into it
    if cfg!(test) {
        return;
    }
    let dir = std::path::Path::new(SNAPSHOT_DIR);
    std::fs::create_dir_all(dir).ok();
    for (name, pairs) in snapshots {
        let path = dir.join(format!("{}.snap", name));
        let content: String = pairs.iter()
            .map(|(input, output)| format!("{} = {}", input, output))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, content).ok();
    }
}

/// Compare new snapshots with existing ones, returning regression issues.
fn compare_snapshots(
    old: &BTreeMap<String, Vec<(String, String)>>,
    new: &BTreeMap<String, Vec<(String, String)>>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (name, old_pairs) in old {
        if let Some(new_pairs) = new.get(name) {
            let old_map: BTreeMap<&str, &str> = old_pairs.iter()
                .map(|(i, o)| (i.as_str(), o.as_str())).collect();
            let new_map: BTreeMap<&str, &str> = new_pairs.iter()
                .map(|(i, o)| (i.as_str(), o.as_str())).collect();
            for (input, old_output) in &old_map {
                if let Some(new_output) = new_map.get(input) {
                    if old_output != new_output {
                        issues.push(Issue::RegressionChange {
                            func: name.clone(),
                            input_desc: input.to_string(),
                            was: old_output.to_string(),
                            now: new_output.to_string(),
                        });
                    }
                }
            }
        }
    }
    issues
}

// --- Main check runner ---

/// Result of running `crow check`.
pub struct CheckResult {
    pub test_counts: TestCounts,
    pub pure_fns: BTreeSet<String>,
    pub impure_fns: BTreeSet<String>,
    pub issues: Vec<Issue>,
    pub total_functions: usize,
    pub edge_cases_generated: usize,
    pub snapshots_saved: usize,
    pub snapshots_unchanged: usize,
    /// Map of function name → declaration line number.
    pub fn_lines: BTreeMap<String, usize>,
    /// One line per derivation: what it reads and who holds it.
    pub derivations: Vec<String>,
    /// The app baseline: recorded on the first check, diffed after.
    pub baseline: Option<crate::baseline::Summary>,
    /// The derived artifacts, for `--show`.
    pub artifacts: Option<crate::baseline::Artifacts>,
}

/// One line per derivation — its read set, its tables and freshness, and
/// whether the app or each session holds it. The same analysis both
/// execution modes obey, shown so a reader can check it against the code.
pub fn describe_derivations(stmts: &[Stmt]) -> Vec<String> {
    let derive_names: BTreeSet<String> = stmts.iter()
        .filter_map(|s| if let Stmt::Derive { name, .. } = s { Some(name.clone()) } else { None })
        .collect();
    if derive_names.is_empty() {
        return Vec::new();
    }
    let bodies: BTreeMap<String, Vec<Stmt>> = stmts.iter()
        .filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None })
        .collect();
    let params: BTreeMap<String, Vec<String>> = stmts.iter()
        .filter_map(|s| if let Stmt::FnDecl { name, params, .. } = s { Some((name.clone(), params.clone())) } else { None })
        .collect();
    let fresh_of: BTreeMap<String, f64> = stmts.iter()
        .filter_map(|s| if let Stmt::Derive { name, fresh, .. } = s { Some((name.clone(), *fresh)) } else { None })
        .collect();
    let sets = crate::derive::read_sets(stmts, &derive_names, &bodies, &params);
    sets.iter().map(|(name, rs)| {
        let mut parts: Vec<String> = Vec::new();
        let mut reads: Vec<String> = rs.facts.iter().chain(rs.derives.iter()).cloned().collect();
        if rs.now { reads.push("now[]".to_string()); }
        if !reads.is_empty() { parts.push(format!("reads {}", reads.join(", "))); }
        if !rs.tables.is_empty() {
            // a table it reads a slice of says which: `cards where board = board_id`
            let tables = rs
                .tables
                .iter()
                .map(|t| match rs.keys.get(t) {
                    Some((column, fact)) => format!("{} where {} = {}", t, column, fact),
                    None => t.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let fresh = fresh_of.get(name).copied().unwrap_or(0.0);
            let how = if !rs.external { String::new() }
                else if fresh > 0.0 { format!(" (external, fresh {}s)", fresh) }
                else { " (external, every tick)".to_string() };
            parts.push(format!("tables {}{}", tables, how));
        }
        if parts.is_empty() { parts.push("reads nothing".to_string()); }
        let holder = match &rs.per_session {
            None => "held by the app".to_string(),
            Some(why) => format!("held by each session ({})", why),
        };
        format!("{}: {} — {}", name, parts.join(" · "), holder)
    }).collect()
}

/// Run the full check pipeline on parsed + type-inferred code.
pub fn run_check(
    stmts: &[Stmt],
    ctx: &TypeContext,
    _verbose: bool,
) -> CheckResult {
    let all_fns: BTreeSet<String> = ctx.functions.keys().cloned().collect();
    let pure_fns = find_pure_functions(ctx);
    let impure_fns: BTreeSet<String> = all_fns.difference(&pure_fns).cloned().collect();
    let total_functions = all_fns.len();

    let mut issues = Vec::new();
    let fn_lines = find_fn_lines(stmts);

    // 1. Static checks (functions + top-level code)
    issues.extend(run_static_checks(ctx, stmts));

    // 1a2. Authored test blocks: truths, laws, scenarios.
    let (test_issues, test_counts) = run_test_blocks(stmts);
    issues.extend(test_issues);

    // 1a3. Layer-0 contract: determinism of pure fns — as the edge cases
    // below, never a view, a handler or a derivation (rendering a view reads
    // derivations, which ask the databases, twice per probe)
    let recursive_for_probe = find_recursive_functions(ctx);
    let reactive_for_probe = reactive_readers(ctx);
    let probed: BTreeSet<String> = pure_fns.iter().filter(|f| !reactive_for_probe.contains(*f)).cloned().collect();
    issues.extend(run_determinism_probe(ctx, stmts, &probed, &recursive_for_probe));

    // 1b. Views/handlers reaching blocking builtins freeze the live loop.
    issues.extend(find_blocking_in_views(ctx, &fn_lines));

    // 1c. Views/handlers serving launch-time query snapshots.
    issues.extend(find_stale_snapshots(ctx, stmts, &fn_lines));

    // 1d. Render-path queries against remote databases.
    issues.extend(find_remote_queries_in_views(ctx, stmts, &fn_lines));

    // 1e. Poll loops deciding on pre-loop snapshots.
    issues.extend(find_stale_loop_reads(stmts));

    // 1e2. Derivations: what each reads and who holds it, for the report.
    let derivations = describe_derivations(stmts);

    // 1f. Outside-world data deciding a branch with no freshness guard.
    issues.extend(find_unguarded_staleness(stmts));

    // 1g. Style-token overrides that name no token.
    for (name, suggestion) in check_style_tokens("css/app.css") {
        issues.push(Issue::UnknownStyleToken { name, suggestion });
    }

    // 2. Edge case generation (pure, non-recursive functions only).
    // Views, handlers, derivations and the helpers they call are excluded:
    // they build UI trees / read and write reactive state, and are
    // exercised by rendering, not by synthetic inputs.
    let recursive_fns = find_recursive_functions(ctx);
    let reactive = reactive_readers(ctx);
    let edge_candidates: BTreeSet<String> = pure_fns
        .iter()
        .filter(|f| !reactive.contains(*f))
        .cloned()
        .collect();
    let (edge_issues, new_snapshots) = run_edge_cases(ctx, stmts, &edge_candidates, &recursive_fns);
    let edge_cases_generated = new_snapshots.values().map(|v| v.len()).sum::<usize>();
    issues.extend(edge_issues);

    // 3. Regression snapshots
    let old_snapshots = load_snapshots();
    let regression_issues = compare_snapshots(&old_snapshots, &new_snapshots);
    let snapshots_unchanged = new_snapshots.len().saturating_sub(
        regression_issues.iter()
            .map(|i| match i { Issue::RegressionChange { func, .. } => func.clone(), _ => String::new() })
            .collect::<BTreeSet<_>>()
            .len()
    );
    issues.extend(regression_issues);

    // Save new snapshots
    let snapshots_saved = new_snapshots.len();
    save_snapshots(&new_snapshots);

    CheckResult {
        test_counts,
        pure_fns,
        impure_fns,
        issues,
        total_functions,
        edge_cases_generated,
        snapshots_saved,
        snapshots_unchanged,
        fn_lines,
        derivations,
        baseline: None,
        artifacts: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_infer::TypeContext;

    /// A map written as a literal is a declaration's first value, as a list
    /// of literals is: the probes see it (finch's `setup_names`, a map of
    /// names a handler reads, was left out and no probe of the program
    /// lowered — "the declarations do not lower")
    #[test]
    fn a_map_literal_is_a_declaration() {
        let src = "use web, ui\nnames = map[launch = 'VWAP Launch', burst = 'Burst']\nsaid = ''\nfn tell[code]\n    said = names[code]\nview render[]\n    return stack[p['`said`'], button['go', click = tell['launch']]]\npage['/', render]\n";
        let stmts = crate::build(src).unwrap();
        let decls = declarations(&stmts);
        assert!(decls.iter().any(|s| matches!(s, Stmt::Assign { name, .. } if name == "names")), "{:?}", decls);
        assert!(crate::ir_interp::prepare(&decls, "check").is_ok());
    }

    /// A handler reading a map literal the top level declares reads a
    /// defined name: `names = map[launch = 'VWAP Launch', …]` had no entry
    /// in the type context's vars, and finch's next_setup was told it used
    /// an undefined 'setup_names'
    #[test]
    fn a_handler_reads_a_top_level_map_literal() {
        let src = "use web, ui\n\ncount = 0\nnames = map[launch = 'VWAP Launch', burst = 'Burst']\nstatus = ''\n\nfn bump[]\n    count = count + 1\n    status = names['launch']\n\nview render[]\n    return stack[p['`count` `status`'], button['go', bump]]\n\npage['/', render]\nserve[3000]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let issues = run_static_checks(&ctx, &stmts);
        assert!(!issues.iter().any(|i| matches!(i, Issue::UndefinedVariable { .. })), "{:?}", issues);
    }

    /// A fact declared with a negative number is a declaration: the
    /// harness had left `low = -1.0` out of what it set, and a derivation
    /// reading it was told the name was undefined
    #[test]
    fn a_negative_literal_is_a_declaration() {
        let src = "use web, ui\nlow = -1.0\nhigh = 2.0\nderive band\n    return [low, high]\nview render[]\n    return p['`band`']\npage['/', render]\n";
        let stmts = crate::build(src).unwrap();
        let decls = declarations(&stmts);
        assert!(decls.iter().any(|s| matches!(s, Stmt::Assign { name, .. } if name == "low")), "{:?}", decls);
        assert!(crate::ir_interp::prepare(&decls, "check").is_ok());
    }

    #[test]
    fn later_is_the_sanctioned_blocking_boundary() {
        // A handler that reaches sleep[] directly is flagged; the same slow
        // work scheduled via later[] is the prescribed fix and must be clean.
        let src = "use web, ui\n\ncount = 0\n\nfn slow_work[]\n    sleep[2]\n    count = count + 1\n\nfn go[]\n    later[slow_work]\n\nview render[]\n    return div[p['`count`'], button['go', click = go]]\n\nserve[3000]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let issues = find_blocking_in_views(&ctx, &find_fn_lines(&stmts));
        assert!(
            issues.is_empty(),
            "later[]-scheduled work must not be flagged as blocking, got {:?}",
            issues
        );
    }

    #[test]
    fn remote_query_in_view_flagged_sqlite_not() {
        let src = "use web, ui, db\n\npg = connect['postgres://prod/trading']\nlocal = connect['sqlite://cache.db']\n\nview render[]\n    live = query[pg, 'SELECT * FROM trades']\n    cached = query[local, 'SELECT * FROM summary']\n    return div[table[live], table[cached]]\n\nserve[3000]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let issues =
            find_remote_queries_in_views(&ctx, &stmts, &find_fn_lines(&stmts));
        assert!(
            issues.iter().any(|i| matches!(i,
                Issue::RemoteQueryInView { view, conn, .. }
                    if view == "render" && conn == "pg")),
            "expected remote-query finding for pg, got {:?}",
            issues
        );
        assert!(
            !issues.iter().any(|i| matches!(i,
                Issue::RemoteQueryInView { conn, .. } if conn == "local")),
            "sqlite render-time queries are blessed and must not be flagged"
        );
    }

    #[test]
    fn stale_loop_read_flagged_requery_clears() {
        let src = "use db\n\nconn = connect['sqlite://x.db']\nprices = query[conn, 'SELECT px FROM ticks']\n\nwhile true\n    log[len[prices]]\n    sleep[5]\n";
        let stmts = crate::build(src).unwrap();
        let issues = find_stale_loop_reads(&stmts);
        assert!(
            issues.iter().any(|i| matches!(i,
                Issue::StaleLoopRead { var, source_line, .. }
                    if var == "prices" && *source_line == 4)),
            "expected stale loop read, got {:?}",
            issues
        );

        let src2 = "use db\n\nconn = connect['sqlite://x.db']\nprices = query[conn, 'SELECT px FROM ticks']\n\nwhile true\n    prices = query[conn, 'SELECT px FROM ticks']\n    log[len[prices]]\n    sleep[5]\n";
        let stmts2 = crate::build(src2).unwrap();
        let issues2 = find_stale_loop_reads(&stmts2);
        assert!(
            issues2.is_empty(),
            "re-query inside the loop should clear it, got {:?}",
            issues2
        );
    }

    #[test]
    fn chart_element_renders_in_the_harness() {
        let src = std::fs::read_to_string("tests/chart_harness.co").unwrap();
        let stmts = crate::build(&src).unwrap();
        let (issues, counts) = run_test_blocks(&stmts);
        // the candle chart, and the study with panes, points and a seek target
        assert_eq!(counts.blocks, 2);
        assert!(
            issues.is_empty(),
            "chart harness assertions failed: {:?}",
            issues.iter().map(|i| i.summary()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn style_token_check_catches_typos() {
        let dir = std::env::temp_dir().join(format!("crow_css_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("app.css");
        std::fs::write(&f, ":root { --crow-accnet: #fff; --crow-up: #0f0; --crow-brand: red }").unwrap();
        let found = check_style_tokens(f.to_str().unwrap());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "--crow-accnet");
        assert_eq!(found[0].1, "--crow-accent");
        assert_eq!(found[1].0, "--crow-brand");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unguarded_staleness_flags_flow_and_guard_clears() {
        // A read flows through arithmetic into a decision: flagged.
        let src = "use db\nconn = connect['sqlite://x.db']\npx = query[conn, 'SELECT last FROM ticks']\nnotional = px.last * 100.0\nif notional gt 1000.0\n    log['big']\n";
        let stmts = crate::build(src).unwrap();
        let issues = find_unguarded_staleness(&stmts);
        assert!(
            issues.iter().any(|i| matches!(i, Issue::UnguardedStaleness { var, .. } if var == "notional")),
            "expected staleness finding, got {:?}", issues
        );

        // Same program with a freshness guard: clean.
        let src2 = "use db\nconn = connect['sqlite://x.db']\npx = query[conn, 'SELECT last FROM ticks']\nif now[] - captured[px] gt 10.0\n    log['stale']\nnotional = px.last * 100.0\nif notional gt 1000.0\n    log['big']\n";
        let stmts2 = crate::build(src2).unwrap();
        assert!(
            find_unguarded_staleness(&stmts2).is_empty(),
            "a captured[] guard should clear the finding"
        );
    }

    #[test]
    fn test_blocks_run_truths_laws_and_scenarios() {
        let src = "fn fee[notional]\n    if notional lt 0\n        return error['negative', 'validation']\n    return max[1.0, notional * 0.001]\ntest[n]\n    fee[100.0] eq 1.0\n    fee[-5.0].kind eq 'validation'\n    if n gte 0.0\n        fee[n] gte 1.0\n\ntest floor_holds[]\n    assert[fee[100.0] eq 1.0]\n";
        let stmts = crate::build(src).unwrap();
        let (issues, counts) = run_test_blocks(&stmts);
        assert!(issues.is_empty(), "expected all truths to hold, got {:?}", issues);
        assert_eq!(counts.blocks, 2);
        // (the line under the precondition is a truth too; it had not been)
        assert_eq!(counts.truths, 3, "two bare boolean lines are truths, and the one under the law's precondition a third");
        assert_eq!(counts.laws, 1, "the parameterized block is a law");
        assert!(counts.inputs_run > 1, "law must run over generated inputs");
    }

    #[test]
    fn app_harness_click_state_html() {
        let src = "use web, ui\n\nstatus = 'ready'\n\nfn go[]\n    status = 'loading'\n\nview render[]\n    return div[h1['Book'], p['s: `status`']]\n\ntest refresh_flow[]\n    a = app[render]\n    assert['ready' in html[a]]\n    click[a, go]\n    assert[a.status eq 'loading']\n    assert['loading' in html[a]]\n";
        let stmts = crate::build(src).unwrap();
        let (issues, counts) = run_test_blocks(&stmts);
        assert!(issues.is_empty(), "harness scenario should pass, got {:?}", issues);
        assert_eq!(counts.blocks, 1);
    }

    #[test]
    fn app_harness_later_runs_inline() {
        // The handler schedules slow work with later[]; in tests it runs
        // inline, so the state is updated by the next line — deterministic.
        let src = "use web, ui\n\nstatus = 'ready'\n\nfn slow_work[]\n    status = 'updated'\n\nfn go[]\n    later[slow_work]\n\nview render[]\n    return div[p['`status`']]\n\ntest refresh[]\n    a = app[render]\n    click[a, go]\n    assert[a.status eq 'updated']\n";
        let stmts = crate::build(src).unwrap();
        let (issues, _) = run_test_blocks(&stmts);
        assert!(issues.is_empty(), "later must run inline in tests, got {:?}", issues);
    }

    #[test]
    fn failing_truth_reports_operands() {
        let src = "fn fee[n]\n    return n * 2.0\ntest[]\n    fee[2.0] eq 5.0\n";
        let stmts = crate::build(src).unwrap();
        let (issues, _) = run_test_blocks(&stmts);
        assert_eq!(issues.len(), 1);
        let d = format!("{:?}", issues[0]);
        assert!(d.contains("left:") && d.contains("4") && d.contains("right:") && d.contains("5"),
            "failure must show both operands, got {}", d);
    }

    #[test]
    fn failing_law_reports_the_input() {
        let src = "fn half[x]\n    return x / 2\ntest[n]\n    half[4] eq 2\n    half[n] gt 0\n";
        let stmts = crate::build(src).unwrap();
        let (issues, _) = run_test_blocks(&stmts);
        assert_eq!(issues.len(), 1, "law fails at some generated input");
        let d = format!("{:?}", issues[0]);
        assert!(d.contains("at n ="), "must report the failing input, got {}", d);
    }

    #[test]
    fn stale_snapshot_detected_and_refresh_clears_it() {
        // trades queried once at startup, served by the view forever → flagged.
        let src = "use web, ui, db\n\nconn = connect['sqlite://x.db']\ntrades = query[conn, 'SELECT * FROM trades']\n\nview render[]\n    return div[table[trades]]\n\nserve[3000]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let fn_lines = find_fn_lines(&stmts);
        let issues = find_stale_snapshots(&ctx, &stmts, &fn_lines);
        assert!(
            issues.iter().any(|i| matches!(i,
                Issue::StaleSnapshot { view, var, source_line, .. }
                    if view == "render" && var == "trades" && *source_line == 4)),
            "expected stale snapshot on trades, got {:?}",
            issues
        );

        // Same program plus a handler that re-queries → hazard cleared.
        let src2 = "use web, ui, db\n\nconn = connect['sqlite://x.db']\ntrades = query[conn, 'SELECT * FROM trades']\n\nfn refresh[]\n    trades = query[conn, 'SELECT * FROM trades']\n\nview render[]\n    return div[table[trades], button['refresh', click = refresh]]\n\nserve[3000]\n";
        let stmts2 = crate::build(src2).unwrap();
        let mut ctx2 = TypeContext::new();
        ctx2.infer_program(&stmts2);
        let issues2 = find_stale_snapshots(&ctx2, &stmts2, &find_fn_lines(&stmts2));
        assert!(
            !issues2.iter().any(|i| matches!(i, Issue::StaleSnapshot { .. })),
            "handler refresh should clear the hazard, got {:?}",
            issues2
        );
    }

    #[test]
    fn blocking_in_view_detected_transitively() {
        let src = "use web, ui\n\ncount = 0\n\nfn slow_fetch[]\n    sleep[2]\n\nfn refresh_data[]\n    slow_fetch[]\n\nview render[]\n    return div[p['count: `count`'], button['refresh', click = refresh_data]]\n\nserve[3000]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let fn_lines = find_fn_lines(&stmts);
        let issues = find_blocking_in_views(&ctx, &fn_lines);
        // The handler reaches sleep through a helper — flagged with the path.
        assert!(
            issues.iter().any(|i| matches!(i,
                Issue::BlockingInView { view, builtin, path, .. }
                    if view == "refresh_data" && builtin == "sleep"
                       && path == &vec!["refresh_data".to_string(), "slow_fetch".to_string(), "sleep".to_string()])),
            "expected transitive blocking issue, got {:?}",
            issues
        );
        // The view itself is clean — not flagged.
        assert!(
            !issues.iter().any(|i| matches!(i,
                Issue::BlockingInView { view, .. } if view == "render")),
            "render does not block and must not be flagged"
        );
    }

    /// Helper: parse source, run type inference, return (stmts, ctx)
    fn parse_and_infer(src: &str) -> (Vec<Stmt>, TypeContext) {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        (stmts, ctx)
    }

    // === Purity detection ===

    #[test]
    fn test_pure_function() {
        let (_, ctx) = parse_and_infer("fn add[a, b]\n    a + b\nx = add[1, 2]\n");
        let pure = find_pure_functions(&ctx);
        assert!(pure.contains("add"), "add should be pure");
    }

    #[test]
    fn test_impure_function_log() {
        let (_, ctx) = parse_and_infer("fn greet[name]\n    log['hi {name}']\n");
        let pure = find_pure_functions(&ctx);
        assert!(!pure.contains("greet"), "greet calls log, should be impure");
    }

    /// A password hash is salted afresh each call; a digest is not
    #[test]
    fn a_password_hash_is_not_pure() {
        let src = "fn digest[pw]\n    return hash[pw, 'password']\nfn sum[pw]\n    return hash[pw, 'sha256']\nlog[sum[digest['a']]]\n";
        let stmts = crate::build(src).unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let pure = find_pure_functions(&ctx);
        assert!(!pure.contains("digest"), "a salted hash is random");
        assert!(pure.contains("sum"), "a digest is pure");
    }

    #[test]
    fn test_impure_transitive() {
        let src = "fn inner[]\n    log['x']\nfn outer[]\n    inner[]\n";
        let (_, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        assert!(!pure.contains("outer"), "outer calls impure inner, should be impure");
        assert!(!pure.contains("inner"), "inner calls log, should be impure");
    }

    #[test]
    fn test_mixed_purity() {
        let src = "fn pure_fn[x]\n    x + 1\nfn impure_fn[x]\n    log[x]\nx = pure_fn[5]\n";
        let (_, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        assert!(pure.contains("pure_fn"));
        assert!(!pure.contains("impure_fn"));
    }

    // === Recursion detection ===

    #[test]
    fn test_direct_recursion() {
        let src = "fn fib[n]\n    if n lte 1\n        return n\n    return fib[n - 1] + fib[n - 2]\nlog[fib[5]]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(recursive.contains("fib"), "fib should be detected as recursive");
    }

    #[test]
    fn test_non_recursive() {
        let src = "fn double[x]\n    x * 2\nx = double[5]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(!recursive.contains("double"), "double should not be recursive");
    }

    #[test]
    fn test_indirect_recursion() {
        let src = "fn a[n]\n    if n lte 0\n        return 0\n    b[n - 1]\nfn b[n]\n    a[n - 1]\nlog[a[3]]\n";
        let (_, ctx) = parse_and_infer(src);
        let recursive = find_recursive_functions(&ctx);
        assert!(recursive.contains("a"), "a should be detected as indirectly recursive");
        assert!(recursive.contains("b"), "b should be detected as indirectly recursive");
    }

    // === Static checks: unused variables ===

    #[test]
    fn test_unused_var_in_function() {
        let src = "fn foo[x]\n    temp = 5\n    x + 1\nresult = foo[3]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, .. } if var == "temp")).collect();
        assert!(!unused.is_empty(), "should detect unused 'temp'");
    }

    #[test]
    fn test_unused_param() {
        let src = "fn foo[x, y]\n    x + 1\nresult = foo[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, .. } if var == "y")).collect();
        assert!(!unused.is_empty(), "should detect unused param 'y'");
    }

    /// A route calls its handler with the request, read or not: an unread
    /// request is no finding (the fix it had offered, removing it, makes a
    /// handler no route can call)
    #[test]
    fn a_route_handlers_unread_request_is_no_finding() {
        let src = "use web\nfn health[req]\n    return text['ok']\nfn other[req, extra]\n    return text['ok']\nroute['/health', health]\nroute['/limited', health, limit = 5]\nserve[3000]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        assert!(!issues.iter().any(|i| matches!(i, Issue::UnusedVariable { func, .. } if func == "health")), "{:?}", issues.iter().map(|i| i.summary()).collect::<Vec<_>>());
        // a function no route calls is checked as any other
        assert!(issues.iter().any(|i| matches!(i, Issue::UnusedVariable { func, var, .. } if func == "other" && var == "extra")));
    }

    #[test]
    fn test_no_false_positive_used_var() {
        let src = "fn add[a, b]\n    a + b\nresult = add[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused_in_add: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { func, .. } if func == "add")).collect();
        assert!(unused_in_add.is_empty(), "add uses both params, no unused vars expected");
    }

    #[test]
    fn test_unused_var_top_level() {
        let src = "x = 42\ny = 10\nlog[y]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unused: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnusedVariable { var, func, .. } if var == "x" && func == "<top-level>")).collect();
        assert!(!unused.is_empty(), "should detect unused top-level 'x'");
    }

    // === Static checks: unreachable code ===

    #[test]
    fn test_unreachable_after_return() {
        let src = "fn foo[x]\n    return x\n    y = 5\nresult = foo[1]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unreachable: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnreachableCode { .. })).collect();
        assert!(!unreachable.is_empty(), "should detect unreachable code after return");
    }

    #[test]
    fn test_no_unreachable_normal() {
        let src = "fn foo[x]\n    y = x + 1\n    return y\nresult = foo[1]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let issues = run_static_checks(&ctx, &stmts);
        let unreachable: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::UnreachableCode { .. })).collect();
        assert!(unreachable.is_empty(), "no unreachable code expected");
    }

    // === Edge case generation ===

    #[test]
    fn test_edge_case_crash_detected() {
        let src = "fn share[a, b]\n    a / b\nresult = share[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, _snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        let crashes: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::EdgeCaseCrash { error, .. } if error.contains("division by zero"))).collect();
        assert!(!crashes.is_empty(), "share should crash dividing by zero");
    }

    /// Ints grow: a sum of the largest ints is a whole number past 64 bits,
    /// no crash (it had stopped with an overflow)
    #[test]
    fn test_edge_case_a_sum_of_the_largest_ints_grows() {
        let src = "fn add[a, b]\n    a + b\nresult = add[1, 2]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, _snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        let crashes: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::EdgeCaseCrash { .. })).collect();
        assert!(crashes.is_empty(), "{:?}", crashes);
    }

    #[test]
    fn test_edge_case_empty_array_crash() {
        let src = "fn first[items]\n    return items[0]\nnums = [1, 2]\nresult = first[nums]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, _) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        let crashes: Vec<_> = issues.iter().filter(|i| matches!(i, Issue::EdgeCaseCrash { error, .. } if error.contains("index 0 is out of range for an array of 0 items"))).collect();
        assert!(!crashes.is_empty(), "first[[]] should crash with index out of bounds");
    }

    #[test]
    fn test_edge_case_skips_recursive() {
        let src = "fn fib[n]\n    if n lte 1\n        return n\n    return fib[n - 1] + fib[n - 2]\nlog[fib[5]]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        assert!(issues.is_empty(), "recursive fib should be skipped, no issues");
        assert!(snapshots.is_empty(), "recursive fib should be skipped, no snapshots");
    }

    #[test]
    fn test_edge_case_skips_impure() {
        let src = "fn greet[name]\n    log['hi {name}']\ngreet['world']\n";
        let (stmts, ctx) = parse_and_infer(src);
        let pure = find_pure_functions(&ctx);
        let recursive = find_recursive_functions(&ctx);
        let (issues, snapshots) = run_edge_cases(&ctx, &stmts, &pure, &recursive);
        assert!(issues.is_empty(), "impure greet should not be tested");
        assert!(snapshots.is_empty(), "impure greet should not be tested");
    }

    // === Full pipeline ===

    /// The probes see a program's declarations as its tests do — imports,
    /// derivations and facts included, so a ui app's declarations lower and
    /// its pure functions are probed — but never probe a view: rendering one
    /// computes the derivations it reads (a clock here, the databases in
    /// finch) twice per probe. The view is not reported nondeterministic
    /// and the pure function is probed.
    #[test]
    fn the_probes_lower_a_ui_app_and_leave_its_views_alone() {
        let src = "use web, ui\nn = 0\nderive stamp\n    return fmt[now[], style = 'iso']\nfn half[x]\n    return x / 2.0\nlog[half[4.0]]\nview v[]\n    return p['`stamp` `n`']\npage['/', v]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let result = run_check(&stmts, &ctx, false);
        assert!(!result.issues.iter().any(|i| matches!(i, Issue::Nondeterministic { .. })), "{:?}", result.issues);
        assert!(result.edge_cases_generated > 0, "the pure function was not probed");
    }

    #[test]
    fn test_run_check_clean_program() {
        let src = "fn double[x]\n    x * 2\nresult = double[5]\nlog[result]\n";
        let (stmts, ctx) = parse_and_infer(src);
        let result = run_check(&stmts, &ctx, false);
        assert!(result.pure_fns.contains("double"));
        assert!(result.total_functions == 1);
    }
}