//! Expressions.

use super::*;

/// A string as a Rust literal: `"…".to_string()`.
pub(crate) fn rust_str_lit(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    format!("\"{}\".to_string()", out)
}

/// A string as a format string piece: braces doubled, escapes kept.
fn fmt_piece(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// The builtins whose own value is a result, whatever their arguments.
const FALLIBLE_BUILTINS: &[&str] = &[
    "error", "hash", "shell", "request", "find", "input", "read", "write", "mkdir", "ls", "rm", "mv", "cp", "captured", "moment", "env", "json", "encode", "mail", "query",
    "sign", "verify", "socket", "send", "recv",
    // (`query`'s writing twin answers a result as `query` does)
    "__query_write",
    // (db's `describe` answers its frame or why not: inside a function, where
    // the connection can fail, it had been wrapped twice and not built —
    // crow check --self's live cases)
    "describe",
];

impl Emitter {
    pub(crate) fn expr(&mut self, e: &Expr) -> String {
        // an argument a call bound before checking its arguments (its place
        // among them kept) is read by the name it is bound to, however a
        // builtin prints it
        if !self.unwrap_as.is_empty()
            && !matches!(e, Expr::Convert { how: Convert::Unwrap, .. })
            && let Some(bound) = self.unwrap_as.get(&(e as *const Expr as usize))
        {
            return bound.clone();
        }
        match e {
            Expr::Const(c, _) => self.constant(c),
            Expr::Local(v, _) | Expr::Move(v, _) => self.local(v),
            Expr::Load(f, _) => self.load(f),
            Expr::Derived(d, _) => {
                if self.declared(d) {
                    d.clone()
                } else if self.in_main {
                    // the top level, once, as the program starts: the
                    // derivation over the facts as declared
                    format!("__derive_{}(&__crow_state)", d)
                } else {
                    format!("__derive_{}(state)", d)
                }
            }
            Expr::Constant(name, _) => {
                let bare = name.rsplit('.').next().unwrap_or(name);
                match bare {
                    "pi" => "std::f64::consts::PI".to_string(),
                    "e" => "std::f64::consts::E".to_string(),
                    "inf" => "f64::INFINITY".to_string(),
                    "nan" => "f64::NAN".to_string(),
                    _ => name.replace('.', "_"),
                }
            }
            // (a function of a group — a list of several signatures — is
            // its name as text, compiled: described and called by name)
            Expr::FnRef(name, Ty::Fns(_)) => format!("{:?}.to_string()", crate::lower::shown(name)),
            Expr::FnRef(name, _) => self.fn_ref(name),
            Expr::Call { .. } | Expr::Effect { .. } => self.call(e),
            Expr::Convert { how, expr, ty } => match how {
                // an int held as a growing one, or a growing one handed where
                // 64 bits are taken (refused past them, as the interpreter
                // refuses it): item by item in a list or a map
                // (a value that can fail grows as its payload, its failure
                // kept — the value itself, a list's or a map's items)
                Convert::ToBig => grown(&self.tt(expr), &self.owned_expr(expr)),
                // an int, or a float literal, as the decimal it is; an int or a
                // decimal as the fraction it is (a value that can fail
                // converts as its payload)
                Convert::ToDecimal | Convert::ToFraction => {
                    let inner = self.owned_expr(expr);
                    let from = Self::inner(&self.tt(expr));
                    let one = |v: &str| -> String {
                        match (how, &from) {
                            (Convert::ToDecimal, Ty::Int) => format!("crow_runtime::decimal::Dec::of_int(&crow_runtime::int::Int::from({}))", v),
                            (Convert::ToDecimal, Ty::Big) => format!("crow_runtime::decimal::Dec::of_int(&({}))", v),
                            (Convert::ToDecimal, Ty::Float) => format!("crow_runtime::decimal::of_literal({})", v),
                            (Convert::ToFraction, Ty::Int) => format!("crow_runtime::fraction::Frac::of_int(&crow_runtime::int::Int::from({}))", v),
                            (Convert::ToFraction, Ty::Big) => format!("crow_runtime::fraction::Frac::of_int(&({}))", v),
                            (Convert::ToFraction, Ty::Decimal) => format!("crow_runtime::fraction::Frac::of_dec(&({}))", v),
                            _ => v.to_string(),
                        }
                    };
                    if matches!(ty, Ty::Result(_)) && matches!(self.tt(expr), Ty::Result(_)) {
                        format!("{{ let _r = &({}); CrowResult {{ ok: _r.ok, value: {}, reason: _r.reason.clone(), kind: _r.kind.clone(), trace: _r.trace.clone(), captured: _r.captured }} }}", inner, one("_r.value"))
                    } else {
                        one(&inner)
                    }
                }
                // arithmetic of ints proven within 64 bits: the plain operator
                // (no overflow branch, so a loop's arithmetic vectorizes)
                Convert::Proven => match Emitter::raw(expr) {
                    Expr::Binary { op, l, r, .. } if matches!(op.as_str(), "add" | "sub" | "mul") => {
                        let (a, b) = (self.operand(l), self.operand(r));
                        let sym = match op.as_str() {
                            "add" => "+",
                            "sub" => "-",
                            _ => "*",
                        };
                        format!("({} {} {})", a, sym, b)
                    }
                    Expr::Unary { op, expr: x, .. } if op == "neg" => format!("(-{})", self.operand(x)),
                    other => self.expr(other),
                },
                // (a value that can fail keeps its failure, a list's or a
                // map's items each; a failed one's placeholder holds no
                // number past 64 bits)
                Convert::ToSmall(who) => {
                    let inner = self.expr(expr);
                    shrunk(&self.tt(expr), &inner, &who.text(), &self.line_now())
                }
                Convert::IntToFloat => {
                    let inner = self.expr(expr);
                    // a dual number is the float already (the growing whole
                    // number `floor` of one answers is the dual's own floor)
                    if self.emitting_dual {
                        return inner;
                    }
                    // (and so is the traced number `floor` of a traced one is)
                    if self.emitting_traced && self.traced_whole(expr) {
                        return inner;
                    }
                    // an int that grows: its nearest float
                    let big = matches!(Self::inner(&self.tt(expr)), Ty::Big);
                    if big && matches!(ty, Ty::Result(_)) {
                        return format!("{{ let _r = &({}); CrowResult {{ ok: _r.ok, value: (_r.value).to_f64(), reason: _r.reason.clone(), kind: _r.kind.clone(), trace: _r.trace.clone(), captured: _r.captured }} }}", inner);
                    }
                    if big {
                        return format!("({}).to_f64()", inner);
                    }
                    if self.emitting_traced {
                        format!("crow_runtime::autograd::Traced::number(({}) as f64)", inner)
                    } else if matches!(ty, Ty::Result(_)) {
                        // a value that can fail, converted as one: its payload
                        // converted, its failure kept (a conversion typed plain
                        // reads the payload the propagation unwrapped)
                        format!("{{ let _r = {}; CrowResult {{ ok: _r.ok, value: (_r.value as f64), reason: _r.reason, kind: _r.kind, trace: _r.trace, captured: _r.captured }} }}", inner)
                    } else {
                        format!("(({}) as f64)", inner)
                    }
                }
                Convert::IntsToFloats => {
                    let inner = self.owned_expr(expr);
                    // numbers held as a tensor are floats already
                    if matches!(Self::inner(&self.tt(expr)), Ty::Tensor(..)) {
                        return inner;
                    }
                    // (a map keeps its keys' kind: one keyed by ints had been
                    // collected keyed by text — crow check --self)
                    let map = match Self::inner(&self.tt(expr)) {
                        Ty::Map(crate::ir::Key::Int, v) if matches!(v.as_ref(), Ty::Big) => "into_iter().map(|(k, v)| (k, v.to_f64())).collect::<std::collections::BTreeMap<i64, f64>>()",
                        Ty::Map(crate::ir::Key::Int, _) => "into_iter().map(|(k, v)| (k, v as f64)).collect::<std::collections::BTreeMap<i64, f64>>()",
                        Ty::Map(_, v) if matches!(v.as_ref(), Ty::Big) => "into_iter().map(|(k, v)| (k, v.to_f64())).collect::<std::collections::BTreeMap<String, f64>>()",
                        Ty::Map(..) => "into_iter().map(|(k, v)| (k, v as f64)).collect::<std::collections::BTreeMap<String, f64>>()",
                        Ty::List(i) if matches!(i.as_ref(), Ty::Big) => "into_iter().map(|v| v.to_f64()).collect::<Vec<f64>>()",
                        _ => "into_iter().map(|v| v as f64).collect::<Vec<f64>>()",
                    };
                    if matches!(ty, Ty::Result(_)) {
                        format!("{{ let _r = {}; CrowResult {{ ok: _r.ok, value: _r.value.{}, reason: _r.reason, kind: _r.kind, trace: _r.trace, captured: _r.captured }} }}", inner, map)
                    } else {
                        format!("({}).{}", inner, map)
                    }
                }
                Convert::Unwrap => {
                    if let Some(bound) = self.unwrap_as.get(&(e as *const Expr as usize)) {
                        return bound.clone();
                    }
                    let inner = self.expr(expr);
                    if matches!(self.tt(expr), Ty::Result(_)) {
                        format!("({}).value", inner)
                    } else {
                        inner
                    }
                }
                // a value a guard made known good: its payload, copied out
                // unless this is the variable's last read (a move)
                Convert::Known => {
                    let inner = self.expr(expr);
                    match self.tt(expr) {
                        Ty::Result(p) => {
                            // (a last read moves the payload only out of a local the
                            // function owns: a loop's borrowed item is copied)
                            let reread = matches!(expr.as_ref(), Expr::Local(..) | Expr::Derived(..) | Expr::Load(..)) || (matches!(expr.as_ref(), Expr::Move(..)) && !self.moved_local(expr));
                            if reread && !Self::is_copy_type(&p) {
                                format!("({}).value.clone()", inner)
                            } else {
                                format!("({}).value", inner)
                            }
                        }
                        _ => inner,
                    }
                }
                // a tensor's numbers as a list's items, ints where the list
                // holds ints; a list's numbers as a tensor of one axis
                Convert::ToList => {
                    let inner = self.expr(expr);
                    let ints = matches!(ty.payload(), Ty::List(e) if **e == Ty::Int);
                    let as_list = if ints { ".to_vec().into_iter().map(|x| x as i64).collect::<Vec<i64>>()" } else { ".to_vec()" };
                    // (a tensor that can fail: its payload converted, its
                    // failure kept — it had been left a tensor in a result
                    // of a list, which did not build)
                    if matches!(ty, Ty::Result(_)) && matches!(self.tt(expr), Ty::Result(p) if matches!(p.as_ref(), Ty::Tensor(..))) {
                        return format!("{{ let _r = {}; CrowResult {{ ok: _r.ok, value: (_r.value){}, reason: _r.reason, kind: _r.kind, trace: _r.trace, captured: _r.captured }} }}", inner, as_list);
                    }
                    if !matches!(self.tt(expr), Ty::Tensor(..)) {
                        return inner;
                    }
                    format!("({}){}", inner, as_list)
                }
                Convert::ToTensor => {
                    // (an empty list literal where a tensor is held — a map's
                    // value, a list's item — is the empty tensor: `vec![]`
                    // names no item type to map over)
                    if matches!(Emitter::raw(expr), Expr::ListLit(items, _) if items.is_empty()) {
                        self.needs_crow_runtime = true;
                        return "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])".to_string();
                    }
                    let inner = self.expr(expr);
                    match self.tt(expr) {
                        // a mask of a matrix's shape (its rows of bools, deeper
                        // for more axes): a matrix of 1 and 0, or the reason
                        // its rows are not one length — the interpreter's words
                        t @ Ty::List(_) if t.truths_depth() >= 2 => {
                            self.needs_crow_runtime = true;
                            format!("crow_runtime::mask::tensor_of(&({})){}", inner, self.or_stop())
                        }
                        // numbers that can fail: a failed one stops where
                        // they are read, with the interpreter's words (a
                        // missing cell's, or the item's reason)
                        Ty::List(item) if matches!(item.as_ref(), Ty::Result(_)) => {
                            self.needs_crow_runtime = true;
                            format!(
                                "{{ let _xs = &({}); let mut _v: Vec<f64> = Vec::with_capacity(_xs.len()); for (_i, _x) in _xs.iter().enumerate() {{ if !_x.ok {{ crow_stop({}, if _x.kind == \"missing\" {{ crow_runtime::stops::missing_cell(_i) }} else {{ format!(\"item {{}} of the list failed: {{}}\", _i, _x.reason) }}) }} _v.push(_x.value as f64); }} let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}",
                                inner, self.line_now()
                            )
                        }
                        // a list of rows is a matrix: rows of one shape, or the
                        // reason (the interpreter's words)
                        Ty::List(item) if matches!(item.as_ref(), Ty::Tensor(..)) => {
                            self.needs_crow_runtime = true;
                            format!("crow_runtime::tensor::CrowTensor::from_elements(&({})){}", inner, self.or_stop())
                        }
                        // a list of lists of numbers is a matrix as a list of
                        // rows is: each list a row, every row one length, or
                        // the reason (the interpreter's words)
                        Ty::List(item) if matches!(item.as_ref(), Ty::List(e) if matches!(e.as_ref(), Ty::Int | Ty::Float | Ty::Big)) => {
                            self.needs_crow_runtime = true;
                            let each = if matches!(item.as_ref(), Ty::List(e) if **e == Ty::Big) { "x.to_f64()" } else { "*x as f64" };
                            format!(
                                "{{ let _rows: Vec<crow_runtime::tensor::CrowTensor> = ({}).iter().map(|r| {{ let _v: Vec<f64> = r.iter().map(|x| {}).collect(); let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}).collect(); crow_runtime::tensor::CrowTensor::from_elements(&_rows){} }}",
                                inner, each, self.or_stop()
                            )
                        }
                        // a mask where numbers are taken: true 1, false 0
                        Ty::List(item) if *item == Ty::Bool => {
                            self.needs_crow_runtime = true;
                            format!("{{ let _v: Vec<f64> = ({}).iter().map(|x| if *x {{ 1.0 }} else {{ 0.0 }}).collect(); let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}", inner)
                        }
                        // whole numbers that grow: each its nearest float
                        Ty::List(item) if *item == Ty::Big => {
                            self.needs_crow_runtime = true;
                            format!("{{ let _v: Vec<f64> = ({}).iter().map(|x| x.to_f64()).collect(); let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}", inner)
                        }
                        Ty::List(_) => {
                            self.needs_crow_runtime = true;
                            format!("{{ let _v: Vec<f64> = ({}).iter().map(|x| *x as f64).collect(); let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}", inner)
                        }
                        // a frame's column where numbers are taken: its
                        // numbers, or the reason it has none (a missing cell,
                        // text) — the rule the interpreter reads its cells by
                        Ty::Column(_) => {
                            self.needs_crow_runtime = true;
                            format!("({}).numbers(){}", inner, self.or_stop())
                        }
                        _ => inner,
                    }
                }
            },
            Expr::Binary { op, l, r, .. } => self.binary(op, l, r),
            Expr::Unary { op, expr, .. } => self.unary(op, expr),
            Expr::Index { base, indices, .. } => self.index(base, indices, e),
            Expr::Field { base, name, .. } => self.field(base, name),
            Expr::Meta { base, which } => self.meta(base, *which),
            Expr::Interp(pieces) => self.interp(pieces),
            Expr::ListLit(items, ty) => self.list_lit(items, ty),
            Expr::MapLit(pairs, ty) => self.map_lit(pairs, ty),
            Expr::RecordLit { name, fields } => self.record_lit(name, fields),
            Expr::Tree { element, attrs, children } => self.tree(element, attrs, children),
            Expr::ForExpr { var, iter, body, parallel, .. } => self.for_expr(var, iter, body, *parallel, e),
            Expr::Deferred { name, .. } => format!("compile_error!(\"a held call to '{}' outside an event attribute\")", name),
        }
    }

    fn constant(&self, c: &Const) -> String {
        match c {
            Const::Int(n) => {
                if self.emitting_dual {
                    format!("CrowDual::from({}_i64)", n)
                } else {
                    format!("{}_i64", n)
                }
            }
            Const::Float(f) => {
                let s = format!("{}", f);
                let lit = if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
                    if s == "inf" {
                        "f64::INFINITY".to_string()
                    } else if s == "-inf" {
                        "f64::NEG_INFINITY".to_string()
                    } else if s == "NaN" {
                        "f64::NAN".to_string()
                    } else if s.contains('e') && !s.contains('.') {
                        format!("{}_f64", s.replacen('e', ".0e", 1))
                    } else {
                        format!("{}_f64", s)
                    }
                } else {
                    format!("{}.0_f64", s)
                };
                if self.emitting_dual {
                    format!("CrowDual::from({})", lit)
                } else if self.emitting_traced {
                    format!("crow_runtime::autograd::Traced::number({})", lit)
                } else {
                    lit
                }
            }
            Const::Bool(b) => b.to_string(),
            Const::Str(s) => rust_str_lit(s),
            Const::Unit => "()".to_string(),
        }
    }

    fn local(&self, v: &str) -> String {
        if self.plain_vecs.contains(v) {
            return self.vector_as_tensor(v);
        }
        // a list, a map or a text the function reads where its caller
        // holds it: the caller's place
        if self.lent_here.contains(v) {
            return format!("(*{})", v);
        }
        if self.static_handle_vars.contains(v) && !self.declared(v) {
            return format!("__crow_handle_{}.get().unwrap().clone()", v);
        }
        v.to_string()
    }

    fn load(&self, f: &str) -> String {
        if self.static_handle_vars.contains(f) {
            return format!("__crow_handle_{}.get().unwrap().clone()", f);
        }
        // a fact of the session, read in place where the body reads it
        if self.facts_in_state && self.reactive_vars.contains_key(f) {
            return format!("__facts(&*state).{}", Self::rust_field(f));
        }
        f.to_string()
    }

    fn fn_ref(&self, name: &str) -> String {
        let specialized = self.fns.keys().any(|k| k.rsplit_once("__").map_or(false, |(b, s)| b == name && s.chars().all(|c| c.is_ascii_digit())));
        if specialized {
            return format!(
                "compile_error!(\"'{}' is used at multiple type signatures and cannot be passed as a value — a function value has one type. Wrap it in a single-type function\")",
                name
            );
        }
        Self::rust_fn_name(name)
    }

    /// An expression as an owned value: a non-copy variable read where a
    /// value is kept (an assignment, a list, a return) is cloned, so the
    /// variable stays usable; a by-reference parameter too.
    /// A variable read, or a field of one at any depth (`h.walk.past`):
    /// what a value taken from it must copy, since the variable owns it.
    pub(crate) fn place_read(e: &Expr) -> bool {
        match Emitter::raw(e) {
            Expr::Local(..) | Expr::Move(..) | Expr::Load(..) => true,
            Expr::Field { base, .. } => Emitter::place_read(base),
            _ => false,
        }
    }

    pub(crate) fn owned_expr(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        // a field of a local is copied out of it (`c = g.last`): a struct
        // parameter is borrowed, and a local read again later still owns it;
        // a field that can hold a failed value is never Copy
        if let Expr::Field { base, .. } = e {
            if Emitter::place_read(base) {
                let t = self.tt(e);
                let non_copy = matches!(t, Ty::Result(_))
                    || matches!(Self::inner(&t), Ty::Tensor(..) | Ty::List(_) | Ty::Map(..) | Ty::Str | Ty::Fns(_) | Ty::Markup | Ty::Frame(_) | Ty::Record(_) | Ty::OptRecord(_) | Ty::Json | Ty::Big | Ty::Decimal | Ty::Fraction);
                if non_copy && !s.ends_with(".clone()") {
                    return format!("{}.clone()", s);
                }
            }
        }
        if let Expr::Local(name, ty) | Expr::Move(name, ty) = e {
            // (a plain vector used as a value is a tensor made for it)
            if self.plain_vecs.contains(name.as_str()) {
                return s;
            }
            if self.ref_params.contains(name) {
                return format!("{}.clone()", s);
            }
            if self.static_handle_vars.contains(name) && !self.declared(name) {
                return s;
            }
            if self.moved_local(e) {
                return s;
            }
            let mut t = self.local_ty(name).unwrap_or_else(|| as_inferred(ty));
            if matches!(t, Ty::Unknown) && self.reactive_event_handlers.contains(&self.cur_fn) {
                t = Ty::Str;
            }
            // (a column, a sparse matrix and a handle too: `b = a` of one,
            // then `a` read again, had moved it — crow check --self)
            let non_copy = matches!(
                t.text(),
                Ty::Str
                    | Ty::Fns(_)
                    | Ty::Markup
                    | Ty::List(_)
                    | Ty::Tensor(..)
                    | Ty::Map(..)
                    | Ty::Record(_)
                    | Ty::OptRecord(_)
                    | Ty::Frame(_)
                    | Ty::Column(_)
                    | Ty::Sparse
                    | Ty::Json
                    | Ty::Bytes
                    | Ty::Handle(_)
                    | Ty::Result(_)
                    | Ty::Big
                    | Ty::Decimal
                    | Ty::Fraction
            );
            if non_copy {
                return format!("{}.clone()", s);
            }
        }
        // a derivation read at the top of the function is a local too; read
        // anywhere else it is a fresh value
        let local_read = match e {
            Expr::Load(name, ty) => Some((name, ty)),
            Expr::Derived(name, ty) if self.declared(name) => Some((name, ty)),
            _ => None,
        };
        if let Some((name, ty)) = local_read {
            if self.static_handle_vars.contains(name) {
                return s;
            }
            let t = self.local_ty(name).unwrap_or_else(|| as_inferred(ty));
            if matches!(t.text(), Ty::Str | Ty::List(_) | Ty::Tensor(..) | Ty::Map(..) | Ty::Record(_) | Ty::Frame(_) | Ty::Json | Ty::Bytes | Ty::Handle(_) | Ty::Result(_) | Ty::Big | Ty::Decimal | Ty::Fraction) {
                return format!("{}.clone()", s);
            }
        }
        s
    }

    /// The collection argument of an in-place builtin: moved when it is the
    /// variable this statement assigns, cloned otherwise.
    pub(crate) fn moved_or_cloned(&self, arg: &Expr, s: &str) -> String {
        let raw = Emitter::raw(arg);
        if matches!(raw, Expr::Local(n, _) | Expr::Move(n, _) if self.move_target.as_deref() == Some(n.as_str())) || self.moved_local(arg) {
            s.to_string()
        } else if Emitter::fresh_value(arg) {
            // a value an expression makes is the call's own: nothing to copy
            s.to_string()
        } else {
            format!("{}.clone()", s)
        }
    }

    /// Whether an expression makes a value of its own (a call's answer, a
    /// literal list, an interpolation, an item copied out): nothing else
    /// holds it, so it can be moved where it is used. A name's value — a
    /// local, a fact, a derivation (a local at the top of a view), a field
    /// of one — is held, and is borrowed or copied.
    pub(crate) fn fresh_value(e: &Expr) -> bool {
        !matches!(Emitter::raw(e), Expr::Local(..) | Expr::Move(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Field { .. } | Expr::Constant(..) | Expr::FnRef(..) | Expr::Const(..) | Expr::Meta { .. })
    }

    /// A local read for the last time (the IR's `move`) that the function
    /// owns and holds by name: printed bare where a value is kept, it moves
    /// instead of being copied (`return out` had copied the list it
    /// returned). A parameter lent by the caller, a handle, a plain vector
    /// and a text builder are printed as something else, and copy.
    /// Whether a collecting loop may take what it walks: a local's last
    /// read the function owns, a list bound by value where a failed one was
    /// refused, or a list made where the loop starts (a call's answer, a
    /// literal).
    fn consumable(&self, e: &Expr) -> bool {
        self.moved_local(e)
            || matches!(Emitter::raw(e), Expr::Local(n, _) if self.owned_lists.contains(n))
            || matches!(Emitter::raw(e), Expr::Call { .. } | Expr::ListLit(..))
    }

    pub(crate) fn moved_local(&self, e: &Expr) -> bool {
        match Emitter::raw(e) {
            Expr::Move(n, _) => {
                self.declared(n)
                    && !self.borrowed(n)
                    && !self.ref_params.contains(n)
                    && !self.lent_here.contains(n)
                    && !self.static_handle_vars.contains(n)
                    && !self.plain_vecs.contains(n)
                    && !self.text_builders.contains_key(n)
            }
            _ => false,
        }
    }

    // ── operators ───────────────────────────────────────────────────

    fn operand(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        // a block (a cell read, an index with its bounds) is parenthesised
        // too: at the start of a statement Rust reads `{ … } > 1.0` as a
        // block and a stray operator
        if matches!(e, Expr::Binary { .. }) || s.starts_with('{') {
            format!("({})", s)
        } else {
            s
        }
    }

    /// A tensor, or a list whose items are numbers or lists of numbers.
    fn holds_numbers(t: &Ty) -> bool {
        match t {
            Ty::Tensor(..) => true,
            Ty::List(inner) => matches!(inner.as_ref(), Ty::Int | Ty::Float) || (matches!(inner.as_ref(), Ty::List(_) | Ty::Tensor(..)) && Self::holds_numbers(inner)),
            _ => false,
        }
    }

    /// Two operands that hold numbers in different Rust types.
    fn numbers_mix(a: &Ty, b: &Ty) -> bool {
        Self::holds_numbers(a) && Self::holds_numbers(b) && a != b
    }

    pub(crate) fn binary(&mut self, op: &str, left: &Expr, right: &Expr) -> String {
        // an int made a float for a float operand: when the int can fail, the
        // operator unwraps it and converts the value
        let strip = |e: &Expr, me: &Emitter| -> Expr {
            match e {
                Expr::Convert { how: Convert::IntToFloat, expr, .. } if matches!(me.tt(expr), Ty::Result(_)) => expr.as_ref().clone(),
                other => other.clone(),
            }
        };
        let left = &strip(left, self);
        let right = &strip(right, self);
        if self.emitting_traced {
            if let Some(s) = self.traced_binary(op, left, right) {
                return s;
            }
        }
        if matches!(op, "eq" | "ne") {
            if let Some(s) = self.optbox_comparison(left, op, right) {
                return s;
            }
            // a slice of text compared, where it lies (texts.rs)
            if let Some(s) = self.slice_compared(left, right).or_else(|| self.slice_compared(right, left)) {
                return if op == "ne" { format!("!{}", s) } else { s };
            }
            // a character of text compared, where it lies (the other side
            // first only where reading it does nothing: a name, a literal)
            let plain = |e: &Expr| matches!(e, Expr::Const(..) | Expr::Local(..));
            if let Some(s) = self.char_compared(left, right).or_else(|| if plain(left) { self.char_compared(right, left) } else { None }) {
                return if op == "ne" { format!("(!{})", s) } else { s };
            }
        }
        let l = self.operand(left);
        let r = self.operand(right);
        let lt = self.tt(left);
        let rt = self.tt(right);
        let l_res = matches!(lt, Ty::Result(_));
        let r_res = matches!(rt, Ty::Result(_));
        let lti = Self::inner(&lt);
        let rti = Self::inner(&rt);
        let op_str = match op {
            "add" => "+",
            "sub" => "-",
            "mul" => "*",
            "div" => "/",
            "mod" => "%",
            "pow" => "pow",
            "elemmul" => ".*",
            "elemdiv" => "./",
            "elempow" => ".^",
            "eq" => "==",
            "ne" => "!=",
            "gt" => ">",
            "lt" => "<",
            "gte" => ">=",
            "lte" => "<=",
            // (a mask's items, joined item by item)
            "and" => "&",
            "or" => "|",
            other => other,
        };
        // `and`/`or` of truths: the right side only where the left does not
        // decide (spec, Operators), an error on the left the value
        if matches!(op, "and" | "or") && crate::ir::short_circuits(left, right) && lti == Ty::Bool && rti == Ty::Bool {
            let and = op == "and";
            if !l_res && !r_res {
                return format!("(({}) {} ({}))", l, if and { "&&" } else { "||" }, r);
            }
            let ok = |v: &str| format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", v);
            let c = self.binop_counter;
            self.binop_counter += 1;
            let (bind, lv) = if l_res { (format!("let _scl_{c} = &({});", l), format!("_scl_{c}.value")) } else { (format!("let _scl_{c}: bool = {};", l), format!("_scl_{c}")) };
            let decided = ok(if and { "false" } else { "true" });
            let rest = if r_res { format!("({}).clone()", r) } else { ok(&r) };
            let fail = if l_res {
                format!("if !_scl_{c}.ok {{ CrowResult {{ ok: false, value: false, reason: _scl_{c}.reason.clone(), kind: _scl_{c}.kind.clone(), trace: _scl_{c}.trace.clone(), captured: 0.0 }} }} else ")
            } else {
                String::new()
            };
            let test = if and { format!("!{}", lv) } else { lv };
            return format!("{{ {} {}if {} {{ {} }} else {{ {} }} }}", bind, fail, test, decided, rest);
        }
        if l_res || r_res {
            // an operator's value is its failed operand, unchanged; both
            // operands are evaluated, the left first, and the left's error
            // decides before the right's, as in the interpreter
            let ctr = self.binop_counter;
            self.binop_counter += 1;
            let l_val = if l_res { format!("_binl_{}.value", ctr) } else { format!("(*_binl_{})", ctr) };
            let r_val = if r_res { format!("_binr_{}.value", ctr) } else { format!("(*_binr_{})", ctr) };
            let mask = self.mask(op, &l_val, &lti, &r_val, &rti);
            let compared = mask.clone().or_else(|| self.comparison(op, &l_val, &lti, &r_val, &rti));
            // (a tensor compared with a number is a mask, as tensor_arith
            // prints it: a failed number beside one had answered one bool's
            // default, and the branches had not agreed — crow check --self)
            let tensor_and_number = |a: &Ty, b: &Ty| matches!(a, Ty::Tensor(..)) && matches!(b, Ty::Int | Ty::Float);
            let result_type = match op {
                _ if mask.is_some() => Ty::List(Box::new(Ty::Bool)),
                // (of the tensor's shape, as `Ty::mask_of` says for the
                // lowering: a matrix's is its rows of bools)
                "eq" | "ne" | "gt" | "lt" | "gte" | "lte" if !Self::numbers_mix(&lti, &rti) && tensor_and_number(&lti, &rti) => lti.mask_of(),
                "eq" | "ne" | "gt" | "lt" | "gte" | "lte" if !Self::numbers_mix(&lti, &rti) && tensor_and_number(&rti, &lti) => rti.mask_of(),
                "and" | "or" if lti.is_truths() || rti.is_truths() => {
                    if lti.truths_depth() >= rti.truths_depth() { lti.clone() } else { rti.clone() }
                }
                "eq" | "ne" | "gt" | "lt" | "gte" | "lte" | "and" | "or" => Ty::Bool,
                "mod" if matches!(lti, Ty::Big) => Ty::Big,
                "pow" if matches!(lti, Ty::Decimal | Ty::Fraction) => lti.clone(),
                // (a growing int beside a float is its nearest float)
                _ if (matches!(lti, Ty::Big) && rti == Ty::Float) || (lti == Ty::Float && matches!(rti, Ty::Big)) => Ty::Float,
                "mod" => Ty::Int,
                // the seconds between two moments; a moment on or back
                "sub" if matches!((&lti, &rti), (Ty::Moment, Ty::Moment)) => Ty::Float,
                "add" | "sub" if matches!(lti, Ty::Moment) || matches!(rti, Ty::Moment) => Ty::Moment,
                // (arithmetic with a tensor on either side is a tensor: a
                // failed number beside one had answered a number's default,
                // and the two branches had not agreed — crow check --self)
                _ if matches!(lti, Ty::Tensor(..)) => lti.clone(),
                _ if matches!(rti, Ty::Tensor(..)) => rti.clone(),
                _ => lti.unify(&rti),
            };
            let default_val = result_type.default_rust_value();
            let inner_expr = match (&lti, &rti) {
                _ if compared.is_some() => compared.unwrap_or_default(),
                _ if matches!(op, "eq" | "ne") && Self::numbers_mix(&lti, &rti) => {
                    let not = if op == "ne" { "!" } else { "" };
                    format!("{}crow_runtime::tensor::numbers_equal(&{}, &{})", not, l_val, r_val)
                }
                _ if self.tensor_arith(op, &l_val, &lti, &r_val, &rti).is_some() => self.tensor_arith(op, &l_val, &lti, &r_val, &rti).unwrap_or_default(),
                // (a cast before `<` in its own parentheses: `x as f64 < y` reads
                // `f64<` as a type's arguments, and did not build)
                // (a power is the power arm's: `pow` is no Rust operator)
                (Ty::Int, Ty::Float) if op != "pow" => format!("(({}) as f64) {} {}", l_val, op_str, r_val),
                (Ty::Float, Ty::Int) if op != "pow" => format!("{} {} (({}) as f64)", l_val, op_str, r_val),
                // (a growing int beside a float: its nearest float)
                (Ty::Big, Ty::Float) if op != "pow" => format!("({}).to_f64() {} {}", l_val, op_str, r_val),
                (Ty::Float, Ty::Big) if op != "pow" => format!("{} {} ({}).to_f64()", l_val, op_str, r_val),
                (Ty::Int, Ty::Int) if self.int_op(op, &l_val, &r_val).is_some() => self.int_op(op, &l_val, &r_val).unwrap_or_default(),
                (Ty::Big, Ty::Big) if self.big_op(op, &l_val, &r_val).is_some() => self.big_op(op, &l_val, &r_val).unwrap_or_default(),
                _ if self.exact_op(op, &l_val, &lti, &r_val, &rti).is_some() => self.exact_op(op, &l_val, &lti, &r_val, &rti).unwrap_or_default(),
                _ if self.moment_op(op, &l_val, &lti, &r_val, &rti).is_some() => self.moment_op(op, &l_val, &lti, &r_val, &rti).unwrap_or_default(),
                _ if op == "pow" => {
                    let as_float = |v: &str, t: &Ty| if matches!(t, Ty::Big) { format!("({}).to_f64()", v) } else { format!("(({}) as f64)", v) };
                    if lti == Ty::Float || rti == Ty::Float {
                        format!("({}).powf({})", as_float(&l_val, &lti), as_float(&r_val, &rti))
                    } else {
                        format!("({}).pow(({}) as u32)", l_val, r_val)
                    }
                }
                _ => format!("{} {} {}", l_val, op_str, r_val),
            };
            // an operand that has no value (an `error[…]`) always fails:
            // its error is the operator's value, and there is no other
            let valueless = (l_res && matches!(lti, Ty::Unit)) || (r_res && matches!(rti, Ty::Unit));
            let mut result = if valueless {
                "unreachable!()".to_string()
            } else {
                format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", inner_expr)
            };
            if r_res {
                let err = format!(
                    "CrowResult {{ ok: false, value: {}, reason: _binr_{c}.reason.clone(), kind: _binr_{c}.kind.clone(), trace: _binr_{c}.trace.clone(), captured: 0.0 }}",
                    default_val,
                    c = ctr
                );
                result = format!("if !_binr_{c}.ok {{ {} }} else {{ {} }}", err, result, c = ctr);
            }
            if l_res {
                let err = format!(
                    "CrowResult {{ ok: false, value: {}, reason: _binl_{c}.reason.clone(), kind: _binl_{c}.kind.clone(), trace: _binl_{c}.trace.clone(), captured: 0.0 }}",
                    default_val,
                    c = ctr
                );
                result = format!("if !_binl_{c}.ok {{ {} }} else {{ {} }}", err, result, c = ctr);
            }
            return format!("{{ let _binl_{c} = &({}); let _binr_{c} = &({}); {} }}", l, r, result, c = ctr);
        }
        let l_sparse = matches!(lti, Ty::Sparse);
        let r_sparse = matches!(rti, Ty::Sparse);
        if l_sparse && r_sparse && op == "add" {
            return format!("{}.add(&{})", l, r);
        }
        if l_sparse && matches!(rti, Ty::Tensor(..)) && op == "mul" {
            return format!("{}.matmul_dense(&{}){}", l, r, self.or_stop());
        }
        // (a frame's column in arithmetic arrives as its numbers, converted
        // by the lowering the same way in both modes)
        // numbers held two ways (a list built by `push` against a literal's
        // tensor, a list of rows against a matrix) compare by value and
        // shape, as in the interpreter; both sides evaluated, the left first
        if matches!(op, "eq" | "ne") && Self::numbers_mix(&lti, &rti) {
            let not = if op == "ne" { "!" } else { "" };
            return format!("({}{{ let _l = &({}); let _r = &({}); crow_runtime::tensor::numbers_equal(_l, _r) }})", not, l, r);
        }
        let cmp = matches!(op, "gt" | "lt" | "gte" | "lte" | "eq" | "ne");
        if let Some(code) = self.tensor_arith(op, &l, &lti, &r, &rti) {
            return code;
        }
        let bool_list = |t: &Ty| matches!(t, Ty::List(i) if matches!(i.as_ref(), Ty::Bool));
        // masks of a matrix's shape joined truth by truth (of two shapes, the
        // reason, as the interpreter's), or each truth with one bool
        if matches!(op, "and" | "or") && (lti.truths_depth() >= 2 || rti.truths_depth() >= 2) {
            self.needs_crow_runtime = true;
            let both = op == "and";
            if lti.is_truths() && rti.is_truths() {
                return format!("crow_runtime::mask::joined(&({}), &({}), {}).unwrap_or_else(|e| crow_stop({}, e))", l, r, both, self.line_now());
            }
            if lti.is_truths() {
                return format!("{{ let _m = &({}); let _s: bool = {}; crow_runtime::mask::joined_with(_m, _s, {}) }}", l, r, both);
            }
            return format!("{{ let _s: bool = {}; crow_runtime::mask::joined_with(&({}), _s, {}) }}", l, r, both);
        }
        if matches!(op, "and" | "or") && bool_list(&lti) && bool_list(&rti) {
            return format!("{}.iter().zip({}.iter()).map(|(_a, _b)| *_a {} *_b).collect::<Vec<bool>>()", l, r, op_str);
        }
        // (a bool joined with a mask joins each item, as the interpreter
        // does: `true and f.on` had not built — crow check --self)
        if matches!(op, "and" | "or") && lti == Ty::Bool && bool_list(&rti) {
            return format!("{{ let _s: bool = {}; ({}).iter().map(|_b| _s {} *_b).collect::<Vec<bool>>() }}", l, r, op_str);
        }
        if matches!(op, "and" | "or") && bool_list(&lti) && rti == Ty::Bool {
            return format!("{{ let _m = &({}); let _s: bool = {}; _m.iter().map(|_a| *_a {} _s).collect::<Vec<bool>>() }}", l, r, op_str);
        }
        if cmp {
            if let Some(c) = self.comparison(op, &l, &lti, &r, &rti) {
                return c;
            }
        }
        if self.emitting_dual {
            if op == "pow" {
                return format!("({}).powf({})", l, r);
            }
            return format!("{} {} {}", l, op_str, r);
        }
        if matches!((&lti, &rti), (Ty::Int, Ty::Int)) {
            if let Some(checked) = self.int_op(op, &l, &r) {
                return checked;
            }
        }
        if matches!((&lti, &rti), (Ty::Big, Ty::Big)) {
            if let Some(code) = self.big_op(op, &l, &r) {
                return code;
            }
        }
        if let Some(code) = self.exact_op(op, &l, &lti, &r, &rti) {
            return code;
        }
        if let Some(code) = self.moment_op(op, &l, &lti, &r, &rti) {
            return code;
        }
        if op == "pow" {
            if lti == Ty::Float || rti == Ty::Float {
                return format!("(({}) as f64).powf(({}) as f64)", l, r);
            }
            return format!("({}).pow(({}) as u32)", l, r);
        }
        match (&lti, &rti) {
            (Ty::Int, Ty::Float) => format!("(({}) as f64) {} {}", l, op_str, r),
            (Ty::Float, Ty::Int) => format!("{} {} (({}) as f64)", l, op_str, r),
            _ => format!("{} {} {}", l, op_str, r),
        }
    }

    /// `t + s`, `s + t` and `t - s` of a moment and seconds: the moment on
    /// or back, or the program stops on the line where it lies past the
    /// moments one holds (the interpreter's ops call the same function).
    fn moment_op(&self, op: &str, l: &str, lti: &Ty, r: &str, rti: &Ty) -> Option<String> {
        let secs = |t: &Ty| matches!(t, Ty::Int | Ty::Float);
        let (t, s, f) = match (op, lti, rti) {
            ("add", Ty::Moment, s) if secs(s) => (l, r, "plus"),
            ("add", s, Ty::Moment) if secs(s) => (r, l, "plus"),
            ("sub", Ty::Moment, s) if secs(s) => (l, r, "less"),
            _ => return None,
        };
        Some(format!("crow_runtime::moments::{}(&({}), ({}) as f64){}", f, t, s, self.or_stop()))
    }

    /// Arithmetic and comparison with a tensor on either side, as the
    /// runtime does them: two tensors element by element (a shape
    /// mismatch stops the program with the runtime's words, never a bare
    /// panic), a tensor and a number through the tensor's own methods.
    /// None where neither side is a tensor. The same code serves an
    /// operand that can fail: its payload is the tensor (the fuzzer found
    /// `tensor + float[len[f]]` written as a bare `+`, seed 1791237824).
    /// The Rust type of a mask `depth` lists deep (`Vec<Vec<bool>>` for a
    /// matrix's).
    fn truths_type(depth: usize) -> String {
        (0..depth).fold("bool".to_string(), |t, _| format!("Vec<{}>", t))
    }

    fn tensor_arith(&self, op: &str, l: &str, lti: &Ty, r: &str, rti: &Ty) -> Option<String> {
        let op_str = match op {
            "add" => "+",
            "sub" => "-",
            "mul" => "*",
            "div" => "/",
            "mod" => "%",
            "eq" => "==",
            "ne" => "!=",
            "gt" => ">",
            "lt" => "<",
            "gte" => ">=",
            "lte" => "<=",
            other => other,
        };
        let l_t = matches!(lti, Ty::Tensor(..));
        let r_t = matches!(rti, Ty::Tensor(..));
        let cmp = matches!(op, "gt" | "lt" | "gte" | "lte" | "eq" | "ne");
        if l_t && r_t {
            return Some(match op {
                "add" => format!("{}.try_elementwise(&{}, \"+\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "sub" => format!("{}.try_elementwise(&{}, \"-\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "mul" => format!("{}.try_matmul(&{}).unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "div" => "compile_error!(\"use ./ for element-wise tensor division\")".to_string(),
                "elemmul" => format!("{}.try_elementwise(&{}, \".*\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "elemdiv" => format!("{}.try_elementwise(&{}, \"./\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "elempow" => format!("{}.try_elementwise(&{}, \".^\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.line_now()),
                "eq" => format!("{} == {}", l, r),
                "ne" => format!("{} != {}", l, r),
                _ => format!("{} {} {}", l, op_str, r),
            });
        }
        if l_t && matches!(rti, Ty::Int | Ty::Float) {
            return Some(match op {
                "add" => format!("{}.add_scalar({} as f64)", l, r),
                "sub" => format!("{}.sub_scalar({} as f64)", l, r),
                "mul" | "elemmul" => format!("{}.mul_scalar({} as f64)", l, r),
                "div" | "elemdiv" => format!("{}.div_scalar({} as f64)", l, r),
                "elempow" => format!("{}.map(|x| x.powf({} as f64))", l, r),
                // (a mask of the tensor's shape: a matrix's, its rows)
                _ if cmp && lti.mask_of().truths_depth() >= 2 => format!(
                    "{{ let _s = {} as f64; let _t = &({}); let _f: Vec<bool> = _t.to_vec().iter().map(|&_v| _v {} _s).collect(); crow_runtime::mask::shaped::<{}>(_f, _t.shape()) }}",
                    r,
                    l,
                    op_str,
                    Self::truths_type(lti.mask_of().truths_depth())
                ),
                _ if cmp => format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _v {} _s).collect::<Vec<bool>>() }}", r, l, op_str),
                _ => format!("{} {} {}", l, op_str, r),
            });
        }
        if r_t && matches!(lti, Ty::Int | Ty::Float) {
            if cmp && rti.mask_of().truths_depth() >= 2 {
                return Some(format!(
                    "{{ let _s = {} as f64; let _t = &({}); let _f: Vec<bool> = _t.to_vec().iter().map(|&_v| _s {} _v).collect(); crow_runtime::mask::shaped::<{}>(_f, _t.shape()) }}",
                    l,
                    r,
                    op_str,
                    Self::truths_type(rti.mask_of().truths_depth())
                ));
            }
            if cmp {
                return Some(format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _s {} _v).collect::<Vec<bool>>() }}", l, r, op_str));
            }
            return Some(match op {
                "add" => format!("{}.add_scalar({} as f64)", r, l),
                "sub" => format!("({}).neg().add_scalar({} as f64)", r, l),
                "mul" | "elemmul" => format!("{}.mul_scalar({} as f64)", r, l),
                "div" | "elemdiv" => format!("{}.scalar_div({} as f64)", r, l),
                // (a number raised to each of the tensor's numbers)
                "elempow" => format!("{{ let _s = {} as f64; {}.map(|x| _s.powf(x)) }}", l, r),
                _ => format!("{} {} {}", l, op_str, r),
            });
        }
        None
    }

    /// A mask: a frame's column, or a list of numbers, text or bools,
    /// compared with one value — each cell by the function the interpreter
    /// calls for its cells too (crow_runtime::mask), a missing one false,
    /// an order between kinds with none the reason the program stops with.
    /// The side written first is evaluated first. None when neither side
    /// is one.
    fn mask(&self, op: &str, l: &str, lt: &Ty, r: &str, rt: &Ty) -> Option<String> {
        let cmp = crow_runtime::mask::Cmp::named(op)?;
        let cell = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool | Ty::Moment);
        // a column (true) or a list of cells (false)
        let many = |t: &Ty| match t {
            Ty::Column(_) => Some(true),
            Ty::List(item) if cell(item) => Some(false),
            _ => None,
        };
        let call = |many: &str, column: bool, cmp: crow_runtime::mask::Cmp, value: &str| {
            if column {
                format!("({}).mask(crow_runtime::mask::Cmp::{:?}, crow_runtime::mask::Cell::from({})){}", many, cmp, value, self.or_stop())
            } else {
                format!("crow_runtime::mask::list(&{}, crow_runtime::mask::Cmp::{:?}, crow_runtime::mask::Cell::from({})){}", many, cmp, value, self.or_stop())
            }
        };
        match (many(lt), many(rt)) {
            (Some(column), None) if cell(rt) => Some(call(l, column, cmp, &format!("&({})", r))),
            (None, Some(column)) if cell(lt) => Some(format!("{{ let _v = &({}); {} }}", l, call(r, column, cmp.flipped(), "_v"))),
            _ => None,
        }
    }

    /// A comparison the runtime decides (crow_runtime::mask): a mask, a
    /// frame's column compared whole, or a parsed value compared with a
    /// value — none for one Rust's operators compute (two numbers, two texts,
    /// two bools; two values of two kinds the lowering refused).
    fn comparison(&self, op: &str, l: &str, lt: &Ty, r: &str, rt: &Ty) -> Option<String> {
        self.mask(op, l, lt, r, rt).or_else(|| self.whole(op, l, lt, r, rt)).or_else(|| self.json(op, l, lt, r, rt))
    }

    /// A frame's column compared whole (`eq`, `ne`) with a column, a list
    /// or a tensor: as two lists are, cell by cell, a missing cell equal to
    /// nothing, two kinds the reason (crow_runtime::mask), both sides
    /// evaluated, the left first. (Compiled, a column had equalled one with
    /// the same missing cells where the cells held text, and one compared
    /// with a list had not built.) None when neither side is a column
    /// compared whole.
    fn whole(&self, op: &str, l: &str, lt: &Ty, r: &str, rt: &Ty) -> Option<String> {
        if !matches!(op, "eq" | "ne") {
            return None;
        }
        let cmp = crow_runtime::mask::Cmp::named(op)?;
        let with = |t: &Ty| match t {
            Ty::Column(_) => Some("whole"),
            Ty::List(item) if matches!(item.as_ref(), Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool) => Some("whole_list"),
            Ty::Tensor(..) => Some("whole_tensor"),
            _ => None,
        };
        let (column, other, method) = match (lt, rt) {
            (Ty::Column(_), _) => ("_l", "_r", with(rt)?),
            (_, Ty::Column(_)) => ("_r", "_l", with(lt)?),
            _ => return None,
        };
        Some(format!("{{ let _l = &({}); let _r = &({}); {}.{}(crow_runtime::mask::Cmp::{:?}, {}){} }}", l, r, column, method, cmp, other, self.or_stop()))
    }

    /// A parsed value compared (`eq`, `ne`) with a number, text or a bool,
    /// or with another parsed value: compared when they are of one kind,
    /// the reason when they are not — their kinds show only as the program
    /// runs — by the function the interpreter calls too
    /// (crow_runtime::mask::json_holds, json_pair). (Compiled, JSON's own
    /// equality had answered `false` for a number and text; interpreted, a
    /// parsed `1` had not equalled `1.0`.)
    fn json(&self, op: &str, l: &str, lt: &Ty, r: &str, rt: &Ty) -> Option<String> {
        // (an order too: two parsed values order as the program runs, two
        // numbers or two texts, and the runtime stops on another pair)
        let cmp = crow_runtime::mask::Cmp::named(op)?;
        // (a moment too: a loaded column typed by its uses met a cell of a
        // column nothing typed, and the compiled program had not built)
        let cell = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Str | Ty::Markup | Ty::Bool | Ty::Moment);
        let call = match (lt, rt) {
            (Ty::Json, Ty::Json) => format!("crow_runtime::mask::json_pair(crow_runtime::mask::Cmp::{:?}, _l, _r)", cmp),
            (Ty::Json, t) if cell(t) => format!("crow_runtime::mask::json_holds(crow_runtime::mask::Cmp::{:?}, _l, crow_runtime::mask::Cell::from(_r))", cmp),
            (t, Ty::Json) if cell(t) => format!("crow_runtime::mask::json_holds(crow_runtime::mask::Cmp::{:?}, _r, crow_runtime::mask::Cell::from(_l))", cmp),
            _ => return None,
        };
        Some(format!("{{ let _l = &({}); let _r = &({}); {}{} }}", l, r, call, self.or_stop()))
    }

    /// An int operator that can fail — past 64 bits, a division by zero, a
    /// negative power — checked by the runtime, which has the words both
    /// modes stop with (crow_runtime::stops).
    /// Arithmetic and comparison of exact numbers — two decimals, two
    /// fractions (the lowering converted an int beside one), a power by an
    /// int — as the runtime computes them: a quotient's digits and a zero
    /// divisor stopping in its words. None for any other pair.
    fn exact_op(&self, op: &str, l: &str, lt: &Ty, r: &str, rt: &Ty) -> Option<String> {
        let line = self.line_now();
        let exact = matches!((lt, rt), (Ty::Decimal, Ty::Decimal) | (Ty::Fraction, Ty::Fraction));
        if op == "pow" && matches!(lt, Ty::Decimal | Ty::Fraction) {
            let e = match rt {
                Ty::Int => format!("crow_runtime::int::Int::from({})", r),
                Ty::Big => format!("({}).clone()", r),
                _ => return None,
            };
            return Some(format!("({}).pow(&{}).unwrap_or_else(|e| crow_stop({}, e))", l, e, line));
        }
        if !exact {
            return None;
        }
        Some(match op {
            "add" | "sub" | "mul" => format!("({}).{}(&({}))", l, op, r),
            "div" => format!("({}).div(&({})).unwrap_or_else(|e| crow_stop({}, e))", l, r, line),
            "eq" => format!("(({}).cmp(&({})) == std::cmp::Ordering::Equal)", l, r),
            "ne" => format!("(({}).cmp(&({})) != std::cmp::Ordering::Equal)", l, r),
            "gt" => format!("(({}).cmp(&({})) == std::cmp::Ordering::Greater)", l, r),
            "lt" => format!("(({}).cmp(&({})) == std::cmp::Ordering::Less)", l, r),
            "gte" => format!("(({}).cmp(&({})) != std::cmp::Ordering::Less)", l, r),
            "lte" => format!("(({}).cmp(&({})) != std::cmp::Ordering::Greater)", l, r),
            _ => return None,
        })
    }

    /// Arithmetic and comparison of two ints that may grow past 64 bits
    /// (`Ty::Big`, the width pass's): the runtime's exact operations, a
    /// quotient's and a remainder's zero and a negative power stopping in
    /// the words the 64-bit ones use.
    fn big_op(&self, op: &str, l: &str, r: &str) -> Option<String> {
        let line = self.line_now();
        Some(match op {
            "add" | "sub" | "mul" => format!("({}).{}(&({}))", l, op, r),
            "div" | "pow" => format!("({}).{}(&({})).unwrap_or_else(|e| crow_stop({}, e))", l, op, r, line),
            "mod" => format!("({}).rem(&({})).unwrap_or_else(|e| crow_stop({}, e))", l, r, line),
            // (compared through the traits' functions, each side by
            // reference: a loop's borrowed item, `&Int`, is coerced where `<`
            // had refused it — crow check --self)
            "eq" => format!("<crow_runtime::int::Int as PartialEq>::eq(&({}), &({}))", l, r),
            "ne" => format!("<crow_runtime::int::Int as PartialEq>::ne(&({}), &({}))", l, r),
            "gt" => format!("<crow_runtime::int::Int as PartialOrd>::gt(&({}), &({}))", l, r),
            "lt" => format!("<crow_runtime::int::Int as PartialOrd>::lt(&({}), &({}))", l, r),
            "gte" => format!("<crow_runtime::int::Int as PartialOrd>::ge(&({}), &({}))", l, r),
            "lte" => format!("<crow_runtime::int::Int as PartialOrd>::le(&({}), &({}))", l, r),
            _ => return None,
        })
    }

    pub(crate) fn int_op(&self, op: &str, l: &str, r: &str) -> Option<String> {
        let f = match op {
            "add" => "add",
            "sub" => "sub",
            "mul" => "mul",
            "div" => "div",
            "mod" => "rem",
            "pow" => "pow",
            _ => return None,
        };
        Some(format!("crow_runtime::stops::{}({}, {}).unwrap_or_else(|e| crow_stop({}, e))", f, l, r, self.line_now()))
    }

    fn unary(&mut self, op: &str, e: &Expr) -> String {
        let ty = Self::inner(&self.tt(e));
        // an operator computes with an error (spec, Errors): a value that
        // failed is the value of `not` and of a minus sign, as of `x + 1` —
        // compiled, `not` of one had stopped the program where the
        // interpreter answered the error (crow check --self)
        if matches!(self.tt(e), Ty::Result(_)) {
            let ctr = self.binop_counter;
            self.binop_counter += 1;
            // (a conversion of a value that can fail — a list or a column
            // made a tensor to be made negative — converts its payload: the
            // value that can fail is what is bound)
            let (inner, payload) = match e {
                Expr::Convert { how, expr: held, ty: to } if !matches!(how, Convert::Unwrap | Convert::Known) && matches!(self.tt(held), Ty::Result(_)) => {
                    let name = format!("_unv_{}", ctr);
                    let local = Expr::Local(name.clone(), self.tt(held).payload().clone());
                    let conv = Expr::Convert { how: how.clone(), expr: Box::new(local), ty: to.payload().clone() };
                    let printed = self.expr(&conv);
                    (self.operand(held), format!("{{ let {} = _un_{}.value.clone(); {} }}", name, ctr, printed))
                }
                _ => (self.operand(e), format!("_un_{}.value.clone()", ctr)),
            };
            let turned = match op {
                "neg" if matches!(ty, Ty::Tensor(..) | Ty::Big | Ty::Decimal | Ty::Fraction) => format!("({}).neg()", payload),
                "neg" if matches!(ty, Ty::Int) => format!("crow_runtime::stops::neg({}).unwrap_or_else(|e| crow_stop({}, e))", payload, self.line_now()),
                "neg" => format!("(-({}))", payload),
                _ if ty.truths_depth() >= 2 => format!("crow_runtime::mask::negated(&({}))", payload),
                _ if matches!(&ty, Ty::List(i) if matches!(i.as_ref(), Ty::Bool)) => format!("{}.iter().map(|_b| !*_b).collect::<Vec<bool>>()", payload),
                _ => format!("(!({}))", payload),
            };
            let default_val = ty.default_rust_value();
            return format!(
                "{{ let _un_{c} = &({}); if !_un_{c}.ok {{ CrowResult {{ ok: false, value: {}, reason: _un_{c}.reason.clone(), kind: _un_{c}.kind.clone(), trace: _un_{c}.trace.clone(), captured: 0.0 }} }} else {{ CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} }}",
                inner,
                default_val,
                turned,
                c = ctr
            );
        }
        let inner = self.operand(e);
        match op {
            "neg" => {
                if self.emitting_traced && matches!(ty, Ty::Float | Ty::Tensor(..)) {
                    format!("crow_runtime::autograd::neg({})", Self::tr(&inner))
                } else if matches!(ty, Ty::Tensor(..) | Ty::Big | Ty::Decimal | Ty::Fraction) {
                    format!("{}.neg()", inner)
                } else if matches!(ty, Ty::Int) && !matches!(Emitter::raw(e), Expr::Const(..)) && !self.emitting_dual {
                    // only the smallest int has no negation
                    format!("crow_runtime::stops::neg({}).unwrap_or_else(|e| crow_stop({}, e))", inner, self.line_now())
                } else {
                    format!("(-{})", inner)
                }
            }
            _ => {
                if ty.truths_depth() >= 2 {
                    // (a mask of a matrix's shape: each truth turned)
                    format!("crow_runtime::mask::negated(&({}))", inner)
                } else if matches!(&ty, Ty::List(i) if matches!(i.as_ref(), Ty::Bool)) {
                    format!("{}.iter().map(|_b| !*_b).collect::<Vec<bool>>()", inner)
                } else if matches!(self.tt(e), Ty::Result(_)) {
                    let on_error = self.refuse("_c");
                    format!("(!({{ let _c = &({}); if !_c.ok {{ {} }} _c.value }}))", inner, on_error)
                } else {
                    format!("(!({}))", inner)
                }
            }
        }
    }

    /// `t.left eq 0`, the none test (spec, A Record or None): whether a
    /// link holds none — read where it lies — and of a record, which always
    /// holds one, never.
    fn optbox_comparison(&mut self, left: &Expr, op: &str, right: &Expr) -> Option<String> {
        if !matches!(op, "eq" | "ne") {
            return None;
        }
        let side = if matches!(right, Expr::Const(Const::Int(0), _)) {
            left
        } else if matches!(left, Expr::Const(Const::Int(0), _)) {
            right
        } else {
            return None;
        };
        let none = op == "eq";
        match self.tt(side) {
            Ty::OptRecord(_) => {
                let link = self.link_place(side).unwrap_or_else(|| self.expr(side));
                Some(format!("({}).{}()", link, if none { "is_none" } else { "is_some" }))
            }
            Ty::Record(_) => {
                let record = self.expr(side);
                Some(format!("{{ let _ = &({}); {} }}", record, !none))
            }
            _ => None,
        }
    }

    // ── indexing, fields, meta ──────────────────────────────────────

    fn index(&mut self, base: &Expr, indices: &[Expr], whole: &Expr) -> String {
        // state.get['key', default] and state.set['key', value]: a render or
        // a handler given the state reads and writes it by the value's type
        if let Expr::Field { base: obj, name, .. } = base {
            if self.needs_ui && indices.len() == 2 && matches!(name.as_str(), "get" | "set") && matches!(Emitter::raw(obj), Expr::Local(..) | Expr::Move(..)) {
                let o = self.expr(obj);
                let k = self.expr(&indices[0]);
                let v = self.expr(&indices[1]);
                if name == "set" {
                    self.needs_serde_json = true;
                    return format!("{}.set(&{}, serde_json::json!({}))", o, k, v);
                }
                match self.tt(&indices[1]) {
                    Ty::Int => return format!("{}.get_i64(&{}, {})", o, k, v),
                    Ty::Float => return format!("{}.get_f64(&{}, {})", o, k, v),
                    Ty::Moment => return format!("{}.get_moment(&{}, {})", o, k, v),
                    Ty::Str => return format!("{}.get_str(&{}, &{})", o, k, v),
                    Ty::Bool => return format!("{}.get_bool(&{}, {})", o, k, v),
                    _ => {}
                }
            }
        }
        let bty = self.tt(base);
        let is_result = matches!(bty, Ty::Result(_));
        // a base that can fail, indexed: its error is the index's value, as
        // the interpreter hands it on (the payload was read regardless: a
        // failed JSON parse, indexed, was null)
        if is_result && matches!(self.tt(whole), Ty::Result(_)) {
            let raw_base = self.expr(base);
            let payload = Expr::Local("_ib".to_string(), Self::inner(&bty).clone());
            let read = self.index(&payload, indices, whole);
            let default = Self::inner(&self.tt(whole)).default_rust_value();
            let read = if matches!(Self::inner(&bty), Ty::Json) { read } else { format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", read) };
            return format!(
                "{{ let _b = &({}); if !_b.ok {{ CrowResult {{ ok: false, value: {}, reason: _b.reason.clone(), kind: _b.kind.clone(), trace: _b.trace.clone(), captured: 0.0 }} }} else {{ let _ib = _b.value.clone(); {} }} }}",
                raw_base, default, read
            );
        }
        let inner = Self::inner(&bty);
        let raw_base = self.expr(base);
        let b = if is_result { format!("{}.value", raw_base) } else { raw_base };
        // a function value applied is a call, whose arguments are not indexes
        if let Ty::Fn(ps, ret) = &inner {
            return self.apply_fn_value(&b, ps, ret, indices, whole);
        }
        // an index that failed halts with its reason, as in the interpreter;
        // one that did not is its value
        let idx_strs: Vec<String> = indices
            .iter()
            .map(|i| {
                let s = self.expr(i);
                if matches!(self.tt(i), Ty::Result(_)) {
                    format!("{{ let _ix = &({}); if !_ix.ok {{ crow_stop({}, _ix.reason.clone()); }} _ix.value.clone() }}", s, self.line_now())
                } else {
                    s
                }
            })
            .collect();
        let idx = idx_strs.first().cloned().unwrap_or_default();
        let idx_ty = indices.first().map(|x| Self::inner(&self.tt(x)));
        match &inner {
            Ty::Tensor(el, dims) => {
                if matches!(idx_ty, Some(Ty::Tensor(..))) {
                    return format!("{}.try_gather(&{}){}", b, idx, self.or_stop());
                }
                // a row or a number as the type decided, through the runtime's
                // one read (crow_runtime::core): an index outside, or a value
                // of another rank, stops the program with the runtime's words
                let line = self.line_now();
                self.needs_crow_runtime = true;
                if matches!(crate::type_infer::TypeContext::tensor_item(el, dims), Ty::Tensor(..)) {
                    return format!("{{ let _i = {}; crow_runtime::core::row_at(&{}, _i).unwrap_or_else(|e| crow_stop({}, e)) }}", idx, b, line);
                }
                let cast = if matches!(el.as_ref(), Ty::Int) { " as i64" } else { "" };
                // a plain vector's number, read where it lies (vectors.rs):
                // a vector of whole numbers holds them as they are read
                if let Some(v) = self.plain_vector(base) {
                    let cast = if self.int_vecs.contains(v) { "" } else { cast };
                    return format!("{{ let _i = {}; {}[{}]{} }}", idx, v, self.index_at("_i", &format!("{}.len()", v)), cast);
                }
                // a parameter's number, read through the view of its numbers
                if let Some(view) = self.read_view(base) {
                    return format!("{{ let _i = {}; {}[{}]{} }}", idx, view, self.index_at("_i", &format!("{}.len()", view)), cast);
                }
                format!("{{ let _i = {}; crow_runtime::core::number_at(&{}, _i).unwrap_or_else(|e| crow_stop({}, e)){} }}", idx, b, line, cast)
            }
            Ty::List(el) => {
                let suffix = if Self::is_copy_type(el) { "" } else { ".clone()" };
                format!("{{ let _i = {}; let _v = &{}; _v[{}]{} }}", idx, b, self.index_at("_i", "_v.len()"), suffix)
            }
            // a character, read with what is known of the text's start (a
            // text read by position keeps it; every character had been
            // collected first, for each position read)
            Ty::Str | Ty::Markup => {
                self.needs_crow_runtime = true;
                let line = self.line_now();
                match self.indexed_text(base) {
                    Some(x) => format!("crow_runtime::text::char_at_known(&{x}, &{x}__known, ({idx}) as i64).unwrap_or_else(|e| crow_stop({line}, e))"),
                    None => format!("crow_runtime::text::char_at_known(&{b}, &crow_runtime::text::Known::new(), ({idx}) as i64).unwrap_or_else(|e| crow_stop({line}, e))"),
                }
            }
            Ty::Column(_) => {
                let getter = Self::cell_getter(&Self::inner(&self.tt(whole)));
                // a column that can have missing cells: the cell is a result,
                // a missing one an error
                if matches!(self.tt(whole), Ty::Result(_)) {
                    return format!(
                        "{{ let _i = {}; let _c = &{}; let _idx = {}; let _ok = _c.valid[_idx]; let _why = if _ok {{ String::new() }} else {{ \"missing\".to_string() }}; CrowResult {{ ok: _ok, value: _c.{}(_idx), reason: _why.clone(), kind: _why, trace: vec![], captured: 0.0 }} }}",
                        idx, b, self.index_at("_i", "_c.len()"), getter
                    );
                }
                format!("{{ let _i = {}; let _c = &{}; _c.{}({}) }}", idx, b, getter, self.index_at("_i", "_c.len()"))
            }
            Ty::Map(..) => {
                self.needs_crow_runtime = true;
                format!("crow_runtime::core::map_key(&{}, &({})).unwrap_or_else(|e| crow_stop({}, e))", b, idx_strs.join(", "), self.line_now())
            }
            // an element of a JSON value: JSON's null is an error value of
            // reason `null`, as the interpreter reads it
            Ty::Json => {
                let ie = match idx_ty.clone() {
                    Some(Ty::Int) => format!("{} as usize", idx),
                    _ => format!("&*{}", idx),
                };
                format!(
                    "{{ let _e = {}[{}].clone(); if _e.is_null() {{ CrowResult {{ ok: false, value: serde_json::Value::Null, reason: \"null\".to_string(), kind: \"parse\".to_string(), trace: vec![], captured: 0.0 }} }} else {{ CrowResult {{ ok: true, value: _e, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} }}",
                    b, ie
                )
            }
            _ => format!("{}({})", b, idx_strs.join(", ")),
        }
    }

    /// A function value applied, `f[x]`: an argument that failed is the
    /// call's value, the first such in order, and the function never runs,
    /// as the interpreter hands it on (the compiled program had stopped on
    /// it, and had not built a function returning such a call). Otherwise
    /// each argument goes as its parameter takes it: one that can fail, to a
    /// parameter that can fail, as it is; an int to a float, as a float.
    fn apply_fn_value(&mut self, f: &str, ps: &[Ty], ret: &Ty, args: &[Expr], whole: &Expr) -> String {
        let answers_failure = matches!(self.tt(whole), Ty::Result(_));
        let mut binds = String::new();
        let mut failing: Vec<usize> = Vec::new();
        let mut vals: Vec<String> = Vec::new();
        for (k, a) in args.iter().enumerate() {
            let param = ps.get(k).cloned().unwrap_or(Ty::Unknown);
            let takes_failure = matches!(param, Ty::Result(_));
            let as_float = |v: String, t: &Ty| if matches!(param, Ty::Float) && matches!(t, Ty::Int) { format!("{} as f64", v) } else { v };
            // (a value the function takes is its own: a local read again
            // later, or one this function was lent, is copied in — a
            // parameter lent its record had been handed as a reference)
            let s = if matches!(self.tt(a), Ty::Result(_)) { self.expr(a) } else { self.owned_expr(a) };
            vals.push(match self.tt(a) {
                Ty::Result(t) if answers_failure => {
                    binds.push_str(&format!("let _fa{} = {}; ", k, s));
                    failing.push(k);
                    if takes_failure { format!("_fa{}.clone()", k) } else { as_float(format!("_fa{}.value.clone()", k), &t) }
                }
                // (a call the IR says cannot fail: the argument goes as its
                // parameter takes it, and a failed one stops)
                Ty::Result(_) if takes_failure => s,
                Ty::Result(t) => as_float(format!("{{ let _ix = &({}); if !_ix.ok {{ crow_stop({}, _ix.reason.clone()); }} _ix.value.clone() }}", s, self.line_now()), &t),
                // (a tensor a gradient is handed is read: the local, or the
                // record it is a field of, lives on)
                t => as_float(s, &t),
            });
        }
        let call = format!("{}({})", f, vals.join(", "));
        if failing.is_empty() {
            return call;
        }
        let default = Self::inner(&self.tt(whole)).default_rust_value();
        let good = if matches!(ret, Ty::Result(_)) {
            call
        } else {
            format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", call)
        };
        let mut out = format!("{{ {}", binds);
        for k in failing {
            out.push_str(&format!(
                "if !_fa{k}.ok {{ CrowResult {{ ok: false, value: {d}, reason: _fa{k}.reason.clone(), kind: _fa{k}.kind.clone(), trace: _fa{k}.trace.clone(), captured: 0.0 }} }} else ",
                k = k,
                d = default
            ));
        }
        out.push_str(&format!("{{ {} }} }}", good));
        out
    }

    pub(crate) fn cell_getter(t: &Ty) -> &'static str {
        match t {
            Ty::Int => "get_i64",
            Ty::Float => "get_f64",
            Ty::Moment => "get_moment",
            Ty::Bool => "get_bool",
            // a column nothing typed: the cell as the value it is
            Ty::Json => "get_json",
            _ => "get_str",
        }
    }

    fn field(&mut self, base: &Expr, field: &str) -> String {
        // (a record or a link read through on the way is read where it
        // lies: `t.left.right` had counted `t.left` up and down)
        let obj = self.link_place(base).unwrap_or_else(|| self.expr(base));
        let oty = self.tt(base);
        // a value that failed has no fields: where the lowering could not
        // refuse it first (a fact's declared value), the read refuses it,
        // with the reason the interpreter gives (it had read the failed
        // value's placeholder)
        let obj = if matches!(oty, Ty::Result(_)) {
            format!(
                "({{ let _rf = ({}).clone(); if !_rf.ok {{ crow_stop({}, format!(\"cannot read '{}' of an error value — {{}}\", _rf.reason)) }} _rf }})",
                obj, self.line_now(), field
            )
        } else {
            obj
        };
        // a recursive link: unwrap the box — the program stops where
        // there is none, as the interpreter stops reading a field of 0
        let stop = format!("crow_stop({}, \"cannot access field '{}' on this value\".to_string())", self.line_now(), field);
        if let Ty::OptRecord(sname) = &oty {
            let is_copy = self.structs.get(sname).and_then(|s| s.fields.iter().find(|(f, _)| f == field).map(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool))).unwrap_or(false);
            let suffix = if is_copy { "" } else { ".clone()" };
            return format!("{}.as_ref().unwrap_or_else(|| {}).{}{}", obj, stop, Self::rust_field(field), suffix);
        }
        if let Ty::Result(inner) = &oty {
            if let Ty::OptRecord(sname) = inner.as_ref() {
                let is_copy = self.structs.get(sname).and_then(|s| s.fields.iter().find(|(f, _)| f == field).map(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool))).unwrap_or(false);
                let suffix = if is_copy { "" } else { ".clone()" };
                return format!("{}.value.as_ref().unwrap_or_else(|| {}).{}{}", obj, stop, Self::rust_field(field), suffix);
            }
            if let Ty::Record(sname) = inner.as_ref() {
                let is_copy = self.structs.get(sname).and_then(|s| s.fields.iter().find(|(f, _)| f == field).map(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool))).unwrap_or(false);
                let suffix = if is_copy { "" } else { ".clone()" };
                if matches!(sname.as_str(), "CrowShell" | "CrowFindResult" | "CrowReply" | "CrowAnswer") {
                    let suffix = if field == "status" { "" } else { ".clone()" };
                    return format!("{}.value.{}{}", obj, field, suffix);
                }
                return format!("{}.value.{}{}", obj, Self::rust_field(field), suffix);
            }
            if matches!(inner.as_ref(), Ty::Map(..)) {
                self.needs_crow_runtime = true;
                return format!("crow_runtime::core::map_key(&{}.value, &String::from(\"{}\")).unwrap_or_else(|e| crow_stop({}, e))", obj, field, self.line_now());
            }
        }
        let oinner = Self::inner(&oty);
        if matches!(oinner, Ty::Json) {
            let o = if matches!(oty, Ty::Result(_)) { format!("{}.value", obj) } else { obj.clone() };
            return format!("{}.get(\"{}\").cloned().unwrap_or(serde_json::Value::Null)", o, field);
        }
        if matches!(field, "year" | "month" | "day" | "hour" | "minute" | "second" | "weekday" | "offset") && matches!(oinner, Ty::Moment) {
            // a moment's field, read in its own zone; a moment that can fail
            // (`moment[…]`'s answer) arrives read once it is known to have
            // come, as the interpreter reads it
            let t = if matches!(oty, Ty::Result(_)) { format!("({}).value", obj) } else { obj.clone() };
            let method = if field == "offset" { "offset_minutes" } else { field };
            return format!("({}).{}()", t, method);
        }
        // a moment's field over a column of moments: a column of ints
        if matches!(oinner, Ty::Column(_)) && crate::lower_ir::moment_field_name(field) {
            self.needs_crow_runtime = true;
            return format!("crow_runtime::verbs::moment_field(&{}, \"{}\").unwrap_or_else(|e| crow_stop({}, e))", obj, field, self.line_now());
        }
        if matches!(oinner, Ty::Tensor(..)) {
            let t = if matches!(oty, Ty::Result(_)) { format!("{}.value.clone()", obj) } else { obj.clone() };
            match field {
                "shape" => return format!("{}.shape().iter().map(|&s| s as i64).collect::<Vec<i64>>()", if self.emitting_traced { format!("({}).value", Self::tr(&t)) } else { t.to_string() }),
                "dtype" => return "\"float\".to_string()".to_string(),
                "device" => return format!("if {}.is_gpu() {{ \"gpu\".to_string() }} else {{ \"cpu\".to_string() }}", t),
                "grad" => return "\"none\".to_string()".to_string(),
                _ => {}
            }
        }
        if matches!(oinner, Ty::Frame(_)) {
            let f = if matches!(oty, Ty::Result(_)) { format!("{}.value.clone()", obj) } else { obj.clone() };
            return match field {
                "shape" => format!("vec![{f}.nrows() as i64, {f}.ncols() as i64]", f = f),
                "columns" => format!("{}.columns.iter().map(|c: &String| c.clone()).collect::<Vec<String>>()", f),
                "dtypes" => format!("{}.dtypes().iter().map(|d: &&str| d.to_string()).collect::<Vec<String>>()", f),
                _ => format!("{}.col_native(\"{}\").unwrap_or_else(|| crow_stop({}, crow_runtime::stops::no_column(\"{}\")))", f, field, self.line_now(), field),
            };
        }
        if matches!(oty, Ty::Map(..)) {
            self.needs_crow_runtime = true;
            return format!("crow_runtime::core::map_key(&{}, &String::from(\"{}\")).unwrap_or_else(|e| crow_stop({}, e))", obj, field, self.line_now());
        }
        if let Ty::Record(sname) = &oty {
            if matches!(sname.as_str(), "CrowShell" | "CrowFindResult" | "CrowReply" | "CrowAnswer") {
                return format!("{}.{}", obj, field);
            }
        }
        format!("{}.{}", obj, Self::rust_field(field))
    }

    fn meta(&mut self, base: &Expr, which: Meta) -> String {
        let obj = self.expr(base);
        let oty = self.tt(base);
        let name = match which {
            Meta::Ok | Meta::Valid => "ok",
            Meta::Reason => "reason",
            Meta::Kind => "kind",
            Meta::Trace => "trace",
        };
        if matches!(oty, Ty::Result(_)) {
            return match name {
                "ok" => format!("{}.ok", obj),
                "trace" => format!("{}.trace.clone()", obj),
                other => format!("{}.{}.clone()", obj, other),
            };
        }
        if which == Meta::Valid || (name == "ok" && matches!(oty, Ty::Column(_))) {
            return format!("{}.ok()", obj);
        }
        // a record did not fail, whatever its fields hold (each field's
        // `.ok` asks it): no record has a field of these names, so `p.ok`
        // had not built
        // a dynamic value that is JSON null is an error value, reason 'null'
        if matches!(oty, Ty::Json) {
            return match name {
                "ok" => format!("!({}).is_null()", obj),
                "reason" => format!("(if ({}).is_null() {{ \"null\".to_string() }} else {{ String::new() }})", obj),
                "trace" => "Vec::<String>::new()".to_string(),
                _ => format!("(if ({}).is_null() {{ \"parse\".to_string() }} else {{ String::new() }})", obj),
            };
        }
        // a value that cannot fail did not; what was read for it is still
        // evaluated — a call that acts, an index that can stop — as the
        // interpreter evaluates it (compiled, it had been skipped)
        let answer = match name {
            "ok" => "true",
            "trace" => "Vec::<String>::new()",
            _ => "String::new()",
        };
        match base {
            Expr::Const(..) | Expr::Local(..) | Expr::Move(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => answer.to_string(),
            _ => format!("{{ let _ = {}; {} }}", obj, answer),
        }
    }

    // ── text ────────────────────────────────────────────────────────

    fn interp(&mut self, pieces: &[Piece]) -> String {
        let mut fmt = String::new();
        let mut args: Vec<String> = Vec::new();
        let mut fallible: Vec<(String, String)> = Vec::new();
        for p in pieces {
            match p {
                Piece::Text(t) => fmt.push_str(&fmt_piece(t)),
                Piece::Expr(x) => {
                    let mut rust = self.expr(x);
                    let ty = self.tt(x);
                    let inner = Self::inner(&ty);
                    if matches!(ty, Ty::Result(_)) {
                        let b = format!("_ip{}", fallible.len());
                        fallible.push((b.clone(), format!("({}).clone()", rust)));
                        rust = format!("{}.value", b);
                    }
                    // an error with no value (error[…]): the text is its
                    // error, and the value it would show never comes
                    if matches!(inner, Ty::Unit) {
                        fmt.push_str("{}");
                        args.push("\"\"".to_string());
                        continue;
                    }
                    match &inner {
                        // a column as the one display shows it, numbers bare
                        // (it had been its cells' text, quoted)
                        Ty::Column(_) => {
                            self.needs_crow_runtime = true;
                            fmt.push_str("{}");
                            args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", rust));
                        }
                        // a dynamic value as the one display shows it, text
                        // bare (it had shown with JSON's quotes: `"x"`); and
                        // the layer's handles and bytes, which have no text of
                        // Rust's own (a stream interpolated had not built)
                        Ty::Json | Ty::Bytes | Ty::Handle(_) => {
                            self.needs_crow_runtime = true;
                            fmt.push_str("{}");
                            args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", rust));
                        }
                        t if self.kernel_displayable(t) => {
                            self.needs_crow_runtime = true;
                            fmt.push_str("{}");
                            args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", rust));
                        }
                        Ty::List(_) | Ty::Map(..) | Ty::Record(_) => {
                            fmt.push_str("{:?}");
                            args.push(rust);
                        }
                        _ => {
                            fmt.push_str("{}");
                            args.push(rust);
                        }
                    }
                }
            }
        }
        if args.is_empty() {
            let raw: String = pieces.iter().filter_map(|p| if let Piece::Text(t) = p { Some(t.clone()) } else { None }).collect();
            return rust_str_lit(&raw);
        }
        if fallible.is_empty() {
            return format!("format!(\"{}\", {})", fmt, args.join(", "));
        }
        let binds: String = fallible.iter().map(|(b, e)| format!("let {} = {}; ", b, e)).collect();
        let trace = self.trace_format_expr();
        let mut out = format!("{{ {}", binds);
        for (b, _) in &fallible {
            out.push_str(&format!(
                "if !{b}.ok {{ let mut _err = CrowResult {{ ok: false, value: String::new(), reason: {b}.reason.clone(), kind: {b}.kind.clone(), trace: {b}.trace.clone(), captured: 0.0 }}; crow_trace_push(&mut _err.trace, {trace}); _err }} else ",
                b = b,
                trace = trace
            ));
        }
        out.push_str(&format!("{{ CrowResult {{ ok: true, value: format!(\"{}\", {}), reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} }}", fmt, args.join(", ")));
        out
    }

    // ── literals ────────────────────────────────────────────────────

    fn list_lit(&mut self, items: &[Expr], ty: &Ty) -> String {
        let lt = as_inferred(ty).text();
        if let Ty::Tensor(..) = &lt {
            if !items.is_empty() {
                let elem_is_tensor = items.iter().any(|i| matches!(self.tt(i), Ty::Tensor(..)));
                self.needs_crow_runtime = true;
                if elem_is_tensor {
                    // (each row owned: a local read here is copied, and
                    // lives on for the reads after — `stack[[v, w]]` had
                    // moved `v` into the list, and a loop over it later
                    // did not build)
                    let elems: Vec<String> = items.iter().map(|i| self.owned_expr(i)).collect();
                    return format!("crow_runtime::tensor::CrowTensor::from_elements(&[{}]){}", elems.join(", "), self.or_stop());
                }
                // (a literal's numbers are constants, in a traced copy too:
                // the tensor they make is wrapped as a constant where it is
                // used — each number had been printed as a traced one, and
                // `as f64` did not build)
                let was = std::mem::replace(&mut self.emitting_traced, false);
                let vals: Vec<String> = items.iter().map(|i| format!("{} as f64", self.expr(i))).collect();
                self.emitting_traced = was;
                return format!("crow_runtime::tensor::CrowTensor::from_vec(vec![{}], vec![{}])", vals.join(", "), items.len());
            }
        }
        let mut inner_type = Ty::Unknown;
        for i in items {
            inner_type = inner_type.unify(&self.tt(i));
        }
        // a list of results holds a plain item as a result that did not fail
        let of_results = matches!(&lt, Ty::List(inner) if matches!(inner.as_ref(), Ty::Result(_)));
        let vals: Vec<String> = if let Ty::Fn(_, _) = &inner_type {
            let ft = inner_type.to_rust_type();
            items.iter().map(|i| format!("{} as {}", self.expr(i), ft)).collect()
        } else if self.in_view() && !items.is_empty() && items.iter().all(|i| self.makes_node(i)) {
            items.iter().map(|i| format!("({}).to_html()", self.expr(i))).collect()
        } else {
            items
                .iter()
                .map(|i| {
                    let s = self.owned_expr(i);
                    match &lt {
                        Ty::List(item) if of_results => self.as_item(s, &self.tt(i), item),
                        _ => s,
                    }
                })
                .collect()
        };
        if items.is_empty() {
            if let Ty::List(inner) = &lt {
                if !matches!(inner.as_ref(), Ty::Unknown) {
                    return format!("Vec::<{}>::new()", inner.to_rust_type());
                }
            }
            return "vec![]".to_string();
        }
        format!("vec![{}]", vals.join(", "))
    }

    /// A value as an item of a list whose items can fail: a plain value is a
    /// result that did not fail, and an error without a value (`error[…]`)
    /// is one of the item's type, its value the type's default.
    pub(crate) fn as_item(&self, s: String, from: &Ty, item: &Ty) -> String {
        let Ty::Result(inner) = item else { return s };
        match from {
            Ty::Result(v) if matches!(v.as_ref(), Ty::Unit) && !matches!(inner.as_ref(), Ty::Unit) => format!(
                "{{ let _e = {}; CrowResult {{ ok: false, value: {}, reason: _e.reason, kind: _e.kind, trace: _e.trace, captured: 0.0 }} }}",
                s,
                inner.default_rust_value()
            ),
            Ty::Result(_) => s,
            _ => format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", s),
        }
    }

    /// A list of plain values where a list whose items can fail is held (a
    /// record's field another record gave such items): each item a result
    /// that did not fail (compiled, `Bag[[7, 8]]` beside `Bag[parsed]` had
    /// not built — crow check --self).
    pub(crate) fn as_items(s: String, from: &Ty, held: &Ty) -> String {
        match (from, held) {
            (Ty::List(item), Ty::List(wanted)) if wanted.is_result() && !item.is_result() && !matches!(item.as_ref(), Ty::Unknown) => format!(
                "({}).into_iter().map(|_x| CrowResult {{ ok: true, value: _x, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}).collect::<Vec<_>>()",
                s
            ),
            _ => s,
        }
    }

    fn map_lit(&mut self, pairs: &[(String, Expr)], ty: &Ty) -> String {
        if pairs.is_empty() {
            // (typed where nothing else would type it: a map of text a
            // builtin takes whatever its values show as)
            if matches!(ty, Ty::Map(_, e) if **e == Ty::Str) {
                return "BTreeMap::<String, String>::new()".to_string();
            }
            // (typed where its type is known: `len[map[]]` had left Rust
            // nothing to infer it from — crow check --self)
            if let Ty::Map(key, value) = ty
                && !value.has_unknown()
            {
                let key = if matches!(key, crate::ir::Key::Int) { "i64" } else { "String" };
                return format!("BTreeMap::<{}, {}>::new()", key, value.to_rust_type());
            }
            return "BTreeMap::new()".to_string();
        }
        let int_keys = matches!(ty, Ty::Map(crate::ir::Key::Int, _));
        let key_lit = |k: &str| if int_keys { format!("{}_i64", k) } else { rust_str_lit(k) };
        let val_types: Vec<Ty> = pairs.iter().map(|(_, v)| self.tt(v)).collect();
        let mut val_type = Ty::Unknown;
        for t in &val_types {
            val_type = val_type.unify(t);
        }
        let mixed = val_types.iter().any(|t| t != &val_type && *t != Ty::Unknown);
        let entries: Vec<String> = if let Ty::Fn(_, _) = &val_type {
            let ft = val_type.to_rust_type();
            pairs.iter().map(|(k, v)| format!("({}, {} as {})", key_lit(k), self.expr(v), ft)).collect()
        } else if mixed {
            pairs
                .iter()
                .zip(val_types.iter())
                .map(|((k, v), vt)| {
                    let s = self.expr(v);
                    let s = if matches!(vt, Ty::Str) { s } else { format!("{}.to_string()", s) };
                    format!("({}, {})", key_lit(k), s)
                })
                .collect()
        } else {
            pairs.iter().map(|(k, v)| format!("({}, {})", key_lit(k), self.expr(v))).collect()
        };
        format!("BTreeMap::from([{}])", entries.join(", "))
    }

    fn record_lit(&mut self, name: &str, fields: &[Expr]) -> String {
        let def = match self.structs.get(name) {
            Some(d) => d.clone(),
            None => return format!("compile_error!(\"unknown struct '{}'\")", name),
        };
        let mut inits = Vec::new();
        for ((fname, fty), e) in def.fields.iter().zip(fields.iter()) {
            // a variable read before its last use is copied into the field
            // (`Tape[ts, cs, cs, cs, vs]` moved `cs` three times); the last,
            // a move, hands it over
            let val = if Emitter::place_read(e) || matches!(Emitter::raw(e), Expr::Derived(..)) { self.owned_expr(e) } else { self.expr(e) };
            let is_ref = matches!(Emitter::raw(e), Expr::Local(n, _) | Expr::Move(n, _) if self.ref_params.contains(n));
            let owned = if is_ref { format!("{}.clone()", val) } else { val };
            if let Ty::OptRecord(_) = fty {
                if matches!(Emitter::raw(e), Expr::Const(Const::Int(0), _)) {
                    inits.push(format!("{}: None", Self::rust_field(fname)));
                } else if matches!(Self::inner(&self.tt(e)), Ty::OptRecord(_)) {
                    // (a record or none already: a child handed on)
                    inits.push(format!("{}: {}", Self::rust_field(fname), owned));
                } else {
                    inits.push(format!("{}: Some(crow_runtime::link::new({}))", Self::rust_field(fname), owned));
                }
            } else {
                // a field that can hold a failed value takes a plain one as
                // ok (`Last[false, …]` where another literal gave it a
                // fallible comparison)
                let owned = self.as_item(owned, &self.tt(e), &crate::lower_ir::as_inferred(fty));
                let owned = Self::as_items(owned, &self.tt(e), &crate::lower_ir::as_inferred(fty));
                inits.push(format!("{}: {}", Self::rust_field(fname), owned));
            }
        }
        format!("{} {{ {} }}", name, inits.join(", "))
    }

    // ── the collecting loop ─────────────────────────────────────────

    /// A body whose last statement is an `if` with no `else` ending in a
    /// value: a filter.
    fn filter_parts(body: &[Stmt]) -> Option<(&Expr, &Expr)> {
        let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)))?;
        if let Stmt::If { cond, then, els: None } = last {
            let last_then = then.iter().rev().find(|s| !matches!(s, Stmt::Line(_)))?;
            if let Stmt::Do(v) = last_then {
                // only when the if holds nothing else
                let others = then.iter().filter(|s| !matches!(s, Stmt::Line(_))).count();
                let before = body.iter().filter(|s| !matches!(s, Stmt::Line(_))).count();
                if others == 1 && before == 1 {
                    return Some((cond, v));
                }
            }
        }
        None
    }

    fn for_expr(&mut self, var: &str, iter: &Expr, body: &[Stmt], parallel: bool, whole: &Expr) -> String {
        // a list that can fail is refused where the loop starts, like a
        // condition (a function hands it back, a handler stops, the top
        // level halts), and the loop walks its value
        if let Ty::Result(_) = self.tt(iter) {
            let list = format!("_list_{}", self.binop_counter);
            self.binop_counter += 1;
            let printed = self.expr(iter);
            let lty = self.tt(iter);
            let on_error = self.refuse_at(&list, &Decision::Loop);
            self.declare(&list, lty);
            // (the list's last read is bound by value, and walked by value)
            let owned = self.consumable(iter);
            if owned {
                self.owned_lists.insert(list.clone());
            }
            let value = Expr::Convert { how: Convert::Unwrap, expr: Box::new(Expr::Local(list.clone(), iter.ty())), ty: iter.ty().payload().clone() };
            let walked = self.for_expr(var, &value, body, parallel, whole);
            let bound = if owned { format!("({})", printed) } else { format!("&({})", printed) };
            return format!("{{ let {l} = {b}; if !{l}.ok {{ {e} }} {w} }}", l = list, b = bound, e = on_error, w = walked);
        }
        // in parallel as the lowering decided, on the cores `cpu[…]` allows
        // where the loop starts (crow_runtime::cores)
        let use_par = parallel;
        // a body that can leave the function (a return, an error handed
        // back, a handler's stop) or that yields on some paths only gathers
        // in a loop: a closure can neither return from the function nor
        // skip an iteration (the lowering never marks the first parallel)
        // a body that does not end in its value (an if whose every arm
        // yields) gathers too: each arm pushes what it yields
        let gathers = !ends_in_value(body) && Self::filter_parts(body).is_none();
        if !use_par && (leaves(body) || (!always_yields(body) && Self::filter_parts(body).is_none()) || gathers) {
            return self.gathering_loop(var, iter, body, whole);
        }
        let iter_str = self.expr(iter);
        if use_par {
            self.needs_rayon = true;
        }
        let it = self.tt(iter);
        let (iter_str, elem) = self.loop_items(iter_str, &it, iter);
        let is_copy = Self::is_copy_type(&elem);
        // a list the loop reads for the last time is walked by value: each
        // item is moved into the body, no copy made, and the list goes as
        // the loop does (k-nucleotide's 2 million lines had each been copied
        // to be uppercased, and the list had lived to the program's end)
        let consumes = !is_copy && matches!(it, Ty::List(_)) && self.consumable(iter);
        let method = match (use_par, consumes) {
            (true, true) => "into_par_iter",
            (true, false) => "par_iter",
            (false, true) => "into_iter",
            (false, false) => "iter",
        };
        // an item that is not Copy is borrowed from what the loop walks; the
        // body reads it as its own value, as the interpreter's does
        let (param, rebind) = if is_copy {
            (format!("|&{}|", var), String::new())
        } else if consumes {
            (format!("|__item_{}|", var), format!("let {v} = __item_{v}; ", v = var))
        } else {
            (format!("|__item_{}|", var), format!("let {v} = __item_{v}.clone(); ", v = var))
        };

        if let Some((cond, val)) = Self::filter_parts(body) {
            self.push_scope();
            self.declare(var, elem.clone());
            let c = self.expr(cond);
            let c = self.guard_condition(cond, c);
            let mut v = self.owned_expr(val);
            let mut vt = self.tt(val);
            if self.makes_node(val) {
                v = format!("({}).to_html()", v);
                vt = Ty::Str;
            }
            self.pop_scope();
            return Self::on_cores(format!("{}.{}().filter_map({} {{ {}if {} {{ Some({}) }} else {{ None }} }}).collect::<Vec<{}>>()", iter_str, method, param, rebind, c, v, vt.to_rust_type()), use_par);
        }

        // a body that yields on some paths only, or from the arms of a
        // branch, run in parallel: each iteration gathers what it yields
        // (none or one) and the lists are joined in input order; a
        // sequential one gathered in a loop above
        if !always_yields(body) || gathers {
            let list = match whole.ty() {
                Ty::List(item) => Ty::List(item),
                _ => Ty::List(Box::new(Ty::Unknown)),
            };
            let list_ty = as_inferred(&list);
            let one = format!("_yielded_{}", self.binop_counter);
            self.binop_counter += 1;
            let saved_out = std::mem::take(&mut self.out);
            let saved_indent = self.indent;
            self.indent = saved_indent + 1;
            self.push_scope();
            self.declare(var, elem);
            if !rebind.is_empty() {
                self.line(rebind.trim_end());
            }
            self.line(&format!("let mut {}: {} = Vec::new();", one, list_ty.to_rust_type()));
            self.declare(&one, list_ty.clone());
            self.closure_depth += 1;
            for s in yielding(body, &one, &list) {
                self.emit_stmt(&s);
            }
            self.closure_depth -= 1;
            self.line(&one);
            self.pop_scope();
            let closure_body = std::mem::replace(&mut self.out, saved_out);
            self.indent = saved_indent;
            let pad = "    ".repeat(self.indent);
            return Self::on_cores(format!("{}.{}().flat_map_iter({} {{\n{}{}}}).collect::<{}>()", iter_str, method, param, closure_body, pad, list_ty.to_rust_type()), use_par);
        }

        // the body's statements, then its last value
        let saved_out = std::mem::take(&mut self.out);
        let saved_indent = self.indent;
        self.indent = saved_indent + 1;
        self.push_scope();
        self.declare(var, elem);
        if !rebind.is_empty() {
            self.line(rebind.trim_end());
        }
        let last = body.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
        let (init, last_expr) = match last {
            Some(i) if matches!(body[i], Stmt::Do(_)) => (&body[..i], Some(&body[i])),
            _ => (body, None),
        };
        self.closure_depth += 1;
        for s in init {
            self.emit_stmt(s);
        }
        self.closure_depth -= 1;
        let mut last_type = Ty::Unknown;
        if let Some(Stmt::Do(e)) = last_expr {
            // (a list or record the function holds, yielded as each item —
            // a row of bases per sample — is the item's own copy: the
            // closure had moved it out of the function)
            let mut s = self.owned_expr(e);
            last_type = self.tt(e);
            if self.makes_node(e) {
                s = format!("({}).to_html()", s);
                last_type = Ty::Str;
            }
            let pad = "    ".repeat(self.indent);
            self.out.push_str(&format!("{}{}\n", pad, s));
        }
        self.pop_scope();
        let closure_body = std::mem::replace(&mut self.out, saved_out);
        self.indent = saved_indent;
        let pad = "    ".repeat(self.indent);
        Self::on_cores(format!("{}.{}().map({} {{\n{}{}}}).collect::<Vec<{}>>()", iter_str, method, param, closure_body, pad, last_type.to_rust_type()), use_par)
    }

    /// A parallel loop run on the cores `cpu[…]` allows where it starts
    /// (crow_runtime::cores); a loop in order as it is.
    pub(crate) fn on_cores(s: String, parallel: bool) -> String {
        if parallel { format!("crow_runtime::cores::run(|| {})", s) } else { s }
    }

    /// A collecting loop as a loop that gathers into a list: each value
    /// the body yields is pushed, and the loop is the statement form's.
    fn gathering_loop(&mut self, var: &str, iter: &Expr, body: &[Stmt], whole: &Expr) -> String {
        let list = match whole.ty() {
            Ty::List(item) => Ty::List(item),
            _ => Ty::List(Box::new(Ty::Unknown)),
        };
        let list_ty = as_inferred(&list);
        let acc = format!("_gathered_{}", self.binop_counter);
        self.binop_counter += 1;
        let each = Stmt::For { var: var.to_string(), iter: iter.clone(), body: yielding(body, &acc, &list), parallel: false };
        let saved_out = std::mem::take(&mut self.out);
        let saved_indent = self.indent;
        self.indent = saved_indent + 1;
        self.line(&format!("let mut {}: {} = Vec::new();", acc, list_ty.to_rust_type()));
        self.declare(&acc, list_ty);
        self.emit_stmt(&each);
        self.line(&acc);
        let inner = std::mem::replace(&mut self.out, saved_out);
        self.indent = saved_indent;
        format!("{{\n{}{}}}", inner, "    ".repeat(self.indent))
    }

    // ── calls ───────────────────────────────────────────────────────

    /// Does a call's own value (before an argument's error propagates
    /// through it) already carry a status?
    fn base_is_result(&self, name: &str, target: &Target, raw_args: &[Expr]) -> bool {
        match target {
            Target::Row(row) => matches!(row.fails, crate::table::Fails::Value(_)),
            Target::User(n) => self.fns.get(n).map_or(false, |f| matches!(f.ret, Ty::Result(_))),
            Target::Module(m, f) => self.registry.type_of_in(m, f, &raw_args.iter().map(|a| self.tt(a)).collect::<Vec<_>>()).map_or(false, |t| matches!(t, Ty::Result(_))),
            // `json[map]` and `json[frame]` answer a response, never a status
            // (the row of the name, `json[text]`, parses and can fail)
            Target::Runtime(_) if name == "json" && raw_args.first().is_some_and(|a| crate::table::Arg::of_type(&Self::inner(&self.tt(a))).json_response()) => false,
            Target::Runtime(_) if self.row_for(name, raw_args).is_some() => {
                matches!(self.row_for(name, raw_args).map(|r| r.fails), Some(crate::table::Fails::Value(_)))
            }
            Target::Runtime(_) => {
                if name == "set" {
                    return false;
                }
                let types: Vec<Ty> = raw_args.iter().map(|a| self.tt(a)).collect();
                let kinds: Vec<crate::table::Arg> = types.iter().map(crate::table::Arg::of_type).collect();
                if let Some(m) = crate::table::module_of_bare(name, &self.registry.providers_of(name), &kinds) {
                    if let Some(t) = self.registry.type_of_in(&m, name, &types) {
                        return matches!(t, Ty::Result(_));
                    }
                }
                // (math's sign[x] is no token's)
                FALLIBLE_BUILTINS.contains(&name) && !(name == "sign" && raw_args.len() != 2)
            }
        }
    }

    /// What a call needs built into the runtime: the feature its row
    /// declares (a response as Arrow, the GPU, the Parquet kernels called as
    /// themselves in a module's own test — inside a module's function,
    /// inlined, the program's call has decided), and the format a table's
    /// path may name (`Prop::Table`): a literal says it, an interpolation
    /// whose first text is a local path and whose last an extension says
    /// it, any other may name one (crow_runtime::sources::table_feature). A
    /// URL or an S3 object needs nothing built in: the protocol is written
    /// in Crow.
    fn note_sources(&mut self, name: &str, args: &[Expr]) {
        use crow_runtime::sources::{format_of, is_remote, table_feature};
        if !self.in_crow_fn {
            self.note_feature(name);
        }
        let Some(at) = crate::table::rows_named(name).flat_map(|r| r.props.iter()).find_map(|p| match p {
            crate::table::Prop::Table(i) => Some(*i),
            _ => None,
        }) else {
            return;
        };
        // whether the path may be remote, and the format it names: known to
        // be none (an extensionless path), one, or not known at all
        let (remote, named) = match args.get(at) {
            Some(Expr::Const(Const::Str(p), _)) => (is_remote(p), Some(format_of(p))),
            Some(Expr::Interp(pieces)) => {
                let text = |p: Option<&Piece>| match p {
                    Some(Piece::Text(t)) => t.clone(),
                    _ => String::new(),
                };
                let (head, tail) = (text(pieces.first()), text(pieces.last()));
                let scheme = ["http://", "https://", "s3://"].iter().any(|s| s.starts_with(head.as_str()) || head.starts_with(s));
                (scheme, format_of(&tail).map(Some))
            }
            _ => (true, None),
        };
        if let Some(f) = table_feature(remote, named) {
            self.runtime_features.insert(f.to_string());
        }
    }

    fn call(&mut self, e: &Expr) -> String {
        // a text builder's join and its count (builders.rs)
        if let Some(text) = self.builder_text(e) {
            return text;
        }
        // a slice or a length of a text read by position (texts.rs)
        if let Some(text) = self.text_read(e) {
            return text;
        }
        if let Some(x) = builders::measured(e).filter(|x| self.text_builders.contains_key(*x)) {
            return format!("{x}__count");
        }
        // a plain vector's length, measured where it lies (vectors.rs)
        if let Expr::Call { target: Target::Row(row), args, .. } = e
            && row.id == "core.len_tensor"
            && args.len() == 1
            && let Some(v) = self.plain_vector(&args[0])
        {
            return format!("({}.len() as i64)", v);
        }
        if let Expr::Call { target: Target::Row(row), args, .. } = e
            && row.id == "core.len_tensor"
            && args.len() == 1
            && let Some(view) = self.read_view(&args[0])
        {
            return format!("({}.len() as i64)", view);
        }
        let (mut target, args, propagates) = match e {
            Expr::Call { target, args, propagates, .. } => (target.clone(), args.clone(), *propagates),
            Expr::Effect { name, row, args, propagates, .. } => {
                let t = match (row, name.split_once('.')) {
                    (Some(row), _) => Target::Row(row),
                    (None, Some((m, f))) => Target::Module(m.to_string(), f.to_string()),
                    (None, None) => Target::Runtime(name.clone()),
                };
                (t, args.clone(), *propagates)
            }
            _ => unreachable!(),
        };
        // a builtin written in Crow that the lowering left to the values
        // (an argument's kind unknown to it: `frame.load[upload.path]`
        // from a map's value) is its row, picked here as the interpreter
        // picks it from the values
        if let Target::Module(m, f) = &target {
            if let Some(row) = self.crow_row_left(m, f, &args) {
                target = Target::Row(row);
            }
        }
        if let Target::Runtime(n) = &target {
            if let Some(row) = self.crow_row_left("", n, &args) {
                target = Target::Row(row);
            }
        }
        // a qualified call to a registry module is its bare call, pinned
        let mut pinned: Option<String> = None;
        if let Target::Module(m, f) = &target {
            if crate::modules::by_path(m).is_some_and(|e| e.pinned) && self.registry.has_function(f) {
                pinned = Some(m.clone());
                target = Target::Runtime(f.clone());
            }
        }
        // a module's row is its module's
        if let Target::Row(row) = &target {
            if !row.module.is_empty() {
                pinned = Some(row.module.to_string());
            }
        }
        let name = match &target {
            Target::Row(row) => row.name.to_string(),
            Target::Runtime(p) => RUNTIME_TABLE.iter().find(|(_, r)| *r == p.as_str()).map(|(b, _)| b.to_string()).unwrap_or_else(|| p.clone()),
            Target::User(n) => n.clone(),
            Target::Module(_, f) => f.clone(),
        };
        let raw_args: Vec<Expr> = args.iter().map(|a| Emitter::raw(a).clone()).collect();
        if !matches!(target, Target::User(_)) {
            self.note_sources(&name, &raw_args);
        }
        let current_pin = self.pinned_module.clone();
        // a module's function named like an element (`frame.stack`) is the
        // module's: it propagates, where the element renders a failed child
        let from_module = pinned.is_some();
        let saved_pin = std::mem::replace(&mut self.pinned_module, pinned.or(current_pin));

        // (a builtin written in Crow is the program's function its body
        // is inlined as: its arguments are handed as a function's are — a
        // local read cloned, a parameter it borrows referenced)
        let crow_fn: Option<String> = match &target {
            Target::Row(row) if row.is_crow() => Some(crate::table::crow_fn_name(row)),
            _ => None,
        };
        let callee_fn: Option<&str> = match &target {
            Target::User(n) => Some(n.as_str()),
            _ => crow_fn.as_deref(),
        };
        let user = callee_fn.is_some();
        let ref_flags: Vec<bool> = match callee_fn {
            Some(n) => match (self.by_ref_params.get(n), self.fns.get(n)) {
                (Some(set), Some(f)) => f.params.iter().map(|(p, _)| set.contains(p)).collect(),
                _ => vec![false; raw_args.len()],
            },
            None => vec![false; raw_args.len()],
        };
        let param_tys: Vec<Ty> = callee_fn.and_then(|n| self.fns.get(n)).map(|f| f.params.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
        let mut arg_strs: Vec<String> = Vec::with_capacity(raw_args.len());
        for (i, a) in raw_args.iter().enumerate() {
            let by_ref = ref_flags.get(i).copied().unwrap_or(false);
            // a record or none (a tree's child, 0 where there is none)
            // handed to a function: where it takes a record, the record it
            // holds — the program stops where it holds none, as the
            // interpreter stops reading a field of 0; where it takes a
            // record or none, a record is wrapped, 0 is none, and one of
            // its own kind passes as it is
            let takes = param_tys.get(i).map(|t| t.payload().clone());
            let given = self.tt(a);
            let none_literal = matches!(a, Expr::Const(Const::Int(0), _));
            let stop = format!("crow_stop({}, \"a record is needed here, and this value is 0 (none)\".to_string())", self.line_now());
            match (takes, &given) {
                (Some(Ty::Record(_)), Ty::OptRecord(_)) => {
                    let s = self.link_place(a).unwrap_or_else(|| self.expr(a));
                    arg_strs.push(if by_ref { format!("({}).as_deref().unwrap_or_else(|| {})", s, stop) } else { format!("({}).as_deref().unwrap_or_else(|| {}).clone()", s, stop) });
                    continue;
                }
                (Some(Ty::OptRecord(_)), Ty::Record(_)) => {
                    let s = self.owned_expr(a);
                    arg_strs.push(if by_ref { format!("&Some(crow_runtime::link::new({}))", s) } else { format!("Some(crow_runtime::link::new({}))", s) });
                    continue;
                }
                (Some(Ty::OptRecord(_)), _) if none_literal => {
                    arg_strs.push(if by_ref { "&None".to_string() } else { "None".to_string() });
                    continue;
                }
                // (a link handed on: lent where it lies, or one count up —
                // a local read for the last time moved)
                (Some(Ty::OptRecord(_)), Ty::OptRecord(_)) => {
                    let s = match Emitter::raw(a) {
                        Expr::Local(v, _) | Expr::Move(v, _) if self.ref_params.contains(v) => {
                            if by_ref { v.clone() } else { format!("(*{}).clone()", v) }
                        }
                        Expr::Local(v, _) | Expr::Move(v, _) if !by_ref && self.moved_local(a) => v.clone(),
                        _ => match self.link_place(a) {
                            Some(place) if by_ref => format!("&{}", place),
                            Some(place) => format!("{}.clone()", place),
                            None if by_ref => format!("&{}", self.expr(a)),
                            // (a call's answer, or a field read already its own count)
                            None => self.expr(a),
                        },
                    };
                    arg_strs.push(s);
                    continue;
                }
                _ => {}
            }
            // (a builtin takes a record or none as it is: `type[t.left]`,
            // `str[t.left]` of none had unwrapped it, a panic)
            if by_ref {
                if user && let Some(s) = self.optbox_ref(a) {
                    arg_strs.push(s);
                } else if let Expr::Local(v, _) | Expr::Move(v, _) = a {
                    if self.ref_params.contains(v) {
                        arg_strs.push(v.clone());
                    } else if self.moved_local(a) {
                        // (read for the last time: freed as the statement ends)
                        arg_strs.push(format!("&{{{}}}", v));
                    } else {
                        arg_strs.push(format!("&{}", v));
                    }
                } else {
                    let s = self.expr(a);
                    arg_strs.push(format!("&{}", s));
                }
            } else if user && let Some(s) = self.optbox_value(a) {
                arg_strs.push(s);
            } else {
                let local_read = match a {
                    Expr::Local(..) | Expr::Move(..) | Expr::Load(..) => true,
                    Expr::Derived(d, _) => self.declared(d),
                    // a field of a local (a struct parameter is borrowed:
                    // its list is cloned, never moved out of it)
                    Expr::Field { base, .. } => matches!(Emitter::raw(base), Expr::Local(..) | Expr::Move(..) | Expr::Load(..)),
                    _ => false,
                };
                // (a parsed value, a column and a sparse matrix are no
                // copies either: `kind_of[p]` then `p.ok` had moved `p`)
                let needs_clone = user
                    && local_read
                    && matches!(
                        Self::inner(&self.tt(a)),
                        Ty::Tensor(..)
                            | Ty::List(_)
                            | Ty::Map(..)
                            | Ty::Str
                            | Ty::Markup
                            | Ty::Frame(_)
                            | Ty::Record(_)
                            | Ty::Bytes
                            | Ty::Handle(_)
                            | Ty::Big
                            | Ty::Decimal
                            | Ty::Fraction
                            | Ty::Json
                            | Ty::Column(_)
                            | Ty::Sparse
                    );
                let s = self.expr(a);
                let moved = self.moved_local(a);
                arg_strs.push(if needs_clone && !moved && !(matches!(a, Expr::Load(n, _) if self.static_handle_vars.contains(n))) { format!("{}.clone()", s) } else { s });
            }
        }

        // an argument's error is the call's value (spec, auto-propagation),
        // unless the IR says the call takes it as it is (a collecting loop
        // gathering what an iteration yielded, errors included)
        // (a function of the program's named like an element is a call,
        // not the element: it hands a failed argument on)
        let skip = (!propagates && matches!(target, Target::Runtime(_) | Target::Row(_)))
            || matches!(name.as_str(), "log" | "assert" | "cpu" | "gpu" | "type" | "error" | "captured" | "running")
            || (self.needs_ui && !from_module && !matches!(target, Target::User(_)) && (name == "chart" || name == "table" || crate::lower::UI_ELEMENTS.contains(&name.as_str())));
        if !skip {
            // an argument that converts a failed value's payload — `list(unwrap
            // push(…))`, a tensor that can fail as a list's items — fails as
            // the value does: the call binds the value, and the conversion
            // is printed over its payload
            let through: Vec<Option<(usize, String)>> = raw_args
                .iter()
                .map(|a| self.unwrapped_through(a).map(|(node, inner)| (node as *const Expr as usize, self.expr(inner))))
                .collect();
            let result_idx: Vec<usize> = raw_args
                .iter()
                .enumerate()
                .filter(|(i, a)| matches!(self.tt(a), Ty::Result(_)) || through[*i].is_some())
                .map(|(i, _)| i)
                .collect();
            if !result_idx.is_empty() {
                let orig: Vec<String> = result_idx.iter().map(|&i| through[i].as_ref().map_or_else(|| arg_strs[i].clone(), |(_, s)| s.clone())).collect();
                // a view, or a control, renders a failed argument as its reason
                let renders = match &target {
                    Target::User(v) => self.views.contains(v),
                    Target::Runtime(_) | Target::Row(_) => {
                        self.in_view() && crate::lower::is_ui_control(&name, raw_args.first().map_or(false, |x| matches!(Self::inner(&self.tt(x)), Ty::Frame(_))))
                    }
                    Target::Module(..) => false,
                };
                if renders {
                    {
                        let mut bound = arg_strs.clone();
                        let mut pre = String::new();
                        let mut checks: Vec<String> = Vec::new();
                        for (k, &i) in result_idx.iter().enumerate() {
                            pre.push_str(&format!("let _va{} = &({}); ", k, orig[k]));
                            checks.push(format!("if !_va{k}.ok {{ CrowVNode::failed(&_va{k}.reason) }} else", k = k));
                            bound[i] = match &through[i] {
                                Some((node, _)) => self.printed_over(&raw_args[i], *node, format!("_va{}.value.clone()", k)),
                                None => format!("_va{}.value.clone()", k),
                            };
                        }
                        let call = match &target {
                            Target::User(v) => self.user_call(v, &raw_args, bound),
                            _ => self.dispatch(&target, &name, &raw_args, &bound, e),
                        };
                        self.pinned_module = saved_pin;
                        return format!("{{ {}{} {{ {} }} }}", pre, checks.join(" "), call);
                    }
                }
                let mut unwrapped = arg_strs.clone();
                for &i in &result_idx {
                    // a value that has none (an `error[…]`) never reaches the call
                    unwrapped[i] = if let Some((node, _)) = &through[i] {
                        // (a conversion of the payload is a value of its own:
                        // lent to a parameter the callee borrows — a failed
                        // tensor's numbers as the list a copy of `ask` takes
                        // had been handed by value to `&Vec<f64>`, and the
                        // program did not build — crow check --self)
                        let printed = self.printed_over(&raw_args[i], *node, format!("_prop_{}.value.clone()", i));
                        if ref_flags.get(i).copied().unwrap_or(false) { format!("&({})", printed) } else { printed }
                    } else if matches!(Self::inner(&self.tt(&raw_args[i])), Ty::Unit) {
                        "unreachable!()".to_string()
                    } else if ref_flags.get(i).copied().unwrap_or(false) {
                        // a struct the callee borrows is lent, not copied
                        format!("&_prop_{}.value", i)
                    } else if self.moved_local(&raw_args[i]) {
                        // a local read for the last time was bound by value:
                        // its payload goes on as it is (a list then cut where
                        // it lies, `rows` sliced to one section had been
                        // copied, half of a 4-million-line file beside it)
                        format!("_prop_{}.value", i)
                    } else {
                        format!("_prop_{}.value.clone()", i)
                    };
                }
                // an argument that cannot fail but can stop or act (an index,
                // a call) is evaluated in its place among those that can: a
                // call evaluates every argument, left to right, before it
                // hands the leftmost failure on (spec, Auto-Propagation) —
                // compiled, a failed argument had skipped every argument
                // after it, and one printed in the call itself, so
                // `send[failed, j]` refused the socket where the interpreter
                // stopped on reading `j`
                let mut hoisted: Vec<(usize, String)> = Vec::new();
                for (j, a) in raw_args.iter().enumerate() {
                    if !result_idx.contains(&j) && !Self::quiet(a) {
                        let name = format!("_pa_{}", j);
                        hoisted.push((j, std::mem::replace(&mut unwrapped[j], name.clone())));
                        self.unwrap_as.insert(a as *const Expr as usize, name);
                    }
                }
                let inner_expr = self.dispatch(&target, &name, &raw_args, &unwrapped, e);
                for (j, _) in &hoisted {
                    self.unwrap_as.remove(&(&raw_args[*j] as *const Expr as usize));
                }
                let full = self.tt(e);
                let inner_type = match Self::inner(&full) {
                    Ty::Map(k, v) => Ty::Map(k, Box::new(Self::inner(&v))),
                    Ty::List(v) => Ty::List(Box::new(Self::inner(&v))),
                    o => o,
                };
                let already = self.base_is_result(&name, &target, &raw_args);
                let stop_as = (self.reactive_epilogue.is_some() && crate::table::held(&name)).then(|| self.shown_fn_name());
                self.pinned_module = saved_pin;
                // a value a name holds is borrowed to be checked and read; any
                // other (a call's answer) is bound as it is
                // (one read for the last time is moved in, and freed with the
                // block: a file's lines had lived to the end of the program)
                let held: Vec<bool> = result_idx
                    .iter()
                    .map(|&i| match &through[i] {
                        Some(_) => self.unwrapped_through(&raw_args[i]).is_none_or(|(_, inner)| !Emitter::fresh_value(inner) && !self.moved_local(inner)),
                        None => !Emitter::fresh_value(&raw_args[i]) && !self.moved_local(&raw_args[i]),
                    })
                    .collect();
                return self.propagation_block(&result_idx, &orig, &held, &hoisted, &inner_expr, &inner_type, already, stop_as);
            }
        }
        let out = self.dispatch(&target, &name, &raw_args, &arg_strs, e);
        self.pinned_module = saved_pin;
        out
    }

    /// The failed value a value conversion reads the payload of, through
    /// the unwrap the lowering marked it with: the unwrap node and what it
    /// unwraps, when that can fail (`list(unwrap push(…))`,
    /// `tensor(unwrap xs)`); none for an argument that is not such a
    /// conversion (a bare unwrap is read past by `raw`).
    fn unwrapped_through<'a>(&self, a: &'a Expr) -> Option<(&'a Expr, &'a Expr)> {
        match a {
            // (every conversion of a value reads its payload: `big(unwrap
            // may(…))` had read a failed value's default, 0, where the
            // interpreter's call failed with it — crow check --self)
            Expr::Convert { how: Convert::ToList | Convert::ToTensor | Convert::IntToFloat | Convert::IntsToFloats | Convert::ToBig | Convert::ToDecimal | Convert::ToFraction | Convert::Proven | Convert::ToSmall(_), expr, .. } => match expr.as_ref() {
                node @ Expr::Convert { how: Convert::Unwrap, expr: inner, .. } if matches!(self.tt(inner), Ty::Result(_)) => Some((node, inner)),
                deeper => self.unwrapped_through(deeper),
            },
            _ => None,
        }
    }

    /// An argument printed with one of its unwraps read as the payload a
    /// propagation bound (`_prop_0.value.clone()`).
    fn printed_over(&mut self, a: &Expr, node: usize, bound: String) -> String {
        self.unwrap_as.insert(node, bound);
        let printed = self.expr(a);
        self.unwrap_as.remove(&node);
        printed
    }

    /// An argument whose evaluation can neither stop nor act — a value a
    /// name holds, a literal of such — whose place among a call's arguments
    /// does not matter.
    fn quiet(e: &Expr) -> bool {
        match e {
            Expr::Const(..) | Expr::Local(..) | Expr::Move(..) | Expr::Load(..) | Expr::Derived(..) | Expr::Constant(..) | Expr::FnRef(..) => true,
            Expr::Field { base, .. } | Expr::Meta { base, .. } => Self::quiet(base),
            Expr::Convert { how, expr, .. } => !matches!(how, Convert::ToSmall(_)) && Self::quiet(expr),
            Expr::ListLit(items, _) => items.iter().all(Self::quiet),
            Expr::MapLit(items, _) => items.iter().all(|(_, v)| Self::quiet(v)),
            Expr::RecordLit { fields, .. } => fields.iter().all(Self::quiet),
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn propagation_block(&self, idx: &[usize], orig: &[String], held: &[bool], hoisted: &[(usize, String)], inner: &str, inner_type: &Ty, already: bool, stop_as: Option<String>) -> String {
        let default_val = inner_type.default_rust_value();
        let trace = self.trace_format_expr();
        let error_result = |i: usize| -> String {
            if let Some(who) = &stop_as {
                return format!("{{ state.stop(format!(\"{} stopped: {{}}\", _prop_{}.reason)); return; }}", who, i);
            }
            format!(
                "{{ let mut _err = CrowResult {{ ok: false, value: {}, reason: _prop_{i}.reason.clone(), kind: _prop_{i}.kind.clone(), trace: _prop_{i}.trace.clone(), captured: 0.0 }}; crow_trace_push(&mut _err.trace, {}); _err }}",
                default_val,
                trace,
                i = i
            )
        };
        // the payload read where it lies when the call only reads it (`&x` of
        // a copy of `x` is `&x`); a value a name holds is borrowed, and its
        // payload copied only where the call keeps it; a call's answer is
        // bound as it is and its payload moved into the call (the value is
        // read once: copying it whole twice had been the price of every
        // read of a list a file's lines filled)
        let mut inner = inner.to_string();
        for (pos, &i) in idx.iter().enumerate() {
            let copy = format!("_prop_{}.value.clone()", i);
            for (from, to) in [(format!("&({})", copy), format!("&(_prop_{}.value)", i)), (format!("&*({})", copy), format!("&*(_prop_{}.value)", i)), (format!("&{}", copy), format!("&_prop_{}.value", i))] {
                inner = inner.replace(&from, &to);
            }
            let payload = format!("_prop_{}.value", i);
            if !held.get(pos).copied().unwrap_or(false) && inner.matches(&copy).count() == 1 && inner.matches(&payload).count() == 1 {
                inner = inner.replace(&copy, &format!("_prop_{}.value", i));
            }
        }
        let mut result = if already {
            inner
        } else {
            format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", inner)
        };
        // every argument bound in its place, then the leftmost failure is
        // the call's value
        let mut lets: Vec<(usize, String)> = hoisted.iter().map(|(j, s)| (*j, format!("let _pa_{} = {};", j, s))).collect();
        let mut checks = String::new();
        for (pos, &i) in idx.iter().enumerate() {
            let bound = if held.get(pos).copied().unwrap_or(false) { format!("&({})", orig[pos]) } else { format!("({})", orig[pos]) };
            lets.push((i, format!("let _prop_{i} = {};", bound, i = i)));
            checks.push_str(&format!("if !_prop_{i}.ok {{ {} }} else ", error_result(i), i = i));
        }
        lets.sort_by_key(|(at, _)| *at);
        let lets: Vec<String> = lets.into_iter().map(|(_, l)| l).collect();
        result = format!("{{ {} {}{{ {} }} }}", lets.join(" "), checks, result);
        result
    }

    /// A struct field that is a recursive link, handed by reference to a
    /// function that takes the record: the record it holds, or the program
    /// stops where it holds none (it had unwrapped, a panic).
    fn optbox_ref(&mut self, e: &Expr) -> Option<String> {
        if let Expr::Field { base, name, .. } = e {
            if let Ty::Record(s) = self.tt(base) {
                if self.structs.get(&s).map_or(false, |d| d.fields.iter().any(|(f, t)| f == name && matches!(t, Ty::OptRecord(_)))) {
                    let obj = self.expr(base);
                    return Some(format!("{}.{}.as_deref().unwrap_or_else(|| {})", obj, Self::rust_field(name), self.none_stop()));
                }
            }
        }
        None
    }

    /// A struct field that is a recursive link, handed by value to a
    /// function that takes the record: a copy of the record it holds (its
    /// own links shared), or the program stops where it holds none.
    fn optbox_value(&mut self, e: &Expr) -> Option<String> {
        if let Expr::Field { base, name, .. } = e {
            if let Ty::Record(s) = self.tt(base) {
                if self.structs.get(&s).map_or(false, |d| d.fields.iter().any(|(f, t)| f == name && matches!(t, Ty::OptRecord(_)))) {
                    let obj = self.expr(base);
                    return Some(format!("{}.{}.as_deref().unwrap_or_else(|| {}).clone()", obj, Self::rust_field(name), self.none_stop()));
                }
            }
        }
        None
    }

    /// A record's field or a link read where it lies — a place, for the
    /// caller to borrow or copy: a local, a field of a record, a field
    /// read through a link (none stopping as a field read of 0 does), at
    /// any depth below a local. None for anything else.
    fn link_place(&mut self, e: &Expr) -> Option<String> {
        match e {
            Expr::Local(v, _) | Expr::Move(v, _) => {
                if self.ref_params.contains(v) {
                    return Some(format!("(*{})", v));
                }
                let plain = self.declared(v) && !self.plain_vecs.contains(v) && !self.lent_here.contains(v) && !self.static_handle_vars.contains(v);
                plain.then(|| v.clone())
            }
            Expr::Field { base, name, .. } => {
                let bt = self.tt(base);
                let known = |s: &String| self.structs.get(s).is_some_and(|d| d.fields.iter().any(|(f, _)| f == name));
                match &bt {
                    Ty::Record(s) if known(s) => {
                        let b = self.link_place(base)?;
                        Some(format!("{}.{}", b, Self::rust_field(name)))
                    }
                    Ty::OptRecord(s) if known(s) => {
                        let b = self.link_place(base)?;
                        Some(format!("{}.as_ref().unwrap_or_else(|| crow_stop({}, \"cannot access field '{}' on this value\".to_string())).{}", b, self.line_now(), name, Self::rust_field(name)))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The stop where a record is needed and a link holds none, as the
    /// interpreter stops reading a field of 0.
    fn none_stop(&self) -> String {
        format!("crow_stop({}, \"a record is needed here, and this value is 0 (none)\".to_string())", self.line_now())
    }

    /// A core builtin's row for a call the IR leaves by name: the one its
    /// arguments' types pick (crate::table); the interpreter picks by the
    /// values.
    fn row_for(&self, name: &str, args: &[Expr]) -> Option<&'static crate::table::Row> {
        let kinds: Vec<crate::table::Arg> = args.iter().map(|x| crate::table::Arg::of_type(&self.tt(x))).collect();
        crate::table::core_row(name, &kinds)
    }

    /// A builtin written in Crow the lowering left to the values (an
    /// argument's kind unknown to it): its row by the arguments' types
    /// here, or, where they still say nothing, by their count — one row
    /// of the name takes that many. The module is the whole path before
    /// the name (`math.nn.relu`).
    fn crow_row_left(&self, m: &str, f: &str, args: &[Expr]) -> Option<&'static crate::table::Row> {
        let (m, f) = format!("{}.{}", m, f).rsplit_once('.').map(|(a, b)| (a.to_string(), b.to_string()))?;
        let kinds: Vec<crate::table::Arg> = args.iter().map(|x| crate::table::Arg::of_type(&self.tt(x))).collect();
        crate::table::crow_row(&m, &f, &kinds).or_else(|| crate::table::ROWS.iter().find(|r| r.is_crow() && r.module == m && r.name == f && r.params.len() == args.len()))
    }

    /// A call, by what it names.
    fn dispatch(&mut self, target: &Target, name: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        // (a row whose function is the web runtime's — a response made from
        // bytes — brings the crate, whatever the program imports)
        if let Target::Row(row) = target {
            if row.path.starts_with("crow_web::") {
                self.needs_crow_web = true;
            }
        }
        if self.emitting_traced {
            if let Some(s) = self.traced_dispatch(target, name, args, arg_strs, whole) {
                return s;
            }
        }
        match target {
            // math on the dual numbers a function grad differentiates is the
            // derivative's own form
            // (by the name the program wrote: `floor[x]` of a float of any
            // size is the growing form's row, which takes no dual)
            Target::Row(row) if row.module.is_empty() && self.emitting_dual && matches!(crate::table::written_name(row), "abs" | "sqrt" | "floor" | "ceil" | "round") => {
                let name = crate::table::written_name(row);
                self.builtin(name, args, arg_strs, whole).unwrap_or_else(|| format!("{}({})", name, arg_strs.join(", ")))
            }
            // (math's functions of a dual number are the dual's own methods,
            // each carrying its derivative: `sin`, `exp`, `ln`, …)
            Target::Row(_) | Target::Runtime(_) if self.emitting_dual && arg_strs.len() == 1 && crate::table::has(name, crate::table::Prop::Dual) => format!("({}).{}()", arg_strs[0], name),
            // a builtin written in Crow: the program's function its body is
            // inlined as (an effect's row reaches here too); a reader's
            // value carries the moment it was read, as a Rust reader's does
            // a message is held to its handler's commit by the session layer,
            // in both modes: the arm holds it, the row's function sends it
            Target::Row(row) if row.is_crow() && row.id == "web.mail.mail" => self.builtin("mail", args, arg_strs, whole).unwrap_or_default(),
            Target::Row(row) if row.is_crow() => {
                let call = self.user_call(&crate::table::crow_fn_name(row), args, arg_strs.to_vec());
                if crate::table::stamps(row.name) && matches!(row.fails, crate::table::Fails::Value(_)) {
                    self.needs_crow_runtime = true;
                    format!("{{ let mut _r = {}; if _r.ok {{ _r.captured = crow_runtime::moments::now(); }} _r }}", call)
                } else {
                    call
                }
            }
            Target::Row(row) if row.module.is_empty() => self.table_call(row, args, arg_strs, whole),
            Target::Row(row) => self.module_row_call(row, args, arg_strs),
            Target::User(n) => self.user_call(n, args, arg_strs.to_vec()),
            Target::Module(m, f) => self.module_call(m, f, args, arg_strs, whole),
            // (math on the dual numbers a function grad differentiates is
            // the derivative's own form)
            Target::Runtime(_) => match self
                .row_for(name, args)
                .filter(|_| !(self.emitting_dual && matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round")))
            {
                Some(row) => self.table_call(row, args, arg_strs, whole),
                None => match self.builtin(name, args, arg_strs, whole) {
                    Some(s) => s,
                    None => format!("{}({})", name, arg_strs.join(", ")),
                },
            },
        }
    }

    /// A module's row: its runtime function, each argument printed, a view's
    /// tree handed to it as its markup; it stops on the line its call is on
    /// (modules::STOP_LINE).
    /// Whether a value is traced inside a function's traced copy: a number
    /// or a tensor there is a traced value (crow_runtime::autograd::Traced).
    fn traced_type(t: &Ty) -> bool {
        matches!(Self::inner(t), Ty::Float | Ty::Tensor(..))
    }

    /// A growing form's call a traced copy answers by the name the program
    /// wrote (`floor` of a traced number is a traced number, not the whole
    /// number the IR types it as).
    fn traced_whole(&self, e: &Expr) -> bool {
        match Self::raw(e) {
            Expr::Call { target: Target::Row(row), args, .. } => crate::table::written_name(row) != row.name && args.iter().any(|a| Self::traced_type(&self.tt(a))),
            _ => false,
        }
    }

    /// A value as the autograd kernel's operand (`&Traced`): a traced value
    /// borrowed, a plain number or tensor — an int, a list's item, a
    /// global's — as a constant (crow_runtime::autograd::AsTraced).
    fn tr(s: &str) -> String {
        format!("&*crow_runtime::autograd::AsTraced::traced(&({}))", s)
    }

    /// An operand of a traced operator.
    fn traced_operand(&mut self, e: &Expr) -> String {
        let s = self.operand(e);
        Self::tr(&s)
    }

    /// An operator inside a function's traced copy: the autograd kernel's,
    /// recorded on the tape, a shape that does not meet stopping the
    /// program with the operator's words; a comparison of two numbers
    /// decides as it would. None where neither operand is traced (ints,
    /// text, bools).
    fn traced_binary(&mut self, op: &str, left: &Expr, right: &Expr) -> Option<String> {
        let (lt, rt) = (self.tt(left), self.tt(right));
        if !Self::traced_type(&lt) && !Self::traced_type(&rt) {
            return None;
        }
        let both_tensors = matches!(Self::inner(&lt), Ty::Tensor(..)) && matches!(Self::inner(&rt), Ty::Tensor(..));
        let l = self.traced_operand(left);
        let r = self.traced_operand(right);
        let stop = self.or_stop();
        let f = match op {
            "add" => "add",
            "sub" => "sub",
            "mul" if both_tensors => "matmul",
            "mul" | "elemmul" => "mul",
            "div" if both_tensors => return Some("compile_error!(\"use ./ for element-wise tensor division\")".to_string()),
            "div" | "elemdiv" => "div",
            "pow" | "elempow" => "pow",
            "eq" | "ne" | "gt" | "lt" | "gte" | "lte" => {
                let sign = match op {
                    "eq" => "==",
                    "ne" => "!=",
                    "gt" => ">",
                    "lt" => "<",
                    "gte" => ">=",
                    _ => "<=",
                };
                return Some(format!("(({}).as_number().unwrap_or(f64::NAN) {} ({}).as_number().unwrap_or(f64::NAN))", l, sign, r));
            }
            other => return Some(format!("compile_error!(\"grad: `{}` is not differentiable\")", other)),
        };
        Some(format!("crow_runtime::autograd::{}({}, {}){}", f, l, r, stop))
    }

    /// A call inside a function's traced copy: a differentiable builtin
    /// handed a traced value is the autograd kernel's, a call handed none
    /// answers a constant (a constructor's tensor, `float` of an int), a
    /// function of the program's is its own traced copy. None where the
    /// call neither takes nor answers a traced value (a range, text).
    fn traced_dispatch(&mut self, target: &Target, name: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> Option<String> {
        if let Target::User(n) = target {
            if self.traced_functions.contains(n) {
                let passed: Vec<String> = args
                    .iter()
                    .zip(arg_strs)
                    .map(|(a, s)| {
                        if Self::traced_type(&self.tt(a)) {
                            let raw = self.expr(a);
                            format!("crow_runtime::autograd::AsTraced::traced(&({})).into_owned()", raw)
                        } else {
                            s.clone()
                        }
                    })
                    .collect();
                let call = format!("_traced_{}({})", Self::rust_fn_name(n), passed.join(", "));
                return Some(self.at_call_line(n, call));
            }
            return None;
        }
        let traced_arg = args.iter().any(|x| Self::traced_type(&self.tt(x)));
        if !traced_arg {
            let wrap = match Self::inner(&self.tt(whole)) {
                Ty::Float if !matches!(self.tt(whole), Ty::Result(_)) => "crow_runtime::autograd::Traced::number",
                Ty::Tensor(..) if !matches!(self.tt(whole), Ty::Result(_)) => "crow_runtime::autograd::Traced::from",
                _ => return None,
            };
            let inner = self.dispatch_plain(target, name, args, arg_strs, whole);
            return Some(format!("{}({})", wrap, inner));
        }
        let a = |i: usize| Self::tr(&arg_strs.get(i).cloned().unwrap_or_default());
        let plain = |i: usize| arg_strs.get(i).cloned().unwrap_or_default();
        let stop = self.or_stop();
        let n = args.len();
        // (by the name the program wrote: `floor[s]` of a float of any size
        // is the growing form's row)
        let name = match target {
            Target::Row(row) => crate::table::written_name(row),
            _ => name,
        };
        Some(match (name, n) {
            ("sum", 1) | ("mean", 1) | ("max", 1) | ("min", 1) | ("exp", 1) | ("ln", 1) | ("sqrt", 1) | ("tanh", 1) | ("abs", 1) | ("erf", 1) | ("floor", 1) | ("ceil", 1) | ("round", 1) | ("transpose", 1) | ("flatten", 1)
            | ("sin", 1) | ("cos", 1) | ("tan", 1) | ("asin", 1) | ("acos", 1) | ("atan", 1) | ("sinh", 1) | ("cosh", 1) | ("log2", 1) | ("log10", 1) | ("sign", 1) => {
                format!("crow_runtime::autograd::{}({})", name, a(0))
            }
            ("sum", 2) | ("mean", 2) | ("max", 2) | ("min", 2) => format!("crow_runtime::autograd::{}_axis({}, ({}) as usize){}", name, a(0), plain(1), stop),
            ("clip", 3) => format!("crow_runtime::autograd::clip({}, {}, {}){}", a(0), a(1), a(2), stop),
            ("onehot", 2) => format!("crow_runtime::autograd::Traced::constant(crow_runtime::math::onehot(&({}).value, ({}) as i64){})", a(0), plain(1), stop),
            ("reshape", k) if k >= 2 => {
                let sizes: Vec<String> = (1..k).map(|i| format!("({}) as usize", plain(i))).collect();
                format!("crow_runtime::autograd::reshape({}, vec![{}]){}", a(0), sizes.join(", "), stop)
            }
            ("len", 1) => format!("crow_runtime::core::len_tensor(&({}).value)", a(0)),
            ("float", 1) | ("tensor", 1) => format!("({}).clone()", a(0)),
            ("int", 1) => format!("(({}).as_number().unwrap_or(0.0) as i64)", a(0)),
            _ => format!("compile_error!(\"grad: `{}` is not differentiable\")", name),
        })
    }

    /// A call printed as it is outside a traced copy (a constructor's
    /// tensor, wrapped as a constant by the caller).
    fn dispatch_plain(&mut self, target: &Target, name: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        let was = std::mem::replace(&mut self.emitting_traced, false);
        let out = self.dispatch(target, name, args, arg_strs, whole);
        self.emitting_traced = was;
        out
    }

    fn module_row_call(&mut self, row: &crate::table::Row, args: &[Expr], arg_strs: &[String]) -> String {
        let mut strs = arg_strs.to_vec();
        for (i, a) in args.iter().enumerate() {
            if let Expr::Call { target: Target::User(v), .. } = a {
                if self.views.contains(v) {
                    strs[i] = format!("({}).to_html()", strs[i]);
                }
            }
        }
        self.needs_crow_runtime = true;
        crate::table::print(row, &strs, crate::modules::STOP_LINE, None)
            .map(|code| code.replace(crate::modules::STOP_LINE, &self.line_now()))
            .unwrap_or_else(|e| format!("compile_error!({:?})", e))
    }

    /// A builtin with a row: its runtime function, each argument printed as
    /// its kind takes it, its failure as the row says (crate::table). The
    /// interpreter calls the same function through the same row. A
    /// collection the function makes its answer of is moved when it is the
    /// variable the statement assigns (`xs = push[xs, x]` grows it in place)
    /// and copied otherwise; an item it keeps is copied from a variable, and
    /// into a list or a map whose items can fail it is an item that did not
    /// (a gathering push, a collecting loop's, keeps it as it was yielded).
    fn table_call(&mut self, row: &crate::table::Row, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        use crate::table::Kind;
        self.needs_crow_runtime = true;
        if row.ret == Kind::Shell {
            self.needs_shell_struct = true;
        }
        if row.ret == Kind::Reply {
            self.needs_reply_struct = true;
        }
        // a parsed JSON value is serde's, which the program then names
        if row.ret == Kind::Json || row.ret == Kind::JsonList || row.params.contains(&Kind::Json) {
            self.needs_serde_json = true;
        }
        let mut strs = arg_strs.to_vec();
        // a local read for the last time, handed where the row only reads
        // it, goes into a temporary the statement frees (`&({rows})[..]`):
        // a list of a file's lines had lived on to the end of the program
        // after its last slice
        if let Some(kinds) = crate::table::kinds_of(row.params, strs.len()) {
            for (i, (k, a)) in kinds.iter().zip(args).enumerate() {
                if k.borrows() && self.moved_local(a) {
                    strs[i] = format!("{{{}}}", strs[i]);
                }
            }
        }
        if let Some(kinds) = crate::table::kinds_of(row.params, strs.len()) {
            let items = kinds.iter().zip(args).find_map(|(k, a)| match (k, Self::inner(&self.tt(a))) {
                (Kind::List | Kind::Seq, Ty::List(it)) | (Kind::MapOf | Kind::MapRef, Ty::Map(_, it)) => Some(*it),
                _ => None,
            });
            let gathered = matches!(whole, Expr::Call { propagates: false, .. });
            for (i, (k, a)) in kinds.iter().zip(args).enumerate() {
                let s = strs[i].clone();
                strs[i] = match k {
                    k if k.owned() => self.moved_or_cloned(a, &s),
                    // a list whose items can fail, joined: its items, a failed
                    // one stopping the program with its reason, as the
                    // interpreter reads its words (table::read_words)
                    Kind::Words if matches!(Self::inner(&self.tt(a)), Ty::List(it) if it.is_result()) => {
                        let line = self.line_now();
                        format!("{{ let _xs = &({}); let mut _w = Vec::with_capacity(_xs.len()); for (_i, _x) in _xs.iter().enumerate() {{ if !_x.ok {{ crow_stop({}, if _x.kind == \"missing\" {{ crow_runtime::stops::missing_cell(_i) }} else {{ format!(\"item {{}} of the list failed: {{}}\", _i, _x.reason) }}) }} _w.push(_x.value.clone()); }} _w }}", s, line)
                    }
                    Kind::Item | Kind::ItemRef => {
                        let at = self.tt(a);
                        // (a local read for the last time moves into the collection)
                        let s = if *k == Kind::Item && !self.moved_local(a) { Self::kept(a, &s) } else { s };
                        // a function kept is the collection's function type
                        let s = match &at {
                            Ty::Fn(..) => format!("({} as {})", s, at.to_rust_type()),
                            _ => s,
                        };
                        match &items {
                            Some(it @ Ty::Result(_)) => {
                                let from = if gathered { at.clone() } else { Self::inner(&at) };
                                self.as_item(s, &from, it)
                            }
                            _ => s,
                        }
                    }
                    _ => s,
                };
            }
        }
        // a text nothing else holds is changed where it lies (table::taken_path,
        // the interpreter's `take` bridges): the variable the statement
        // assigns, read for the last time, or another call's answer
        if let (Some(path), Some(first)) = (crate::table::taken_path(row.id), args.first()) {
            let raw = Emitter::raw(first);
            let taken = matches!(raw, Expr::Local(n, _) | Expr::Move(n, _) if self.move_target.as_deref() == Some(n.as_str()))
                || self.moved_local(first)
                || matches!(raw, Expr::Call { propagates: false, .. } | Expr::Call { target: Target::Row(_) | Target::User(_), .. } | Expr::Interp(_));
            let held = match Self::inner(&self.tt(first)) {
                Ty::Str => true,
                // (a list's items are moved with it: `lines` drops its empty
                // last piece without copying the rest)
                Ty::List(_) => row.id == "core.slice",
                // (a tensor's numbers are appended where they lie: `xs =
                // push[xs, v]` at the top level had copied the tensor at
                // every push, 400,000 pushes in 18 s)
                Ty::Tensor(..) => row.id == "core.push_number",
                _ => false,
            };
            if taken && held && row.params.len() == strs.len() {
                let line = self.line_now();
                let mut printed = vec![strs[0].clone()];
                printed.extend(row.params.iter().zip(&strs).skip(1).map(|(k, s)| k.print(s, &line)));
                let call = format!("{}({})", path, printed.join(", "));
                return match row.fails {
                    crate::table::Fails::Stops => format!("{}.unwrap_or_else(|e| crow_stop({}, e))", call, line),
                    _ => call,
                };
            }
        }
        let trace = self.trace_format_expr();
        crate::table::print(row, &strs, &self.line_now(), Some(&trace)).unwrap_or_else(|e| format!("compile_error!({:?})", e))
    }

    /// A user function's call: a view's or an element's tree handed to it
    /// is its markup; a handler calling a helper hands it the state and
    /// reads back what it wrote; a view calling a view hands it the state.
    /// The line a stop names: the statement's, or, inside a standard
    /// module's function written in Crow, the line of the call that
    /// reached it (crow_runtime::stops::call_line, noted by `at_call_line`).
    /// The position `i` names in a sequence of `len` (each printed Rust),
    /// or the program stops on this statement's line — inside a standard
    /// module's function written in Crow, on the call's, read only when it
    /// stops (crow_runtime::stops::at: no call the optimizer cannot see
    /// through, so a loop's reads of a list it does not write are hoisted).
    pub(crate) fn index_at(&self, i: &str, len: &str) -> String {
        if self.in_crow_fn {
            format!("crow_runtime::stops::at_call({}, {})", i, len)
        } else {
            format!("crow_runtime::stops::at({}, {}, {})", i, len, self.current_line)
        }
    }

    pub(crate) fn line_now(&self) -> String {
        if self.in_crow_fn {
            "crow_runtime::stops::call_line()".to_string()
        } else {
            self.current_line.to_string()
        }
    }

    /// A call of a standard module's function written in Crow, the call's
    /// line noted first so a stop inside it names this line (inside such a
    /// function the note already holds the program's line).
    pub(crate) fn at_call_line(&mut self, callee: &str, call: String) -> String {
        if crate::modules::is_crow_fn(callee) && !self.in_crow_fn {
            self.needs_crow_runtime = true;
            format!("{{ crow_runtime::stops::at_line({}); {} }}", self.current_line, call)
        } else {
            call
        }
    }

    pub(crate) fn user_call(&mut self, name: &str, args: &[Expr], mut arg_strs: Vec<String>) -> String {
        // inside a dual version, a helper's dual version
        if self.emitting_dual && (self.grad_functions.contains(name) || self.dual_helpers.contains(name)) {
            return format!("_dual_{}({})", Self::rust_fn_name(name), arg_strs.join(", "));
        }
        let params: Vec<Ty> = self.fns.get(name).map(|f| f.params.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
        for (i, a) in args.iter().enumerate() {
            if self.makes_node(a) {
                if let Some(s) = arg_strs.get_mut(i) {
                    *s = format!("({}).to_html()", s);
                }
            }
            // an empty list where the function takes numbers as a tensor (a
            // numeric literal at another call typed the parameter)
            if matches!(Emitter::raw(a), Expr::ListLit(items, _) if items.is_empty()) && matches!(params.get(i), Some(Ty::Tensor(..))) {
                self.needs_crow_runtime = true;
                if let Some(s) = arg_strs.get_mut(i) {
                    *s = "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])".to_string();
                }
            }
        }
        let cur = self.cur_fn.clone();
        let helper_call = self.reactive_event_handlers.contains(name) && self.reactive_event_handlers.contains(&cur);
        let threads = helper_call || (self.reactive_render_fns.contains(name) && (self.reactive_render_fns.contains(&cur) || self.reactive_event_handlers.contains(&cur)));
        let all = if threads {
            let mut v = vec!["state".to_string()];
            v.extend(arg_strs.iter().cloned());
            v
        } else {
            arg_strs.clone()
        };
        if helper_call {
            // the helper reads and writes the facts the handler does, in
            // the one state they are handed: its arguments first, so none
            // holds the state while the helper takes it
            let stop = self.stop_check();
            let bound: Vec<String> = arg_strs.iter().enumerate().map(|(i, a)| format!("let _a{} = {};", i, a)).collect();
            let mut passed = vec!["state".to_string()];
            passed.extend((0..arg_strs.len()).map(|i| format!("_a{}", i)));
            return format!("{{ {} let _r = {}({}); {} _r }}", bound.join(" "), Self::rust_fn_name(name), passed.join(", "), stop);
        }
        // a helper outside the handler class that a render calls with no state
        let call = format!("{}({})", Self::rust_fn_name(name), all.join(", "));
        self.at_call_line(name, call)
    }
}

/// Whether a collecting loop's body can leave the function: a return, an
/// error handed back from a condition, a handler's stop.
fn leaves(body: &[Stmt]) -> bool {
    let mut out = false;
    visit_stmts(body, &mut |s| out |= matches!(s, Stmt::Return(_) | Stmt::Propagate { .. }));
    // a collecting loop inside over a list that can fail refuses it there
    visit_block(body, &mut |e| out |= matches!(e, Expr::ForExpr { iter, .. } if iter.ty().is_result()));
    out
}

/// Whether a collecting loop's body ends in its value itself.
fn ends_in_value(body: &[Stmt]) -> bool {
    matches!(body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))), Some(Stmt::Do(_)))
}

/// Whether every pass through a collecting loop's body yields a value: it
/// ends in one, or in a branch whose every arm does.
fn always_yields(body: &[Stmt]) -> bool {
    match body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
        Some(Stmt::Do(_)) => true,
        Some(Stmt::If { then, els: Some(e), .. }) => always_yields(then) && always_yields(e),
        _ => false,
    }
}

/// A collecting loop's body with each value it yields pushed onto `acc`:
/// the last statement yields, or the last of each arm of a final branch.
fn yielding(body: &[Stmt], acc: &str, list: &Ty) -> Block {
    let last = body.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
    body.iter()
        .enumerate()
        .map(|(i, s)| match s {
            Stmt::Do(e) if Some(i) == last => Stmt::Set {
                var: acc.to_string(),
                expr: Expr::Call {
                    target: Target::Runtime("core.push".to_string()),
                    args: vec![Expr::Move(acc.to_string(), list.clone()), e.clone()],
                    ty: list.clone(),
                    propagates: false,
                },
            },
            Stmt::If { cond, then, els } if Some(i) == last => Stmt::If {
                cond: cond.clone(),
                then: yielding(then, acc, list),
                els: els.as_ref().map(|b| yielding(b, acc, list)),
            },
            other => other.clone(),
        })
        .collect()
}

/// A value of type `t` (ints in 64 bits) as the same value growing: each
/// int an `Int`, through lists, maps and values that can fail (a failed
/// one's failure kept).
fn grown(t: &Ty, v: &str) -> String {
    match t {
        Ty::Int | Ty::Unknown => format!("crow_runtime::int::Int::from({})", v),
        Ty::Result(i) => format!("{{ let _r = {}; CrowResult {{ ok: _r.ok, value: {}, reason: _r.reason, kind: _r.kind, trace: _r.trace, captured: _r.captured }} }}", v, grown(i, "_r.value")),
        Ty::List(i) => format!("({}).into_iter().map(|_x| {}).collect::<Vec<_>>()", v, grown(i, "_x")),
        Ty::Map(_, i) => format!("({}).into_iter().map(|(_k, _x)| (_k, {})).collect::<std::collections::BTreeMap<_, _>>()", v, grown(i, "_x")),
        _ => v.to_string(),
    }
}

/// A value of type `t` (ints that grow) as the same value in 64 bits, read
/// where it lies: an int past them stops the program on `line`, naming
/// `what` took it (`Who::text`).
fn shrunk(t: &Ty, v: &str, what: &str, line: &str) -> String {
    match t {
        Ty::Big | Ty::Unknown => format!("({}).small_for({:?}).unwrap_or_else(|e| crow_stop({}, e))", v, what, line),
        Ty::Result(i) => format!("{{ let _r = &({}); CrowResult {{ ok: _r.ok, value: {}, reason: _r.reason.clone(), kind: _r.kind.clone(), trace: _r.trace.clone(), captured: _r.captured }} }}", v, shrunk(i, "_r.value", what, line)),
        Ty::List(i) => format!("({}).iter().map(|_x| {}).collect::<Vec<_>>()", v, shrunk(i, "(*_x)", what, line)),
        Ty::Map(_, i) => format!("({}).iter().map(|(_k, _x)| (_k.clone(), {})).collect::<std::collections::BTreeMap<_, _>>()", v, shrunk(i, "(*_x)", what, line)),
        _ => format!("({}).clone()", v),
    }
}
