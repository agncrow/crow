//! What a call site says of its callee's signature, and what inference
//! keeps around a body while its calls are read: the rules inference's walk
//! and the lowering's call scan share (F5, core-rewrite-notes §10).

use super::*;

/// What `enter_fn_scan` saved: restored by `leave_fn_scan`.
pub(crate) struct FnScan {
    saved_vars: BTreeMap<String, CrowType>,
    saved_fn_name: Option<String>,
    saved_in_view: bool,
    saved_assigned: Option<std::collections::HashSet<String>>,
}

/// What `enter_derive_scan` saved: restored by `leave_derive_scan`.
pub(crate) struct DeriveScan {
    saved_vars: BTreeMap<String, CrowType>,
    saved_fn_name: Option<String>,
    saved_assigned: Option<std::collections::HashSet<String>>,
}

impl TypeContext {
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
    pub(crate) fn note_call(&mut self, callee: &Expr, args: &[Expr], arg_types: &[CrowType]) {
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
                                    None if matches!(p.1, CrowType::Unknown) => p.1 = CrowType::Str,
                                    None => {}
                                }
                            }
                        }
                    }
                }
            }

            // Search builtins: the predicate's parameter is the
            // array's element type (any[arr, pred] etc.)
            if matches!(name.as_str(), "any" | "all" | "find") && args.len() == 2 {
                if let Expr::Ident(fn_name) = &args[1] {
                    let elem = match arg_types.first() {
                        Some(CrowType::Array(inner)) => Some(inner.as_ref().clone()),
                        Some(CrowType::NumArray(inner, _)) => Some(inner.as_ref().clone()),
                        _ => None,
                    };
                    if let Some(elem) = elem {
                        if let Some(sig) = self.functions.get_mut(fn_name) {
                            if let Some(p) = sig.params.first_mut() {
                                p.1 = p.1.unify(&elem);
                            }
                        }
                    }
                }
            }

            // later[f, args…] IS a call of f with those args — read it
            // as one so f's params unify and monomorphization emits it
            // even when later[] is its only call site.
            if name == "later" && !args.is_empty() {
                if let Expr::Ident(target) = &args[0] {
                    if self.functions.contains_key(target) {
                        let callee = Expr::Ident(target.clone());
                        self.note_call(&callee, &args[1..], arg_types.get(1..).unwrap_or(&[]));
                    }
                }
            }

            // db transact[conn, fn]: the callback's parameter is a
            // db connection handle
            if name == "transact" && args.len() == 2 {
                if let Expr::Ident(fn_name) = &args[1] {
                    if let Some(sig) = self.functions.get_mut(fn_name) {
                        if let Some(p) = sig.params.first_mut() {
                            p.1 = CrowType::DbTx;
                        }
                    }
                }
            }

            // Is it a function call?
            if self.functions.contains_key(name) {
                let sig = self.functions.get(name).unwrap().clone();
                let mut new_params = sig.params.clone();
                let mut normalized_sig: Vec<CrowType> = Vec::with_capacity(arg_types.len());
                for (i, at) in arg_types.iter().enumerate() {
                    // Normalize types for function params:
                    // - OptBoxStruct -> Struct (the Rust backend unwraps before passing)
                    // - Result<T> -> T (auto-propagation unwraps at call site)
                    // - an error with no value (error[…]) says nothing of the
                    //   parameter: that call never reaches the body
                    let normalized = match at {
                        CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                        CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Void) => CrowType::Unknown,
                        CrowType::Result(inner) => *inner.clone(),
                        other => other.clone(),
                    };
                    normalized_sig.push(normalized.clone());
                    if i < new_params.len() {
                        new_params[i].1 = new_params[i].1.unify(&normalized);
                    }
                }
                // F2: record this call's concrete signature for the
                // monomorphization pass (skip if anything is unknown)
                if normalized_sig.len() == sig.params.len() && normalized_sig.iter().all(Self::type_is_concrete) {
                    let entry = self.call_signatures.entry(name.clone()).or_default();
                    let dup = entry.iter().any(|existing| Self::same_signature(existing, &normalized_sig));
                    if !dup {
                        entry.push(normalized_sig);
                    }
                }
                self.functions.get_mut(name).unwrap().params = new_params;

                // Propagate types to functions passed as arguments:
                // If param type is Fn(...) and arg is a function name, update that function's signature
                let updated_sig = self.functions.get(name).unwrap().clone();
                for (i, arg) in args.iter().enumerate() {
                    if i < updated_sig.params.len() {
                        if let CrowType::Fn(param_types, ret_type) = &updated_sig.params[i].1 {
                            if let Expr::Ident(fn_name) = arg {
                                if self.functions.contains_key(fn_name) {
                                    let target_sig = self.functions.get(fn_name).unwrap().clone();
                                    let mut new_target_params = target_sig.params.clone();
                                    for (j, pt) in param_types.iter().enumerate() {
                                        if j < new_target_params.len() {
                                            new_target_params[j].1 = new_target_params[j].1.unify(pt);
                                        }
                                    }
                                    let new_ret = target_sig.return_type.unify(ret_type);
                                    let target = self.functions.get_mut(fn_name).unwrap();
                                    target.params = new_target_params;
                                    target.return_type = new_ret;
                                }
                            }
                        }
                    }
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
                if let Some(existing @ CrowType::Fn(_, _)) = self.vars.get(name) {
                    let fn_type = CrowType::Fn(arg_types.to_vec(), Box::new(CrowType::Unknown));
                    let unified = existing.unify(&fn_type);
                    self.vars.insert(name.clone(), unified);
                }
            }
        }
        // Module function/struct call: module.func[args] or module.Struct[args]
        if let Expr::FieldAccess { object, field } = callee {
            if let Expr::Ident(mod_name) = object.as_ref() {
                let prefixed = format!("{}_{}", mod_name, field);
                let text: Vec<CrowType> = arg_types.iter().map(|t| t.text()).collect();
                if self.functions.contains_key(&prefixed) {
                    let sig = self.functions.get(&prefixed).unwrap().clone();
                    let mut new_params = sig.params.clone();
                    for (i, at) in text.iter().enumerate() {
                        if i < new_params.len() {
                            let normalized = match at {
                                CrowType::OptBoxStruct(n) => CrowType::Struct(n.clone()),
                                other => other.clone(),
                            };
                            new_params[i].1 = new_params[i].1.unify(&normalized);
                        }
                    }
                    self.functions.get_mut(&prefixed).unwrap().params = new_params;
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

    /// A call through a loop's variable over a list of named functions
    /// (`for f in [a, b]`, then `f[x]`): each of them takes the arguments'
    /// types (read as text) when it takes as many.
    pub(crate) fn note_through_list(&mut self, f: &str, types: &[CrowType]) {
        if let Some(sig) = self.functions.get_mut(f) {
            if types.len() == sig.params.len() {
                for (p, t) in sig.params.iter_mut().zip(types) {
                    p.1 = p.1.unify(t);
                }
            }
        }
    }

    /// A loop's variable while its calls are read: an item of what it
    /// walks, a list that can fail refused first; left bound after.
    pub(crate) fn scan_loop_var(&mut self, var: &str, iter_type: CrowType) {
        let item = Self::loop_item(Self::walked(iter_type));
        if item != CrowType::Unknown {
            self.vars.insert(var.to_string(), item);
        }
    }

    /// An assignment while calls are read, once its value's calls are:
    /// the variable takes the value's type (a function's first assignment
    /// of a name binds it, spec §7).
    pub(crate) fn note_scanned_assign(&mut self, name: &str, ty: CrowType) {
        if self.fresh_binding(name) {
            self.vars.insert(name.to_string(), ty);
        } else {
            let existing = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
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
        FnScan { saved_vars, saved_fn_name, saved_in_view, saved_assigned }
    }

    /// Out of a function's body after its calls were read: reactive state
    /// it refined keeps the refinement, and a parameter called as a
    /// function keeps what its calls said of it.
    pub(crate) fn leave_fn_scan(&mut self, name: &str, scan: FnScan) {
        let FnScan { saved_vars, saved_fn_name, saved_in_view, saved_assigned } = scan;
        self.body_assigned = saved_assigned;
        // A module-level variable assigned in this body (reactive
        // state written by a handler) keeps the refined type: an
        // empty default learns its element type from the handler.
        // Only a ui program's facts: anywhere else a body's first
        // assignment is a local of its own (spec §7), and a local
        // named like a module variable says nothing about it.
        let module_vars: Vec<String> = saved_vars.keys().filter(|m| self.is_ui && self.facts.contains(*m)).cloned().collect();
        let mut refined: Vec<(String, CrowType)> = Vec::new();
        for m in &module_vars {
            if let (Some(before), Some(after)) = (saved_vars.get(m), self.vars.get(m)) {
                let open = matches!(before, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Unknown))
                    || matches!(before, CrowType::Map(inner) if matches!(inner.as_ref(), CrowType::Unknown))
                    || matches!(before, CrowType::Unknown);
                if open && before != after {
                    refined.push((m.clone(), before.unify(after)));
                }
            }
        }

        // After scanning body, propagate any updated Fn-typed param types back.
        // This handles `f[result, array[i]]` where `f` is a known function param.
        if let Some(sig) = self.functions.get(name).cloned() {
            let mut new_params = sig.params.clone();
            for (i, (pname, _pty)) in sig.params.iter().enumerate() {
                if let Some(updated_type) = self.vars.get(pname) {
                    if let CrowType::Fn(_, _) = updated_type {
                        new_params[i].1 = new_params[i].1.unify(updated_type);
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
