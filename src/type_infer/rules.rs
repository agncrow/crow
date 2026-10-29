//! The typing rules: a node's type from its children's, shared by
//! inference and the lowering (core-rewrite-notes §10, the fold's second step).

use super::*;

impl TypeContext {
    /// The call's type given its arguments' types, inferred once by the
    /// caller: a nested chain (a map literal is set[set[set[map[], …]]]
    /// after lowering) is walked once per level, not twice — twice per
    /// level doubled the work for every entry, and a 30-entry literal
    /// never finished.
    pub fn infer_call_base_typed(&self, name: &str, args: &[Expr], _callee: &Expr, arg_types: &[Ty], full: &[Ty]) -> Ty {
        let arg_ty = |a: &Expr| -> Ty {
            match args.iter().position(|x| std::ptr::eq(x, a)) {
                Some(i) => arg_types.get(i).cloned().unwrap_or(Ty::Unknown),
                None => Ty::Unknown,
            }
        };
        // a body's own list, map or text is indexed, whatever a module
        // another file imports names the same — the environment first, as
        // both backends look (a part's `rsi[jg]` where the program imports
        // chart)
        if self.body_assigned.as_ref().is_some_and(|a| a.contains(name)) {
            // (a local not typed yet is still not a module's function)
            let var_type = self.vars.get(name).cloned().unwrap_or(Ty::Unknown);
            return self.index_type(var_type, arg_types.first().cloned());
        }
        // Struct constructor
        if self.structs.contains_key(name) {
            return Ty::Record(name.to_string());
        }
        // Inside a view an element, a chart, an icon or a bound input is
        // markup (a `select` of two arguments is the bound input there, as
        // both backends read it)
        if self.in_view
            && (crate::lower::UI_ELEMENTS.contains(&name)
                || matches!(name, "chart" | "icon" | "progress" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                || (name == "select" && args.len() == 2))
        {
            return Ty::Markup;
        }
        // A view's tree is markup at every boundary a type can see — the
        // interpreter's views return strings, and compiled ones render to
        // one wherever a string is expected (a list, a module argument) —
        // and markup, never text, where it is placed as a child.
        if self.views.contains(name) && self.calls_fn(name) {
            return Ty::Markup;
        }
        // User-defined function call (specialization-aware: a monomorphized
        // callee's return type depends on this call's argument types); a
        // program's function is invisible inside a standard module's (sys's
        // `answer[…]` is its row, whatever a program's `fn answer` answers)
        if let Some(sig) = self.functions.get(name).filter(|_| self.calls_fn(name)) {
            if let Some(mangled) = self.specialization_for(name, full) {
                if let Some(clone_sig) = self.functions.get(&mangled) {
                    return clone_sig.return_type.clone();
                }
            }
            // a function called at several signatures and not copied per
            // signature yet answers what its copy will: nothing known until
            // then (its one signature's answer, a column, had typed the
            // parameter of a function handed `may[1, …]`, and that guess
            // stayed: the compiled program handed an int where a column
            // was taken)
            if self.awaits_clones(name) {
                return Ty::Unknown;
            }
            return sig.return_type.clone();
        }
        // a builtin with a row is typed by its row, the one place that says
        // what it answers and whether it can fail (crate::table): the row
        // its arguments' kinds pick, as both backends pick it
        let kinds: Vec<crate::table::Arg> = args.iter().map(|a| crate::table::Arg::of_type(&arg_ty(a))).collect();
        // a form that declares its answer (`Prop::Answers`) likewise; one
        // answering its walked argument's item (`find`) answers that item
        if let Some(row) = crate::table::form_answer(name, &kinds) {
            let t = match row.ret {
                crate::table::Kind::Item => {
                    let walked = crate::table::walked_param(name, args.len()).and_then(|i| args.get(i)).map(|a| arg_ty(a)).unwrap_or(Ty::Unknown);
                    match walked.payload() {
                        Ty::List(inner) => inner.as_ref().clone(),
                        Ty::Tensor(inner, dims) => Self::tensor_item(inner, dims),
                        Ty::Column(cell) => cell.as_ref().clone(),
                        _ => Ty::Unknown,
                    }
                }
                // (a record of any kind: the row's record argument's — and a
                // revision answers what its function answers, a failure
                // where the function's answer can fail)
                crate::table::Kind::Record => {
                    let held = row.ty_given(&args.iter().map(|a| arg_ty(a)).collect::<Vec<_>>());
                    let fails = crate::table::fn_params(name, args.len()).iter().any(|(i, sig)| {
                        sig.ret == crate::table::Kind::Record && matches!(args.get(*i), Some(Expr::Ident(f)) if self.functions.get(f).is_some_and(|s| s.return_type.is_result()))
                    });
                    return if fails { Ty::Result(Box::new(held.payload().clone())) } else { held };
                }
                k => k.ty(),
            };
            return match row.fails {
                crate::table::Fails::Value(_) => Ty::Result(Box::new(t)),
                _ => t,
            };
        }
        if let Some(row) = crate::table::core_row(name, &kinds) {
            // a collection's answer is what it was given — a list of its
            // items, a tensor of its numbers — which its rule below reads
            // from the arguments
            let given = row.ret.generic() || (row.ret == crate::table::Kind::Tensor && row.params.first() == Some(&crate::table::Kind::Tensor));
            if !given {
                // an argument whose kind shows only as the program runs
                // picks the row then: the call's type is what every row of
                // the name answers, or not known (the first row's answer
                // had been taken — `abs` an int — and a float came)
                if kinds.iter().any(|k| matches!(k, crate::table::Arg::Unknown | crate::table::Arg::AnyList)) {
                    let answers: Vec<Ty> = crate::table::ROWS
                        .iter()
                        .filter(|r| r.module.is_empty() && r.name == name && crate::table::kinds_of(r.params, args.len()).is_some())
                        .map(|r| r.ty())
                        .collect();
                    if answers.iter().any(|t| *t != answers[0]) {
                        return Ty::Unknown;
                    }
                }
                return row.ty();
            }
        }
        // Module function dispatch (overlapping builtins: check arg type)
        let is_overlapping = crate::table::CORE_AND_MATH.contains(&name);
        if is_overlapping && self.registry.has_function(name) {
            // (by what each argument holds: one that can fail is handed on
            // as its error before the call, and the call is wrapped as it
            // can fail where it is typed — `max[w]` of a tensor that can
            // fail had been typed as nothing known, and the compiled program
            // did not build, the fuzzer's seed 1792144449)
            let arg_types: Vec<Ty> = args.iter().map(|a| arg_ty(a).payload().clone()).collect();
            // (a list of numbers is taken where a tensor is: `max` of a list
            // a function computed is math's, of the whole list — as the
            // lowering picks it)
            let has_numarray = arg_types.iter().any(|t| matches!(t, Ty::Tensor(..)) || matches!(t, Ty::List(i) if matches!(i.as_ref(), Ty::Int | Ty::Float)));
            if has_numarray {
                if let Some(t) = self.registry.type_of(name, &arg_types) {
                    return t;
                }
            }
        }
        // An element-wise math function over a plain number is a number
        if crate::modules::math::SCALAR_UNARY.contains(&name) && args.len() == 1 && self.registry.has_function(name) {
            let t = arg_ty(&args[0]);
            let t = if let Ty::Result(i) = t { *i } else { t };
            if matches!(t, Ty::Int | Ty::Float) { return Ty::Float; }
        }
        // A frame function whose first argument is not a frame is the core
        // builtin of the same name (sort[xs], join[parts, sep] with frame
        // imported). While that argument's type is not known yet (a
        // derivation or a function read before its body was typed: unknown,
        // or nothing until inference knows), neither is the call's: a later
        // round decides, where a guess would stick in what returns it
        let arg0 = args.first().map(|a| Self::walked(arg_ty(a)));
        let frame_core_fallback = self.registry.module_name_of(name) == Some("frame")
            && crate::table::has_core_rows(name)
            && !matches!(arg0, Some(Ty::Frame(_) | Ty::Column(_)));
        if frame_core_fallback && matches!(arg0, Some(Ty::Unknown | Ty::Unit)) {
            return Ty::Unknown;
        }
        // Non-overlapping module functions — unless a row of the core's takes
        // these kinds: the core's row is the one both backends pick (`slice`
        // of text where a module in scope answers `slice` of bytes)
        let kinds: Vec<crate::table::Arg> = args.iter().map(|a| crate::table::Arg::of_type(&arg_ty(a))).collect();
        let cores = crate::table::has_core_rows(name) && !kinds.iter().any(|k| matches!(k, crate::table::Arg::Unknown | crate::table::Arg::AnyList)) && crate::table::core_row(name, &kinds).is_some();
        if !is_overlapping && !frame_core_fallback && !cores && self.registry.has_function(name) {
            // A name two modules export (stack, unique, load: frame + math)
            // resolves by its first argument's type — the same dispatch the
            // emitters use — so a qualified call that lowering made bare
            // keeps its type.
            // the module the call reaches, as the lowering decides it
            // (crate::table::module_of_bare: one rule for both)
            let providers = self.registry.providers_of(name);
            let full: Vec<Ty> = args.iter().map(|a| arg_ty(a)).collect();
            match crate::table::module_of_bare(name, &providers, &kinds) {
                Some(m) => {
                    if let Some(t) = self.registry.type_of_in(&m, name, &full) {
                        return t;
                    }
                }
                // (the core's, where its rows take these kinds: the module's
                // type had been the call's — `slice[[], 1, 3]` under `use
                // sys` typed as the layer's bytes, its `type` `unknown`
                // compiled where the interpreter said `array`)
                None if crate::table::core_row(name, &kinds).is_some() => {}
                None => {
                    if let Some(t) = self.registry.type_of(name, &full) {
                        return t;
                    }
                }
            }
        }
        // a module's function the program does not import, called bare where
        // nothing in scope takes this many arguments (`join[a, b, 'k']`
        // without `use frame`): the program is refused for the import, and
        // the call is typed as the module's, so what the program does with
        // the value is judged in the same run (spec, Error Messages, rule 3)
        if !self.registry.has_function(name) && !self.functions.contains_key(name) && !self.vars.contains_key(name) {
            let core_takes = crate::table::ROWS.iter().any(|r| {
                let fixed = r.params.iter().filter(|k| !k.is_rest()).count();
                r.module.is_empty() && r.name == name && (r.params.len() == args.len() || (r.params.iter().any(|k| k.is_rest()) && args.len() >= fixed))
            });
            if !core_takes {
                let stdlib = crate::modules::ModuleRegistry::with_stdlib();
                let providers = stdlib.providers_of(name);
                let full: Vec<Ty> = args.iter().map(arg_ty).collect();
                if let Some(m) = crate::table::module_of_bare(name, &providers, &kinds)
                    && let Some(t) = stdlib.type_of_in(&m, name, &full)
                {
                    return t;
                }
            }
        }

        // Builtins (check before vars to prevent builtins from being shadowed by Fn inference)
        match name {
            "log" | "cpu" | "gpu" | "assert" => Ty::Unit,
            "type" => Ty::Str,
            // an error has no value, only a reason: a failed nothing, which
            // any value it meets decides the type of (unify)
            "error" => Ty::Result(Box::new(Ty::Unit)),
            // (an empty map is open: its first key decides its keys' kind)
            "map" => Ty::Map(crate::ir::Key::Open, Box::new(Ty::Unknown)),
            "range" => Ty::List(Box::new(Ty::Int)),
            // Math builtins (scalar)
            "abs" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let Ty::Result(inner) = t { return *inner; }
                    return t;
                }
                Ty::Int
            }
            "sqrt" => Ty::Float,
            "floor" | "ceil" | "round" => Ty::Int,
            // String builtins (`contains` over a frame's column is a mask)
            "contains" => {
                if matches!(args.first().map(arg_ty), Some(Ty::Column(_))) {
                    Ty::List(Box::new(Ty::Bool))
                } else {
                    Ty::Bool
                }
            }
            // Immutable collection builtins (return same type as first arg)
            "push" => {
                // push[array, val] returns Array/NumArray with element type from val
                // (a holder that can fail is read by what it holds: a tensor
                // that can fail pushed to had been typed a list, and the
                // compiled program's tensor push did not build into it)
                if let Some(arg) = args.first() {
                    let arr_type = Self::walked(arg_ty(arg));
                    if args.len() == 2 {
                        // a value that can fail is handed on as the call's
                        // error, never pushed: the items are its value's
                        let val_type = Self::walked(arg_ty(&args[1]));
                        let existing_inner = match &arr_type {
                            Ty::List(inner) => *inner.clone(),
                            Ty::Tensor(inner, dims) => Self::tensor_item(inner, dims),
                            _ => Ty::Unknown,
                        };
                        let unified = existing_inner.unify(&val_type);
                        // Preserve NumArray if input was NumArray and new value is numeric
                        if matches!(&arr_type, Ty::Tensor(..)) && matches!(unified, Ty::Int | Ty::Float) {
                            return Ty::tensor(unified);
                        }
                        // push[m, row] on a rank-2 tensor keeps the tensor type
                        if matches!(&arr_type, Ty::Tensor(..)) && matches!(val_type, Ty::Tensor(..)) {
                            return arr_type;
                        }
                        return Ty::List(Box::new(unified));
                    }
                    return arr_type;
                }
                Ty::Unknown
            }
            // the two joined: the first's form (a list, a tensor, text), items
            // from both — an empty `[]` takes its items from what it is
            // joined with
            // `xs[mask]`, `t[mask]`: the items, the rows, where it is true
            "__pick_items" => args.first().map(arg_ty).map(Self::walked).unwrap_or(Ty::Unknown),
            "__pick_rows" => picked_rows_type(args.first().map(arg_ty).map(Self::walked)),
            // `map[keys, values]`: a map of the values' kind
            "__map_of" => map_of_type(args.first().map(arg_ty).map(Self::walked), args.get(1).map(arg_ty).map(Self::walked)),
            // one list: its items joined — chunks of bytes, lists of text
            "concat" if args.len() == 1 => joined_type(Self::walked(arg_ty(&args[0]))),
            "concat" if args.len() >= 2 => {
                let first = Self::walked(arg_ty(&args[0]));
                let second = Self::walked(arg_ty(&args[1]));
                match (&first, &second) {
                    (Ty::List(a), Ty::List(b) | Ty::Tensor(b, _)) => Ty::List(Box::new(a.unify(b))),
                    (Ty::Tensor(a, _), Ty::Tensor(b, _) | Ty::List(b)) => Ty::tensor(a.unify(b)),
                    (Ty::Unknown, _) => second,
                    _ => first,
                }
            }
            "pop" | "sort" | "reverse" | "concat" => {
                if let Some(arg) = args.first() {
                    // Base type = with fallible args already unwrapped (the
                    // propagation block feeds these builtins plain values).
                    let t = arg_ty(arg);
                    if let Ty::Result(inner) = t {
                        return *inner;
                    }
                    return t;
                }
                Ty::Unknown
            }
            "set" => {
                // set[map, key, val] returns Map with value type from val arg
                if let Some(arg) = args.first() {
                    let map_type = arg_ty(arg);
                    if args.len() == 3 {
                        let val_type = arg_ty(&args[2]);
                        // A fallible value propagates out of the call (the map
                        // is built from the unwrapped value or not at all): the
                        // result is a fallible map of plain values, never a map
                        // of results. The same holds for a fallible map argument.
                        let (map_fallible, map_type) = match map_type {
                            Ty::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let (val_fallible, val_type) = match val_type {
                            Ty::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let (existing_key, existing_inner) = if let Ty::Map(key, inner) = &map_type {
                            (*key, *inner.clone())
                        } else {
                            (crate::ir::Key::Open, Ty::Unknown)
                        };
                        // the key's kind, by the key handed (an open map takes it)
                        let key = existing_key.unify(key_kind_of(&Self::walked(arg_ty(&args[1])))).unwrap_or(existing_key);
                        let out = Ty::Map(key, Box::new(existing_inner.unify(&val_type)));
                        return if map_fallible || val_fallible { Ty::Result(Box::new(out)) } else { out };
                    }
                    return map_type;
                }
                Ty::Unknown
            }
            "del" => {
                if let Some(arg) = args.first() {
                    return arg_ty(arg);
                }
                Ty::Unknown
            }
            // Map builtins
            // (a map no key has been written into yet says nothing of its
            // keys' kind: read as text, the guess had become the kind a list
            // filled from them was first known to hold, and the int keys
            // `set` wrote later were refused against it — parity 315)
            "keys" => match args.first().map(arg_ty).map(Self::walked) {
                Some(Ty::Map(crate::ir::Key::Open, _)) => Ty::List(Box::new(Ty::Unknown)),
                Some(Ty::Map(key, _)) => Ty::List(Box::new(key.ty())),
                // (nor does a value not known yet — a function's answer
                // before its body is typed: guessed as text, an int-keyed
                // map's keys bound at the top level were refused as a
                // reassignment; Rosetta Code, Achilles numbers)
                Some(Ty::Unknown) | None => Ty::List(Box::new(Ty::Unknown)),
                _ => Ty::List(Box::new(Ty::Str)),
            },
            "has" => Ty::Bool,
            "values" => {
                // (through a failure: `values[f[t]]` of a function that can
                // fail is the map's values, the call's step wrapping them as
                // one that can fail — read as nothing known, the compiled
                // failure had held a list of text; Rosetta Code, Graph
                // colouring)
                if let Some(arg) = args.first()
                    && let Ty::Map(_, inner) = arg_ty(arg).payload()
                {
                    return Ty::List(inner.clone());
                }
                Ty::List(Box::new(Ty::Unknown))
            }
            "slice" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let Ty::Result(inner) = t { return *inner; }
                    return t;
                }
                Ty::Unknown
            }
            "len" => Ty::Int,
            // CLI / IO
            "args" => Ty::List(Box::new(Ty::Str)),
            "input" => Ty::Result(Box::new(Ty::Str)),
            // System builtins
            // Moments are numbers (epoch seconds); constructors are fallible
            "now" => Ty::Moment,
            "captured" => Ty::Result(Box::new(Ty::Moment)),
            "chart" => Ty::Str,
            "sleep" => Ty::Int,
            // the session's notice: an effect held to the handler's commit,
            // and the record the runtime keeps (notice.text, notice.at)
            "notice" => Ty::Unit,
            // the page's signed-in reader: a map of strings the runtime
            // provides, set by two effects held to the handler's commit
            "__reader" => Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)),
            // a read a guard narrowed (lower.rs): the value, never its failure
            "__known" => arg_types.first().cloned().map(Self::walked).unwrap_or(Ty::Unknown),
            "sign_in" | "sign_out" | "download" => Ty::Unit,
            // the files a test's session was handed, as maps of name and text
            "downloads" => Ty::List(Box::new(Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)))),
            // a message: whether the server took it; and the messages a
            // test's session sent, as maps of to, subject and body
            "mails" => Ty::List(Box::new(Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)))),
            "__notice_text" => Ty::Str,
            "__notice_at" => Ty::Float,
            "env" => Ty::Result(Box::new(Ty::Str)),
            "exit" => Ty::Int,
            // Web builtins (available when `use web` is active)
            "route" => Ty::Handle(&crate::table::WEBAPP),
            "serve" => Ty::Unit,
            "respond" | "redirect" => Ty::Handle(&crate::table::RESPONSE),
            // the other responses: a value packed, a frame as Arrow, bytes,
            // and a body its feed puts as it goes
            "arrow" | "bytes" => Ty::Handle(&crate::table::RESPONSE),
            // a test's page as its html, from its app session; in a web app,
            // the response
            "html" => match arg_types.first() {
                Some(Ty::Handle(h)) if h.name == crate::table::APP.name => Ty::Str,
                _ => Ty::Handle(&crate::table::RESPONSE),
            },
            // a test's app session, and the elements a selector picks from
            // its page: records of text (tag, text, class, each attribute)
            "app" => Ty::Handle(&crate::table::APP),
            "elements" => Ty::List(Box::new(Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)))),
            // the layout engine's boxes a selector picks: a list, each box
            // read by its field as the program runs
            "box" => Ty::List(Box::new(Ty::Unknown)),
            // `json[map]` (use web): `json[text]` is its row
            "json" | "text" => Ty::Handle(&crate::table::RESPONSE),
            // db builtins
            "query" => Ty::List(Box::new(Ty::Json)),
            // Web.auth builtins: a token, or why not; a token's claims, or why
            // not (expired, a signature that does not match, not a token)
            // Web.ws: a socket, or why not; send answers empty text or why
            // not, recv the next message or why not (closed, a wait passed)
            // Reactive data sources (UI)
            "poll" | "fetch" => Ty::Frame(None),
            // Autograd
            "grad" => {
                // grad[f] is a function of f's parameters (`gradient_type`)
                if let Some(Expr::Ident(fn_name)) = args.first() {
                    if let Some(t) = self.gradient_type(fn_name) {
                        return t;
                    }
                }
                Ty::Fn(vec![Ty::Float], Box::new(Ty::Float))
            }
            _ => {
                // a variable indexed, or a function value applied: the one
                // index rule (index_type)
                match self.vars.get(name) {
                    Some(var_type) => self.index_type(var_type.clone(), arg_types.first().cloned()),
                    None => Ty::Unknown,
                }
            },
        }
    }

    /// A name read as a value: a variable, a module constant, a
    /// derivation's value, or a function held as a value.
    pub(crate) fn ident_type(&self, name: &str) -> Ty {
        // User variables shadow module constants (matches codegen +
        // interpreter scoping: environment first, then constants)
        if let Some(ty) = self.vars.get(name) {
            return ty.clone();
        }
        // Module constants: pi, e, inf, nan
        if self.registry.has_constant(name) {
            return Ty::Float;
        }
        // A derivation's name is its value, wherever it is read — as
        // a call argument too — never a function reference
        if self.derives.contains(name) {
            return self.functions.get(name).map(|s| s.return_type.clone()).unwrap_or(Ty::Unknown);
        }
        // Check if it's a known function name — return as Ty::Fn
        if let Some(sig) = self.functions.get(name).filter(|_| self.calls_fn(name)) {
            let param_types: Vec<Ty> = sig.params.iter().map(|(_, t)| t.clone()).collect();
            return Ty::Fn(param_types, Box::new(sig.return_type.clone()));
        }
        Ty::Unknown
    }

    /// An operator's type from its operands'.
    pub(crate) fn binop_type(&self, op: &BinOp, lt: Ty, rt: Ty) -> Ty {
        // Check if either operand is CrowResult (for auto-propagation)
        let has_result = matches!(&lt, Ty::Result(_)) || matches!(&rt, Ty::Result(_));
        // Unwrap to get inner types for determining result type
        let lt_inner = if let Ty::Result(inner) = &lt { *inner.clone() } else { lt };
        let rt_inner = if let Ty::Result(inner) = &rt { *inner.clone() } else { rt };
        // a side nothing has typed yet makes an arithmetic answer nothing
        // has typed: a guess (`x - max[x]` read as a number while `x` was
        // unknown) had stuck as the function's return type, since a type
        // only widens from round to round, and a test's call of the
        // function — typed last — was refused by it
        // (a comparison of such a side is nothing has typed too: it may
        // be a mask — `t.filled eq true` of a frame a function is not yet
        // called with — and a bool guessed here had been refused where a
        // mask is taken; a test's bare truth is found by its syntax, not
        // by this type)
        if matches!(lt_inner, Ty::Unknown) || matches!(rt_inner, Ty::Unknown) {
            return if has_result { Ty::Result(Box::new(Ty::Unknown)) } else { Ty::Unknown };
        }
        // a moment: on or back by seconds (a moment), the seconds between
        // two (a number), and compared with a moment (a bool); anything
        // else the lowering refuses
        // (a column or a list compared with a moment is a mask, typed below)
        let moment = |t: &Ty| matches!(t, Ty::Moment);
        let scalar = |t: &Ty| matches!(t, Ty::Moment | Ty::Int | Ty::Float | Ty::Unknown | Ty::Json);
        if (moment(&lt_inner) || moment(&rt_inner)) && scalar(&lt_inner) && scalar(&rt_inner) {
            let number = |t: &Ty| matches!(t, Ty::Int | Ty::Float);
            let t = match op {
                BinOp::Sub if moment(&lt_inner) && moment(&rt_inner) => Ty::Float,
                BinOp::Add | BinOp::Sub if moment(&lt_inner) && number(&rt_inner) => Ty::Moment,
                BinOp::Add if number(&lt_inner) && moment(&rt_inner) => Ty::Moment,
                BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte if moment(&lt_inner) && moment(&rt_inner) => Ty::Bool,
                _ => Ty::Unknown,
            };
            return if has_result { Ty::Result(Box::new(t)) } else { t };
        }
        // exact numbers: an int meeting a decimal is the decimal it is, an int
        // or a decimal meeting a fraction the fraction it is; a float beside
        // one is typed as the exact kind, the lowering reading a literal as
        // the number it writes and refusing any other (spec, Numbers That
        // Keep Every Digit)
        let exact = |t: &Ty| matches!(t, Ty::Decimal | Ty::Fraction);
        if exact(&lt_inner) || exact(&rt_inner) {
            let number = |t: &Ty| matches!(t, Ty::Int | Ty::Big | Ty::Float | Ty::Decimal | Ty::Fraction | Ty::Json);
            let kind = if matches!(lt_inner, Ty::Fraction) || matches!(rt_inner, Ty::Fraction) { Ty::Fraction } else { Ty::Decimal };
            let t = match op {
                _ if !number(&lt_inner) || !number(&rt_inner) => Ty::Unknown,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => kind,
                // a power of an exact number: by an int its own kind, by a
                // decimal a decimal
                BinOp::Pow if exact(&lt_inner) => kind,
                BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte => Ty::Bool,
                _ => Ty::Unknown,
            };
            return if has_result { Ty::Result(Box::new(t)) } else { t };
        }
        // Check if either side is a frame column (FrameCol or legacy Array(Str))
        let is_frame_col = |t: &Ty| matches!(t, Ty::Column(_)) || matches!(t, Ty::List(inner) if matches!(inner.as_ref(), Ty::Str));
        let result_type = match op {
            BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
            | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => {
                // Boolean-mask semantics: comparisons on frame columns and
                // logical ops on boolean arrays are element-wise → Array(Bool)
                let is_bool_mask = |t: &Ty| t.is_truths();
                // a tensor compared with a number is a mask of its shape (a
                // matrix's, its rows of bools: spec, Boolean Masks)
                let tensor_vs_scalar = (matches!(lt_inner, Ty::Tensor(..))
                    && matches!(rt_inner, Ty::Int | Ty::Float))
                    || (matches!(rt_inner, Ty::Tensor(..))
                        && matches!(lt_inner, Ty::Int | Ty::Float));
                if tensor_vs_scalar {
                    if matches!(lt_inner, Ty::Tensor(..)) { lt_inner.mask_of() } else { rt_inner.mask_of() }
                } else if matches!(op, BinOp::And | BinOp::Or) && (lt_inner.truths_depth() > 1 || rt_inner.truths_depth() > 1) {
                    // (two masks of a matrix's shape joined: that shape)
                    if lt_inner.truths_depth() >= rt_inner.truths_depth() { lt_inner.clone() } else { rt_inner.clone() }
                } else if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte) {
                    // a column (or a list of text, or a mask) compared with
                    // one value is a mask, a cell at a time (spec, Boolean
                    // Masks); compared with a list, a column or a tensor it is
                    // one truth, as two lists compare (spec, Comparison), and
                    // so are two masks compared whole. Typed as masks, `true
                    // and df.id eq ['a', 'b']` had joined item by item: the
                    // lowering ran the right side's statements before the
                    // left side, and the emitter printed a mask's `and` over a
                    // bool (no method `iter` for `bool`) — parity 302
                    // (a list of numbers beside one number is a mask too, as
                    // both modes answer it — `3 gte ys` of a list made by
                    // push had been typed one truth: `if 3 gte ys` ran
                    // interpreted and did not build compiled, and `type` of
                    // it said `bool` — crow check --self, decompress of a
                    // list for an int)
                    let number_list = |t: &Ty| matches!(t, Ty::List(i) if matches!(i.as_ref(), Ty::Int | Ty::Float | Ty::Big));
                    let cells = |t: &Ty| is_frame_col(t) || is_bool_mask(t) || number_list(t);
                    let one_value = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Str | Ty::Bool | Ty::Moment | Ty::Json);
                    if (cells(&lt_inner) && one_value(&rt_inner)) || (cells(&rt_inner) && one_value(&lt_inner)) {
                        Ty::List(Box::new(Ty::Bool))
                    } else {
                        Ty::Bool
                    }
                } else if is_frame_col(&lt_inner) || is_frame_col(&rt_inner)
                    || is_bool_mask(&lt_inner) || is_bool_mask(&rt_inner)
                {
                    Ty::List(Box::new(Ty::Bool))
                } else {
                    Ty::Bool
                }
            }
            BinOp::Mod => Ty::Int,
            BinOp::Pow => {
                // ^ returns Float if either operand is Float, else Int
                if lt_inner == Ty::Float || rt_inner == Ty::Float {
                    Ty::Float
                } else {
                    Ty::Int
                }
            }
            BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow => {
                // Element-wise tensor ops return NumArray; frame
                // columns bridge into tensors
                if matches!(&lt_inner, Ty::Column(_))
                    || matches!(&rt_inner, Ty::Column(_))
                {
                    Ty::tensor(Ty::Float)
                } else if matches!(&lt_inner, Ty::Tensor(..)) {
                    lt_inner.clone()
                } else if matches!(&rt_inner, Ty::Tensor(..)) {
                    rt_inner.clone()
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Mul => {
                // Frame column arithmetic with scalar → FrameCol
                if is_frame_col(&lt_inner) && matches!(&rt_inner, Ty::Int | Ty::Float) {
                    Ty::Column(Box::new(Ty::Unknown))
                } else if is_frame_col(&rt_inner) && matches!(&lt_inner, Ty::Int | Ty::Float) {
                    Ty::Column(Box::new(Ty::Unknown))
                // sparse * dense → NumArray (matmul result)
                } else if matches!(&lt_inner, Ty::Sparse) && matches!(&rt_inner, Ty::Tensor(..)) {
                    Ty::tensor(Ty::Float)
                // * on tensor-tensor is matmul (returns NumArray)
                } else if matches!(&lt_inner, Ty::Tensor(..)) || matches!(&rt_inner, Ty::Tensor(..)) {
                    Ty::tensor(Ty::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Add => {
                // Frame column arithmetic → FrameCol
                if (is_frame_col(&lt_inner) && matches!(&rt_inner, Ty::Int | Ty::Float))
                    || (is_frame_col(&rt_inner) && matches!(&lt_inner, Ty::Int | Ty::Float)) {
                    Ty::Column(Box::new(Ty::Unknown))
                // sparse + sparse → Sparse
                } else if matches!(&lt_inner, Ty::Sparse) && matches!(&rt_inner, Ty::Sparse) {
                    Ty::Sparse
                } else if matches!(&lt_inner, Ty::Tensor(..)) || matches!(&rt_inner, Ty::Tensor(..)) {
                    Ty::tensor(Ty::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Sub | BinOp::Div => {
                // Frame column arithmetic → FrameCol
                if (is_frame_col(&lt_inner) && matches!(&rt_inner, Ty::Int | Ty::Float))
                    || (is_frame_col(&rt_inner) && matches!(&lt_inner, Ty::Int | Ty::Float)) {
                    Ty::Column(Box::new(Ty::Unknown))
                // scalar - tensor, tensor - scalar, etc. → NumArray
                } else if matches!(&lt_inner, Ty::Tensor(..)) || matches!(&rt_inner, Ty::Tensor(..)) {
                    Ty::tensor(Ty::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
        };
        // If either operand was CrowResult, wrap the result type
        if has_result {
            Ty::Result(Box::new(result_type))
        } else {
            result_type
        }
    }

    /// A unary operator's type from its operand's.
    pub(crate) fn unary_type(&self, op: &UnaryOp, t: Ty) -> Ty {
        match op {
            UnaryOp::Not => {
                // not on a boolean mask negates element-wise → Array(Bool);
                // a value that can fail stays one (an operator computes with
                // an error: `not` of one that failed is that error)
                let fails = t.is_result();
                let inner = if let Ty::Result(i) = t { *i } else { t };
                let turned = if inner.is_truths() { inner.clone() } else { Ty::Bool };
                if fails {
                    Ty::Result(Box::new(turned))
                } else {
                    turned
                }
            }
            UnaryOp::Neg => t,
        }
    }

    /// A call by name's type from its arguments' types, an argument's
    /// failure made the call's (auto-propagation) unless the callee renders
    /// what it is given.
    /// `grad[f]`'s type: a function of f's parameters — of numbers, each
    /// a float, answering a float (one parameter) or a list of floats; of
    /// tensors, answering a tensor or a list of them, each shaped like its
    /// parameter (the list's items have what the parameters' shapes share:
    /// a rank only when every parameter's is that rank). None where `f` is
    /// no function of the program's; parameters the seek only guessed
    /// floats are not known until a call of the gradient says.
    pub(crate) fn gradient_type(&self, fn_name: &str) -> Option<Ty> {
        let sig = self.functions.get(fn_name)?;
        let n = sig.params.len();
        // (parameters not known yet — a function only a test hands to
        // `grad`, typed by the test's call later — are not floats either:
        // a guess of floats had stuck to the parameter the gradient was
        // handed to, and the tensors it was applied with never reached it)
        let unknown = sig.params.iter().any(|(_, t)| matches!(t, Ty::Unknown));
        if self.grad_forced.contains(fn_name) || unknown {
            return Some(Ty::Fn(vec![Ty::Unknown; n], Box::new(Ty::Unknown)));
        }
        let tensors = sig.params.iter().any(|(_, t)| matches!(t, Ty::Tensor(..)));
        if tensors {
            let param_types: Vec<Ty> = sig.params.iter().map(|(_, t)| t.clone()).collect();
            let shapes: Vec<&Option<super::Dims>> = param_types
                .iter()
                .filter_map(|t| match t {
                    Ty::Tensor(_, d) => Some(d),
                    _ => None,
                })
                .collect();
            let dims = match shapes.first() {
                Some(Some(first)) if shapes.iter().all(|d| d.as_ref().is_some_and(|d| d.len() == first.len())) => {
                    shapes.iter().skip(1).fold(Some(first.clone()), |acc, d| super::join_dims(&acc, d))
                }
                _ => None,
            };
            let one = Ty::Tensor(Box::new(Ty::Float), dims);
            let ret = if n == 1 { one } else { Ty::List(Box::new(one)) };
            return Some(Ty::Fn(param_types, Box::new(ret)));
        }
        let param_types = vec![Ty::Float; n];
        let ret = if n == 1 { Ty::Float } else { Ty::List(Box::new(Ty::Float)) };
        Some(Ty::Fn(param_types, Box::new(ret)))
    }

    pub(crate) fn named_call_type(&self, name: &str, args: &[Expr], callee: &Expr, arg_types: &[Ty], full: &[Ty]) -> Ty {
        // db's transact[conn, f]: what `f` answers, as a value that can
        // fail — `f`'s error when it failed (the transaction rolled back),
        // the server's when the commit was refused
        if name == "transact" && args.len() == 2 && !self.functions.contains_key(name) {
            if let Expr::Ident(f) = &args[1] {
                if let Some(sig) = self.functions.get(f) {
                    let payload = match &sig.return_type {
                        Ty::Result(inner) => inner.as_ref().clone(),
                        other => other.clone(),
                    };
                    return Ty::Result(Box::new(payload));
                }
            }
        }
        let base = self.infer_call_base_typed(name, args, callee, arg_types, full);
        // Auto-propagation: wrap in CrowResult if any arg is CrowResult
        // a view renders what it is given: a failed argument shows
        // its reason inside the tree, it never fails the view; a record
        // holds a failed value in its field, as a module's record does
        // (an element's name is the element inside a view, and a function
        // of the program's elsewhere: `h1[x]` outside a view hands a failed
        // `x` on as any call does — the fuzzer's helpers are named h1, h2;
        // and so is a module's: the frame's `stack[rows, more]` of a frame
        // that can fail had dropped the failure, read as the element)
        let element = crate::lower::UI_ELEMENTS.contains(&name) && (self.in_view || (!self.calls_fn(name) && self.registry.providers_of(name).is_empty()));
        let skip = matches!(name, "log" | "assert" | "type" | "cpu" | "gpu" | "error" | "table" | "chart" | "__known")
            || self.structs.contains_key(name)
            || element
            || (self.in_view && crate::lower::is_ui_control(name, matches!(arg_types.first().cloned().map(Self::walked), Some(Ty::Frame(_)))))
            || self.views.contains(name);
        // (a call that answers nothing, handed a value that can fail,
        // answers that failure when it failed: typed as nothing, the
        // failure had vanished — dropped by its line in both modes, and the
        // compiled program had not built where the call was a function's
        // last line; Rosetta Code, Hex dump)
        if !skip && !matches!(base, Ty::Result(_)) {
            let has_result_arg = arg_types.iter().any(|t| matches!(t, Ty::Result(_)));
            if has_result_arg {
                return Ty::Result(Box::new(base));
            }
        }
        base
    }

    /// A call through a module's name (`math.zeros[…]`, `m.f[…]` after
    /// `use m`): its type, when the module is known.
    pub(crate) fn module_call_type(&self, module: &str, field: &str, arg_types: &[Ty]) -> Option<Ty> {
        // web.new[] → WebApp
        if module == "web" && field == "new" {
            return Some(Ty::Handle(&crate::table::WEBAPP));
        }
        // Registry-module prefixed call (math.zeros[...],
        // or via alias: m.zeros after `use math as m`)
        let real = self.module_aliases.get(module).cloned().unwrap_or_else(|| module.to_string());
        if crate::modules::by_path(&real).is_some_and(|m| m.typed) && self.registry.has_function(field) {
            if let Some(t) = self.registry.type_of_in(&real, field, arg_types) {
                // auto-propagation applies to qualified calls too:
                // a fallible argument makes the result fallible
                if !matches!(t, Ty::Result(_) | Ty::Unit) && arg_types.iter().any(|a| matches!(a, Ty::Result(_))) {
                    return Some(Ty::Result(Box::new(t)));
                }
                return Some(t);
            }
        }
        // a standard module the program does not import, called through its
        // name, is refused (add `use m`): the call is typed as the module's,
        // so what the program goes on to do with the value is judged in the
        // same run (spec, Error Messages, rule 3)
        if crate::modules::by_path(&real).is_some_and(|m| m.typed && !m.internal) && !self.registry.has_function(field) {
            let stdlib = crate::modules::ModuleRegistry::with_stdlib();
            if stdlib.providers_of(field).contains(&real.as_str()) {
                if let Some(t) = stdlib.type_of_in(&real, field, arg_types) {
                    return Some(t);
                }
            }
        }
        let prefixed = format!("{}_{}", module, field);
        // Module struct constructor
        if self.structs.contains_key(&prefixed) {
            return Some(Ty::Record(prefixed));
        }
        // Module function call
        self.functions.get(&prefixed).map(|sig| sig.return_type.clone())
    }

    /// An index's type (`xs[i]`, `m[k]`, `s[i]`, a function value applied)
    /// from the indexed value's type and the first index's — the one rule,
    /// for a variable indexed and for any value (three copies had
    /// disagreed: a column's cell text in one and dynamic in another, and
    /// two dropped the failure of a value that can fail).
    pub(crate) fn index_type(&self, callee_type: Ty, index: Option<Ty>) -> Ty {
        match callee_type {
            // a value that can fail, indexed: its error is the index's value,
            // as the interpreter hands it on (a failed JSON parse indexed)
            Ty::Result(inner) => match self.index_type(*inner, index) {
                Ty::Unknown => Ty::Unknown,
                t @ Ty::Result(_) => t,
                t => Ty::Result(Box::new(t)),
            },
            Ty::List(inner) => *inner,
            Ty::Tensor(inner, dims) => {
                // Advanced indexing: tensor[tensor] → tensor (gather)
                if matches!(index, Some(Ty::Tensor(..))) {
                    return Ty::Tensor(inner, None);
                }
                Self::tensor_item(&inner, &dims)
            }
            // a cell of a column whose type nothing told: dynamic until
            // int[], float[] or str[] reads it (typed frames)
            Ty::Column(_) => Ty::Json,
            Ty::Map(_, inner) => *inner,
            // a character of text
            Ty::Str | Ty::Markup => Ty::Str,
            // an element can be JSON's null, an error value of reason `null`
            Ty::Json => Ty::Result(Box::new(Ty::Json)),
            Ty::Fn(_, ret) => *ret,
            _ => Ty::Unknown,
        }
    }

    /// A field's type (`x.f`) from the object's type; a frame's schema and a
    /// module's variables are looked up by the object's name.
    pub(crate) fn field_type(&self, object: &Expr, obj_type: Ty, field: &str) -> Ty {
        // Json object field access stays Json (dynamic values)
        {
            let inner = match &obj_type { Ty::Result(i) => i.as_ref().clone(), o => o.clone() };
            if matches!(inner, Ty::Json)
                && !matches!(field, "ok" | "reason" | "kind" | "trace")
            {
                return Ty::Json;
            }
        }
        // a route's request: its method, path and body are text, its
        // headers, query, path params and form maps of text, and its files
        // a list of maps of text (name, type, size, path, field)
        if matches!(&obj_type, Ty::WebRequest) {
            return match field {
                "method" | "path" | "body" => Ty::Str,
                "headers" | "query" | "params" | "form" => Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)),
                "files" => Ty::List(Box::new(Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)))),
                _ => Ty::Unknown,
            };
        }
        // .ok on a frame column → its rows' validity; every other
        // value's .ok is its own status, a list's included
        if field == "ok" && matches!(&obj_type, Ty::Column(_)) {
            return Ty::List(Box::new(Ty::Bool));
        }
        // a moment's fields, read in its own zone (of a moment that can fail
        // too, read once it is known to have come)
        if matches!(&obj_type, Ty::Moment) || matches!(&obj_type, Ty::Result(inner) if matches!(inner.as_ref(), Ty::Moment)) {
            match field {
                "year" | "month" | "day" | "hour" | "minute" | "second" | "weekday" | "offset" => return Ty::Int,
                _ => {}
            }
        }
        // a test's app session reads its own reactive state: a derivation
        // or a fact of that name, as the harness reads it
        if matches!(&obj_type, Ty::Handle(h) if h.name == crate::table::APP.name) && !matches!(field, "ok" | "reason" | "kind" | "trace") {
            return if self.derives.contains(field) || self.facts.contains(field) { self.vars.get(field).cloned().unwrap_or(Ty::Unknown) } else { Ty::Unknown };
        }
        // Error-value metadata fields
        match field {
            "ok" => return Ty::Bool,
            "reason" | "kind" => return Ty::Str,
            "trace" => return Ty::List(Box::new(Ty::Str)),
            _ => {}
        }
        // Map dot-access: m.key reads a value of the map's value type
        {
            let inner = match &obj_type { Ty::Result(i) => i.as_ref().clone(), o => o.clone() };
            if let Ty::Map(_, v) = inner {
                return *v;
            }
        }
        // NumArray properties (also handle Result(NumArray))
        let is_numarray = matches!(obj_type, Ty::Tensor(..))
            || matches!(&obj_type, Ty::Result(inner) if matches!(inner.as_ref(), Ty::Tensor(..)));
        if is_numarray {
            match field {
                "shape" => return Ty::List(Box::new(Ty::Int)),
                "dtype" | "device" | "grad" => return Ty::Str,
                "len" => return Ty::Int,
                _ => {}
            }
        }
        // Frame properties
        let is_frame = matches!(obj_type, Ty::Frame(_))
            || matches!(&obj_type, Ty::Result(inner) if matches!(inner.as_ref(), Ty::Frame(_)));
        if is_frame {
            match field {
                "shape" => return Ty::List(Box::new(Ty::Int)),
                "columns" | "dtypes" => return Ty::List(Box::new(Ty::Str)),
                _ => return Ty::Column(Box::new(Ty::Unknown)), // column access by name → typed column
            }
        }
        // FrameCol properties: its validity, and a moment's fields over a
        // column of moments (the lowering types the column by its cells)
        if matches!(obj_type, Ty::Column(_)) {
            match field {
                "ok" => return Ty::List(Box::new(Ty::Bool)),
                "year" | "month" | "day" | "hour" | "minute" | "second" | "weekday" | "offset" => return Ty::Column(Box::new(Ty::Int)),
                _ => return Ty::Unknown,
            }
        }
        // Auto-unwrap: field access on Result(Struct) reads the value's
        // field; OptBoxStruct (recursive struct link) unwraps the box
        let struct_obj = match &obj_type {
            Ty::Record(n) | Ty::OptRecord(n) => Some(n.clone()),
            Ty::Result(inner) => match inner.as_ref() {
                Ty::Record(n) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(name) = struct_obj {
            if let Some(sig) = self.structs.get(&name) {
                for (fname, ftype) in &sig.fields {
                    if fname == field {
                        return ftype.clone();
                    }
                }
            }
        }
        // Module variable access: module.var → look up module_var
        if let Expr::Ident(mod_name) = object {
            let prefixed = format!("{}_{}", mod_name, field);
            if let Some(t) = self.vars.get(&prefixed) {
                return t.clone();
            }
            // Also check if it's a module function reference
            if let Some(sig) = self.functions.get(&prefixed) {
                let param_types: Vec<Ty> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                return Ty::Fn(param_types, Box::new(sig.return_type.clone()));
            }
        }
        Ty::Unknown
    }

    /// A collecting loop's type: a list of what its body yields (the
    /// caller has the yield type — the walker from the tree, and one day
    /// the lowering from the lowered body). An unknown yield stays unknown
    /// for a later pass to settle; falling back to the element type is
    /// wrong whenever the body maps elements to a different type.
    pub(crate) fn for_type(yield_type: Ty) -> Ty {
        Ty::List(Box::new(yield_type))
    }

    /// A list literal's type from its items': numbers make a tensor, and
    /// nested numeric rows a tensor of higher rank — one more dimension
    /// than its rows, each unknown until the lowering counts the literal
    /// (shapes.rs). A tensor's element is always a number: its rank is the
    /// count of its dimensions. (A matrix literal had been a tensor of row
    /// tensors, a second form of one type beside the flat one every
    /// operation answers, and the two had not fit each other.)
    pub(crate) fn array_type(items: impl Iterator<Item = Ty>) -> Ty {
        let mut any = false;
        let mut inner = Ty::Unknown;
        for t in items {
            any = true;
            inner = inner.unify(&t);
        }
        if !any {
            return Ty::List(Box::new(Ty::Unknown));
        }
        // (items nothing has typed yet: a list or a tensor, as they turn
        // out — numbers make the literal a tensor — so nothing is said of it
        // yet. Typed a list, the guess had stuck to a parameter it was
        // handed through a push, and the final pass, which knew the items
        // were ints, handed the parameter a list of tensors: "the lowered
        // program disagrees with itself", found porting Magpie's catalog)
        if matches!(inner, Ty::Unknown) {
            return Ty::Unknown;
        }
        // All-numeric array literals become NumArray (backed by CrowTensor).
        match inner {
            Ty::Int | Ty::Float => Ty::tensor(inner),
            Ty::Tensor(elem, dims) => {
                let mut rank = vec![None];
                rank.extend(dims.unwrap_or_else(|| vec![None]));
                Ty::Tensor(elem, Some(rank))
            }
            inner => Ty::List(Box::new(inner)),
        }
    }

    /// A map literal's type from its values'.
    pub(crate) fn map_type(values: impl Iterator<Item = Ty>) -> Ty {
        let mut vtype = Ty::Unknown;
        for t in values {
            vtype = vtype.unify(&t);
        }
        Ty::Map(crate::ir::Key::Text, Box::new(vtype))
    }
}

/// `t[mask]`: the tensor's type with its first dimension unknown. (A
/// helper, not inline: the inference's recursive frame is measured against
/// a 43-entry map literal in the debug build, parity 48.)
#[inline(never)]
fn picked_rows_type(base: Option<Ty>) -> Ty {
    match base {
        Some(Ty::Tensor(item, dims)) => Ty::Tensor(item, dims.map(|mut d| { if let Some(first) = d.first_mut() { *first = None; } d })),
        _ => Ty::tensor(Ty::Float),
    }
}

/// `map[keys, values]`: a map keyed by the keys' kind — text, or ints (a
/// tensor of them too) — of the values' kind (a tensor's are floats).
#[inline(never)]
fn map_of_type(keys: Option<Ty>, values: Option<Ty>) -> Ty {
    let key = match keys {
        Some(Ty::List(item)) => key_kind_of(&item),
        Some(Ty::Tensor(item, _)) => key_kind_of(&item),
        _ => crate::ir::Key::Open,
    };
    match values {
        Some(Ty::List(item)) => Ty::Map(key, item),
        Some(Ty::Tensor(..)) => Ty::Map(key, Box::new(Ty::Float)),
        _ => Ty::Map(key, Box::new(Ty::Unknown)),
    }
}

/// The key kind a value of this type is: an int keys by ints, text by
/// text, anything else decides nothing (open).
pub(crate) fn key_kind_of(t: &Ty) -> crate::ir::Key {
    match t {
        Ty::Int => crate::ir::Key::Int,
        Ty::Str | Ty::Markup => crate::ir::Key::Text,
        _ => crate::ir::Key::Open,
    }
}

/// `concat[list]`: the items joined — bytes of chunks, a list of the lists' items.
#[inline(never)]
fn joined_type(list: Ty) -> Ty {
    match list {
        Ty::List(item) => match *item {
            Ty::List(inner) => Ty::List(inner),
            Ty::Bytes => Ty::Bytes,
            other => Ty::List(Box::new(other)),
        },
        t => t,
    }
}
