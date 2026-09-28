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
            let var_type = self.vars.get(name).cloned().unwrap_or(CrowType::Unknown);
            let unwrapped = match var_type {
                CrowType::Result(inner) => *inner,
                other => other,
            };
            return match unwrapped {
                CrowType::Array(inner) | CrowType::NumArray(inner) | CrowType::Map(inner) => *inner,
                CrowType::Str | CrowType::FrameCol => CrowType::Str,
                CrowType::Fn(_, ret) => *ret,
                // a local not typed yet is still not a module's function
                _ => CrowType::Unknown,
            };
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
        // what it answers and whether it can fail (crate::table)
        if !self.registry.has_function(name) {
            if let Some(row) = crate::table::core(name) {
                return row.crow_type();
            }
        }
        // Module function dispatch (overlapping builtins: check arg type)
        let is_overlapping = matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max");
        if is_overlapping && self.registry.has_function(name) {
            let arg_types: Vec<CrowType> = args.iter().map(|a| arg_ty(a)).collect();
            let has_numarray = arg_types.iter().any(|t| matches!(t, CrowType::NumArray(_)));
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
            && !matches!(arg0, Some(CrowType::Frame | CrowType::FrameCol));
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
                    Some(CrowType::Frame) | Some(CrowType::FrameCol) => Some("frame"),
                    Some(CrowType::NumArray(_)) | Some(CrowType::Array(_)) => Some("math"),
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
            "min" | "max" => {
                if args.len() >= 2 {
                    let t1 = arg_ty(&args[0]);
                    let t2 = arg_ty(&args[1]);
                    let t1u = if let CrowType::Result(inner) = t1 { *inner } else { t1 };
                    let t2u = if let CrowType::Result(inner) = t2 { *inner } else { t2 };
                    if t1u == CrowType::Float || t2u == CrowType::Float {
                        return CrowType::Float;
                    }
                    return t1u;
                }
                CrowType::Int
            }
            // String builtins
            "trim" | "upper" | "lower" | "replace" | "join" | "fmt" => CrowType::Str,
            // an algorithm it does not know, or a stored hash it cannot read
            "hash" => CrowType::Result(Box::new(CrowType::Str)),
            "split" => CrowType::Array(Box::new(CrowType::Str)),
            "starts" | "ends" | "contains" => CrowType::Bool,
            // Conversion builtins — fallible when parsing from string
            "int" => {
                if let Some(arg) = args.first() {
                    let arg_type = arg_ty(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str | CrowType::Json) {
                        return CrowType::Result(Box::new(CrowType::Int));
                    }
                }
                CrowType::Int
            }
            "float" => {
                if let Some(arg) = args.first() {
                    let arg_type = arg_ty(arg);
                    let unwrapped = if let CrowType::Result(inner) = &arg_type { inner.as_ref() } else { &arg_type };
                    if matches!(unwrapped, CrowType::Str | CrowType::Json) {
                        return CrowType::Result(Box::new(CrowType::Float));
                    }
                }
                CrowType::Float
            }
            "str" => CrowType::Str,
            // Immutable collection builtins (return same type as first arg)
            "push" => {
                // push[array, val] returns Array/NumArray with element type from val
                if let Some(arg) = args.first() {
                    let arr_type = arg_ty(arg);
                    if args.len() == 2 {
                        // a value that can fail is handed on as the call's
                        // error, never pushed: the items are its value's
                        let val_type = Self::walked(arg_ty(&args[1]));
                        let existing_inner = match &arr_type {
                            CrowType::Array(inner) => *inner.clone(),
                            CrowType::NumArray(inner) => *inner.clone(),
                            _ => CrowType::Unknown,
                        };
                        let unified = existing_inner.unify(&val_type);
                        // Preserve NumArray if input was NumArray and new value is numeric
                        if matches!(&arr_type, CrowType::NumArray(_)) && matches!(unified, CrowType::Int | CrowType::Float) {
                            return CrowType::NumArray(Box::new(unified));
                        }
                        // push[m, row] on a rank-2 tensor keeps the tensor type
                        if matches!(&arr_type, CrowType::NumArray(_)) && matches!(val_type, CrowType::NumArray(_)) {
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
                    (CrowType::Array(a), CrowType::Array(b) | CrowType::NumArray(b)) => CrowType::Array(Box::new(a.unify(b))),
                    (CrowType::NumArray(a), CrowType::NumArray(b) | CrowType::Array(b)) => CrowType::NumArray(Box::new(a.unify(b))),
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
            // shell/env are readers: error values on failure, stamped with a
            // capture moment on success (same shape as read[])
            "shell" => CrowType::Result(Box::new(CrowType::Struct("CrowShell".to_string()))),
            // Regex + search builtins
            "any" | "all" => CrowType::Bool,
            "find" => {
                // Dispatch: find[str, pattern] → Result(record with the groups);
                // find[array, predicate] → Result(element)
                let arg0 = args.first().map(|a| arg_ty(a)).unwrap_or(CrowType::Unknown);
                match arg0 {
                    CrowType::Array(inner) => CrowType::Result(inner),
                    CrowType::NumArray(inner) => match inner.as_ref() {
                        CrowType::NumArray(_) => CrowType::Result(inner),
                        _ => CrowType::Result(inner),
                    },
                    _ => CrowType::Result(Box::new(CrowType::Struct("CrowFindResult".to_string()))),
                }
            }
            // CLI / IO
            "args" => CrowType::Array(Box::new(CrowType::Str)),
            "input" => CrowType::Result(Box::new(CrowType::Str)),
            // File I/O
            "read" => CrowType::Result(Box::new(CrowType::Str)),
            "write" => CrowType::Result(Box::new(CrowType::Str)),
            // File system builtins
            "exists" => CrowType::Bool,
            "mkdir" => CrowType::Result(Box::new(CrowType::Str)),
            "ls" => CrowType::Result(Box::new(CrowType::Array(Box::new(CrowType::Str)))),
            "rm" => CrowType::Result(Box::new(CrowType::Str)),
            "mv" => CrowType::Result(Box::new(CrowType::Str)),
            "cp" => CrowType::Result(Box::new(CrowType::Str)),
            // System builtins
            "rand" => {
                if args.is_empty() { CrowType::Float } else { CrowType::Int }
            }
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
            // JSON builtins
            "json" => CrowType::Result(Box::new(CrowType::Json)),
            "encode" => CrowType::Result(Box::new(CrowType::Str)),
            // Web builtins (available when `use web` is active)
            "route" => CrowType::WebApp,
            "serve" => CrowType::Void,
            "respond" => CrowType::WebResponse,
            "redirect" => CrowType::WebResponse,
            "html" => CrowType::WebResponse,
            "text" => CrowType::WebResponse,
            // Web.db builtins
            "sqlite" => CrowType::WebDb,
            "exec" => CrowType::Void,
            "query" => CrowType::Array(Box::new(CrowType::Json)),
            // Web.auth builtins
            "sign" => CrowType::Str,
            "verify" => CrowType::Json,
            // Reactive data sources (UI)
            "poll" | "fetch" => CrowType::Frame,
            "today" => CrowType::Str,
            // Autograd
            "grad" => {
                // grad[f] returns a function with the same arity as f, but all params are Float
                // and return is Float (1 param) or Array<Float> (N params)
                if let Some(Expr::Ident(fn_name)) = args.first() {
                    if let Some(sig) = self.functions.get(fn_name) {
                        let n = sig.params.len();
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
                // Check variable type: could be a function call, array/map indexing, etc.
                if let Some(var_type) = self.vars.get(name) {
                    // Unwrap Result for indexing purposes
                    let unwrapped = match var_type {
                        CrowType::Result(inner) => inner.as_ref(),
                        other => other,
                    };
                    match unwrapped {
                        CrowType::Fn(_, ret) => return *ret.clone(),
                        CrowType::Array(inner) => return *inner.clone(),
                        CrowType::NumArray(inner) => return *inner.clone(),
                        CrowType::Map(inner) => return *inner.clone(),
                        CrowType::Str => return CrowType::Str,
                        // a cell of a column whose type nothing told: dynamic
                        // until int[], float[] or str[] reads it (typed frames)
                        CrowType::FrameCol => return CrowType::Json,
                        _ => {}
                    }
                }
                CrowType::Unknown
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

    /// The type the database told us for a frame's column, `frame.col`:
    /// the type of each of its cells.
    pub(crate) fn column_type(&self, object: &Expr, field: &str) -> Option<CrowType> {
        let v = read_var(object)?;
        self.frame_schemas.get(v).and_then(|s| s.get(field)).cloned()
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
                let tensor_vs_scalar = (matches!(lt_inner, CrowType::NumArray(_))
                    && matches!(rt_inner, CrowType::Int | CrowType::Float))
                    || (matches!(rt_inner, CrowType::NumArray(_))
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
                    CrowType::NumArray(Box::new(CrowType::Float))
                } else if matches!(&lt_inner, CrowType::NumArray(_)) {
                    lt_inner.clone()
                } else if matches!(&rt_inner, CrowType::NumArray(_)) {
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
                } else if matches!(&lt_inner, CrowType::Sparse) && matches!(&rt_inner, CrowType::NumArray(_)) {
                    CrowType::NumArray(Box::new(CrowType::Float))
                // * on tensor-tensor is matmul (returns NumArray)
                } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                    CrowType::NumArray(Box::new(CrowType::Float))
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
                } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                    CrowType::NumArray(Box::new(CrowType::Float))
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
                } else if matches!(&lt_inner, CrowType::NumArray(_)) || matches!(&rt_inner, CrowType::NumArray(_)) {
                    CrowType::NumArray(Box::new(CrowType::Float))
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
        let skip = matches!(name, "log" | "assert" | "type" | "cpu" | "gpu" | "error" | "table" | "chart" | "__known")
            || self.structs.contains_key(name)
            || crate::lower::UI_ELEMENTS.contains(&name)
            || (self.in_view && crate::lower::is_ui_control(name, matches!(arg_types.first().cloned().map(Self::walked), Some(CrowType::Frame))))
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

    /// An index's type (`xs[i]`, `m[k]`, a function value applied) from the
    /// indexed value's type and the first index's.
    pub(crate) fn index_type(&self, callee_type: CrowType, index: Option<CrowType>) -> CrowType {
        match callee_type {
            CrowType::Array(inner) => *inner,
            CrowType::NumArray(inner) => {
                // Advanced indexing: tensor[tensor] → tensor (gather)
                if matches!(index, Some(CrowType::NumArray(_))) {
                    return CrowType::NumArray(inner);
                }
                *inner
            }
            // a cell of a column whose type nothing told: dynamic until
            // int[], float[] or str[] reads it (typed frames)
            CrowType::FrameCol => CrowType::Json,
            CrowType::Map(inner) => *inner,
            CrowType::Json => CrowType::Json,
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
        let is_numarray = matches!(obj_type, CrowType::NumArray(_))
            || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::NumArray(_)));
        if is_numarray {
            match field {
                "shape" => return CrowType::Array(Box::new(CrowType::Int)),
                "dtype" | "device" | "grad" => return CrowType::Str,
                "len" => return CrowType::Int,
                _ => {}
            }
        }
        // Frame properties
        let is_frame = matches!(obj_type, CrowType::Frame)
            || matches!(&obj_type, CrowType::Result(inner) if matches!(inner.as_ref(), CrowType::Frame));
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
    /// nested numeric rows a tensor of higher rank.
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
        // Nested numeric literals build higher-rank tensors: the inner
        // type of a rank-2 tensor is the rank-1 row tensor.
        if matches!(inner, CrowType::Int | CrowType::Float) {
            CrowType::NumArray(Box::new(inner))
        } else if matches!(inner, CrowType::NumArray(_)) {
            CrowType::NumArray(Box::new(inner))
        } else {
            CrowType::Array(Box::new(inner))
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
