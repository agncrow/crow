//! Functions, derivations and statements.

use super::program::{call_name, visit_block};
use super::*;

impl Emitter {
    /// The derivations a block reads directly (inside text too).
    pub(crate) fn derivations_read(block: &[Stmt]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        visit_block(block, &mut |e| {
            if let Expr::Derived(d, _) = e {
                out.insert(d.clone());
            }
        });
        out
    }

    // ── functions ───────────────────────────────────────────────────

    pub(crate) fn emit_function(&mut self, f: &Fn) {
        let name = f.name.clone();
        let refs = self.by_ref_params.get(&name).cloned().unwrap_or_default();
        let is_live_render = self.live_render_fns.contains(&name);
        let is_live_handle = self.live_handle_fns.contains(&name);
        let is_reactive_render = self.reactive_render_fns.contains(&name);
        let is_reactive_handler = self.reactive_event_handlers.contains(&name);
        let is_component_view = self.views.contains(&name);

        let param_ty = |t: &Ty| -> Ty { as_inferred(t) };
        let params: Vec<String> = if is_reactive_render {
            let mut ps = vec!["state: &CrowLiveState".to_string()];
            for (p, t) in &f.params {
                let rt = match param_ty(t) {
                    Ty::Str | Ty::Markup | Ty::Unknown => "String".to_string(),
                    other => other.to_rust_type(),
                };
                ps.push(format!("{}: {}", p, rt));
            }
            ps
        } else if is_reactive_handler {
            let mut ps = vec!["state: &mut CrowLiveState".to_string()];
            for (p, t) in &f.params {
                let rt = match param_ty(t) {
                    Ty::Int => "i64".to_string(),
                    Ty::Float => "f64".to_string(),
                    Ty::Bool => "bool".to_string(),
                    Ty::Str | Ty::Markup | Ty::Unknown => "String".to_string(),
                    other => other.to_rust_type(),
                };
                ps.push(format!("{}: {}", p, rt));
            }
            ps
        } else if is_live_render {
            f.params.iter().enumerate().map(|(i, (p, _))| if i == 0 { format!("{}: &CrowLiveState", p) } else { format!("{}: String", p) }).collect()
        } else if is_live_handle {
            f.params
                .iter()
                .enumerate()
                .map(|(i, (p, _))| match i {
                    0 => format!("{}: &mut CrowLiveState", p),
                    1 => format!("{}: &str", p),
                    2 => format!("{}: Option<&str>", p),
                    _ => format!("{}: String", p),
                })
                .collect()
        } else {
            let changed = Self::mutated_in(&f.body);
            f.params
                .iter()
                .map(|(p, t)| {
                    let rt = param_ty(t).to_rust_type();
                    if refs.contains(p) {
                        format!("{}: &{}", p, rt)
                    } else if changed.contains(p) {
                        // a parameter the body changes — `c = false`,
                        // `xs[0] = 5`, `p.x = 3` — is the body's own copy,
                        // as the interpreter's is (compiled, it had not built)
                        format!("mut {}: {}", p, rt)
                    } else {
                        format!("{}: {}", p, rt)
                    }
                })
                .collect()
        };
        let ret_ty = as_inferred(&f.ret);
        let ret_str = if is_live_render || is_component_view {
            " -> CrowVNode".to_string()
        } else if is_live_handle {
            String::new()
        } else {
            match &ret_ty {
                Ty::Unit => String::new(),
                t => format!(" -> {}", t.to_rust_type()),
            }
        };
        self.line("#[allow(unused_mut, unused_variables, non_snake_case, unreachable_code)]");
        self.line(&format!("fn {}({}){} {{", Self::rust_fn_name(&name), params.join(", "), ret_str));
        self.indent += 1;

        // the function's own context, restored when it is printed
        let saved = (
            std::mem::take(&mut self.scopes),
            std::mem::take(&mut self.ref_params),
            std::mem::take(&mut self.mutated),
            self.cur_ret.take(),
            self.returns_unit,
            std::mem::take(&mut self.cur_fn),
            self.view_mark,
        );
        // a recursive function's calls nest to the limit and no deeper, as
        // the interpreter counts them (crow_runtime::stops::DEPTH)
        if f.recursive {
            self.needs_crow_runtime = true;
            self.line(&format!("let __crow_depth = crow_runtime::stops::Depth::enter().unwrap_or_else(|e| crow_stop({}, e));", crate::ir::first_line(&f.body)));
        }
        self.scopes = vec![HashMap::new()];
        self.ref_params = refs;
        self.cur_ret = Some(ret_ty.clone());
        self.returns_unit = ret_str.is_empty();
        self.cur_fn = name.clone();
        self.mutated = Self::mutated_in(&f.body);
        for (p, t) in &f.params {
            let mut ty = as_inferred(t);
            if (is_reactive_handler || is_reactive_render) && matches!(ty, Ty::Unknown) {
                ty = Ty::Str;
            }
            self.declare(p, ty);
        }
        // a function that calls cpu[…] puts its caller's count back as it
        // returns (crow_runtime::cores)
        if crate::ir::sets_cores(&f.body) {
            self.needs_crow_runtime = true;
            self.line("let _cores = crow_runtime::cores::Restore::new();");
        }
        // the vectors the body only reads and writes by index, kept plain
        // (vectors.rs); a parameter's is made as the body starts
        let saved_vecs = std::mem::take(&mut self.plain_vecs);
        let saved_builders = std::mem::take(&mut self.text_builders);
        if !(is_reactive_render || is_reactive_handler || is_live_render || is_live_handle || is_component_view || self.emitting_dual || self.emitting_traced) {
            // and the lists of text it only builds and joins, kept as text
            // (builders.rs)
            self.text_builders = builders::text_builders(f);
            self.plain_vecs = vectors::plain_vectors(f);
            for (p, _) in &f.params {
                if self.plain_vecs.contains(p) {
                    self.needs_crow_runtime = true;
                    self.line(&format!("let mut {p}: Vec<f64> = {p}.to_vec();"));
                }
            }
        }

        self.view_mark = is_reactive_render && is_component_view && !self.derive_fns.contains(&name);
        if self.view_mark {
            self.line("let __view = state.view_enter();");
        }
        // the session's facts are read and written in place, where the body
        // does; a handler, which writes them, reads a derivation where it
        // uses it too — the memo answers at once unless a fact it reads has
        // moved — and a view, which writes none, reads its derivations once
        // as it starts
        let saved_facts = self.facts_in_state;
        self.facts_in_state = is_reactive_render || is_reactive_handler;
        if is_reactive_render {
            if !self.derive_fns.is_empty() {
                for d in Self::derivations_read(&f.body) {
                    if d == name {
                        continue;
                    }
                    let dty = self.derives.get(&d).map(|x| as_inferred(&x.ty)).unwrap_or(Ty::Unknown);
                    self.line(&format!("let {} = __derive_{}(state);", d, d));
                    self.declare(&d, dty);
                }
            }
        }

        // a handler is void: its last statement runs for its effect and the
        // epilogue follows it; a function with a value returns its last one
        let mut body = f.body.clone();
        let last = body.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
        if is_reactive_handler || is_live_handle {
            if let Some(i) = last {
                if let Stmt::Return(Some(e)) = &body[i] {
                    body[i] = Stmt::Do(e.clone());
                }
            }
        } else if !matches!(ret_ty, Ty::Unit) {
            if let Some(i) = last {
                if let Stmt::Do(e) = &body[i] {
                    body[i] = Stmt::Return(Some(e.clone()));
                }
            }
        }
        if is_reactive_handler {
            self.needs_serde_json = true;
            let mut lines: Vec<String> = Vec::new();
            // the writes this handler made, each with what it says about its
            // rows (crow_db records them): the readers of their tables look
            // again, and a derivation over a slice they did not touch keeps
            // its value
            if self.needs_crow_db {
                if self.derive_fns.is_empty() {
                    lines.push("let _ = crow_db::take_write_keys();".to_string());
                } else {
                    lines.push("state.note_writes(crow_db::take_write_keys());".to_string());
                }
            } else if !self.derive_fns.is_empty() {
                for t in &f.tables_written {
                    lines.push(format!("state.note_table_write(\"{}\");", t));
                }
            }
            self.reactive_epilogue = Some(lines);
        }

        self.emit_block(&body);

        self.facts_in_state = saved_facts;
        if let Some(lines) = self.reactive_epilogue.take() {
            for l in &lines {
                self.line(l);
            }
        }
        let (scopes, refs, mutated, ret, unit, cur, mark) = saved;
        self.scopes = scopes;
        self.ref_params = refs;
        self.mutated = mutated;
        self.cur_ret = ret;
        self.returns_unit = unit;
        self.cur_fn = cur;
        self.view_mark = mark;
        self.plain_vecs = saved_vecs;
        self.text_builders = saved_builders;
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    /// A derivation: its body, and the memoizing reader the state asks
    /// for the cached value or a recomputation.
    pub(crate) fn emit_derive(&mut self, d: &Derive) {
        let name = d.name.clone();
        let rt = as_inferred(&d.ty);
        let ty = match &rt {
            Ty::Unit | Ty::Unknown => "String".to_string(),
            t => t.to_rust_type(),
        };
        self.line("#[allow(unused_mut, unused_variables, non_snake_case, unreachable_code)]");
        self.line(&format!("fn __derive_body_{}(state: &CrowLiveState) -> {} {{", name, ty));
        self.indent += 1;
        let saved = (std::mem::take(&mut self.scopes), self.cur_ret.take(), self.returns_unit, std::mem::take(&mut self.cur_fn), std::mem::take(&mut self.mutated));
        self.scopes = vec![HashMap::new()];
        self.cur_ret = Some(rt.clone());
        self.returns_unit = false;
        self.cur_fn = name.clone();
        self.mutated = Self::mutated_in(&d.body);
        let saved_facts = std::mem::replace(&mut self.facts_in_state, true);
        for other in Self::derivations_read(&d.body) {
            if other == name {
                continue;
            }
            let oty = self.derives.get(&other).map(|x| as_inferred(&x.ty)).unwrap_or(Ty::Unknown);
            self.line(&format!("let {} = __derive_{}(state);", other, other));
            self.declare(&other, oty);
        }
        let mut body = d.body.clone();
        if let Some(i) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if let Stmt::Do(e) = &body[i] {
                body[i] = Stmt::Return(Some(e.clone()));
            }
        }
        self.emit_block(&body);
        self.facts_in_state = saved_facts;
        let (scopes, ret, unit, cur, mutated) = saved;
        self.scopes = scopes;
        self.cur_ret = ret;
        self.returns_unit = unit;
        self.cur_fn = cur;
        self.mutated = mutated;
        self.indent -= 1;
        self.line("}");
        self.line("");

        let rs = &d.spec;
        let eq = if matches!(rt, Ty::Int | Ty::Float | Ty::Str | Ty::Bool) { "|a, b| a == b" } else { "|_a, _b| false" };
        let list = |xs: &BTreeSet<String>| xs.iter().map(|x| format!("\"{}\"", x)).collect::<Vec<_>>().join(", ");
        self.line(&format!(
            "static __DERIVE_SPEC_{n}: crow_web::live::DeriveSpec = crow_web::live::DeriveSpec {{ name: \"{n}\", facts: &[{f}], derives: &[{dd}], tables: &[{t}], now: {now}, external: {ext}, fresh: {fresh:?}, shared: {sh}, keys: &[{k}] }};",
            n = name,
            f = list(&rs.facts),
            dd = list(&rs.derives),
            t = list(&rs.tables),
            now = rs.now,
            ext = rs.external,
            fresh = d.fresh,
            sh = rs.shared,
            k = rs.keys.iter().map(|(t, (c, f))| format!("(\"{}\", \"{}\", \"{}\")", t, c, f)).collect::<Vec<_>>().join(", ")
        ));
        self.line(&format!("fn __derive_{}(state: &CrowLiveState) -> {} {{", name, ty));
        self.indent += 1;
        for other in &rs.derives {
            if other != &name {
                self.line(&format!("let _ = __derive_{}(state);", other));
            }
        }
        if rs.tables.is_empty() {
            self.line(&format!("state.derive(&__DERIVE_SPEC_{n}, __derive_body_{n}, {eq})", n = name, eq = eq));
        } else {
            let pending = match &rt {
                Ty::Result(inner) => format!(
                    "Some((|| CrowResult {{ ok: false, value: {}, reason: \"loading\".to_string(), kind: \"pending\".to_string(), trace: vec![], captured: 0.0 }}) as fn() -> {})",
                    inner.default_rust_value(),
                    ty
                ),
                _ => "None".to_string(),
            };
            self.line(&format!("state.derive_bg(&__DERIVE_SPEC_{n}, __derive_body_{n}, {eq}, {pending}, __derive_{n})", n = name, eq = eq, pending = pending));
        }
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    /// A function `grad` differentiates over tensors, or one it reaches:
    /// its copy over traced values (crow_runtime::autograd::Traced), every
    /// number and tensor traced, every operation the kernel's.
    pub(crate) fn emit_traced_function(&mut self, f: &Fn) {
        let traced = |t: &Ty| matches!(as_inferred(t), Ty::Float | Ty::Tensor(..));
        let params: Vec<String> = f
            .params
            .iter()
            .map(|(p, t)| if traced(t) { format!("{}: crow_runtime::autograd::Traced", p) } else { format!("{}: {}", p, as_inferred(t).to_rust_type()) })
            .collect();
        let ret = if traced(&f.ret) { "crow_runtime::autograd::Traced".to_string() } else { as_inferred(&f.ret).to_rust_type() };
        self.needs_crow_runtime = true;
        self.line("#[allow(unused_mut, unused_variables, non_snake_case, unreachable_code)]");
        self.line(&format!("fn _traced_{}({}) -> {} {{", Self::rust_fn_name(&f.name), params.join(", "), ret));
        self.indent += 1;
        let saved = (
            std::mem::take(&mut self.scopes),
            std::mem::take(&mut self.ref_params),
            std::mem::take(&mut self.mutated),
            self.cur_ret.take(),
            self.returns_unit,
            std::mem::take(&mut self.cur_fn),
        );
        self.scopes = vec![HashMap::new()];
        self.cur_ret = Some(as_inferred(&f.ret));
        self.returns_unit = false;
        self.cur_fn = f.name.clone();
        self.emitting_traced = true;
        self.mutated = Self::mutated_in(&f.body);
        for (p, t) in &f.params {
            self.declare(p, as_inferred(t));
        }
        let mut body = f.body.clone();
        if let Some(i) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if let Stmt::Do(e) = &body[i] {
                body[i] = Stmt::Return(Some(e.clone()));
            }
        }
        for s in &body {
            self.emit_stmt(s);
        }
        self.emitting_traced = false;
        let (scopes, refs, mutated, ret, unit, cur) = saved;
        self.scopes = scopes;
        self.ref_params = refs;
        self.mutated = mutated;
        self.cur_ret = ret;
        self.returns_unit = unit;
        self.cur_fn = cur;
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    /// A function `grad` differentiates, over dual numbers.
    pub(crate) fn emit_dual_function(&mut self, f: &Fn) {
        let params: Vec<String> = f.params.iter().map(|(p, _)| format!("{}: CrowDual", p)).collect();
        self.line("#[allow(unused_mut, unused_variables, non_snake_case)]");
        self.line(&format!("fn _dual_{}({}) -> CrowDual {{", Self::rust_fn_name(&f.name), params.join(", ")));
        self.indent += 1;
        let saved = (
            std::mem::take(&mut self.scopes),
            std::mem::take(&mut self.ref_params),
            std::mem::take(&mut self.mutated),
            self.cur_ret.take(),
            self.returns_unit,
            std::mem::take(&mut self.cur_fn),
        );
        self.scopes = vec![HashMap::new()];
        self.cur_ret = Some(Ty::Float);
        self.returns_unit = false;
        self.cur_fn = f.name.clone();
        self.emitting_dual = true;
        self.mutated = Self::mutated_in(&f.body);
        for (p, _) in &f.params {
            self.declare(p, Ty::Float);
        }
        let mut body = f.body.clone();
        if let Some(i) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if let Stmt::Do(e) = &body[i] {
                body[i] = Stmt::Return(Some(e.clone()));
            }
        }
        for s in &body {
            self.emit_stmt(s);
        }
        self.emitting_dual = false;
        let (scopes, refs, mutated, ret, unit, cur) = saved;
        self.scopes = scopes;
        self.ref_params = refs;
        self.mutated = mutated;
        self.cur_ret = ret;
        self.returns_unit = unit;
        self.cur_fn = cur;
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    // ── handlers and their helpers ──────────────────────────────────

    /// After a helper a handler called: when it stopped, the handler stops.
    pub(crate) fn stop_check(&self) -> String {
        if self.returns_unit && self.reactive_event_handlers.contains(&self.cur_fn) {
            "if state.is_stopped() { return; } ".to_string()
        } else {
            String::new()
        }
    }


    /// What a decision on an error does here: a handler stops, a function
    /// with a value hands the error back, a void function returns, the top
    /// level halts. `v` names the failed value.
    pub(crate) fn refuse(&self, v: &str) -> String {
        self.refuse_at(v, &Decision::Branch)
    }

    /// `refuse` for a refusal of any kind: a halt names it (`cannot loop
    /// over an error value`).
    pub(crate) fn refuse_at(&self, v: &str, at: &Decision) -> String {
        if self.reactive_epilogue.is_some() {
            format!("{{ state.stop(format!(\"{} stopped: {{}}\", {v}.reason)); return; }}", self.shown_fn_name(), v = v)
        } else if self.returns_unit && !self.cur_fn.is_empty() {
            "return;".to_string()
        } else if let Some(ret) = self.cur_ret.clone().filter(|_| !self.cur_fn.is_empty()) {
            if self.view_mark {
                // (through view_exit: the view's level closes, and a value
                // still being caught up with marks it stale)
                return format!("return state.view_exit(__view, CrowVNode::failed(&{v}.reason));", v = v);
            }
            if self.in_view() {
                return format!("return CrowVNode::failed(&{v}.reason);", v = v);
            }
            let inner_default = match &ret {
                Ty::Result(inner) => inner.default_rust_value(),
                other => other.default_rust_value(),
            };
            let trace = self.trace_format_expr();
            if matches!(ret, Ty::Result(_)) {
                format!(
                    "return CrowResult {{ ok: false, value: {}, reason: {v}.reason.clone(), kind: {v}.kind.clone(), trace: {{ let mut _t = {v}.trace.clone(); crow_trace_push(&mut _t, {}); _t }}, captured: 0.0 }};",
                    inner_default,
                    trace,
                    v = v
                )
            } else {
                // a function whose value is not a result cannot hand the error
                // back: the program halts with the reason, as the top level does
                self.halt_at(v, at)
            }
        } else {
            self.halt_at(v, at)
        }
    }

    /// The program stops on a refused error, naming the line and the reason.
    fn halt_at(&self, v: &str, at: &Decision) -> String {
        format!(
            "{{ eprintln!(\"runtime error: line {}: {} — {{}}\", {v}.reason); std::process::exit(1); }}",
            self.line_now(),
            at.refusal(),
            v = v
        )
    }

    /// A condition that is a result: an error refuses, a value decides.
    pub(crate) fn guard_condition(&mut self, cond: &Expr, cond_str: String) -> String {
        if !matches!(self.tt(cond), Ty::Result(_)) {
            return cond_str;
        }
        let on_error = self.refuse("_c");
        format!("{{ let _c = &({}); if !_c.ok {{ {} }} _c.value }}", cond_str, on_error)
    }

    // ── statements ──────────────────────────────────────────────────

    pub(crate) fn emit_block(&mut self, block: &[Stmt]) {
        for s in block {
            self.emit_stmt(s);
        }
    }

    /// A block in a scope of its own.
    fn emit_scoped(&mut self, block: &[Stmt]) {
        self.push_scope();
        self.emit_block(block);
        self.pop_scope();
    }

    pub(crate) fn emit_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Line(n) => self.current_line = *n,
            Stmt::Let { var, ty, expr } => self.emit_let(var, ty, expr),
            Stmt::Set { var, expr } => self.emit_set(var, expr),
            Stmt::Store { fact, expr } => {
                if self.facts_in_state && self.reactive_vars.contains_key(fact) {
                    self.emit_store_fact(fact, expr);
                } else if self.declared(fact) {
                    self.emit_set(fact, expr);
                } else if self.has_page_routes && self.cur_fn.is_empty() {
                    // a page program's top-level assignment set the default
                } else {
                    let v = self.owned_expr(expr);
                    self.line(&format!("{} = {};", fact, v));
                }
            }
            Stmt::SetPath { var, path, expr } => self.emit_set_path(var, path, expr),
            Stmt::If { cond, then, els } => {
                let c = self.expr(cond);
                let c = self.guard_condition(cond, c);
                self.line(&format!("if {} {{", c));
                self.indent += 1;
                self.emit_scoped(then);
                self.indent -= 1;
                if let Some(e) = els {
                    self.line("} else {");
                    self.indent += 1;
                    self.emit_scoped(e);
                    self.indent -= 1;
                }
                self.line("}");
            }
            Stmt::While { cond, body } => {
                let infinite = matches!(cond, Expr::Const(Const::Bool(true), _));
                if infinite {
                    self.line("loop {");
                } else {
                    let c = self.expr(cond);
                    let c = self.guard_condition(cond, c);
                    self.line(&format!("while {} {{", c));
                }
                self.indent += 1;
                self.emit_scoped(body);
                self.indent -= 1;
                self.line("}");
            }
            Stmt::For { var, iter, body, parallel } => self.emit_for(var, iter, body, *parallel),
            Stmt::Return(Some(e)) => self.emit_return(e),
            Stmt::Return(None) => {
                if let Some(lines) = self.reactive_epilogue.clone() {
                    for l in &lines {
                        self.line(l);
                    }
                }
                self.line("return;");
            }
            Stmt::Break => self.line("break;"),
            Stmt::Continue => self.line("continue;"),
            Stmt::Do(e) => self.emit_do(e),
            Stmt::Propagate { values, at } => {
                for v in values {
                    if matches!(self.local_ty(v), Some(Ty::Result(_))) {
                        let on_error = self.refuse_at("_c", at);
                        self.line(&format!("{{ let _c = &{}; if !_c.ok {{ {} }} }}", v, on_error));
                    }
                }
            }
        }
    }

    fn emit_do(&mut self, e: &Expr) {
        // cpu[n] bounds the loops that start after it, where it runs
        // (crow_runtime::cores); gpu[] and nodes[n] are the program's
        // resources
        if call_name(e).as_deref() == Some("cpu")
            && let [n] = super::program::call_args(e)
        {
            let n = self.expr(n);
            let line = self.line_now();
            self.needs_crow_runtime = true;
            self.line(&format!("crow_runtime::cores::set(({}) as i64).unwrap_or_else(|e| crow_stop({}, e));", n, line));
            return;
        }
        if call_name(e).as_deref() == Some("gpu") && super::program::call_args(e).len() <= 1 && matches!(e, Expr::Call { target: Target::Runtime(_), .. }) {
            self.needs_crow_runtime = true;
            self.line("crow_runtime::gpu::set_gpu_enabled(true);");
            return;
        }
        if call_name(e).as_deref() == Some("nodes") && matches!(e, Expr::Call { target: Target::Runtime(_), .. }) {
            if let Some(Expr::Const(Const::Int(n), _)) = super::program::call_args(e).first().map(Emitter::raw) {
                self.needs_crow_runtime = true;
                self.needs_distributed = true;
                self.line(&format!("crow_runtime::distributed::init({});", n));
                return;
            }
        }
        let text = self.expr(e);
        self.line(&format!("{};", text));
    }

    /// Does an expression build a tree: a view's call, or inside a view an
    /// element, a chart, an icon or a bound input?
    pub(crate) fn makes_node(&self, e: &Expr) -> bool {
        match Emitter::raw(e) {
            Expr::Tree { .. } => self.in_view(),
            Expr::Call { target: Target::User(n), .. } => self.views.contains(n),
            Expr::Call { target: Target::Runtime(n), args, .. } => {
                self.in_view()
                    && (matches!(n.as_str(), "chart" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs" | "progress" | "icon") || (n == "select" && args.len() == 2))
            }
            _ => false,
        }
    }

    fn emit_let(&mut self, var: &str, ty: &Ty, expr: &Expr) {
        if self.text_builders.contains_key(var) {
            self.line(&format!("let mut {var}: String = String::new(); let mut {var}__count: i64 = 0;"));
            self.declare(var, as_inferred(ty));
            return;
        }
        if self.plain_vecs.contains(var) {
            let text = self.owned_expr(expr);
            self.line(&format!("let mut {}: Vec<f64> = ({}).to_vec();", var, text));
            self.declare(var, as_inferred(ty));
            return;
        }
        let mut text = self.owned_expr(expr);
        let mut holds_markup = false;
        if self.in_view() && self.makes_node(expr) {
            text = format!("({}).to_html()", text);
            holds_markup = true;
        }
        let mut t = self.tt_full(expr);
        if holds_markup {
            t = Ty::Markup;
        }
        if t.has_unknown() {
            let declared = as_inferred(ty);
            // a first binding that cannot fail is not a result because a
            // later one can (`ts = []`, then `ts = push[ts, float[x]]`): the
            // later is a consumption point, and the variable holds values
            let declared = if matches!(t, Ty::Result(_)) { declared } else { Self::inner(&declared) };
            if !declared.has_unknown() || matches!(t, Ty::Unknown) {
                t = declared;
            }
        }
        if matches!(t, Ty::Int) && matches!(as_inferred(ty), Ty::Float) {
            t = Ty::Float;
            text = format!("({}) as f64", text);
        }
        // a variable holding a record or none (a tree's child, 0 where
        // there is none): 0 is none, a record is wrapped
        if let Ty::OptRecord(_) = as_inferred(ty) {
            if matches!(Emitter::raw(expr), Expr::Const(Const::Int(0), _)) {
                t = as_inferred(ty);
                text = "None".to_string();
            } else if matches!(t, Ty::Record(_)) {
                t = as_inferred(ty);
                text = format!("Some(Box::new({}))", text);
            }
        }
        let kw = if self.mutated.contains(var) { "let mut" } else { "let" };
        let in_live_callback = self.live_render_fns.contains(&self.cur_fn) || self.live_handle_fns.contains(&self.cur_fn) || self.views.contains(&self.cur_fn);
        if self.emitting_dual || self.emitting_traced || in_live_callback || matches!(t, Ty::Unknown) {
            self.line(&format!("{} {} = {};", kw, var, text));
        } else {
            self.line(&format!("{} {}: {} = {};", kw, var, t.to_rust_type(), text));
        }
        self.declare(var, t);
    }

    fn emit_set(&mut self, var: &str, expr: &Expr) {
        if let Some(sep) = self.text_builders.get(var).cloned() {
            match builders::appended(var, expr) {
                Some(piece) => self.emit_append(var, &sep, piece),
                None => self.line(&format!("{var}.clear(); {var}__count = 0;")),
            }
            return;
        }
        if self.plain_vecs.contains(var) {
            let text = self.owned_expr(expr);
            self.line(&format!("{} = ({}).to_vec();", var, text));
            return;
        }
        // `x = push[x, v]` and its kin move `x` into the runtime call so the
        // collection grows in place; every other shape clones
        self.move_target = if self.declared(var) && !self.static_handle_vars.contains(var) && !self.ref_params.contains(var) && Self::in_place_call_on(expr, var) {
            Some(var.to_string())
        } else {
            None
        };
        let mut text = self.owned_expr(expr);
        self.move_target = None;
        if self.in_view() && self.makes_node(expr) {
            text = format!("({}).to_html()", text);
        }
        let new_ty = self.tt(expr);
        if let Some(existing) = self.local_ty(var) {
            let existing = existing.text();
            // a result assigned into a variable whose first value could not
            // fail: the lowering refuses the error first (spec, Reassignment);
            // one whose type only this backend knew is refused the same way
            if let Ty::Result(inner) = &new_ty {
                if existing.compatible(inner) || (matches!(existing, Ty::Float) && matches!(inner.as_ref(), Ty::Int)) {
                    let abort = self.refuse_at("_asg", &Decision::Assign(var.to_string()));
                    let cast = if matches!(existing, Ty::Float) && matches!(inner.as_ref(), Ty::Int) { " as f64" } else { "" };
                    self.line(&format!("{{ let _asg = {}; if !_asg.ok {{ {} }} {} = _asg.value{}; }}", text, abort, var, cast));
                    return;
                }
            }
            if matches!(existing, Ty::Float) && matches!(new_ty, Ty::Int) {
                self.line(&format!("{} = ({}) as f64;", var, text));
                return;
            }
            // a variable holding a record or none given 0, or a record
            if let Ty::OptRecord(_) = &existing {
                if matches!(Emitter::raw(expr), Expr::Const(Const::Int(0), _)) {
                    self.line(&format!("{} = None;", var));
                    return;
                }
                if matches!(new_ty, Ty::Record(_)) {
                    self.line(&format!("{} = Some(Box::new({}));", var, text));
                    return;
                }
            }
            // a plain value given to a variable whose first value could
            // fail: the variable keeps holding a value that can fail, and
            // this one is a success (`cur = loss[…]`, then `cur = inf`:
            // the interpreter had run it, and this backend refused it)
            if let Ty::Result(inner) = &existing {
                if !matches!(new_ty, Ty::Result(_)) && (inner.compatible(&new_ty) || (matches!(inner.as_ref(), Ty::Float) && matches!(new_ty, Ty::Int))) {
                    let cast = if matches!(inner.as_ref(), Ty::Float) && matches!(new_ty, Ty::Int) { " as f64" } else { "" };
                    self.line(&format!("{} = CrowResult {{ ok: true, value: ({}){}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }};", var, text, cast));
                    return;
                }
            }
            if !existing.compatible(&new_ty) {
                self.fail(format!("cannot reassign '{}' from {} to {} — variable type cannot change", var, existing.crow_name(), new_ty.crow_name()));
            }
        }
        self.line(&format!("{} = {};", var, text));
    }

    /// A statement after the lines it needs, in a block of its own when
    /// there are any (the indices a path assignment found first).
    fn line_after(&mut self, found: &[String], text: &str) {
        if found.is_empty() {
            self.line(text);
        } else {
            self.line(&format!("{{ {} {} }}", found.join(" "), text));
        }
    }

    /// `x = push[x, v]`: an in-place builtin whose collection is `x` itself.
    fn in_place_call_on(e: &Expr, var: &str) -> bool {
        match e {
            Expr::Call { target: target @ (Target::Runtime(_) | Target::Row(_)), args, .. } => {
                let path = match target {
                    Target::Row(row) if row.module.is_empty() => row.name,
                    Target::Runtime(path) => path.as_str(),
                    _ => return false,
                };
                // only when the right side reads the variable once (spec:
                // `xs = push[xs, float[len[xs]]]` reads it again after the
                // move, and is a copy)
                let mut again = false;
                for a in args.iter().skip(1) {
                    crate::ir::visit_expr(a, &mut |x| again |= matches!(x, Expr::Local(n, _) | Expr::Move(n, _) if n == var));
                }
                !again && in_place_builtin(path) && matches!(args.first().map(Emitter::raw), Some(Expr::Move(v, _)) | Some(Expr::Local(v, _)) if v == var)
            }
            _ => false,
        }
    }

    fn emit_set_path(&mut self, var: &str, path: &[PathStep], expr: &Expr) {
        if self.plain_vecs.contains(var)
            && let [PathStep::Index(i)] = path
        {
            // the value first, then the index, as the interpreter reads them
            let val = self.expr(expr);
            let idx = self.expr(i);
            let line = self.line_now();
            self.line(&format!("{{ let _v = ({}) as f64; let _i = {}; let _k = crow_runtime::stops::item(_i, {}.len()).unwrap_or_else(|e| crow_stop({}, e)); {}[_k] = _v; }}", val, idx, var, line, var));
            return;
        }
        // a part of a fact of the session (`items[0] = x`, `p.qty = 3`),
        // written in place: the value first, then the write, then the
        // fact's version (typed-values-design.md §6)
        let in_facts = self.facts_in_state && self.reactive_vars.contains_key(var);
        let mut val = self.expr(expr);
        if in_facts {
            self.line("{");
            self.indent += 1;
            self.line(&format!("let _fv = {};", val));
            val = "_fv".to_string();
        }
        self.emit_set_path_with(var, path, &val, in_facts);
        if in_facts {
            self.line(&format!("state.touch(\"{}\");", var));
            self.indent -= 1;
            self.line("}");
        }
    }

    /// A fact of the session written in place: the value converted as an
    /// assignment converts it (an int into a float, a value that can fail
    /// refused first), then the fact's version bumps when the value moved,
    /// as the state's own `set` decides (typed-values-design.md §6).
    fn emit_store_fact(&mut self, fact: &str, expr: &Expr) {
        let existing = self.reactive_vars.get(fact).map(|(t, _)| t.clone()).unwrap_or(Ty::Unknown).text();
        let text = self.owned_expr(expr);
        let new_ty = self.tt(expr);
        let float_from_int = |a: &Ty, b: &Ty| matches!(a, Ty::Float) && matches!(b, Ty::Int);
        let value = match &new_ty {
            Ty::Result(inner) if !matches!(existing, Ty::Result(_)) && (existing.compatible(inner) || float_from_int(&existing, inner)) => {
                let abort = self.refuse_at("_asg", &Decision::Assign(fact.to_string()));
                let cast = if float_from_int(&existing, inner) { " as f64" } else { "" };
                format!("{{ let _asg = {}; if !_asg.ok {{ {} }} _asg.value{} }}", text, abort, cast)
            }
            _ if float_from_int(&existing, &new_ty) => format!("({}) as f64", text),
            _ => {
                if !matches!(existing, Ty::Unknown) && !existing.compatible(&new_ty) {
                    self.fail(format!("cannot reassign '{}' from {} to {} — variable type cannot change", fact, existing.crow_name(), new_ty.crow_name()));
                }
                text
            }
        };
        let field = Self::rust_field(fact);
        let moved = if self.crow_compares(&existing) {
            format!("__facts(&*state).{} != _v", field)
        } else {
            format!("serde_json::to_value(&__facts(&*state).{}).ok() != serde_json::to_value(&_v).ok()", field)
        };
        self.line(&format!("{{ let _v = {}; let _moved = {}; __facts_mut(state).{} = _v; if _moved {{ state.touch(\"{}\"); }} }}", value, moved, field, fact));
    }

    /// Whether values of a type compare in Rust as `eq` compares them.
    pub(crate) fn crow_compares(&self, t: &Ty) -> bool {
        match t {
            Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Markup | Ty::Json | Ty::Tensor(..) | Ty::Frame(_) | Ty::Column(_) | Ty::Unknown => true,
            Ty::List(i) | Ty::Map(_, i) | Ty::Result(i) => self.crow_compares(i),
            Ty::Record(n) => self.record_compares(n, &mut BTreeSet::new()),
            _ => false,
        }
    }

    fn emit_set_path_with(&mut self, var: &str, path: &[PathStep], val: &str, in_facts: bool) {
        let val = val.to_string();
        // the object the last step writes into, and its type; each list
        // index is found before the write (`out[m].status = …` read
        // `out.len()` inside the element it was writing, which Rust's
        // borrows refuse)
        let mut obj = if in_facts {
            format!("__facts_mut(state).{}", Self::rust_field(var))
        } else if self.static_handle_vars.contains(var) {
            format!("__crow_handle_{}.get().unwrap().clone()", var)
        } else {
            var.to_string()
        };
        let mut ty = if in_facts { self.reactive_vars.get(var).map(|(t, _)| t.clone()).unwrap_or(Ty::Unknown).text() } else { self.local_ty(var).unwrap_or(Ty::Unknown).text() };
        // a value that failed has no items to set, as it has none to read
        // (spec, Errors): the program stops as the interpreter does, and a
        // value that holds one is written into (`t[0] = 5.0` on a tensor a
        // moment's text reached had asked the status for its length, and
        // the compiled program did not build)
        if !in_facts {
            if let Ty::Result(inner) = &ty {
                self.line(&format!(
                    "{{ let _c = &{}; if !_c.ok {{ crow_stop({}, format!(\"cannot set an item of an error value — {{}}\", _c.reason)); }} }}",
                    obj,
                    self.line_now()
                ));
                obj = format!("{}.value", obj);
                ty = inner.as_ref().clone();
            }
        }
        let mut found: Vec<String> = Vec::new();
        for step in &path[..path.len().saturating_sub(1)] {
            match step {
                PathStep::Field(f) => {
                    let next = match &ty {
                        Ty::Record(s) => self.structs.get(s).and_then(|d| d.fields.iter().find(|(n, _)| n == f).map(|(_, t)| as_inferred(t))).unwrap_or(Ty::Unknown),
                        Ty::Map(_, inner) => inner.as_ref().clone(),
                        _ => Ty::Unknown,
                    };
                    obj = if matches!(ty, Ty::Map(..)) {
                        format!("(*{}.get_mut(\"{}\").unwrap_or_else(|| crow_stop({}, crow_runtime::stops::no_key(\"{}\"))))", obj, f, self.line_now(), f)
                    } else {
                        format!("{}.{}", obj, Self::rust_field(f))
                    };
                    ty = next;
                }
                PathStep::Index(i) => {
                    let idx = self.expr(i);
                    let next = match &ty {
                        Ty::List(inner) | Ty::Map(_, inner) => inner.as_ref().clone(),
                        _ => Ty::Unknown,
                    };
                    obj = if matches!(ty, Ty::Map(..)) {
                        format!("(*{{ let _k = {i}; {o}.get_mut(&_k).unwrap_or_else(|| crow_stop({l}, crow_runtime::stops::no_key(&_k))) }})", i = idx, o = obj, l = self.line_now())
                    } else {
                        let k = found.len();
                        found.push(format!("let _at{k} = crow_runtime::stops::item({i}, {o}.len()).unwrap_or_else(|e| crow_stop({l}, e));", k = k, i = idx, o = obj, l = self.line_now()));
                        format!("{}[_at{}]", obj, k)
                    };
                    ty = next;
                }
            }
        }
        match path.last() {
            Some(PathStep::Field(f)) => {
                if matches!(ty, Ty::Map(..)) {
                    self.line_after(&found, &format!("{}.insert(\"{}\".to_string(), {});", obj, f, val));
                } else {
                    self.line_after(&found, &format!("{}.{} = {};", obj, Self::rust_field(f), val));
                }
            }
            Some(PathStep::Index(i)) => {
                let idx = self.expr(i);
                match &ty {
                    // the value first, then a copy of the key: `m[k] = m[k] + 1`
                    // reads the key after the insert would have taken it, and
                    // the program may read it again after
                    Ty::Map(..) => self.line_after(&found, &format!("{{ let _v = {}; {}.insert(({}).to_string(), _v); }}", val, obj, idx)),
                    // the value first, then the index, as the interpreter
                    // reads them (a stop in each had reported the other's
                    // first when both failed); a tensor's number is set
                    // where the tensor lies — it had been copied out and
                    // built again at every write, n-body's 75 a step
                    Ty::Tensor(..) => {
                        self.needs_crow_runtime = true;
                        self.line_after(&found, &format!(
                            "{{ let _v = ({}) as f64; let _i = {}; crow_runtime::core::set_flat_at(&mut {}, _i, _v).unwrap_or_else(|e| crow_stop({}, e)); }}",
                            val, idx, obj, self.line_now()
                        ));
                    }
                    _ => self.line_after(&found, &format!("{{ let _v = {}; let _i = {}; let _idx = crow_runtime::stops::item(_i, {}.len()).unwrap_or_else(|e| crow_stop({}, e)); {}[_idx] = _v; }}", val, idx, obj, self.line_now(), obj)),
                }
            }
            None => self.line_after(&found, &format!("{} = {};", obj, val)),
        }
    }

    fn emit_return(&mut self, e: &Expr) {
        if let Some(lines) = self.reactive_epilogue.clone() {
            let text = self.expr(e);
            self.line(&format!("let _ = {};", text));
            for l in &lines {
                self.line(l);
            }
            self.line("return;");
            return;
        }
        let text = self.owned_expr(e);
        let ety = self.tt(e);
        // a record returned where the function answers a record or none (a
        // tree's child, 0 where there is none): wrapped, as a constructor
        // wraps it into such a field; 0 is none
        let wrapped = |target: &Ty, text: String, ety: &Ty| -> String {
            if matches!(target, Ty::OptRecord(_)) {
                if matches!(Emitter::raw(e), Expr::Const(Const::Int(0), _)) {
                    return "None".to_string();
                }
                if matches!(ety, Ty::Record(_)) {
                    return format!("Some(Box::new({}))", text);
                }
            }
            text
        };
        if self.returns_unit {
            self.line(&format!("let _ = {};", text));
            self.line("return;");
        } else if let Some(target @ Ty::OptRecord(_)) = self.cur_ret.clone() {
            let text = wrapped(&target, text, &ety);
            self.line(&format!("return {};", text));
        } else if let Some(Ty::Result(inner)) = self.cur_ret.clone() {
            let text = wrapped(&inner, text, &ety);
            let ety = if matches!(inner.as_ref(), Ty::OptRecord(_)) && matches!(ety, Ty::Record(_)) { inner.as_ref().clone() } else { ety };
            if !matches!(ety, Ty::Result(_)) {
                // a function answering a value JSON can hold (a frame's cell
                // in one branch, an error in another) given text or a
                // number: held as JSON, as the interpreter holds a cell
                let text = if matches!(inner.as_ref(), Ty::Json) && matches!(ety, Ty::Str | Ty::Markup | Ty::Int | Ty::Float | Ty::Bool) {
                    self.needs_serde_json = true;
                    format!("serde_json::json!(({}))", text)
                } else {
                    text
                };
                self.line(&format!("return CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }};", text));
            } else if let Ty::Result(got) = &ety {
                let known = |t: &Ty| !matches!(t, Ty::Unknown);
                // (two frames of other columns are one Rust type: the error
                // passes as it is)
                let frames = matches!((got.as_ref(), inner.as_ref()), (Ty::Frame(_), Ty::Frame(_)));
                if known(got) && known(&inner) && got.as_ref() != inner.as_ref() && !frames {
                    self.line(&format!(
                        "return {{ let _r = {}; CrowResult {{ ok: _r.ok, value: {}, reason: _r.reason.clone(), kind: _r.kind.clone(), trace: _r.trace.clone(), captured: 0.0 }} }};",
                        text,
                        inner.default_rust_value()
                    ));
                } else {
                    self.line(&format!("return {};", text));
                }
            }
        } else if self.view_mark {
            self.line(&format!("return state.view_exit(__view, {});", text));
        } else {
            self.line(&format!("return {};", text));
        }
    }

    /// What a loop walks, and each item's type: a tensor's rows or its
    /// numbers (ints where it holds ints), a parsed JSON value's items (the
    /// runtime's, walked as the interpreter walks the value it parsed to),
    /// a list itself.
    pub(crate) fn loop_items(&mut self, iter_str: String, it: &Ty, iter: &Expr) -> (String, Ty) {
        if let Ty::Column(cell) = iter.ty().payload().clone() {
            return self.column_items(&iter_str, &cell);
        }
        match it {
            // rows or numbers as the type decided, through the runtime's one
            // walk (crow_runtime::core): a value of another rank stops
            Ty::Tensor(inner, dims) => {
                let elem = crate::type_infer::TypeContext::tensor_item(inner, dims);
                self.needs_crow_runtime = true;
                let line = self.line_now();
                let walked = if matches!(elem, Ty::Tensor(..)) {
                    format!("crow_runtime::core::walk_rows(&({})).unwrap_or_else(|e| crow_stop({}, e))", iter_str, line)
                } else if matches!(elem, Ty::Int) {
                    format!("crow_runtime::core::walk_numbers(&({})).unwrap_or_else(|e| crow_stop({}, e)).iter().map(|&v| v as i64).collect::<Vec<i64>>()", iter_str, line)
                } else {
                    format!("crow_runtime::core::walk_numbers(&({})).unwrap_or_else(|e| crow_stop({}, e))", iter_str, line)
                };
                (walked, elem)
            }
            Ty::Json => {
                self.needs_crow_runtime = true;
                self.needs_serde_json = true;
                (format!("crow_runtime::json::items(&({})).unwrap_or_else(|e| crow_stop({}, e))", iter_str, self.line_now()), Ty::Json)
            }
            Ty::List(inner) => (iter_str, inner.as_ref().clone()),
            _ => (iter_str, Ty::Unknown),
        }
    }

    /// A frame's column walked cell by cell, as the interpreter walks it:
    /// each cell as its column's type holds it, and where the column can
    /// have missing cells, each a result — a missing one an error of reason
    /// `missing`, as an index reads it. (A loop over a column did not build
    /// compiled.)
    fn column_items(&mut self, iter_str: &str, cell: &Ty) -> (String, Ty) {
        let cell_ty = as_inferred(cell);
        let getter = Self::cell_getter(&Self::inner(&cell_ty));
        let walked = if matches!(cell_ty, Ty::Result(_)) {
            format!("{{ let _c = {}; (0.._c.len()).map(|_i| {{ let _ok = _c.valid[_i]; let _why = if _ok {{ String::new() }} else {{ \"missing\".to_string() }}; CrowResult {{ ok: _ok, value: _c.{}(_i), reason: _why.clone(), kind: _why, trace: vec![], captured: 0.0 }} }}).collect::<Vec<_>>() }}", iter_str, getter)
        } else {
            format!("{{ let _c = {}; (0.._c.len()).map(|_i| _c.{}(_i)).collect::<Vec<_>>() }}", iter_str, getter)
        };
        (walked, cell_ty)
    }

    fn emit_for(&mut self, var: &str, iter: &Expr, body: &[Stmt], parallel: bool) {
        // `for i in range[n]` and `range[a, b]`: a counted loop, no list
        // built — the compiled fannkuch spent two thirds of its time building
        // one per permutation (a stepped range keeps its list)
        if let Expr::Call { target: crate::ir::Target::Row(row), args, ty, .. } = iter
            && matches!(row.id, "core.range_to" | "core.range_from")
            && !ty.is_result()
        {
            let (lo, hi) = match args.as_slice() {
                [n] => ("0_i64".to_string(), format!("({}) as i64", self.expr(n))),
                [a, b] => (format!("({}) as i64", self.expr(a)), format!("({}) as i64", self.expr(b))),
                _ => unreachable!("a range row takes one or two bounds"),
            };
            let use_par = parallel && !self.emitting_traced;
            self.push_scope();
            if use_par {
                self.needs_rayon = true;
                self.needs_crow_runtime = true;
                self.line(&format!("crow_runtime::cores::run(|| ({}..{}).into_par_iter().for_each(|{}| {{", lo, hi, var));
                self.indent += 1;
                self.declare(var, Ty::Int);
                self.closure_depth += 1;
                self.emit_block(body);
                self.closure_depth -= 1;
                self.indent -= 1;
                self.line("}));");
            } else {
                self.line(&format!("for {} in {}..{} {{", var, lo, hi));
                self.indent += 1;
                self.declare(var, Ty::Int);
                self.emit_block(body);
                self.indent -= 1;
                self.line("}");
            }
            self.pop_scope();
            return;
        }
        let iter_str = self.expr(iter);
        let iter_ty = self.tt(iter);
        let (actual, elem) = self.loop_items(iter_str, &iter_ty, iter);
        let is_copy = Self::is_copy_type(&elem);
        // in parallel as the lowering decided, on the cores `cpu[…]` allows
        // where the loop starts (a traced copy runs its loops in order: one
        // tape records the run)
        let use_par = parallel && !self.emitting_traced;
        // an item of text, or a parsed value, is the loop's own, as the body
        // reads it (a borrowed one compared with a cell's text did not build)
        let text = matches!(elem, Ty::Str | Ty::Markup | Ty::Json);
        self.push_scope();
        if use_par {
            self.needs_rayon = true;
            let param = if is_copy { format!("|&{}|", var) } else { format!("|{}|", var) };
            let each = if text { ".cloned()" } else { "" };
            self.needs_crow_runtime = true;
            self.line(&format!("crow_runtime::cores::run(|| {}.par_iter(){}.for_each({} {{", actual, each, param));
            self.indent += 1;
            self.declare(var, elem);
            self.closure_depth += 1;
            self.emit_block(body);
            self.closure_depth -= 1;
            self.indent -= 1;
            self.line("}));");
        } else {
            let param = if is_copy { format!("&{}", var) } else { var.to_string() };
            let each = if text { ".cloned()" } else { "" };
            self.line(&format!("for {} in {}.iter(){} {{", param, actual, each));
            self.indent += 1;
            self.declare(var, elem);
            self.emit_block(body);
            self.indent -= 1;
            self.line("}");
        }
        self.pop_scope();
    }
}
