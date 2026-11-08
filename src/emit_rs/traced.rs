//! What carries a derivative in a function's traced copy, held in its
//! traced form (lower_ir/grad.rs; `ir::carries_derivative`): a number or a
//! tensor is a `crow_runtime::autograd::Traced` there, as every number and
//! tensor of the copy is; a list of them is a `Vec` of traced values; a
//! record holding them is its twin, `_Traced_<name>`, each field in its
//! traced form. A list or a record that carries no derivative keeps its
//! plain form, as outside. Where the two meet — a plain list given to a
//! name that holds a traced one, a record of constants built in the copy,
//! a twin handed back as a function's answer — two traits written with the
//! program (`_TracedForm`, `_PlainForm`) make one the other's form; each is
//! the identity on the form it makes, so a value already in it is never
//! copied. The interpreter holds a traced value in a list or a record as it
//! holds any value; compiled, a record of them had not built, and a list
//! could not hold one (parity 511, 512, 516).

use super::*;

impl Emitter {
    /// The records with a twin: a field holds a number, a tensor, a list of
    /// them or a record with a twin. Settled only where a function is
    /// traced.
    pub(crate) fn settle_traced_records(&mut self) {
        let mut twins: BTreeSet<String> = BTreeSet::new();
        loop {
            let before = twins.len();
            for (name, def) in &self.structs {
                if twins.contains(name) || matches!(name.as_str(), "CrowFindResult" | "CrowShell" | "CrowReply" | "CrowAnswer") {
                    continue;
                }
                if def.fields.iter().any(|(_, t)| Self::traced_form_in(&as_inferred(t), &twins)) {
                    twins.insert(name.clone());
                }
            }
            if twins.len() == before {
                break;
            }
        }
        self.traced_records = twins;
        // the copies whose answer is a list or a record in its traced form
        let mut answers = BTreeSet::new();
        for name in self.traced_functions.clone() {
            let (Some(f), Some(names)) = (self.fns.get(&name), self.part.active.get(&name)) else { continue };
            if self.held_traced(&as_inferred(&f.ret)) && Self::answers_derivative(f, names) {
                answers.insert(name);
            }
        }
        self.traced_answers = answers;
    }

    fn traced_form_in(t: &Ty, twins: &BTreeSet<String>) -> bool {
        match t {
            Ty::Float | Ty::Tensor(..) => true,
            Ty::Record(n) => twins.contains(n),
            // (a list at any depth: a list of lists of tensors had not
            // built compiled)
            Ty::List(i) => Self::traced_form_in(i, twins),
            _ => false,
        }
    }

    /// Whether a value of the type has a traced form other than its plain
    /// one.
    pub(crate) fn has_traced_form(&self, t: &Ty) -> bool {
        Self::traced_form_in(t, &self.traced_records)
    }

    /// Whether a value of the type is held in its traced form only where
    /// it carries a derivative: a list or a record (a number and a tensor
    /// are traced throughout the copy).
    pub(crate) fn held_traced(&self, t: &Ty) -> bool {
        matches!(t, Ty::List(_) | Ty::Record(_)) && self.has_traced_form(t)
    }

    /// Whether an expression of the traced copy being printed carries a
    /// derivative.
    pub(crate) fn carries(&self, e: &Expr) -> bool {
        self.emitting_traced && crate::ir::carries_derivative(e, &self.active_names)
    }

    /// The Rust type of a value's traced form.
    pub(crate) fn traced_rust_type(&self, t: &Ty) -> String {
        match t {
            Ty::Float | Ty::Tensor(..) => "crow_runtime::autograd::Traced".to_string(),
            Ty::Record(n) if self.traced_records.contains(n) => format!("_Traced_{}", n),
            Ty::List(i) if self.has_traced_form(t) => format!("Vec<{}>", self.traced_rust_type(i)),
            other => other.to_rust_type(),
        }
    }

    /// A value made its traced form: a plain one a constant of the tape, a
    /// traced one itself.
    pub(crate) fn traced_form(&self, s: &str, t: &Ty) -> String {
        if !self.has_traced_form(t) {
            return s.to_string();
        }
        format!("_TracedForm::<{}>::traced_form({})", self.traced_rust_type(t), s)
    }

    /// A value read back in its plain form: a traced one's numbers, a plain
    /// one itself.
    pub(crate) fn plain_form(&self, s: &str, t: &Ty) -> String {
        if !self.has_traced_form(t) {
            return s.to_string();
        }
        format!("_PlainForm::<{}>::plain_form({})", t.to_rust_type(), s)
    }

    /// The value a name of the traced copy is given, in the form the name
    /// holds: a list or a record that carries a derivative in its traced
    /// form, made from a plain one where the value carries none (`outs =
    /// []`, then `outs = push[outs, h]`).
    pub(crate) fn held_form(&self, var: &str, ty: &Ty, expr: &Expr, text: String) -> String {
        // (a number or a tensor is traced throughout the copy, whatever made
        // it: one read out of a record's list or made by a literal of
        // constants is plain where it is read, and is made a constant of the
        // tape where a name holds it, as one that can fail holds its payload
        // — `p = layer.floats[0]` then `p = 0.0`, and `wd = [2, 3, 4]` then
        // `wd = slice[wd, 1, len[wd]]`, had declared a plain value and given
        // it a traced one, and the program did not build; parity 515)
        if self.traced_payload(ty) {
            return self.traced_form(&text, ty);
        }
        if let Ty::Result(inner) = ty
            && self.traced_payload(inner)
        {
            return self.result_form(&text, inner);
        }
        if self.emitting_traced && self.active_names.contains(var) && self.held_traced(ty) && !self.carries(expr) {
            return self.traced_form(&text, ty);
        }
        text
    }

    /// A list in its traced form handed where a tensor is taken
    /// (`tensor[items]`): the tensor of its items on the tape, a list among
    /// them made the tensor of its own items first (`autograd::from_elements`,
    /// each item's gradient its slice).
    pub(crate) fn traced_elements(&mut self, s: &str, t: &Ty) -> String {
        self.needs_crow_runtime = true;
        let stop = self.or_stop();
        match t {
            Ty::List(item) if matches!(item.as_ref(), Ty::List(_)) => {
                let inner = self.traced_elements("_r", item);
                format!("crow_runtime::autograd::from_elements(&({}).iter().map(|_r| {}).collect::<Vec<_>>()){}", s, inner, stop)
            }
            _ => format!("crow_runtime::autograd::from_elements(&({})){}", s, stop),
        }
    }

    /// Whether a payload of the type is traced in the copy being printed: a
    /// number or a tensor is traced throughout a traced copy, the payload
    /// of a value that can fail included (`CrowResult<Traced>`) — a failed
    /// value's placeholder, the call that can fail, the operator beside one
    /// and the copy's own answer each had held a plain one beside a traced
    /// one, and the program had not built (parity 519).
    pub(crate) fn traced_payload(&self, t: &Ty) -> bool {
        self.emitting_traced && matches!(t, Ty::Float | Ty::Tensor(..))
    }

    /// What a failed value holds in its payload's place.
    pub(crate) fn payload_default(&self, t: &Ty) -> String {
        match t {
            _ if self.traced_payload(t) => "crow_runtime::autograd::Traced::number(0.0)".to_string(),
            // (a record with a twin, or a list that may hold traced values:
            // the placeholder of whichever form the value beside it holds,
            // which Rust reads from that value — `lstm_cell`'s state, where
            // an argument could fail, had been a plain record's beside the
            // twin the traced copy answers)
            Ty::Record(n) if self.emitting_traced && self.traced_records.contains(n) => "_Placeholder::placeholder()".to_string(),
            Ty::List(_) if self.emitting_traced && self.has_traced_form(t) => "Vec::new()".to_string(),
            Ty::Result(inner) if self.traced_payload(inner) => {
                format!("CrowResult {{ ok: false, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", self.payload_default(inner))
            }
            _ => t.default_rust_value(),
        }
    }

    /// A value held as the payload of one that can fail: traced where the
    /// copy traces it (a plain one a constant of the tape, a traced one
    /// itself, not copied).
    pub(crate) fn payload_form(&self, v: &str, t: &Ty) -> String {
        if self.traced_payload(t) {
            return format!("_TracedForm::<crow_runtime::autograd::Traced>::traced_form({})", v);
        }
        v.to_string()
    }

    /// A value that can fail, of payload `t`, in the form the copy holds it:
    /// its payload traced where the copy traces it.
    pub(crate) fn result_form(&self, v: &str, t: &Ty) -> String {
        if self.traced_payload(t) {
            return format!("_TracedForm::<CrowResult<crow_runtime::autograd::Traced>>::traced_form({})", v);
        }
        v.to_string()
    }

    /// Whether a traced function's answer carries a derivative: a return
    /// of one, or a last line that is one.
    fn answers_derivative(f: &Fn, names: &BTreeSet<String>) -> bool {
        fn returns(block: &[Stmt], names: &BTreeSet<String>) -> bool {
            block.iter().any(|s| match s {
                Stmt::Return(Some(e)) => crate::ir::carries_derivative(e, names),
                Stmt::If { then, els, .. } => returns(then, names) || els.as_ref().is_some_and(|e| returns(e, names)),
                Stmt::For { body, .. } | Stmt::While { body, .. } => returns(body, names),
                _ => false,
            })
        }
        let last = f.body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
        returns(&f.body, names) || matches!(last, Some(Stmt::Do(e)) if crate::ir::carries_derivative(e, names))
    }

    /// The traced forms' traits, with their forms of numbers, tensors and
    /// lists of them, and each record's twin: written once, where a function
    /// is traced.
    pub(crate) fn emit_traced_forms(&mut self) {
        self.needs_crow_runtime = true;
        let tr = "crow_runtime::autograd::Traced";
        let ct = "crow_runtime::tensor::CrowTensor";
        self.line("// the traced forms of a function's traced copy (emit_rs/traced.rs)");
        self.line("#[allow(dead_code, non_camel_case_types)]");
        self.line("trait _TracedForm<T> { fn traced_form(self) -> T; }");
        self.line("#[allow(dead_code, non_camel_case_types)]");
        self.line("trait _PlainForm<T> { fn plain_form(self) -> T; }");
        let same = |this: &mut Emitter, form: &str| {
            this.line(&format!("impl _TracedForm<{form}> for {form} {{ fn traced_form(self) -> {form} {{ self }} }}"));
        };
        let plain_same = |this: &mut Emitter, form: &str| {
            this.line(&format!("impl _PlainForm<{form}> for {form} {{ fn plain_form(self) -> {form} {{ self }} }}"));
        };
        // (a list's form is its items', at any depth; collected where the
        // items lie when the two forms are one)
        self.line("impl<A: _TracedForm<B>, B> _TracedForm<Vec<B>> for Vec<A> { fn traced_form(self) -> Vec<B> { self.into_iter().map(_TracedForm::<B>::traced_form).collect() } }");
        self.line("impl<A: _PlainForm<B>, B> _PlainForm<Vec<B>> for Vec<A> { fn plain_form(self) -> Vec<B> { self.into_iter().map(_PlainForm::<B>::plain_form).collect() } }");
        // (and a value that can fail's is its payload's, its failure kept)
        self.line("impl<A: Clone + _TracedForm<B>, B: Clone> _TracedForm<CrowResult<B>> for CrowResult<A> { fn traced_form(self) -> CrowResult<B> { CrowResult { ok: self.ok, value: self.value.traced_form(), reason: self.reason, kind: self.kind, trace: self.trace, captured: self.captured } } }");
        self.line("impl<A: Clone + _PlainForm<B>, B: Clone> _PlainForm<CrowResult<B>> for CrowResult<A> { fn plain_form(self) -> CrowResult<B> { CrowResult { ok: self.ok, value: self.value.plain_form(), reason: self.reason, kind: self.kind, trace: self.trace, captured: self.captured } } }");
        same(self, tr);
        self.line(&format!("impl _TracedForm<{tr}> for f64 {{ fn traced_form(self) -> {tr} {{ {tr}::number(self) }} }}"));
        self.line(&format!("impl _TracedForm<{tr}> for {ct} {{ fn traced_form(self) -> {tr} {{ {tr}::constant(self) }} }}"));
        for item in ["f64", ct] {
            plain_same(self, item);
        }
        self.line(&format!("impl _PlainForm<f64> for {tr} {{ fn plain_form(self) -> f64 {{ self.as_number().unwrap_or(f64::NAN) }} }}"));
        self.line(&format!("impl _PlainForm<{ct}> for {tr} {{ fn plain_form(self) -> {ct} {{ self.value }} }}"));
        // (a failed value's placeholder of a record in either form)
        self.line("#[allow(dead_code, non_camel_case_types)]");
        self.line("trait _Placeholder { fn placeholder() -> Self; }");
        for name in self.traced_records.clone() {
            let Some(def) = self.structs.get(&name).cloned() else { continue };
            let twin = format!("_Traced_{}", name);
            let fields: Vec<(String, Ty)> = def.fields.iter().map(|(f, t)| (Self::rust_field(f), as_inferred(t))).collect();
            self.line("#[allow(non_camel_case_types, dead_code)]");
            self.line("#[derive(Clone)]");
            self.line(&format!("struct {} {{", twin));
            for (f, t) in &fields {
                let rust = if self.has_traced_form(t) { self.traced_rust_type(t) } else { t.to_rust_type() };
                self.line(&format!("    {}: {},", f, rust));
            }
            self.line("}");
            let traced: Vec<String> = fields.iter().map(|(f, t)| format!("{}: {}", f, self.traced_form(&format!("self.{}", f), t))).collect();
            let plain: Vec<String> = fields.iter().map(|(f, t)| format!("{}: {}", f, self.plain_form(&format!("self.{}", f), t))).collect();
            same(self, &twin);
            plain_same(self, &name);
            self.line(&format!("impl _TracedForm<{twin}> for {name} {{ fn traced_form(self) -> {twin} {{ {twin} {{ {} }} }} }}", traced.join(", ")));
            self.line(&format!("impl _PlainForm<{name}> for {twin} {{ fn plain_form(self) -> {name} {{ {name} {{ {} }} }} }}", plain.join(", ")));
            self.line(&format!("impl _Placeholder for {name} {{ fn placeholder() -> Self {{ {name}::crow_default() }} }}"));
            self.line(&format!("impl _Placeholder for {twin} {{ fn placeholder() -> Self {{ _TracedForm::<{twin}>::traced_form({name}::crow_default()) }} }}"));
            // (a twin shows as its record does, its numbers read off the
            // tape: `log[state]` inside the copy had not built — and only
            // where its record shows: a record holding a function has no
            // display, and its twin's had not built, parity 518)
            if self.struct_kernel_displayable(&name) {
                self.line(&format!("impl crow_runtime::display::CrowDisplay for {twin} {{ fn crow_repr(&self) -> String {{ crow_runtime::display::CrowDisplay::crow_repr(&_PlainForm::<{name}>::plain_form(self.clone())) }} fn crow_fmt(&self) -> String {{ crow_runtime::display::CrowDisplay::crow_fmt(&_PlainForm::<{name}>::plain_form(self.clone())) }} }}"));
            }
        }
        self.line("");
    }
}
