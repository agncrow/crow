//! Functions, derivations and statements.

use super::program::{call_name, visit_block, visit_expr, visit_stmts};
use super::*;

impl Emitter {
    /// How a fact is read from the session's state.
    pub(crate) fn state_getter(vname: &str, vtype: &CrowType, default: &str) -> String {
        match vtype {
            CrowType::Int => format!("state.get_i64(\"{}\", {})", vname, default),
            CrowType::Float => format!("state.get_f64(\"{}\", {})", vname, default),
            CrowType::Str | CrowType::Markup => format!("state.get_str(\"{}\", &{})", vname, default),
            CrowType::Bool => format!("state.get_bool(\"{}\", {})", vname, default),
            other => format!(
                "state.get(\"{}\").and_then(|_v| serde_json::from_value::<{}>(_v.clone()).ok()).unwrap_or_else(|| {})",
                vname,
                other.to_rust_type(),
                default
            ),
        }
    }

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

    /// The facts a block stores itself.
    pub(crate) fn facts_stored(block: &[Stmt]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        visit_stmts(block, &mut |s| {
            if let Stmt::Store { fact, .. } = s {
                out.insert(fact.clone());
            }
        });
        out
    }

    /// The user functions a block calls, transitively.
    pub(crate) fn reach(&self, block: &[Stmt]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut todo: Vec<Block> = vec![block.to_vec()];
        while let Some(b) = todo.pop() {
            visit_block(&b, &mut |e| {
                let callee = match e {
                    Expr::Call { target: Target::User(n), .. } => Some(n.clone()),
                    Expr::FnRef(n, _) | Expr::Deferred { name: n, .. } => Some(n.clone()),
                    _ => None,
                };
                if let Some(n) = callee {
                    if self.fns.contains_key(&n) && out.insert(n.clone()) {
                        if let Some(f) = self.fns.get(&n) {
                            todo.push(f.body.clone());
                        }
                    }
                }
            });
        }
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

        let param_ty = |t: &Ty| -> CrowType { crow_type(t) };
        let params: Vec<String> = if is_reactive_render {
            let mut ps = vec!["state: &CrowLiveState".to_string()];
            for (p, t) in &f.params {
                let rt = match param_ty(t) {
                    CrowType::Str | CrowType::Markup | CrowType::Unknown => "String".to_string(),
                    other => other.to_rust_type(),
                };
                ps.push(format!("{}: {}", p, rt));
            }
            ps
        } else if is_reactive_handler {
            let mut ps = vec!["state: &mut CrowLiveState".to_string()];
            for (p, t) in &f.params {
                let rt = match param_ty(t) {
                    CrowType::Int => "i64".to_string(),
                    CrowType::Float => "f64".to_string(),
                    CrowType::Bool => "bool".to_string(),
                    CrowType::Str | CrowType::Markup | CrowType::Unknown => "String".to_string(),
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
            f.params
                .iter()
                .map(|(p, t)| {
                    let rt = param_ty(t).to_rust_type();
                    if refs.contains(p) {
                        format!("{}: &{}", p, rt)
                    } else {
                        format!("{}: {}", p, rt)
                    }
                })
                .collect()
        };
        let ret_ty = crow_type(&f.ret);
        let ret_str = if is_live_render || is_component_view {
            " -> CrowVNode".to_string()
        } else if is_live_handle {
            String::new()
        } else {
            match &ret_ty {
                CrowType::Void => String::new(),
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
        self.scopes = vec![HashMap::new()];
        self.ref_params = refs;
        self.cur_ret = Some(ret_ty.clone());
        self.returns_unit = ret_str.is_empty();
        self.cur_fn = name.clone();
        self.mutated = Self::mutated_in(&f.body);
        for (p, t) in &f.params {
            let mut ty = crow_type(t);
            if (is_reactive_handler || is_reactive_render) && matches!(ty, CrowType::Unknown) {
                ty = CrowType::Str;
            }
            self.declare(p, ty);
        }

        self.view_mark = is_reactive_render && is_component_view && !self.derive_fns.contains(&name);
        if self.view_mark {
            self.line("let __view = state.view_enter();");
        }
        if is_reactive_render || is_reactive_handler {
            for (v, (vt, d)) in self.reactive_vars.clone() {
                let kw = if is_reactive_handler { "let mut" } else { "let" };
                self.line(&format!("{} {} = {};", kw, v, Self::state_getter(&v, &vt, &d)));
                self.declare(&v, vt);
            }
            if !self.derive_fns.is_empty() {
                for d in Self::derivations_read(&f.body) {
                    if d == name {
                        continue;
                    }
                    let dty = self.derives.get(&d).map(|x| crow_type(&x.ty)).unwrap_or(CrowType::Unknown);
                    let kw = if is_reactive_handler { "let mut" } else { "let" };
                    self.line(&format!("{} {} = __derive_{}(state);", kw, d, d));
                    if is_reactive_handler {
                        self.handler_derivations.insert(d.clone());
                    }
                    self.declare(&d, dty);
                }
            }
            if is_reactive_render {
                for v in self.data_source_vars.clone() {
                    self.line(&format!("let {} = __json_to_frame(state.get(\"{}\").unwrap_or(&serde_json::Value::Null));", v, v));
                    self.declare(&v, CrowType::Frame);
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
        } else if !matches!(ret_ty, CrowType::Void) {
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
            // only the facts this body stores itself: a helper it called
            // persisted its own writes
            let own: Vec<String> = Self::facts_stored(&f.body).into_iter().filter(|v| self.reactive_vars.contains_key(v)).collect();
            self.reactive_own_writes = own.clone();
            for v in own {
                lines.push(format!("state.set(\"{}\", serde_json::json!({}));", v, v));
            }
            self.reactive_epilogue = Some(lines);
        }

        self.emit_block(&body);

        self.reactive_own_writes.clear();
        self.handler_derivations.clear();
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
        self.indent -= 1;
        self.line("}");
        self.line("");
    }

    /// A derivation: its body, and the memoizing reader the state asks
    /// for the cached value or a recomputation.
    pub(crate) fn emit_derive(&mut self, d: &Derive) {
        let name = d.name.clone();
        let rt = crow_type(&d.ty);
        let ty = match &rt {
            CrowType::Void | CrowType::Unknown => "String".to_string(),
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
        for (v, (vt, dv)) in self.reactive_vars.clone() {
            self.line(&format!("let {} = {};", v, Self::state_getter(&v, &vt, &dv)));
            self.declare(&v, vt);
        }
        for other in Self::derivations_read(&d.body) {
            if other == name {
                continue;
            }
            let oty = self.derives.get(&other).map(|x| crow_type(&x.ty)).unwrap_or(CrowType::Unknown);
            self.line(&format!("let {} = __derive_{}(state);", other, other));
            self.declare(&other, oty);
        }
        for v in self.data_source_vars.clone() {
            self.line(&format!("let {} = __json_to_frame(state.get(\"{}\").unwrap_or(&serde_json::Value::Null));", v, v));
            self.declare(&v, CrowType::Frame);
        }
        let mut body = d.body.clone();
        if let Some(i) = body.iter().rposition(|s| !matches!(s, Stmt::Line(_))) {
            if let Stmt::Do(e) = &body[i] {
                body[i] = Stmt::Return(Some(e.clone()));
            }
        }
        self.emit_block(&body);
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
        let eq = if matches!(rt, CrowType::Int | CrowType::Float | CrowType::Str | CrowType::Bool) { "|a, b| a == b" } else { "|_a, _b| false" };
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
                CrowType::Result(inner) => format!(
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
        self.cur_ret = Some(CrowType::Float);
        self.returns_unit = false;
        self.cur_fn = f.name.clone();
        self.emitting_dual = true;
        self.mutated = Self::mutated_in(&f.body);
        for (p, _) in &f.params {
            self.declare(p, CrowType::Float);
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

    /// Before a handler calls a helper, the facts it stored so far land in
    /// the state, so the helper sees them.
    pub(crate) fn flush_before(&self) -> String {
        self.reactive_own_writes.iter().map(|v| format!("state.set(\"{}\", serde_json::json!({}));", v, v)).collect::<Vec<_>>().join(" ")
    }

    /// After a helper, the facts it (or anything it calls) stored are read
    /// back into the caller's locals.
    pub(crate) fn reload_after(&self, callee: &str) -> String {
        let Some(f) = self.fns.get(callee) else { return String::new() };
        let mut reach = self.reach(&f.body);
        reach.insert(callee.to_string());
        let mut written = BTreeSet::new();
        for n in &reach {
            if let Some(g) = self.fns.get(n) {
                written.extend(Self::facts_stored(&g.body));
            }
        }
        let mut out = String::new();
        for v in written {
            if let Some((vt, d)) = self.reactive_vars.get(&v) {
                if self.declared(&v) {
                    out.push_str(&format!("{} = {}; ", v, Self::state_getter(&v, vt, d)));
                }
            }
        }
        out
    }

    /// The derivations of the handler being printed that these statements
    /// read. Nothing inside a closure: the statement holding it read them.
    fn handler_reads(&self, exprs: &[&Expr]) -> Vec<String> {
        if self.reactive_epilogue.is_none() || self.handler_derivations.is_empty() || exprs.is_empty() || self.closure_depth > 0 {
            return Vec::new();
        }
        let mut reads = BTreeSet::new();
        for e in exprs {
            visit_expr(e, &mut |x| {
                if let Expr::Derived(d, _) = x {
                    reads.insert(d.clone());
                }
            });
        }
        reads.into_iter().filter(|d| self.handler_derivations.contains(d)).collect()
    }

    /// Inside a handler, before a statement that reads derivations: the
    /// facts it wrote land in the state, then each derivation is read again.
    fn refresh_derivations(&mut self, exprs: &[&Expr]) {
        let reads = self.handler_reads(exprs);
        if reads.is_empty() {
            return;
        }
        let flush = self.flush_before();
        if !flush.is_empty() {
            self.line(&flush);
        }
        for d in reads {
            self.line(&format!("{} = __derive_{}(state);", d, d));
        }
    }

    /// The expressions a statement evaluates itself.
    fn own_exprs(s: &Stmt) -> Vec<&Expr> {
        match s {
            Stmt::Let { expr, .. } | Stmt::Set { expr, .. } | Stmt::Store { expr, .. } | Stmt::Do(expr) => vec![expr],
            Stmt::Return(Some(e)) => vec![e],
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => vec![cond],
            Stmt::For { iter, .. } => vec![iter],
            Stmt::SetPath { path, expr, .. } => {
                let mut v: Vec<&Expr> = path.iter().filter_map(|p| if let PathStep::Index(i) = p { Some(i) } else { None }).collect();
                v.push(expr);
                v
            }
            _ => Vec::new(),
        }
    }

    /// What a decision on an error does here: a handler stops, a function
    /// with a value hands the error back, a void function returns, the top
    /// level halts. `v` names the failed value.
    pub(crate) fn refuse(&self, v: &str) -> String {
        self.refuse_at(v, Decision::Branch)
    }

    /// `refuse` for a decision of either kind: the top level's halt names
    /// it (`cannot loop over an error value`).
    pub(crate) fn refuse_at(&self, v: &str, at: Decision) -> String {
        if self.reactive_epilogue.is_some() {
            format!("{{ state.stop(format!(\"{} stopped: {{}}\", {v}.reason)); return; }}", self.shown_fn_name(), v = v)
        } else if self.returns_unit && !self.cur_fn.is_empty() {
            "return;".to_string()
        } else if let Some(ret) = self.cur_ret.clone().filter(|_| !self.cur_fn.is_empty()) {
            if self.view_mark || self.in_view() {
                return format!("return CrowVNode::text(&format!(\"error: {{}}\", {v}.reason));", v = v);
            }
            let inner_default = match &ret {
                CrowType::Result(inner) => inner.default_rust_value(),
                other => other.default_rust_value(),
            };
            let trace = self.trace_format_expr();
            if matches!(ret, CrowType::Result(_)) {
                format!(
                    "return CrowResult {{ ok: false, value: {}, reason: {v}.reason.clone(), kind: {v}.kind.clone(), trace: {{ let mut _t = {v}.trace.clone(); _t.push({}); _t }}, captured: 0.0 }};",
                    inner_default,
                    trace,
                    v = v
                )
            } else {
                // a function whose value is not a result cannot hand the error
                // back: the program halts with the reason, as a decision does
                format!("panic!(\"cannot {} an error value — {{}}\", {v}.reason);", at.verb(), v = v)
            }
        } else {
            format!(
                "{{ eprintln!(\"runtime error: line {}: cannot {} an error value — {{}}\", {v}.reason); std::process::exit(1); }}",
                self.current_line,
                at.verb(),
                v = v
            )
        }
    }

    /// A condition that is a result: an error refuses, a value decides.
    pub(crate) fn guard_condition(&mut self, cond: &Expr, cond_str: String) -> String {
        if !matches!(self.tt(cond), CrowType::Result(_)) {
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
        if !matches!(s, Stmt::While { .. }) {
            let exprs: Vec<Expr> = match s {
                Stmt::For { iter, body, .. } => {
                    let mut v = vec![iter.clone()];
                    visit_block(body, &mut |e| {
                        if let Expr::Derived(..) = e {
                            v.push(e.clone());
                        }
                    });
                    v
                }
                _ => Self::own_exprs(s).into_iter().cloned().collect(),
            };
            let refs: Vec<&Expr> = exprs.iter().collect();
            self.refresh_derivations(&refs);
        }
        match s {
            Stmt::Line(n) => self.current_line = *n,
            Stmt::Let { var, ty, expr } => self.emit_let(var, ty, expr),
            Stmt::Set { var, expr } => self.emit_set(var, expr),
            Stmt::Store { fact, expr } => {
                if self.declared(fact) {
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
                } else if !self.handler_reads(&[cond]).is_empty() {
                    self.line("loop {");
                    self.indent += 1;
                    self.refresh_derivations(&[cond]);
                    let c = self.expr(cond);
                    let c = self.guard_condition(cond, c);
                    self.line(&format!("if !({}) {{ break; }}", c));
                    self.indent -= 1;
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
                    if matches!(self.local_ty(v), Some(CrowType::Result(_))) {
                        let on_error = self.refuse_at("_c", *at);
                        self.line(&format!("{{ let _c = &{}; if !_c.ok {{ {} }} }}", v, on_error));
                    }
                }
            }
            Stmt::Stop { value } => {
                let who = self.shown_fn_name();
                self.line(&format!("{{ state.stop(format!(\"{} stopped: {{}}\", {}.reason)); return; }}", who, value));
            }
        }
    }

    fn emit_do(&mut self, e: &Expr) {
        // cpu[n] bounds the loops that follow; gpu[] and nodes[n] are the
        // program's resources
        if call_name(e).as_deref() == Some("cpu") {
            if let Some(Expr::Const(Const::Int(n), _)) = super::program::call_args(e).first().map(Emitter::raw) {
                self.cpu_count = Some(*n as usize);
                return;
            }
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
        let mut text = self.owned_expr(expr);
        let mut holds_markup = false;
        if self.in_view() && self.makes_node(expr) {
            text = format!("({}).to_html()", text);
            holds_markup = true;
        }
        let mut t = self.tt_full(expr);
        if holds_markup {
            t = CrowType::Markup;
        }
        if format!("{:?}", t).contains("Unknown") {
            let declared = crow_type(ty);
            // a first binding that cannot fail is not a result because a
            // later one can (`ts = []`, then `ts = push[ts, float[x]]`): the
            // later is a consumption point, and the variable holds values
            let declared = if matches!(t, CrowType::Result(_)) { declared } else { Self::inner(&declared) };
            if !format!("{:?}", declared).contains("Unknown") || matches!(t, CrowType::Unknown) {
                t = declared;
            }
        }
        if matches!(t, CrowType::Int) && matches!(crow_type(ty), CrowType::Float) {
            t = CrowType::Float;
            text = format!("({}) as f64", text);
        }
        let kw = if self.mutated.contains(var) { "let mut" } else { "let" };
        let in_live_callback = self.live_render_fns.contains(&self.cur_fn) || self.live_handle_fns.contains(&self.cur_fn) || self.views.contains(&self.cur_fn);
        if self.emitting_dual || in_live_callback || matches!(t, CrowType::Unknown) {
            self.line(&format!("{} {} = {};", kw, var, text));
        } else {
            self.line(&format!("{} {}: {} = {};", kw, var, t.to_rust_type(), text));
        }
        self.declare(var, t);
    }

    fn emit_set(&mut self, var: &str, expr: &Expr) {
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
            // a result assigned into a typed variable is a consumption point:
            // the value lands, an error stops a handler, returns from a
            // function, or renders in a view
            if let CrowType::Result(inner) = &new_ty {
                if existing.compatible(inner) || (matches!(existing, CrowType::Float) && matches!(inner.as_ref(), CrowType::Int)) {
                    let abort = if self.reactive_epilogue.is_some() {
                        format!("{{ state.stop(format!(\"{} stopped: {{}}\", _asg.reason)); return; }}", self.shown_fn_name())
                    } else if matches!(self.cur_ret, Some(CrowType::Result(_))) && !self.cur_fn.is_empty() {
                        "return CrowResult { ok: false, value: Default::default(), reason: _asg.reason.clone(), kind: _asg.kind.clone(), trace: _asg.trace.clone(), captured: 0.0 };".to_string()
                    } else if self.in_view() {
                        "return CrowVNode::text(&format!(\"error: {}\", _asg.reason));".to_string()
                    } else {
                        "return;".to_string()
                    };
                    let cast = if matches!(existing, CrowType::Float) && matches!(inner.as_ref(), CrowType::Int) { " as f64" } else { "" };
                    self.line(&format!("{{ let _asg = {}; if !_asg.ok {{ {} }} {} = _asg.value{}; }}", text, abort, var, cast));
                    return;
                }
            }
            if matches!(existing, CrowType::Float) && matches!(new_ty, CrowType::Int) {
                self.line(&format!("{} = ({}) as f64;", var, text));
                return;
            }
            if !existing.compatible(&new_ty) {
                self.fail(format!("cannot reassign '{}' from {} to {} — variable type cannot change", var, existing.crow_name(), new_ty.crow_name()));
            }
        }
        self.line(&format!("{} = {};", var, text));
    }

    /// `x = push[x, v]`: an in-place builtin whose collection is `x` itself.
    fn in_place_call_on(e: &Expr, var: &str) -> bool {
        match e {
            Expr::Call { target: Target::Runtime(path), args, .. } => {
                in_place_builtin(path) && matches!(args.first().map(Emitter::raw), Some(Expr::Move(v, _)) | Some(Expr::Local(v, _)) if v == var)
            }
            _ => false,
        }
    }

    fn emit_set_path(&mut self, var: &str, path: &[PathStep], expr: &Expr) {
        let val = self.expr(expr);
        // the object the last step writes into, and its type
        let mut obj = if self.static_handle_vars.contains(var) { format!("__crow_handle_{}.get().unwrap().clone()", var) } else { var.to_string() };
        let mut ty = self.local_ty(var).unwrap_or(CrowType::Unknown).text();
        for step in &path[..path.len().saturating_sub(1)] {
            match step {
                PathStep::Field(f) => {
                    let next = match &ty {
                        CrowType::Struct(s) => self.structs.get(s).and_then(|d| d.fields.iter().find(|(n, _)| n == f).map(|(_, t)| crow_type(t))).unwrap_or(CrowType::Unknown),
                        CrowType::Map(inner) => inner.as_ref().clone(),
                        _ => CrowType::Unknown,
                    };
                    obj = if matches!(ty, CrowType::Map(_)) {
                        format!("(*{}.get_mut(\"{}\").unwrap())", obj, f)
                    } else {
                        format!("{}.{}", obj, Self::rust_field(f))
                    };
                    ty = next;
                }
                PathStep::Index(i) => {
                    let idx = self.expr(i);
                    let next = match &ty {
                        CrowType::Array(inner) | CrowType::Map(inner) => inner.as_ref().clone(),
                        _ => CrowType::Unknown,
                    };
                    obj = if matches!(ty, CrowType::Map(_)) {
                        format!("(*{}.get_mut(&{}).unwrap())", obj, idx)
                    } else {
                        format!("{o}[{{ let _i = {i}; if _i < 0 {{ ({o}.len() as i64 + _i) as usize }} else {{ _i as usize }} }}]", o = obj, i = idx)
                    };
                    ty = next;
                }
            }
        }
        match path.last() {
            Some(PathStep::Field(f)) => {
                if matches!(ty, CrowType::Map(_)) {
                    self.line(&format!("{}.insert(\"{}\".to_string(), {});", obj, f, val));
                } else {
                    self.line(&format!("{}.{} = {};", obj, Self::rust_field(f), val));
                }
            }
            Some(PathStep::Index(i)) => {
                let idx = self.expr(i);
                match &ty {
                    CrowType::Map(_) => self.line(&format!("{}.insert({}, {});", obj, idx, val)),
                    CrowType::NumArray(_) => {
                        self.needs_crow_runtime = true;
                        self.line(&format!(
                            "{{ let _i = {}; let mut _tmp = {}.to_vec(); let _idx = if _i < 0 {{ (_tmp.len() as i64 + _i) as usize }} else {{ _i as usize }}; _tmp[_idx] = {} as f64; {} = crow_runtime::tensor::CrowTensor::from_vec(_tmp, {}.shape().to_vec()); }}",
                            idx, obj, val, obj, obj
                        ));
                    }
                    _ => self.line(&format!("{{ let _i = {}; let _idx = if _i < 0 {{ ({}.len() as i64 + _i) as usize }} else {{ _i as usize }}; {}[_idx] = {}; }}", idx, obj, obj, val)),
                }
            }
            None => self.line(&format!("{} = {};", obj, val)),
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
        if self.returns_unit {
            self.line(&format!("let _ = {};", text));
            self.line("return;");
        } else if let Some(CrowType::Result(inner)) = self.cur_ret.clone() {
            if !matches!(ety, CrowType::Result(_)) {
                self.line(&format!("return CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }};", text));
            } else if let CrowType::Result(got) = &ety {
                let known = |t: &CrowType| !matches!(t, CrowType::Unknown);
                if known(got) && known(&inner) && got.as_ref() != inner.as_ref() {
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

    fn emit_for(&mut self, var: &str, iter: &Expr, body: &[Stmt], parallel: bool) {
        let iter_str = self.expr(iter);
        let iter_ty = self.tt(iter);
        let is_tensor = matches!(iter_ty, CrowType::NumArray(_));
        let elem = match &iter_ty {
            CrowType::Array(inner) | CrowType::NumArray(inner) => inner.as_ref().clone(),
            _ => CrowType::Unknown,
        };
        let is_copy = Self::is_copy_type(&elem);
        // in parallel as the lowering decided, unless cpu[1] asked for one core
        let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
        let use_par = !forced_sequential && parallel;
        let actual = if is_tensor {
            if matches!(elem, CrowType::NumArray(_)) {
                format!("{}.rows()", iter_str)
            } else if matches!(elem, CrowType::Int) {
                format!("{}.to_vec().iter().map(|&v| v as i64).collect::<Vec<i64>>()", iter_str)
            } else {
                format!("{}.to_vec()", iter_str)
            }
        } else {
            iter_str
        };
        self.push_scope();
        if use_par {
            self.needs_rayon = true;
            let param = if is_copy { format!("|&{}|", var) } else { format!("|{}|", var) };
            self.line(&format!("{}.par_iter().for_each({} {{", actual, param));
            self.indent += 1;
            self.declare(var, elem);
            self.closure_depth += 1;
            self.emit_block(body);
            self.closure_depth -= 1;
            self.indent -= 1;
            self.line("});");
        } else {
            let param = if is_copy { format!("&{}", var) } else { var.to_string() };
            self.line(&format!("for {} in {}.iter() {{", param, actual));
            self.indent += 1;
            self.declare(var, elem);
            self.emit_block(body);
            self.indent -= 1;
            self.line("}");
        }
        self.pop_scope();
    }
}
