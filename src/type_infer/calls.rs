//! What a call site says of its callee's signature, and what inference
//! keeps around a body while its calls are read: the rules inference's walk
//! and the lowering's call scan share (F5, core-rewrite-notes §10).

use super::*;

/// What `enter_fn_scan` saved: restored by `leave_fn_scan`.
pub(crate) struct FnScan {
    saved_vars: BTreeMap<String, Ty>,
    saved_fn_name: Option<String>,
    saved_in_view: bool,
    saved_assigned: Option<std::collections::HashSet<String>>,
    saved_scope: &'static str,
}

/// What `enter_derive_scan` saved: restored by `leave_derive_scan`.
pub(crate) struct DeriveScan {
    saved_vars: BTreeMap<String, Ty>,
    saved_fn_name: Option<String>,
    saved_assigned: Option<std::collections::HashSet<String>>,
}

impl TypeContext {
    /// Whether a name is a function the function being read can call: one
    /// of the program's — or, inside a standard module's function written
    /// in Crow and inlined, one of the module's own (the inliner prefixes
    /// them; a row's is called bare and is the row's). The program's are
    /// invisible there, so a local of the module's named like one is its
    /// local (`lines[0]` in `sys.read_answer`, in a program with a `lines`).
    pub(crate) fn calls_fn(&self, name: &str) -> bool {
        self.functions.contains_key(name) && (crate::modules::is_crow_fn(name) || !self.current_fn_name.as_deref().is_some_and(crate::modules::is_crow_fn) || self.applying.iter().any(|f| f == name))
    }

    /// Whether a call's arguments' types say anything of its callee: a call
    /// by name, or a call of a function or record a module prefixes.
    pub(crate) fn call_says_of_callee(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Ident(_) => true,
            Expr::FieldAccess { object, field } => match object.as_ref() {
                Expr::Ident(m) => {
                    let prefixed = format!("{}_{}", m, field);
                    self.functions.contains_key(&prefixed) || self.structs.contains_key(&prefixed)
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// What a call says of its callee's signature, given its arguments'
    /// types with markup told apart: a function's parameters take them, a
    /// record's fields do, a handler a table or a search calls takes its
    /// item, a variable called is a function of them. The arguments' own
    /// calls are the caller's to read.
    pub(crate) fn note_call(&mut self, callee: &Expr, args: &[Expr], arg_types: &[Ty]) {
        if let Expr::Ident(name) = callee {
            // table[frame, click = h, key = 'id', actions = map[…]]: the
            // row's key cell is what every one of those handlers
            // receives, so their first parameter is the key column's type
            if name == "table" && args.len() == 2 {
                if let Expr::Map(pairs) = &args[0] {
                    let key_name = pairs.iter().find_map(|(k, v)| match (k, v) {
                        (Expr::Ident(s), Expr::Str(col)) | (Expr::Str(s), Expr::Str(col)) if s == "key" => Some(col.clone()),
                        _ => None,
                    });
                    let schema = arg_types.get(1).and_then(|t| t.frame_cols()).cloned().flatten();
                    let known = match (&key_name, &schema) {
                        (Some(col), Some(sc)) => sc.cols.get(col).cloned(),
                        (None, Some(sc)) => sc.cols.values().next().cloned(),
                        _ => None,
                    };
                    let mut handlers: Vec<String> = Vec::new();
                    for (k, v) in pairs {
                        let kname = match k {
                            Expr::Ident(s) | Expr::Str(s) => s.as_str(),
                            _ => continue,
                        };
                        match (kname, v) {
                            ("click", Expr::Ident(h)) => handlers.push(h.clone()),
                            ("actions", v) => {
                                if let Some(acts) = crate::lower::map_literal_pairs(v) {
                                    for (_, h) in acts {
                                        if let Expr::Ident(f) = &h {
                                            handlers.push(f.clone());
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // (the key column the frame's type knows is the handlers'
                    // parameter, over a guess an earlier round made before
                    // the frame's columns were known; unknown, a key reads as
                    // text)
                    for h in handlers {
                        if let Some(sig) = self.functions.get_mut(&h) {
                            if let Some(p) = sig.params.first_mut() {
                                match &known {
                                    Some(t) => p.1 = t.clone(),
                                    None if matches!(p.1, Ty::Unknown) => p.1 = Ty::Str,
                                    None => {}
                                }
                            }
                        }
                    }
                }
            }

            // a function handed where a row takes one (`Kind::FnOf`): its
            // parameters are what the row's signature declares — an `Item`
            // the walked argument's item (a predicate over a list that can
            // fail is handed its items; fuzz seed 1791429824), a request
            // the request, a connection the transaction's — and where the
            // signature takes the row's remaining arguments (`later[f, a]`
            // is a call of f[a]; `stream[feed, a]` of feed[put, a]) the
            // call is read as one, so f's parameters unify and its copies
            // are made though the row's call is its only call site
            self.hand_to_row(name, args, arg_types);
            // (an option the page calls as an event — a chart's seek — hands
            // its function what the option's signature says)
            for (f, sig) in crate::lower::event_handlers(callee, args) {
                let given: Vec<Ty> = sig.params.iter().map(|k| k.ty()).collect();
                if let Some(fsig) = self.functions.get_mut(f) {
                    for (p, t) in fsig.params.iter_mut().zip(&given) {
                        p.1 = p.1.unify(t);
                    }
                }
            }

            // Is it a function call? (of a function the caller can call:
            // inside a standard module's function, inlined, a program's
            // function is invisible — a local of the module's named like
            // one is indexed, not called)
            if self.calls_fn(name) {
                let sig = self.functions.get(name).unwrap().clone();
                let mut new_params = sig.params.clone();
                let mut normalized_sig: Vec<Ty> = Vec::with_capacity(arg_types.len());
                for (i, at) in arg_types.iter().enumerate() {
                    // Normalize types for function params:
                    // - Result<T> -> T (auto-propagation unwraps at call site)
                    // - an error with no value (error[…]) says nothing of the
                    //   parameter: that call never reaches the body
                    // (a record or none stays one: the parameter holds none
                    // as the interpreter's does, and a field of none stops
                    // where it is read — unwrapped at the call, the
                    // compiled program had stopped before the body's own
                    // `type[t] eq 'int'` could tell none)
                    let normalized = match at {
                        Ty::Result(inner) if matches!(inner.as_ref(), Ty::Unit) => Ty::Unknown,
                        Ty::Result(inner) => *inner.clone(),
                        other => other.clone(),
                    };
                    normalized_sig.push(normalized.clone());
                    if i < new_params.len() {
                        new_params[i].1 = new_params[i].1.unify(&normalized);
                    }
                }
                // F2: record this call's concrete signature for the
                // monomorphization pass (skip if anything is unknown)
                if normalized_sig.len() == sig.params.len() && normalized_sig.iter().all(|t| Self::type_is_concrete(t) || Self::empty_collection(t)) {
                    self.note_signature(name, normalized_sig);
                }
                self.functions.get_mut(name).unwrap().params = new_params;
                self.hand_functions(name, args);
            }

            // a builtin written in Crow (a row of the table whose body is
            // a function of the standard library, inlined): its call is a
            // call of that function with the row's parameters — its
            // contract — however the arguments came
            if !self.functions.contains_key(name) && !self.structs.contains_key(name) {
                // (every module in scope that answers the name: the row the
                // kinds pick is the one noted)
                for m in self.registry.providers_of(name).into_iter().map(str::to_string).collect::<Vec<_>>() {
                    self.note_crow_row(&m, name, arg_types);
                }
                // (a core builtin written in Crow: `request`)
                if crate::table::has_core_rows(name) {
                    self.note_crow_row("", name, arg_types);
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

            // A variable holding a function (`op = double`, then `op[5]`):
            // its calls type its parameters. A variable of any other type
            // is indexed, and a name nothing binds (a builtin, an element)
            // is no variable — guessing a function from its call made a
            // builtin one, and a fact not typed yet one a view indexed.
            if !self.functions.contains_key(name) && !self.structs.contains_key(name) {
                if let Some(existing @ Ty::Fn(params, _)) = self.vars.get(name) {
                    // (a call of another count types the parameters it gives:
                    // its count is refused where it is written, and that is
                    // the one mistake — nothing typed, the function handed as
                    // a value had been refused too)
                    let given: Vec<Ty> = (0..params.len()).map(|i| arg_types.get(i).cloned().unwrap_or(Ty::Unknown)).collect();
                    let fn_type = Ty::Fn(given, Box::new(Ty::Unknown));
                    let unified = existing.unify(&fn_type);
                    self.vars.insert(name.clone(), unified);
                }
            }
        }
        // a builtin written in Crow called through its module's name
        // (`math.nn.relu[…]`)
        if let Some((m, f)) = Self::module_path(callee) {
            let real = self.module_aliases.get(&m).cloned().unwrap_or(m);
            self.note_crow_row(&real, &f, arg_types);
        }
        // Module function/struct call: module.func[args] or module.Struct[args]
        if let Expr::FieldAccess { object, field } = callee {
            if let Expr::Ident(mod_name) = object.as_ref() {
                let prefixed = format!("{}_{}", mod_name, field);
                let text: Vec<Ty> = arg_types.iter().map(|t| t.text()).collect();
                if self.functions.contains_key(&prefixed) {
                    let sig = self.functions.get(&prefixed).unwrap().clone();
                    let mut new_params = sig.params.clone();
                    for (i, at) in text.iter().enumerate() {
                        if i < new_params.len() {
                            new_params[i].1 = new_params[i].1.unify(at);
                        }
                    }
                    self.functions.get_mut(&prefixed).unwrap().params = new_params;
                    self.hand_functions(&prefixed, args);
                }
                // Module struct constructor: update field types from args
                if self.structs.contains_key(&prefixed) {
                    let sig = self.structs.get(&prefixed).unwrap().clone();
                    let mut new_fields = sig.fields.clone();
                    for (i, at) in text.iter().enumerate() {
                        if i < new_fields.len() {
                            new_fields[i].1 = new_fields[i].1.unify(at);
                        }
                    }
                    self.structs.get_mut(&prefixed).unwrap().fields = new_fields;
                }
            }
        }
    }

    /// A call's module path and name, where the callee is qualified:
    /// `m.f` and `a.b.f` (`math.nn.relu`).
    pub(crate) fn module_path(callee: &Expr) -> Option<(String, String)> {
        let Expr::FieldAccess { object, field } = callee else { return None };
        match object.as_ref() {
            Expr::Ident(m) => Some((m.clone(), field.clone())),
            Expr::FieldAccess { object: inner, field: sub } => match inner.as_ref() {
                Expr::Ident(m) => Some((format!("{}.{}", m, sub), field.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    /// A call of a builtin written in Crow (crate::table, `crow!`): the
    /// row its arguments' kinds pick names the program's function its body
    /// is inlined as, whose parameters take the row's kinds — one
    /// signature, whatever the program hands it — so the body is typed
    /// once, as the module's tests type it.
    pub(crate) fn note_crow_row(&mut self, module: &str, name: &str, arg_types: &[Ty]) {
        let kinds: Vec<crate::table::Arg> = arg_types
            .iter()
            .map(|t| match t {
                Ty::Result(inner) => inner.as_ref(),
                other => other,
            })
            .map(crate::table::Arg::of_type)
            .collect();
        let Some(row) = crate::table::crow_row(module, name, &kinds) else { return };
        let f = crate::table::crow_fn_name(row);
        let Some(sig) = self.functions.get(&f).cloned() else { return };
        let given: Vec<Ty> = row.params.iter().map(|k| crate::table::crow_param_type(*k)).collect();
        let mut params = sig.params.clone();
        for (i, t) in given.iter().enumerate() {
            if let Some(p) = params.get_mut(i) {
                p.1 = p.1.unify(t);
            }
        }
        if given.len() == params.len() && given.iter().all(Self::type_is_concrete) {
            let entry = self.call_signatures.entry(f.clone()).or_default();
            if !entry.iter().any(|existing| Self::same_signature(existing, &given)) {
                entry.push(given);
            }
        }
        if let Some(sig) = self.functions.get_mut(&f) {
            sig.params = params;
        }
    }

    /// A function handed as a value takes the types the parameter it is
    /// handed to is applied with (`apply[double, 5]` types `double` by
    /// `f[x]` inside `apply`), and a variable holding a function handed on
    /// (`relay[f, x]` passing its `f` to `apply`) takes them too, so the
    /// function behind it is typed however far it is handed. A module's
    /// function called qualified hands its functions the same way. (A
    /// function handed to `crossval.crossval[…]` had stayed untyped, and
    /// the compiled program left it out; one handed on had been read as
    /// text.)
    /// A function handed where a row takes one, typed by the row's
    /// signature (`crate::table::Sig`, the one declaration): see note_call.
    fn hand_to_row(&mut self, name: &str, args: &[Expr], arg_types: &[Ty]) {
        for (i, sig) in crate::table::fn_params(name, args.len()) {
            let Some(Expr::Ident(f)) = args.get(i) else { continue };
            if !self.functions.contains_key(f) || !self.calls_fn(f) {
                continue;
            }
            let walked = crate::table::walked_param(name, args.len()).and_then(|w| arg_types.get(w)).map(|t| t.payload().clone());
            let resolved: Vec<Ty> = sig
                .params
                .iter()
                .map(|k| match k {
                    crate::table::Kind::Item => match &walked {
                        Some(Ty::List(inner)) => inner.as_ref().clone(),
                        Some(Ty::Tensor(inner, dims)) => Self::tensor_item(inner, dims),
                        Some(Ty::Column(cell)) => cell.as_ref().clone(),
                        _ => Ty::Unknown,
                    },
                    k => k.ty(),
                })
                .collect();
            if sig.rest && sig.params.is_empty() {
                // the row's remaining arguments are the function's, as a
                // call of it hands them
                let callee = Expr::Ident(f.clone());
                self.note_call(&callee, &args[i + 1..], arg_types.get(i + 1..).unwrap_or(&[]));
                continue;
            }
            let Some(fsig) = self.functions.get(f).cloned() else { continue };
            let mut given: Vec<Ty> = resolved.clone();
            if sig.rest {
                given.extend(arg_types.get(i + 1..).unwrap_or(&[]).iter().map(|t| match t {
                    Ty::Result(inner) if matches!(inner.as_ref(), Ty::Unit) => Ty::Unknown,
                    Ty::Result(inner) => *inner.clone(),
                    other => other.clone(),
                }));
            }
            let mut params = fsig.params.clone();
            for (j, (p, t)) in params.iter_mut().zip(&given).enumerate() {
                // what the row fills itself (a feed's put) is the row's,
                // whatever an earlier round guessed; the rest unify
                if j < sig.params.len() && !matches!(t, Ty::Unknown) {
                    p.1 = t.clone();
                } else if !matches!(t, Ty::Unknown) {
                    p.1 = p.1.unify(t);
                }
            }
            if sig.rest && given.len() == params.len() && given.iter().all(Self::type_is_concrete) {
                let entry = self.call_signatures.entry(f.clone()).or_default();
                if !entry.iter().any(|existing| Self::same_signature(existing, &given)) {
                    entry.push(given);
                }
            }
            if let Some(s) = self.functions.get_mut(f) {
                s.params = params;
            }
        }
    }

    /// A function called by name — `apply`'s, or a group's member where
    /// the group was handed — noted as called with the kinds the call
    /// hands it: its parameters unify with them and the signature is kept
    /// for the copies. Wherever the call is lowered: a standard module's
    /// function applies the program's functions it was handed, which it
    /// cannot name (`calls_fn`), so the note does not go through
    /// `note_call`'s scope rule.
    pub(crate) fn note_named_call(&mut self, f: &str, types: &[Ty]) {
        let Some(sig) = self.functions.get(f).cloned() else { return };
        let mut params = sig.params.clone();
        for (p, t) in params.iter_mut().zip(types) {
            p.1 = p.1.unify(t);
        }
        if types.len() == params.len() && types.iter().all(|t| Self::type_is_concrete(t) || Self::empty_collection(t)) {
            self.note_signature(f, types.to_vec());
        }
        if let Some(s) = self.functions.get_mut(f) {
            s.params = params;
        }
    }

    pub(crate) fn hand_functions(&mut self, callee: &str, args: &[Expr]) {
        let handed_to = callee;
        let Some(sig) = self.functions.get(callee).cloned() else { return };
        for (i, arg) in args.iter().enumerate() {
            let Some((_, Ty::Fn(param_types, ret_type))) = sig.params.get(i) else { continue };
            // a gradient handed as a value (`descend[f, grad[f], x0]`): the
            // parameter takes the gradient's type — its answer above all, a
            // tensor or a float, which the parameter's application reads —
            // and the function the gradient is of takes the types the
            // parameter is applied with (compiled, the parameter's answer
            // had been read as text, and the program did not build)
            if let Expr::Call { callee, args: inner } = arg {
                if let (Expr::Ident(g), [Expr::Ident(f)]) = (callee.as_ref(), inner.as_slice()) {
                    if g == "grad" {
                        if let Some(target) = self.functions.get(f).cloned() {
                            let mut params = target.params.clone();
                            // (a parameter the seek only guessed a float takes
                            // the applied type outright, as a direct call of
                            // the gradient gives it)
                            let guessed = self.grad_forced.contains(f);
                            for (j, pt) in param_types.iter().enumerate() {
                                if j < params.len() && !matches!(pt, Ty::Unknown) {
                                    params[j].1 = if guessed { pt.clone() } else { params[j].1.unify(pt) };
                                }
                            }
                            if let Some(t) = self.functions.get_mut(f) {
                                t.params = params;
                            }
                            // (the application says what the function is
                            // of — numbers or tensors — as a direct call of
                            // the gradient would)
                            if !param_types.is_empty() && param_types.iter().all(|t| !matches!(t, Ty::Unknown)) {
                                self.grad_forced.remove(f);
                            }
                        }
                        if let Some(handed) = self.gradient_type(f) {
                            let (param_types, ret_type) = (param_types.clone(), ret_type.clone());
                            let merged = Ty::Fn(param_types, ret_type).unify(&handed);
                            if let Some(c) = self.functions.get_mut(handed_to)
                                && let Some(p) = c.params.get_mut(i)
                            {
                                p.1 = p.1.unify(&merged);
                            }
                        }
                        continue;
                    }
                }
            }
            let Expr::Ident(n) = arg else { continue };
            // what the function knows of itself flows into the parameter
            // too — its answer above all, so a model a handed `fit` answers
            // is typed where the parameter is applied, and the `score` it
            // is then handed to knows its model (settled over the rounds)
            let merged = if let Some(target) = self.functions.get(n).cloned() {
                let mut params = target.params.clone();
                for (j, pt) in param_types.iter().enumerate() {
                    if j < params.len() {
                        params[j].1 = params[j].1.unify(pt);
                    }
                }
                let ret = target.return_type.unify(ret_type);
                if let Some(t) = self.functions.get_mut(n) {
                    t.params = params.clone();
                    t.return_type = ret.clone();
                }
                Ty::Fn(params.into_iter().map(|(_, t)| t).collect(), Box::new(ret))
            } else if let Some(existing @ Ty::Fn(..)) = self.vars.get(n).cloned() {
                let handed = Ty::Fn(param_types.clone(), ret_type.clone());
                let merged = existing.unify(&handed);
                self.vars.insert(n.clone(), merged.clone());
                merged
            } else {
                continue;
            };
            if let Some(c) = self.functions.get_mut(callee)
                && let Some(p) = c.params.get_mut(i)
            {
                p.1 = p.1.unify(&merged);
            }
        }
    }

    /// A call through a loop's variable over a list of named functions
    /// (`for f in [a, b]`, then `f[x]`): each of them takes the arguments'
    /// types (read as text) when it takes as many.
    pub(crate) fn note_through_list(&mut self, f: &str, types: &[Ty]) {
        if let Some(sig) = self.functions.get_mut(f) {
            if types.len() == sig.params.len() {
                for (p, t) in sig.params.iter_mut().zip(types) {
                    p.1 = p.1.unify(t);
                }
            }
        }
    }

    /// A loop's variable while its calls are read: an item of what it
    /// walks, a list that can fail refused first; left bound after. A
    /// binding of its own even where the item is not known yet: it hides a
    /// name bound outside the loop (a module's `for t in tools`, inlined
    /// into a program whose top level holds a `t` that can fail, had read
    /// the program's `t`, and the interpolation of each tool was typed as
    /// one that can fail).
    pub(crate) fn scan_loop_var(&mut self, var: &str, iter_type: Ty) {
        let item = Self::loop_item(Self::walked(iter_type));
        self.vars.insert(var.to_string(), item);
    }

    /// An assignment while calls are read, once its value's calls are:
    /// the variable takes the value's type (a function's first assignment
    /// of a name binds it, spec §7).
    pub(crate) fn note_scanned_assign(&mut self, name: &str, ty: Ty) {
        if self.fresh_binding(name) {
            self.vars.insert(name.to_string(), ty);
        } else {
            let existing = self.vars.get(name).cloned().unwrap_or(Ty::Unknown);
            self.vars.insert(name.to_string(), settle::settle(&existing, ty, self.body_assigned.is_some()));
        }
    }

    /// After an assignment's value's calls are read: a connection it
    /// makes.
    pub(crate) fn note_scanned_value(&mut self, name: &str, value: &Expr) {
        self.note_connection(name, value);
    }

    /// Into a function's body while its calls are read: its parameters
    /// typed from its signature and bound (a frame's with its columns), and
    /// inside a view an element is markup.
    pub(crate) fn enter_fn_scan(&mut self, name: &str) -> FnScan {
        let saved_vars = self.vars.clone();
        let saved_fn_name = self.current_fn_name.replace(name.to_string());
        // inside a view an element is markup, at the calls it makes too
        let saved_in_view = std::mem::replace(&mut self.in_view, self.views.contains(name));
        let mut assigned = std::collections::HashSet::new();
        if let Some(sig) = self.functions.get(name) {
            for (pname, pty) in &sig.params.clone() {
                self.vars.insert(pname.clone(), pty.clone());
                assigned.insert(pname.clone());
            }
        }
        let saved_assigned = self.body_assigned.replace(assigned);
        let saved_scope = self.registry.enter(name);
        FnScan { saved_vars, saved_fn_name, saved_in_view, saved_assigned, saved_scope }
    }

    /// Out of a function's body after its calls were read: reactive state
    /// it refined keeps the refinement, and a parameter called as a
    /// function keeps what its calls said of it.
    pub(crate) fn leave_fn_scan(&mut self, name: &str, scan: FnScan) {
        let FnScan { saved_vars, saved_fn_name, saved_in_view, saved_assigned, saved_scope } = scan;
        self.registry.resume(saved_scope);
        self.body_assigned = saved_assigned;
        // A module-level variable assigned in this body (reactive
        // state written by a handler) keeps the refined type: an
        // empty default learns its element type from the handler.
        // Only a ui program's facts: anywhere else a body's first
        // assignment is a local of its own (spec §7), and a local
        // named like a module variable says nothing about it.
        let module_vars: Vec<String> = saved_vars.keys().filter(|m| self.is_ui && self.facts.contains(*m)).cloned().collect();
        let mut refined: Vec<(String, Ty)> = Vec::new();
        for m in &module_vars {
            if let (Some(before), Some(after)) = (saved_vars.get(m), self.vars.get(m)) {
                let open = matches!(before, Ty::List(inner) if matches!(inner.as_ref(), Ty::Unknown))
                    || matches!(before, Ty::Map(_, inner) if matches!(inner.as_ref(), Ty::Unknown))
                    || matches!(before, Ty::Unknown);
                if open && before != after {
                    refined.push((m.clone(), before.unify(after)));
                }
            }
        }

        // After scanning body, propagate any updated Fn-typed param types back.
        // This handles `f[result, array[i]]` where `f` is a known function param.
        // An open parameter the body settled — a list handed empty and
        // pushed onto, which took its items' kind — is the signature's
        // too, so every caller's empty list is a list of that kind.
        if let Some(sig) = self.functions.get(name).cloned() {
            let mut new_params = sig.params.clone();
            for (i, (pname, pty)) in sig.params.iter().enumerate() {
                if let Some(updated_type) = self.vars.get(pname) {
                    let open = matches!(pty, Ty::Unknown)
                        || matches!(pty, Ty::List(inner) if matches!(inner.as_ref(), Ty::Unknown))
                        || matches!(pty, Ty::Map(_, inner) if matches!(inner.as_ref(), Ty::Unknown));
                    // (a parameter reassigned from a value that can fail —
                    // `words = maybe[1]` — keeps the kind its calls hand: the
                    // payload, never a Result, since a failed argument never
                    // enters a function (spec, Auto-Propagation) and the
                    // assignment propagates the failure. Typed a Result by
                    // its body before a call typed it, the parameter took a
                    // call's plain argument bare and the compiled program did
                    // not build, where a literal argument had typed it first
                    // and the same assignment propagated — the type had
                    // depended on the order; parity 278)
                    let updated = match updated_type {
                        Ty::Result(inner) => inner.as_ref().clone(),
                        t => t.clone(),
                    };
                    if matches!(updated, Ty::Fn(_, _)) || (open && &updated != pty && !matches!(updated, Ty::Unknown)) {
                        new_params[i].1 = new_params[i].1.unify(&updated);
                    }
                }
            }
            self.functions.get_mut(name).unwrap().params = new_params;
        }

        self.current_fn_name = saved_fn_name;
        self.in_view = saved_in_view;
        self.vars = saved_vars;
        for (m, t) in refined {
            self.vars.insert(m, t);
        }
    }

    /// Into a derivation's body while its calls are read: it is read like
    /// a parameterless function's, so the views and helpers it calls learn
    /// their parameters from it, as they would from a handler's.
    pub(crate) fn enter_derive_scan(&mut self, name: &str) -> DeriveScan {
        let saved_vars = self.vars.clone();
        let saved_fn_name = self.current_fn_name.replace(name.to_string());
        let saved_assigned = self.body_assigned.replace(std::collections::HashSet::new());
        DeriveScan { saved_vars, saved_fn_name, saved_assigned }
    }

    /// Out of a derivation's body after its calls were read (the frame it
    /// returns keeps its columns in the derivation's type).
    pub(crate) fn leave_derive_scan(&mut self, scan: DeriveScan) {
        let DeriveScan { saved_vars, saved_fn_name, saved_assigned } = scan;
        self.body_assigned = saved_assigned;
        self.current_fn_name = saved_fn_name;
        self.vars = saved_vars;
    }
}
