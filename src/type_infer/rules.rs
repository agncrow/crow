//! The typing rules: a node's type from its children's, shared by
//! inference and the lowering (core-rewrite-notes §10, the fold's second step).

use super::*;

impl TypeContext {
    /// The call's type given its arguments' types, inferred once by the
    /// caller: a nested chain (a map literal is set[set[set[map[], …]]]
    /// after lowering) is walked once per level, not twice — twice per
    /// level doubled the work for every entry, and a 30-entry literal
    /// never finished.
    pub fn infer_call_base_typed(&self, name: &str, args: &[Expr], _callee: &Expr, arg_types: &[CrowType], full: &[CrowType]) -> CrowType {
        let arg_ty = |a: &Expr| -> CrowType {
            match args.iter().position(|x| std::ptr::eq(x, a)) {
                Some(i) => arg_types.get(i).cloned().unwrap_or(CrowType::Unknown),
                None => CrowType::Unknown,
            }
        };
        // a body's own list, map or text is indexed, whatever a module
        // another file imports names the same — the environment first, as
        // both backends look (a part's `rsi[jg]` where the program imports
        // chart)
        if self.body_assigned.as_ref().is_some_and(|a| a.contains(name)) {
            // (a local not typed yet is still not a module's function)
            let var_type = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
            return self.index_type(var_type, arg_types.first().cloned());
        }
        // Struct constructor
        if self.structs.contains_key(name) {
            return CrowType::Struct(name.to_string());
        }
        // Inside a view an element, a chart, an icon or a bound input is
        // markup (a `select` of two arguments is the bound input there, as
        // both backends read it)
        if self.in_view
            && (crate::lower::UI_ELEMENTS.contains(&name)
                || matches!(name, "chart" | "icon" | "progress" | "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                || (name == "select" && args.len() == 2))
        {
            return CrowType::Markup;
        }
        // A view's tree is markup at every boundary a type can see — the
        // interpreter's views return strings, and compiled ones render to
        // one wherever a string is expected (a list, a module argument) —
        // and markup, never text, where it is placed as a child.
        if self.views.contains(name) {
            return CrowType::Markup;
        }
        // User-defined function call (specialization-aware: a monomorphized
        // callee's return type depends on this call's argument types)
        if let Some(sig) = self.functions.get(name) {
            if let Some(mangled) = self.specialization_for(name, full) {
                if let Some(clone_sig) = self.functions.get(&mangled) {
                    return clone_sig.return_type.clone();
                }
            }
            return sig.return_type.clone();
        }
        // a builtin with a row is typed by its row, the one place that says
        // what it answers and whether it can fail (crate::table): the row
        // its arguments' kinds pick, as both backends pick it
        let kinds: Vec<crate::table::Arg> = args.iter().map(|a| crate::table::Arg::of_type(&arg_ty(a))).collect();
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
                    let answers: Vec<CrowType> = crate::table::ROWS
                        .iter()
                        .filter(|r| r.module.is_empty() && r.name == name && crate::table::kinds_of(r.params, args.len()).is_some())
                        .map(|r| r.crow_type())
                        .collect();
                    if answers.iter().any(|t| *t != answers[0]) {
                        return CrowType::Unknown;
                    }
                }
                return row.crow_type();
            }
        }
        // Module function dispatch (overlapping builtins: check arg type)
        let is_overlapping = matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
        if is_overlapping && self.registry.has_function(name) {
            let arg_types: Vec<CrowType> = args.iter().map(|a| arg_ty(a)).collect();
            let has_numarray = arg_types.iter().any(|t| matches!(t, CrowType::NumArray(..)));
            if has_numarray {
                if let Some(t) = self.registry.type_of(name, &arg_types) {
                    return t;
                }
            }
        }
        // An element-wise math function over a plain number is a number
        if crate::modules::math::SCALAR_UNARY.contains(&name) && args.len() == 1 && self.registry.has_function(name) {
            let t = arg_ty(&args[0]);
            let t = if let CrowType::Result(i) = t { *i } else { t };
            if matches!(t, CrowType::Int | CrowType::Float) { return CrowType::Float; }
        }
        // A frame function whose first argument is not a frame is the core
        // builtin of the same name (sort[xs], join[parts, sep] with frame
        // imported). While that argument's type is not known yet (a
        // derivation or a function read before its body was typed: unknown,
        // or nothing until inference knows), neither is the call's: a later
        // round decides, where a guess would stick in what returns it
        let arg0 = args.first().map(|a| Self::walked(arg_ty(a)));
        let frame_core_fallback = self.registry.module_name_of(name) == Some("frame")
            && crate::checker::BUILTINS.contains(&name)
            && !matches!(arg0, Some(CrowType::Frame(_) | CrowType::FrameCol));
        if frame_core_fallback && matches!(arg0, Some(CrowType::Unknown | CrowType::Void)) {
            return CrowType::Unknown;
        }
        // Non-overlapping module functions
        if !is_overlapping && !frame_core_fallback && self.registry.has_function(name) {
            // A name two modules export (stack, unique, load: frame + math)
            // resolves by its first argument's type — the same dispatch the
            // emitters use — so a qualified call that lowering made bare
            // keeps its type.
            if self.registry.providers_of(name).len() > 1 {
                let arg0 = args.first().map(|a| arg_ty(a));
                let arg0 = arg0.map(|t| if let CrowType::Result(i) = t { *i } else { t });
                let pick = match arg0 {
                    Some(CrowType::Frame(_)) | Some(CrowType::FrameCol) => Some("frame"),
                    Some(CrowType::NumArray(..)) | Some(CrowType::Array(_)) => Some("math"),
                    _ => None,
                };
                if let Some(m) = pick {
                    if let Some(t) = self.registry.type_of_in(m, name, &args.iter().map(|a| arg_ty(a)).collect::<Vec<_>>()) {
                        return t;
                    }
                }
            }
            if let Some(t) = self.registry.type_of(name, &args.iter().map(|a| arg_ty(a)).collect::<Vec<_>>()) {
                return t;
            }
        }

        // Builtins (check before vars to prevent builtins from being shadowed by Fn inference)
        match name {
            "log" | "cpu" | "gpu" | "assert" => CrowType::Void,
            "type" => CrowType::Str,
            // an error has no value, only a reason: a failed nothing, which
            // any value it meets decides the type of (unify)
            "error" => CrowType::Result(Box::new(CrowType::Void)),
            "map" => CrowType::Map(Box::new(CrowType::Unknown)),
            "range" => CrowType::Array(Box::new(CrowType::Int)),
            // Math builtins (scalar)
            "abs" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Int
            }
            "sqrt" => CrowType::Float,
            "floor" | "ceil" | "round" => CrowType::Int,
            // String builtins
            "contains" => CrowType::Bool,
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
                            CrowType::Array(inner) => *inner.clone(),
                            CrowType::NumArray(inner, dims) => Self::tensor_item(inner, dims),
                            _ => CrowType::Unknown,
                        };
                        let unified = existing_inner.unify(&val_type);
                        // Preserve NumArray if input was NumArray and new value is numeric
                        if matches!(&arr_type, CrowType::NumArray(..)) && matches!(unified, CrowType::Int | CrowType::Float) {
                            return CrowType::tensor(unified);
                        }
                        // push[m, row] on a rank-2 tensor keeps the tensor type
                        if matches!(&arr_type, CrowType::NumArray(..)) && matches!(val_type, CrowType::NumArray(..)) {
                            return arr_type;
                        }
                        return CrowType::Array(Box::new(unified));
                    }
                    return arr_type;
                }
                CrowType::Unknown
            }
            // the two joined: the first's form (a list, a tensor, text), items
            // from both — an empty `[]` takes its items from what it is
            // joined with
            "concat" if args.len() >= 2 => {
                let first = Self::walked(arg_ty(&args[0]));
                let second = Self::walked(arg_ty(&args[1]));
                match (&first, &second) {
                    (CrowType::Array(a), CrowType::Array(b) | CrowType::NumArray(b, _)) => CrowType::Array(Box::new(a.unify(b))),
                    (CrowType::NumArray(a, _), CrowType::NumArray(b, _) | CrowType::Array(b)) => CrowType::tensor(a.unify(b)),
                    (CrowType::Unknown, _) => second,
                    _ => first,
                }
            }
            "pop" | "sort" | "reverse" | "concat" => {
                if let Some(arg) = args.first() {
                    // Base type = with fallible args already unwrapped (the
                    // propagation block feeds these builtins plain values).
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t {
                        return *inner;
                    }
                    return t;
                }
                CrowType::Unknown
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
                            CrowType::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let (val_fallible, val_type) = match val_type {
                            CrowType::Result(inner) => (true, *inner),
                            other => (false, other),
                        };
                        let existing_inner = if let CrowType::Map(inner) = &map_type {
                            *inner.clone()
                        } else {
                            CrowType::Unknown
                        };
                        let out = CrowType::Map(Box::new(existing_inner.unify(&val_type)));
                        return if map_fallible || val_fallible { CrowType::Result(Box::new(out)) } else { out };
                    }
                    return map_type;
                }
                CrowType::Unknown
            }
            "del" => {
                if let Some(arg) = args.first() {
                    return arg_ty(arg);
                }
                CrowType::Unknown
            }
            // Map builtins
            "keys" => CrowType::Array(Box::new(CrowType::Str)),
            "has" => CrowType::Bool,
            "values" => {
                if let Some(arg) = args.first() {
                    let map_type = arg_ty(arg);
                    if let CrowType::Map(inner) = map_type {
                        return CrowType::Array(inner);
                    }
                }
                CrowType::Array(Box::new(CrowType::Unknown))
            }
            "slice" => {
                if let Some(arg) = args.first() {
                    let t = arg_ty(arg);
                    if let CrowType::Result(inner) = t { return *inner; }
                    return t;
                }
                CrowType::Unknown
            }
            "len" => CrowType::Int,
            // Regex + search builtins
            "any" | "all" => CrowType::Bool,
            "find" => {
                // Dispatch: find[str, pattern] → Result(record with the groups);
                // find[array, predicate] → Result(element)
                let arg0 = args.first().map(|a| arg_ty(a)).unwrap_or(CrowType::Unknown);
                match arg0 {
                    CrowType::Array(inner) => CrowType::Result(inner),
                    CrowType::NumArray(inner, dims) => CrowType::Result(Box::new(Self::tensor_item(&inner, &dims))),
                    _ => CrowType::Result(Box::new(CrowType::Struct("CrowFindResult".to_string()))),
                }
            }
            // CLI / IO
            "args" => CrowType::Array(Box::new(CrowType::Str)),
            "input" => CrowType::Result(Box::new(CrowType::Str)),
            // System builtins
            "time" => CrowType::Float,
            // Moments are numbers (epoch seconds); constructors are fallible
            "now" => CrowType::Float,
            "captured" => CrowType::Result(Box::new(CrowType::Float)),
            "chart" => CrowType::Str,
            "sleep" => CrowType::Int,
            // Background work: later[] is fire-and-forget (Void — ask
            // running[] for in-flight status)
            "later" => CrowType::Void,
            "running" => CrowType::Bool,
            // the session's notice: an effect held to the handler's commit,
            // and the record the runtime keeps (notice.text, notice.at)
            "notice" => CrowType::Void,
            // the page's signed-in reader: a map of strings the runtime
            // provides, set by two effects held to the handler's commit
            "__reader" => CrowType::Map(Box::new(CrowType::Str)),
            // a read a guard narrowed (lower.rs): the value, never its failure
            "__known" => arg_types.first().cloned().map(Self::walked).unwrap_or(CrowType::Unknown),
            "sign_in" | "sign_out" | "download" => CrowType::Void,
            // the files a test's session was handed, as maps of name and text
            "downloads" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
            // a message: whether the server took it; and the messages a
            // test's session sent, as maps of to, subject and body
            "mail" => CrowType::Result(Box::new(CrowType::Bool)),
            "smtp" => CrowType::Mailer,
            "mails" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
            "__notice_text" => CrowType::Str,
            "__notice_at" => CrowType::Float,
            "env" => CrowType::Result(Box::new(CrowType::Str)),
            "exit" => CrowType::Int,
            // Web builtins (available when `use web` is active)
            "route" => CrowType::WebApp,
            "serve" => CrowType::Void,
            "respond" => CrowType::WebResponse,
            "redirect" => CrowType::WebResponse,
            // the other responses: a value packed, a frame as Arrow, bytes,
            // and a body its feed puts as it goes
            "msgpack" | "arrow" | "bytes" | "stream" => CrowType::WebResponse,
            // a test's page as its html, from its app session; in a web app,
            // the response
            "html" => match arg_types.first() {
                Some(CrowType::App) => CrowType::Str,
                _ => CrowType::WebResponse,
            },
            // a test's app session, and the elements a selector picks from
            // its page: records of text (tag, text, class, each attribute)
            "app" => CrowType::App,
            "elements" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
            // the layout engine's boxes a selector picks: a list, each box
            // read by its field as the program runs
            "box" => CrowType::Array(Box::new(CrowType::Unknown)),
            // `json[map]` (use web): `json[text]` is its row
            "json" => CrowType::WebResponse,
            "text" => CrowType::WebResponse,
            // db builtins
            "exec" => CrowType::Void,
            "query" => CrowType::Array(Box::new(CrowType::Json)),
            // Web.auth builtins: a token, or why not; a token's claims, or why
            // not (expired, a signature that does not match, not a token)
            "sign" if args.len() == 2 => CrowType::Result(Box::new(CrowType::Str)),
            "verify" => CrowType::Result(Box::new(CrowType::Json)),
            // Web.ws: a socket, or why not; send answers empty text or why
            // not, recv the next message or why not (closed, a wait passed)
            "socket" => CrowType::Result(Box::new(CrowType::Socket)),
            "send" | "recv" if matches!(args.first().map(&arg_ty), Some(CrowType::Socket)) => CrowType::Result(Box::new(CrowType::Str)),
            "close" if matches!(args.first().map(&arg_ty), Some(CrowType::Socket)) => CrowType::Str,
            // Reactive data sources (UI)
            "poll" | "fetch" => CrowType::Frame(None),
            "today" => CrowType::Str,
            // Autograd
            "grad" => {
                // grad[f] is a function of f's parameters: of numbers, each
                // a float, answering a float (one parameter) or a list of
                // floats; of tensors, answering a tensor or a list of them
                if let Some(Expr::Ident(fn_name)) = args.first() {
                    if let Some(sig) = self.functions.get(fn_name) {
                        let n = sig.params.len();
                        // (parameters the seek only guessed floats: not known
                        // until a call of the gradient says, numbers or tensors)
                        if self.grad_forced.contains(fn_name) {
                            return CrowType::Fn(vec![CrowType::Unknown; n], Box::new(CrowType::Unknown));
                        }
                        let tensors = sig.params.iter().any(|(_, t)| matches!(t, CrowType::NumArray(..)));
                        if tensors {
                            let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                            // (each gradient is shaped like its parameter: the
                            // list's items have what the parameters' shapes share)
                            let dims = param_types
                                .iter()
                                .filter_map(|t| match t {
                                    CrowType::NumArray(_, d) => Some(d.clone()),
                                    _ => None,
                                })
                                .reduce(|a, b| super::join_dims(&a, &b))
                                .flatten();
                            let one = CrowType::NumArray(Box::new(CrowType::Float), dims);
                            let ret = if n == 1 { one } else { CrowType::Array(Box::new(one)) };
                            return CrowType::Fn(param_types, Box::new(ret));
                        }
                        let param_types = vec![CrowType::Float; n];
                        let ret = if n == 1 {
                            CrowType::Float
                        } else {
                            CrowType::Array(Box::new(CrowType::Float))
                        };
                        return CrowType::Fn(param_types, Box::new(ret));
                    }
                }
                CrowType::Fn(vec![CrowType::Float], Box::new(CrowType::Float))
            }
            _ => {
                // a variable indexed, or a function value applied: the one
                // index rule (index_type)
                match self.vars.get(name) {
                    Some(var_type) => self.index_type(var_type.clone(), arg_types.first().cloned()),
                    None => CrowType::Unknown,
                }
            },
        }
    }

    /// A name read as a value: a variable, a module constant, a
    /// derivation's value, or a function held as a value.
    pub(crate) fn ident_type(&self, name: &str) -> CrowType {
        // User variables shadow module constants (matches codegen +
        // interpreter scoping: environment first, then constants)
        if let Some(ty) = self.vars.get(name) {
            return ty.clone();
        }
        // Module constants: pi, e, inf, nan
        if self.registry.has_constant(name) {
            return CrowType::Float;
        }
        // A derivation's name is its value, wherever it is read — as
        // a call argument too — never a function reference
        if self.derives.contains(name) {
            return self.functions.get(name).map(|s| s.return_type.clone()).unwrap_or(CrowType::Unknown);
        }
        // Check if it's a known function name — return as CrowType::Fn
        if let Some(sig) = self.functions.get(name) {
            let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
            return CrowType::Fn(param_types, Box::new(sig.return_type.clone()));
        }
        CrowType::Unknown
    }

    /// An operator's type from its operands'.
    pub(crate) fn binop_type(&self, op: &BinOp, lt: CrowType, rt: CrowType) -> CrowType {
        // Check if either operand is CrowResult (for auto-propagation)
        let has_result = matches!(&lt, CrowType::Result(_)) || matches!(&rt, CrowType::Result(_));
        // Unwrap to get inner types for determining result type
        let lt_inner = if let CrowType::Result(inner) = &lt { *inner.clone() } else { lt };
        let rt_inner = if let CrowType::Result(inner) = &rt { *inner.clone() } else { rt };
        // Check if either side is a frame column (FrameCol or legacy Array(Str))
        let is_frame_col = |t: &CrowType| matches!(t, CrowType::FrameCol) || matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Str));
        let result_type = match op {
            BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt
            | BinOp::Gte | BinOp::Lte | BinOp::And | BinOp::Or => {
                // Boolean-mask semantics: comparisons on frame columns and
                // logical ops on boolean arrays are element-wise → Array(Bool)
                let is_bool_mask = |t: &CrowType| matches!(t, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Bool));
                // Tensor vs scalar comparisons broadcast to masks too
                let tensor_vs_scalar = (matches!(lt_inner, CrowType::NumArray(..))
                    && matches!(rt_inner, CrowType::Int | CrowType::Float))
                    || (matches!(rt_inner, CrowType::NumArray(..))
                        && matches!(lt_inner, CrowType::Int | CrowType::Float));
                if is_frame_col(&lt_inner) || is_frame_col(&rt_inner)
                    || is_bool_mask(&lt_inner) || is_bool_mask(&rt_inner)
                    || tensor_vs_scalar
                {
                    CrowType::Array(Box::new(CrowType::Bool))
                } else {
                    CrowType::Bool
                }
            }
            BinOp::Mod => CrowType::Int,
            BinOp::Pow => {
                // ^ returns Float if either operand is Float, else Int
                if lt_inner == CrowType::Float || rt_inner == CrowType::Float {
                    CrowType::Float
                } else {
                    CrowType::Int
                }
            }
            BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow => {
                // Element-wise tensor ops return NumArray; frame
                // columns bridge into tensors
                if matches!(&lt_inner, CrowType::FrameCol)
                    || matches!(&rt_inner, CrowType::FrameCol)
                {
                    CrowType::tensor(CrowType::Float)
                } else if matches!(&lt_inner, CrowType::NumArray(..)) {
                    lt_inner.clone()
                } else if matches!(&rt_inner, CrowType::NumArray(..)) {
                    rt_inner.clone()
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Mul => {
                // Frame column arithmetic with scalar → FrameCol
                if is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float) {
                    CrowType::FrameCol
                } else if is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float) {
                    CrowType::FrameCol
                // sparse * dense → NumArray (matmul result)
                } else if matches!(&lt_inner, CrowType::Sparse) && matches!(&rt_inner, CrowType::NumArray(..)) {
                    CrowType::tensor(CrowType::Float)
                // * on tensor-tensor is matmul (returns NumArray)
                } else if matches!(&lt_inner, CrowType::NumArray(..)) || matches!(&rt_inner, CrowType::NumArray(..)) {
                    CrowType::tensor(CrowType::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Add => {
                // Frame column arithmetic → FrameCol
                if (is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float))
                    || (is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float)) {
                    CrowType::FrameCol
                // sparse + sparse → Sparse
                } else if matches!(&lt_inner, CrowType::Sparse) && matches!(&rt_inner, CrowType::Sparse) {
                    CrowType::Sparse
                } else if matches!(&lt_inner, CrowType::NumArray(..)) || matches!(&rt_inner, CrowType::NumArray(..)) {
                    CrowType::tensor(CrowType::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
            BinOp::Sub | BinOp::Div => {
                // Frame column arithmetic → FrameCol
                if (is_frame_col(&lt_inner) && matches!(&rt_inner, CrowType::Int | CrowType::Float))
                    || (is_frame_col(&rt_inner) && matches!(&lt_inner, CrowType::Int | CrowType::Float)) {
                    CrowType::FrameCol
                // scalar - tensor, tensor - scalar, etc. → NumArray
                } else if matches!(&lt_inner, CrowType::NumArray(..)) || matches!(&rt_inner, CrowType::NumArray(..)) {
                    CrowType::tensor(CrowType::Float)
                } else {
                    lt_inner.unify(&rt_inner)
                }
            }
        };
        // If either operand was CrowResult, wrap the result type
        if has_result {
            CrowType::Result(Box::new(result_type))
        } else {
            result_type
        }
    }

    /// A unary operator's type from its operand's.
    pub(crate) fn unary_type(&self, op: &UnaryOp, t: CrowType) -> CrowType {
        match op {
            UnaryOp::Not => {
                // not on a boolean mask negates element-wise → Array(Bool)
                let inner = if let CrowType::Result(i) = t { *i } else { t };
                if matches!(&inner, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                    CrowType::Array(Box::new(CrowType::Bool))
                } else {
                    CrowType::Bool
                }
            }
            UnaryOp::Neg => t,
        }
    }

    /// A call by name's type from its arguments' types, an argument's
    /// failure made the call's (auto-propagation) unless the callee renders
    /// what it is given.
    pub(crate) fn named_call_type(&self, name: &str, args: &[Expr], callee: &Expr, arg_types: &[CrowType], full: &[CrowType]) -> CrowType {
        let base = self.infer_call_base_typed(name, args, callee, arg_types, full);
        // Auto-propagation: wrap in CrowResult if any arg is CrowResult
        // a view renders what it is given: a failed argument shows
        // its reason inside the tree, it never fails the view; a record
        // holds a failed value in its field, as a module's record does
        // (an element's name is the element inside a view, and a function
        // of the program's elsewhere: `h1[x]` outside a view hands a failed
        // `x` on as any call does — the fuzzer's helpers are named h1, h2)
        let element = crate::lower::UI_ELEMENTS.contains(&name) && (self.in_view || !self.functions.contains_key(name));
        let skip = matches!(name, "log" | "assert" | "type" | "cpu" | "gpu" | "error" | "table" | "chart" | "__known")
            || self.structs.contains_key(name)
            || element
            || (self.in_view && crate::lower::is_ui_control(name, matches!(arg_types.first().cloned().map(Self::walked), Some(CrowType::Frame(_)))))
            || self.views.contains(name);
        if !skip && !matches!(base, CrowType::Result(_) | CrowType::Void) {
            let has_result_arg = arg_types.iter().any(|t| matches!(t, CrowType::Result(_)));
            if has_result_arg {
                return CrowType::Result(Box::new(base));
            }
        }
        base
    }

    /// A call through a module's name (`math.zeros[…]`, `m.f[…]` after
    /// `use m`): its type, when the module is known.
    pub(crate) fn module_call_type(&self, module: &str, field: &str, arg_types: &[CrowType]) -> Option<CrowType> {
        // web.new[] → WebApp
        if module == "web" && field == "new" {
            return Some(CrowType::WebApp);
        }
        // Registry-module prefixed call (math.zeros[...],
        // or via alias: m.zeros after `use math as m`)
        let real = self.module_aliases.get(module).cloned().unwrap_or_else(|| module.to_string());
        if matches!(real.as_str(), "math" | "io" | "frame" | "db" | "doc" | "chart" | "math.stats" | "math.nn") && self.registry.has_function(field) {
            if let Some(t) = self.registry.type_of_in(&real, field, arg_types) {
                // auto-propagation applies to qualified calls too:
                // a fallible argument makes the result fallible
                if !matches!(t, CrowType::Result(_) | CrowType::Void) && arg_types.iter().any(|a| matches!(a, CrowType::Result(_))) {
                    return Some(CrowType::Result(Box::new(t)));
                }
                return Some(t);
            }
        }
        let prefixed = format!("{}_{}", module, field);
        // Module struct constructor
        if self.structs.contains_key(&prefixed) {
            return Some(CrowType::Struct(prefixed));
        }
        // Module function call
        self.functions.get(&prefixed).map(|sig| sig.return_type.clone())
    }

    /// An index's type (`xs[i]`, `m[k]`, `s[i]`, a function value applied)
    /// from the indexed value's type and the first index's — the one rule,
    /// for a variable indexed and for any value (three copies had
    /// disagreed: a column's cell text in one and dynamic in another, and
    /// two dropped the failure of a value that can fail).
    pub(crate) fn index_type(&self, callee_type: CrowType, index: Option<CrowType>) -> CrowType {
        match callee_type {
            // a value that can fail, indexed: its error is the index's value,
            // as the interpreter hands it on (a failed JSON parse indexed)
            CrowType::Result(inner) => match self.index_type(*inner, index) {
                CrowType::Unknown => CrowType::Unknown,
                t @ CrowType::Result(_) => t,
                t => CrowType::Result(Box::new(t)),
            },
            CrowType::Array(inner) => *inner,
            CrowType::NumArray(inner, dims) => {
                // Advanced indexing: tensor[tensor] → tensor (gather)
                if matches!(index, Some(CrowType::NumArray(..))) {
                    return CrowType::NumArray(inner, None);
                }
                Self::tensor_item(&inner, &dims)
            }
            // a cell of a column whose type nothing told: dynamic until
            // int[], float[] or str[] reads it (typed frames)
            CrowType::FrameCol => CrowType::Json,
            CrowType::Map(inner) => *inner,
            // a character of text
            CrowType::Str | CrowType::Markup => CrowType::Str,
            // an element can be JSON's null, an error value of reason `null`
            CrowType::Json => CrowType::Result(Box::new(CrowType::Json)),
            CrowType::Fn(_, ret) => *ret,
            _ => CrowType::Unknown,
        }
    }

    /// A field's type (`x.f`) from the object's type; a frame's schema and a
    /// module's variables are looked up by the object's name.
    pub(crate) fn field_type(&self, object: &Expr, obj_type: CrowType, field: &str) -> CrowType {
        // Json object field access stays Json (dynamic values)
        {
            let inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
            if matches!(inner, CrowType::Json)
                && !matches!(field, "ok" | "reason" | "kind" | "trace")
            {
                return CrowType::Json;
            }
        }
        // a route's request: its method, path and body are text, its
        // headers, query, path params and form maps of text, and its files
        // a list of maps of text (name, type, size, path, field)
        if matches!(&obj_type, CrowType::WebRequest) {
            return match field {
                "method" | "path" | "body" => CrowType::Str,
                "headers" | "query" | "params" | "form" => CrowType::Map(Box::new(CrowType::Str)),
                "files" => CrowType::Array(Box::new(CrowType::Map(Box::new(CrowType::Str)))),
                _ => CrowType::Unknown,
            };
        }
        // .ok on a frame column → its rows' validity; every other
        // value's .ok is its own status, a list's included
        if field == "ok" && matches!(&obj_type, CrowType::FrameCol) {
            return CrowType::Array(Box::new(CrowType::Bool));
        }
        // DateTime properties on strings → Int
        if matches!(&obj_type, CrowType::Str) {
            match field {
                "year" | "month" | "day" | "hour" | "minute" | "second" | "offset" => return CrowType::Int,
                _ => {}
            }
        }
        // a test's app session reads its own reactive state: a derivation
        // or a fact of that name, as the harness reads it
        if matches!(&obj_type, CrowType::App) && !matches!(field, "ok" | "reason" | "kind" | "trace") {
            return if self.derives.contains(field) || self.facts.contains(field) { self.vars.get(field).cloned().unwrap_or(CrowType::Unknown) } else { CrowType::Unknown };
        }
        // Error-value metadata fields
        match field {
            "ok" => return CrowType::Bool,
            "reason" | "kind" => return CrowType::Str,
            "trace" => return CrowType::Array(Box::new(CrowType::Str)),
            _ => {}
        }
        // Map dot-access: m.key reads a value of the map's value type
        {
            let inner = match &obj_type { CrowType::Result(i) => i.as_ref().clone(), o => o.clone() };
            if let CrowType::Map(v) = inner {
                return *v;
            }
        }
        // NumArray properties (also handle Result(NumArray))
        let is_numarray = matches!(obj_type, CrowType::NumArray(..))
            || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::NumArray(..)));
        if is_numarray {
            match field {
                "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                "dtype" | "device" | "grad" => return CrowType::Str,
                "len" => return CrowType::Int,
                _ => {}
            }
        }
        // Frame properties
        let is_frame = matches!(obj_type, CrowType::Frame(_))
            || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame(_)));
        if is_frame {
            match field {
                "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                "columns" | "dtypes" => return CrowType::Array(Box::new(CrowType::Str)),
                _ => return CrowType::FrameCol, // column access by name → typed column
            }
        }
        // FrameCol properties
        if matches!(obj_type, CrowType::FrameCol) {
            match field {
                "ok" => return CrowType::Array(Box::new(CrowType::Bool)),
                _ => return CrowType::Unknown,
            }
        }
        // Auto-unwrap: field access on Result(Struct) reads the value's
        // field; OptBoxStruct (recursive struct link) unwraps the box
        let struct_obj = match &obj_type {
            CrowType::Struct(n) | CrowType::OptBoxStruct(n) => Some(n.clone()),
            CrowType::Result(inner) => match inner.as_ref() {
                CrowType::Struct(n) => Some(n.clone()),
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
                let param_types: Vec<CrowType> = sig.params.iter().map(|(_, t)| t.clone()).collect();
                return CrowType::Fn(param_types, Box::new(sig.return_type.clone()));
            }
        }
        CrowType::Unknown
    }

    /// A collecting loop's type: a list of what its body yields (the
    /// caller has the yield type — the walker from the tree, and one day
    /// the lowering from the lowered body). An unknown yield stays unknown
    /// for a later pass to settle; falling back to the element type is
    /// wrong whenever the body maps elements to a different type.
    pub(crate) fn for_type(yield_type: CrowType) -> CrowType {
        CrowType::Array(Box::new(yield_type))
    }

    /// A list literal's type from its items': numbers make a tensor, and
    /// nested numeric rows a tensor of higher rank — one more dimension
    /// than its rows, each unknown until the lowering counts the literal
    /// (shapes.rs). A tensor's element is always a number: its rank is the
    /// count of its dimensions. (A matrix literal had been a tensor of row
    /// tensors, a second form of one type beside the flat one every
    /// operation answers, and the two had not fit each other.)
    pub(crate) fn array_type(items: impl Iterator<Item = CrowType>) -> CrowType {
        let mut any = false;
        let mut inner = CrowType::Unknown;
        for t in items {
            any = true;
            inner = inner.unify(&t);
        }
        if !any {
            return CrowType::Array(Box::new(CrowType::Unknown));
        }
        // All-numeric array literals become NumArray (backed by CrowTensor).
        match inner {
            CrowType::Int | CrowType::Float => CrowType::tensor(inner),
            CrowType::NumArray(elem, dims) => {
                let mut rank = vec![None];
                rank.extend(dims.unwrap_or_else(|| vec![None]));
                CrowType::NumArray(elem, Some(rank))
            }
            inner => CrowType::Array(Box::new(inner)),
        }
    }

    /// A map literal's type from its values'.
    pub(crate) fn map_type(values: impl Iterator<Item = CrowType>) -> CrowType {
        let mut vtype = CrowType::Unknown;
        for t in values {
            vtype = vtype.unify(&t);
        }
        CrowType::Map(Box::new(vtype))
    }
}
