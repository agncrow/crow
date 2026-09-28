//! Expressions.

use super::program::map_literal_pairs;
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
    "error", "hash", "shell", "find", "input", "read", "write", "mkdir", "ls", "rm", "mv", "cp", "captured", "moment", "parse_time", "env", "json", "encode", "mail", "query",
    "exec",
];

impl Emitter {
    pub(crate) fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Const(c, _) => self.constant(c),
            Expr::Local(v, _) | Expr::Move(v, _) => self.local(v),
            Expr::Load(f, _) => self.load(f),
            Expr::Derived(d, _) => {
                if self.declared(d) {
                    d.clone()
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
            Expr::FnRef(name, _) => self.fn_ref(name),
            Expr::Call { .. } | Expr::Effect { .. } => self.call(e),
            Expr::Convert { how, expr, ty } => match how {
                Convert::IntToFloat => {
                    let inner = self.expr(expr);
                    if self.emitting_dual {
                        inner
                    } else {
                        format!("(({}) as f64)", inner)
                    }
                }
                Convert::Unwrap => {
                    let inner = self.expr(expr);
                    if matches!(self.tt(expr), CrowType::Result(_)) {
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
                        CrowType::Result(p) => {
                            let reread = matches!(expr.as_ref(), Expr::Local(..) | Expr::Derived(..) | Expr::Load(..));
                            if reread && !Self::is_copy_type(&p) {
                                format!("({}).value.clone()", inner)
                            } else {
                                format!("({}).value", inner)
                            }
                        }
                        _ => inner,
                    }
                }
                Convert::ToStr => {
                    let inner = self.expr(expr);
                    format!("format!(\"{{}}\", {})", inner)
                }
                // a tensor's numbers as a list's items, ints where the list
                // holds ints; a list's numbers as a tensor of one axis
                Convert::ToList => {
                    let inner = self.expr(expr);
                    if !matches!(self.tt(expr), CrowType::NumArray(_)) {
                        return inner;
                    }
                    if matches!(ty, Ty::List(e) if **e == Ty::Int) {
                        format!("({}).to_vec().into_iter().map(|x| x as i64).collect::<Vec<i64>>()", inner)
                    } else {
                        format!("({}).to_vec()", inner)
                    }
                }
                Convert::ToTensor => {
                    let inner = self.expr(expr);
                    match self.tt(expr) {
                        // numbers that can fail: a failed one stops where
                        // they are read, with the interpreter's words (a
                        // missing cell's, or the item's reason)
                        CrowType::Array(item) if matches!(item.as_ref(), CrowType::Result(_)) => {
                            self.needs_crow_runtime = true;
                            format!(
                                "{{ let _xs = &({}); let mut _v: Vec<f64> = Vec::with_capacity(_xs.len()); for (_i, _x) in _xs.iter().enumerate() {{ if !_x.ok {{ crow_stop({}, if _x.kind == \"missing\" {{ crow_runtime::stops::missing_cell(_i) }} else {{ format!(\"item {{}} of the list failed: {{}}\", _i, _x.reason) }}) }} _v.push(_x.value as f64); }} let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}",
                                inner, self.current_line
                            )
                        }
                        CrowType::Array(_) => {
                            self.needs_crow_runtime = true;
                            format!("{{ let _v: Vec<f64> = ({}).iter().map(|x| *x as f64).collect(); let _n = _v.len(); crow_runtime::tensor::CrowTensor::from_vec(_v, vec![_n]) }}", inner)
                        }
                        // a frame's column where numbers are taken: its
                        // numbers, or the reason it has none (a missing cell,
                        // text) — the rule the interpreter reads its cells by
                        CrowType::FrameCol => {
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
            Expr::MapLit(pairs, _) => self.map_lit(pairs),
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
        if self.static_handle_vars.contains(v) && !self.declared(v) {
            return format!("__crow_handle_{}.get().unwrap().clone()", v);
        }
        v.to_string()
    }

    fn load(&self, f: &str) -> String {
        if self.static_handle_vars.contains(f) {
            return format!("__crow_handle_{}.get().unwrap().clone()", f);
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
                let non_copy = matches!(t, CrowType::Result(_))
                    || matches!(Self::inner(&t), CrowType::NumArray(_) | CrowType::Array(_) | CrowType::Map(_) | CrowType::Str | CrowType::Markup | CrowType::Frame | CrowType::Struct(_) | CrowType::Json);
                if non_copy && !s.ends_with(".clone()") {
                    return format!("{}.clone()", s);
                }
            }
        }
        if let Expr::Local(name, ty) | Expr::Move(name, ty) = e {
            if self.ref_params.contains(name) {
                return format!("{}.clone()", s);
            }
            if self.static_handle_vars.contains(name) && !self.declared(name) {
                return s;
            }
            let mut t = self.local_ty(name).unwrap_or_else(|| crow_type(ty));
            if matches!(t, CrowType::Unknown) && self.reactive_event_handlers.contains(&self.cur_fn) {
                t = CrowType::Str;
            }
            let non_copy = matches!(
                t.text(),
                CrowType::Str | CrowType::Array(_) | CrowType::NumArray(_) | CrowType::Map(_) | CrowType::Struct(_) | CrowType::Frame | CrowType::Json | CrowType::Result(_)
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
            let t = self.local_ty(name).unwrap_or_else(|| crow_type(ty));
            if matches!(t.text(), CrowType::Str | CrowType::Array(_) | CrowType::NumArray(_) | CrowType::Map(_) | CrowType::Struct(_) | CrowType::Frame | CrowType::Json | CrowType::Result(_)) {
                return format!("{}.clone()", s);
            }
        }
        s
    }

    /// The collection argument of an in-place builtin: moved when it is the
    /// variable this statement assigns, cloned otherwise.
    pub(crate) fn moved_or_cloned(&self, arg: &Expr, s: &str) -> String {
        if matches!(Emitter::raw(arg), Expr::Local(n, _) | Expr::Move(n, _) if self.move_target.as_deref() == Some(n.as_str())) {
            s.to_string()
        } else {
            format!("{}.clone()", s)
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
    fn holds_numbers(t: &CrowType) -> bool {
        match t {
            CrowType::NumArray(_) => true,
            CrowType::Array(inner) => matches!(inner.as_ref(), CrowType::Int | CrowType::Float) || (matches!(inner.as_ref(), CrowType::Array(_) | CrowType::NumArray(_)) && Self::holds_numbers(inner)),
            _ => false,
        }
    }

    /// Two operands that hold numbers in different Rust types.
    fn numbers_mix(a: &CrowType, b: &CrowType) -> bool {
        Self::holds_numbers(a) && Self::holds_numbers(b) && a != b
    }

    pub(crate) fn binary(&mut self, op: &str, left: &Expr, right: &Expr) -> String {
        // an int made a float for a float operand: when the int can fail, the
        // operator unwraps it and converts the value
        let strip = |e: &Expr, me: &Emitter| -> Expr {
            match e {
                Expr::Convert { how: Convert::IntToFloat, expr, .. } if matches!(me.tt(expr), CrowType::Result(_)) => expr.as_ref().clone(),
                other => other.clone(),
            }
        };
        let left = &strip(left, self);
        let right = &strip(right, self);
        if matches!(op, "eq" | "ne") {
            if let Some(s) = self.optbox_comparison(left, op, right) {
                return s;
            }
        }
        let l = self.operand(left);
        let r = self.operand(right);
        let lt = self.tt(left);
        let rt = self.tt(right);
        let l_res = matches!(lt, CrowType::Result(_));
        let r_res = matches!(rt, CrowType::Result(_));
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
            // both sides are evaluated, the left first: no short-circuit
            "and" => "&",
            "or" => "|",
            other => other,
        };
        if l_res || r_res {
            // an operator's value is its failed operand, unchanged; both
            // operands are evaluated, the left first, and the left's error
            // decides before the right's, as in the interpreter
            let ctr = self.binop_counter;
            self.binop_counter += 1;
            let result_type = match op {
                "eq" | "ne" | "gt" | "lt" | "gte" | "lte" | "and" | "or" => CrowType::Bool,
                "mod" => CrowType::Int,
                _ => lti.unify(&rti),
            };
            let default_val = result_type.default_rust_value();
            let l_val = if l_res { format!("_binl_{}.value", ctr) } else { format!("(*_binl_{})", ctr) };
            let r_val = if r_res { format!("_binr_{}.value", ctr) } else { format!("(*_binr_{})", ctr) };
            let inner_expr = match (&lti, &rti) {
                _ if matches!(op, "eq" | "ne") && Self::numbers_mix(&lti, &rti) => {
                    let not = if op == "ne" { "!" } else { "" };
                    format!("{}crow_runtime::tensor::numbers_equal(&{}, &{})", not, l_val, r_val)
                }
                (CrowType::Int, CrowType::Float) => format!("({}) as f64 {} {}", l_val, op_str, r_val),
                (CrowType::Float, CrowType::Int) => format!("{} {} ({}) as f64", l_val, op_str, r_val),
                (CrowType::Int, CrowType::Int) if self.int_op(op, &l_val, &r_val).is_some() => self.int_op(op, &l_val, &r_val).unwrap_or_default(),
                _ if op == "pow" => {
                    if lti == CrowType::Float || rti == CrowType::Float {
                        format!("(({}) as f64).powf(({}) as f64)", l_val, r_val)
                    } else {
                        format!("({}).pow(({}) as u32)", l_val, r_val)
                    }
                }
                _ => format!("{} {} {}", l_val, op_str, r_val),
            };
            // an operand that has no value (an `error[…]`) always fails:
            // its error is the operator's value, and there is no other
            let valueless = (l_res && matches!(lti, CrowType::Void)) || (r_res && matches!(rti, CrowType::Void));
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
        let l_sparse = matches!(lti, CrowType::Sparse);
        let r_sparse = matches!(rti, CrowType::Sparse);
        if l_sparse && r_sparse && op == "add" {
            return format!("{}.add(&{})", l, r);
        }
        if l_sparse && matches!(rti, CrowType::NumArray(_)) && op == "mul" {
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
        let l_t = matches!(lti, CrowType::NumArray(_));
        let r_t = matches!(rti, CrowType::NumArray(_));
        let cmp = matches!(op, "gt" | "lt" | "gte" | "lte" | "eq" | "ne");
        if l_t && r_t {
            return match op {
                // a shape mismatch stops the program as the interpreter
                // stops it, with the runtime's words, never a bare panic
                "add" => format!("{}.try_elementwise(&{}, \"+\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "sub" => format!("{}.try_elementwise(&{}, \"-\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "mul" => format!("{}.try_matmul(&{}).unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "div" => "compile_error!(\"use ./ for element-wise tensor division\")".to_string(),
                "elemmul" => format!("{}.try_elementwise(&{}, \".*\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "elemdiv" => format!("{}.try_elementwise(&{}, \"./\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "elempow" => format!("{}.try_elementwise(&{}, \".^\").unwrap_or_else(|e| crow_stop({}, e))", l, r, self.current_line),
                "eq" => format!("{} == {}", l, r),
                "ne" => format!("{} != {}", l, r),
                _ => format!("{} {} {}", l, op_str, r),
            };
        }
        if l_t && matches!(rti, CrowType::Int | CrowType::Float) {
            return match op {
                "add" => format!("{}.add_scalar({} as f64)", l, r),
                "sub" => format!("{}.sub_scalar({} as f64)", l, r),
                "mul" | "elemmul" => format!("{}.mul_scalar({} as f64)", l, r),
                "div" | "elemdiv" => format!("{}.mul_scalar(1.0 / ({} as f64))", l, r),
                "elempow" => format!("{}.map(|x| x.powf({} as f64))", l, r),
                _ if cmp => format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _v {} _s).collect::<Vec<bool>>() }}", r, l, op_str),
                _ => format!("{} {} {}", l, op_str, r),
            };
        }
        if r_t && matches!(lti, CrowType::Int | CrowType::Float) {
            if cmp {
                return format!("{{ let _s = {} as f64; {}.to_vec().iter().map(|&_v| _s {} _v).collect::<Vec<bool>>() }}", l, r, op_str);
            }
            return match op {
                "add" => format!("{}.add_scalar({} as f64)", r, l),
                "sub" => format!("({}).neg().add_scalar({} as f64)", r, l),
                "mul" | "elemmul" => format!("{}.mul_scalar({} as f64)", r, l),
                "div" | "elemdiv" => format!("{{ let _t = &{}; crow_runtime::tensor::CrowTensor::from_vec(_t.to_vec().iter().map(|&v| ({} as f64) / v).collect(), _t.shape().to_vec()) }}", r, l),
                _ => format!("{} {} {}", l, op_str, r),
            };
        }
        let bool_list = |t: &CrowType| matches!(t, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool));
        if matches!(op, "and" | "or") && bool_list(&lti) && bool_list(&rti) {
            return format!("{}.iter().zip({}.iter()).map(|(_a, _b)| *_a {} *_b).collect::<Vec<bool>>()", l, r, op_str);
        }
        if matches!(lti, CrowType::FrameCol) && matches!(rti, CrowType::Str) {
            if cmp {
                return format!("{}.compare_str(&{}, \"{}\")", l, r, op);
            }
            return format!("{} {} {}", l, op_str, r);
        }
        let str_list = matches!(&lti, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Str));
        if (matches!(lti, CrowType::FrameCol) || str_list) && matches!(rti, CrowType::Int | CrowType::Float) {
            if cmp {
                if matches!(lti, CrowType::FrameCol) {
                    return format!("{}.compare({} as f64, \"{}\")", l, r, op);
                }
                return format!("crow_runtime::frame::col_compare(&{}, {} as f64, \"{}\")", l, r, op);
            }
            let arith = match op {
                "add" | "sub" | "mul" | "div" => op,
                _ => return format!("{} {} {}", l, op_str, r),
            };
            return format!("crow_runtime::frame::col_arithmetic(&{}, {} as f64, \"{}\")", l, r, arith);
        }
        if self.emitting_dual {
            if op == "pow" {
                return format!("({}).powf({})", l, r);
            }
            return format!("{} {} {}", l, op_str, r);
        }
        if matches!((&lti, &rti), (CrowType::Int, CrowType::Int)) {
            if let Some(checked) = self.int_op(op, &l, &r) {
                return checked;
            }
        }
        if op == "pow" {
            if lti == CrowType::Float || rti == CrowType::Float {
                return format!("(({}) as f64).powf(({}) as f64)", l, r);
            }
            return format!("({}).pow(({}) as u32)", l, r);
        }
        match (&lti, &rti) {
            (CrowType::Int, CrowType::Float) => format!("({}) as f64 {} {}", l, op_str, r),
            (CrowType::Float, CrowType::Int) => format!("{} {} ({}) as f64", l, op_str, r),
            _ => format!("{} {} {}", l, op_str, r),
        }
    }

    /// An int operator that can fail — past 64 bits, a division by zero, a
    /// negative power — checked by the runtime, which has the words both
    /// modes stop with (crow_runtime::stops).
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
        Some(format!("crow_runtime::stops::{}({}, {}).unwrap_or_else(|e| crow_stop({}, e))", f, l, r, self.current_line))
    }

    fn unary(&mut self, op: &str, e: &Expr) -> String {
        let inner = self.operand(e);
        let ty = Self::inner(&self.tt(e));
        match op {
            "neg" => {
                if matches!(ty, CrowType::NumArray(_)) {
                    format!("{}.neg()", inner)
                } else if matches!(ty, CrowType::Int) && !matches!(Emitter::raw(e), Expr::Const(..)) && !self.emitting_dual {
                    // only the smallest int has no negation
                    format!("crow_runtime::stops::neg({}).unwrap_or_else(|e| crow_stop({}, e))", inner, self.current_line)
                } else {
                    format!("(-{})", inner)
                }
            }
            _ => {
                if matches!(&ty, CrowType::Array(i) if matches!(i.as_ref(), CrowType::Bool)) {
                    format!("{}.iter().map(|_b| !*_b).collect::<Vec<bool>>()", inner)
                } else if matches!(self.tt(e), CrowType::Result(_)) {
                    let on_error = self.refuse("_c");
                    format!("(!({{ let _c = &({}); if !_c.ok {{ {} }} _c.value }}))", inner, on_error)
                } else {
                    format!("(!({}))", inner)
                }
            }
        }
    }

    /// `tree.left eq 0` on a recursive link: is the link empty?
    fn optbox_comparison(&mut self, left: &Expr, op: &str, right: &Expr) -> Option<String> {
        let (field_side, zero_side) = if matches!(right, Expr::Const(Const::Int(0), _)) {
            (left, right)
        } else if matches!(left, Expr::Const(Const::Int(0), _)) {
            (right, left)
        } else {
            return None;
        };
        let _ = zero_side;
        if let Expr::Field { base, name, .. } = field_side {
            if let CrowType::Struct(sname) = self.tt(base) {
                let is_opt = self.structs.get(&sname).map_or(false, |d| d.fields.iter().any(|(f, t)| f == name && matches!(t, Ty::OptRecord(_))));
                if is_opt {
                    let obj = self.expr(base);
                    return match op {
                        "eq" => Some(format!("{}.{}.is_none()", obj, Self::rust_field(name))),
                        "ne" => Some(format!("{}.{}.is_some()", obj, Self::rust_field(name))),
                        _ => None,
                    };
                }
            }
        }
        None
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
                    CrowType::Int => return format!("{}.get_i64(&{}, {})", o, k, v),
                    CrowType::Float => return format!("{}.get_f64(&{}, {})", o, k, v),
                    CrowType::Str => return format!("{}.get_str(&{}, &{})", o, k, v),
                    CrowType::Bool => return format!("{}.get_bool(&{}, {})", o, k, v),
                    _ => {}
                }
            }
        }
        let bty = self.tt(base);
        let is_result = matches!(bty, CrowType::Result(_));
        // a base that can fail, indexed: its error is the index's value, as
        // the interpreter hands it on (the payload was read regardless: a
        // failed JSON parse, indexed, was null)
        if is_result && matches!(self.tt(whole), CrowType::Result(_)) {
            let raw_base = self.expr(base);
            let payload = Expr::Local("_ib".to_string(), crate::lower_ir::ty_of(&Self::inner(&bty)));
            let read = self.index(&payload, indices, whole);
            let default = Self::inner(&self.tt(whole)).default_rust_value();
            let read = if matches!(Self::inner(&bty), CrowType::Json) { read } else { format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", read) };
            return format!(
                "{{ let _b = &({}); if !_b.ok {{ CrowResult {{ ok: false, value: {}, reason: _b.reason.clone(), kind: _b.kind.clone(), trace: _b.trace.clone(), captured: 0.0 }} }} else {{ let _ib = _b.value.clone(); {} }} }}",
                raw_base, default, read
            );
        }
        let inner = Self::inner(&bty);
        let raw_base = self.expr(base);
        let b = if is_result { format!("{}.value", raw_base) } else { raw_base };
        // an index that failed halts with its reason, as in the interpreter;
        // one that did not is its value
        let idx_strs: Vec<String> = indices
            .iter()
            .map(|i| {
                let s = self.expr(i);
                if matches!(self.tt(i), CrowType::Result(_)) {
                    format!("{{ let _ix = &({}); if !_ix.ok {{ crow_stop({}, _ix.reason.clone()); }} _ix.value.clone() }}", s, self.current_line)
                } else {
                    s
                }
            })
            .collect();
        let idx = idx_strs.first().cloned().unwrap_or_default();
        let idx_ty = indices.first().map(|x| Self::inner(&self.tt(x)));
        match &inner {
            CrowType::NumArray(el) => {
                if matches!(idx_ty, Some(CrowType::NumArray(_))) {
                    return format!("{}.try_gather(&{}){}", b, idx, self.or_stop());
                }
                // an index outside stops the program with the runtime's words
                let line = self.current_line;
                if matches!(el.as_ref(), CrowType::NumArray(_)) {
                    return format!("{{ let _i = {}; let _t = &{}; _t.index0(crow_runtime::stops::row(_i, _t.shape()[0]).unwrap_or_else(|e| crow_stop({}, e))) }}", idx, b, line);
                }
                let cast = if matches!(el.as_ref(), CrowType::Int) { " as i64" } else { "" };
                format!("{{ let _i = {}; let _t = &{}; _t.get_flat(crow_runtime::stops::item(_i, _t.len()).unwrap_or_else(|e| crow_stop({}, e))){} }}", idx, b, line, cast)
            }
            CrowType::Array(el) => {
                let suffix = if Self::is_copy_type(el) { "" } else { ".clone()" };
                format!("{{ let _i = {}; let _v = &{}; _v[crow_runtime::stops::item(_i, _v.len()).unwrap_or_else(|e| crow_stop({}, e))]{} }}", idx, b, self.current_line, suffix)
            }
            CrowType::Str | CrowType::Markup => {
                format!("{{ let _s: Vec<char> = {}.chars().collect(); let _i = {}; _s[crow_runtime::stops::char_at(_i, _s.len()).unwrap_or_else(|e| crow_stop({}, e))].to_string() }}", b, idx, self.current_line)
            }
            CrowType::FrameCol => {
                let getter = Self::cell_getter(&Self::inner(&self.tt(whole)));
                // a column that can have missing cells: the cell is a result,
                // a missing one an error
                let line = self.current_line;
                if matches!(self.tt(whole), CrowType::Result(_)) {
                    return format!(
                        "{{ let _i = {}; let _c = &{}; let _idx = crow_runtime::stops::item(_i, _c.len()).unwrap_or_else(|e| crow_stop({}, e)); let _ok = _c.valid[_idx]; let _why = if _ok {{ String::new() }} else {{ \"missing\".to_string() }}; CrowResult {{ ok: _ok, value: _c.{}(_idx), reason: _why.clone(), kind: _why, trace: vec![], captured: 0.0 }} }}",
                        idx, b, line, getter
                    );
                }
                format!("{{ let _i = {}; let _c = &{}; _c.{}(crow_runtime::stops::item(_i, _c.len()).unwrap_or_else(|e| crow_stop({}, e))) }}", idx, b, getter, line)
            }
            CrowType::Map(_) => {
                self.needs_crow_runtime = true;
                format!("crow_runtime::core::map_key(&{}, &*({})).unwrap_or_else(|e| crow_stop({}, e))", b, idx_strs.join(", "), self.current_line)
            }
            // an element of a JSON value: JSON's null is an error value of
            // reason `null`, as the interpreter reads it
            CrowType::Json => {
                let ie = match idx_ty.clone() {
                    Some(CrowType::Int) => format!("{} as usize", idx),
                    _ => format!("&*{}", idx),
                };
                format!(
                    "{{ let _e = {}[{}].clone(); if _e.is_null() {{ CrowResult {{ ok: false, value: serde_json::Value::Null, reason: \"null\".to_string(), kind: \"parse\".to_string(), trace: vec![], captured: 0.0 }} }} else {{ CrowResult {{ ok: true, value: _e, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }} }} }}",
                    b, ie
                )
            }
            CrowType::Fn(ps, _) => {
                let casted: Vec<String> = idx_strs
                    .iter()
                    .enumerate()
                    .map(|(i, s)| if matches!(ps.get(i), Some(CrowType::Float)) && matches!(indices.get(i).map(|x| self.tt(x)), Some(CrowType::Int)) { format!("{} as f64", s) } else { s.clone() })
                    .collect();
                format!("{}({})", b, casted.join(", "))
            }
            _ => format!("{}({})", b, idx_strs.join(", ")),
        }
    }

    pub(crate) fn cell_getter(t: &CrowType) -> &'static str {
        match t {
            CrowType::Int => "get_i64",
            CrowType::Float => "get_f64",
            CrowType::Bool => "get_bool",
            // a column nothing typed: the cell as the value it is
            CrowType::Json => "get_json",
            _ => "get_str",
        }
    }

    fn field(&mut self, base: &Expr, field: &str) -> String {
        let obj = self.expr(base);
        let oty = self.tt(base);
        // a recursive link: unwrap the box
        if let CrowType::OptBoxStruct(sname) = &oty {
            let is_copy = self.structs.get(sname).and_then(|s| s.fields.iter().find(|(f, _)| f == field).map(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool))).unwrap_or(false);
            let suffix = if is_copy { "" } else { ".clone()" };
            return format!("{}.as_ref().expect(\"field access on an empty (0) node\").{}{}", obj, field, suffix);
        }
        if let CrowType::Result(inner) = &oty {
            if let CrowType::Struct(sname) = inner.as_ref() {
                let is_copy = self.structs.get(sname).and_then(|s| s.fields.iter().find(|(f, _)| f == field).map(|(_, t)| matches!(t, Ty::Int | Ty::Float | Ty::Bool))).unwrap_or(false);
                let suffix = if is_copy { "" } else { ".clone()" };
                if matches!(sname.as_str(), "CrowShell" | "CrowFindResult") {
                    let suffix = if field == "status" { "" } else { ".clone()" };
                    return format!("{}.value.{}{}", obj, field, suffix);
                }
                return format!("{}.value.{}{}", obj, Self::rust_field(field), suffix);
            }
            if matches!(inner.as_ref(), CrowType::Map(_)) {
                self.needs_crow_runtime = true;
                return format!("crow_runtime::core::map_key(&{}.value, \"{}\").unwrap_or_else(|e| crow_stop({}, e))", obj, field, self.current_line);
            }
        }
        let oinner = Self::inner(&oty);
        if matches!(oinner, CrowType::Json) {
            let o = if matches!(oty, CrowType::Result(_)) { format!("{}.value", obj) } else { obj.clone() };
            return format!("{}.get(\"{}\").cloned().unwrap_or(serde_json::Value::Null)", o, field);
        }
        if field == "len" {
            if matches!(oty, CrowType::Frame) {
                return format!("({}.nrows() as i64)", obj);
            }
            if matches!(oty, CrowType::Array(_) | CrowType::NumArray(_) | CrowType::Str | CrowType::FrameCol) {
                return format!("({}.len() as i64)", obj);
            }
        }
        if matches!(field, "year" | "month" | "day" | "hour" | "minute" | "second" | "offset") && matches!(oty, CrowType::Str) {
            return format!("crow_runtime::frame::datetime_field(&{}, \"{}\").unwrap_or(0)", obj, field);
        }
        if matches!(oinner, CrowType::NumArray(_)) {
            let t = if matches!(oty, CrowType::Result(_)) { format!("{}.value.clone()", obj) } else { obj.clone() };
            match field {
                "shape" => return format!("{}.shape().iter().map(|&s| s as i64).collect::<Vec<i64>>()", t),
                "dtype" => return "\"float\".to_string()".to_string(),
                "device" => return format!("if {}.is_gpu() {{ \"gpu\".to_string() }} else {{ \"cpu\".to_string() }}", t),
                "grad" => return "\"none\".to_string()".to_string(),
                _ => {}
            }
        }
        if matches!(oinner, CrowType::Frame) {
            let f = if matches!(oty, CrowType::Result(_)) { format!("{}.value.clone()", obj) } else { obj.clone() };
            return match field {
                "shape" => format!("vec![{f}.nrows() as i64, {f}.ncols() as i64]", f = f),
                "columns" => format!("{}.columns.iter().map(|c: &String| c.clone()).collect::<Vec<String>>()", f),
                "dtypes" => format!("{}.dtypes().iter().map(|d: &&str| d.to_string()).collect::<Vec<String>>()", f),
                _ => format!("{}.col_native(\"{}\").unwrap_or_else(|| crow_stop({}, crow_runtime::stops::no_column(\"{}\")))", f, field, self.current_line, field),
            };
        }
        if matches!(oty, CrowType::Map(_)) {
            self.needs_crow_runtime = true;
            return format!("crow_runtime::core::map_key(&{}, \"{}\").unwrap_or_else(|e| crow_stop({}, e))", obj, field, self.current_line);
        }
        if let CrowType::Struct(sname) = &oty {
            if matches!(sname.as_str(), "CrowShell" | "CrowFindResult" | "CrowElapsed") {
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
        if matches!(oty, CrowType::Result(_)) {
            return match name {
                "ok" => format!("{}.ok", obj),
                "trace" => format!("{}.trace.clone()", obj),
                other => format!("{}.{}.clone()", obj, other),
            };
        }
        if which == Meta::Valid || (name == "ok" && matches!(oty, CrowType::FrameCol)) {
            return format!("{}.ok()", obj);
        }
        if let CrowType::Struct(_) = oty {
            return format!("{}.{}", obj, Self::rust_field(name));
        }
        // a dynamic value that is JSON null is an error value, reason 'null'
        if matches!(oty, CrowType::Json) {
            return match name {
                "ok" => format!("!({}).is_null()", obj),
                "reason" => format!("(if ({}).is_null() {{ \"null\".to_string() }} else {{ String::new() }})", obj),
                "trace" => "Vec::<String>::new()".to_string(),
                _ => format!("(if ({}).is_null() {{ \"parse\".to_string() }} else {{ String::new() }})", obj),
            };
        }
        match name {
            "ok" => "true".to_string(),
            "trace" => "Vec::<String>::new()".to_string(),
            _ => "String::new()".to_string(),
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
                    if matches!(ty, CrowType::Result(_)) {
                        let b = format!("_ip{}", fallible.len());
                        fallible.push((b.clone(), format!("({}).clone()", rust)));
                        rust = format!("{}.value", b);
                    }
                    // an error with no value (error[…]): the text is its
                    // error, and the value it would show never comes
                    if matches!(inner, CrowType::Void) {
                        fmt.push_str("{}");
                        args.push("\"\"".to_string());
                        continue;
                    }
                    match &inner {
                        CrowType::FrameCol => {
                            fmt.push_str("{:?}");
                            args.push(format!("{}.to_strings()", rust));
                        }
                        t if self.kernel_displayable(t) => {
                            self.needs_crow_runtime = true;
                            fmt.push_str("{}");
                            args.push(format!("{{ use crow_runtime::display::CrowDisplay as _; ({}).crow_fmt() }}", rust));
                        }
                        CrowType::Array(_) | CrowType::Map(_) | CrowType::Struct(_) => {
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
        let lt = crow_type(ty).text();
        if let CrowType::NumArray(_) = &lt {
            if !items.is_empty() {
                let elem_is_tensor = items.iter().any(|i| matches!(self.tt(i), CrowType::NumArray(_)));
                self.needs_crow_runtime = true;
                if elem_is_tensor {
                    let elems: Vec<String> = items.iter().map(|i| self.expr(i)).collect();
                    return format!("crow_runtime::tensor::CrowTensor::from_elements(&[{}]){}", elems.join(", "), self.or_stop());
                }
                let vals: Vec<String> = items.iter().map(|i| format!("{} as f64", self.expr(i))).collect();
                return format!("crow_runtime::tensor::CrowTensor::from_vec(vec![{}], vec![{}])", vals.join(", "), items.len());
            }
        }
        let mut inner_type = CrowType::Unknown;
        for i in items {
            inner_type = inner_type.unify(&self.tt(i));
        }
        // a list of results holds a plain item as a result that did not fail
        let of_results = matches!(&lt, CrowType::Array(inner) if matches!(inner.as_ref(), CrowType::Result(_)));
        let vals: Vec<String> = if let CrowType::Fn(_, _) = &inner_type {
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
                        CrowType::Array(item) if of_results => self.as_item(s, &self.tt(i), item),
                        _ => s,
                    }
                })
                .collect()
        };
        if items.is_empty() {
            if let CrowType::Array(inner) = &lt {
                if !matches!(inner.as_ref(), CrowType::Unknown) {
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
    pub(crate) fn as_item(&self, s: String, from: &CrowType, item: &CrowType) -> String {
        let CrowType::Result(inner) = item else { return s };
        match from {
            CrowType::Result(v) if matches!(v.as_ref(), CrowType::Void) && !matches!(inner.as_ref(), CrowType::Void) => format!(
                "{{ let _e = {}; CrowResult {{ ok: false, value: {}, reason: _e.reason, kind: _e.kind, trace: _e.trace, captured: 0.0 }} }}",
                s,
                inner.default_rust_value()
            ),
            CrowType::Result(_) => s,
            _ => format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", s),
        }
    }

    fn map_lit(&mut self, pairs: &[(String, Expr)]) -> String {
        if pairs.is_empty() {
            return "BTreeMap::new()".to_string();
        }
        let val_types: Vec<CrowType> = pairs.iter().map(|(_, v)| self.tt(v)).collect();
        let mut val_type = CrowType::Unknown;
        for t in &val_types {
            val_type = val_type.unify(t);
        }
        let mixed = val_types.iter().any(|t| t != &val_type && *t != CrowType::Unknown);
        let entries: Vec<String> = if let CrowType::Fn(_, _) = &val_type {
            let ft = val_type.to_rust_type();
            pairs.iter().map(|(k, v)| format!("({}, {} as {})", rust_str_lit(k), self.expr(v), ft)).collect()
        } else if mixed {
            pairs
                .iter()
                .zip(val_types.iter())
                .map(|((k, v), vt)| {
                    let s = self.expr(v);
                    let s = if matches!(vt, CrowType::Str) { s } else { format!("{}.to_string()", s) };
                    format!("({}, {})", rust_str_lit(k), s)
                })
                .collect()
        } else {
            pairs.iter().map(|(k, v)| format!("({}, {})", rust_str_lit(k), self.expr(v))).collect()
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
                } else {
                    inits.push(format!("{}: Some(Box::new({}))", Self::rust_field(fname), owned));
                }
            } else {
                // a field that can hold a failed value takes a plain one as
                // ok (`Last[false, …]` where another literal gave it a
                // fallible comparison)
                let owned = self.as_item(owned, &self.tt(e), &crate::lower_ir::crow_type(fty));
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
        if let CrowType::Result(_) = self.tt(iter) {
            let list = format!("_list_{}", self.binop_counter);
            self.binop_counter += 1;
            let printed = self.expr(iter);
            let lty = self.tt(iter);
            let on_error = self.refuse_at(&list, &Decision::Loop);
            self.declare(&list, lty);
            let value = Expr::Convert { how: Convert::Unwrap, expr: Box::new(Expr::Local(list.clone(), iter.ty())), ty: iter.ty().payload().clone() };
            let walked = self.for_expr(var, &value, body, parallel, whole);
            return format!("{{ let {l} = &({p}); if !{l}.ok {{ {e} }} {w} }}", l = list, p = printed, e = on_error, w = walked);
        }
        // in parallel as the lowering decided, unless cpu[1] asked for one core
        let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
        let use_par = !forced_sequential && parallel;
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
        let method = if use_par { "par_iter" } else { "iter" };
        let it = self.tt(iter);
        let (iter_str, elem) = self.loop_items(iter_str, &it);
        let is_copy = Self::is_copy_type(&elem);
        // an item that is not Copy is borrowed from what the loop walks; the
        // body reads it as its own value, as the interpreter's does
        let (param, rebind) = if is_copy {
            (format!("|&{}|", var), String::new())
        } else {
            (format!("|__item_{}|", var), format!("let {v} = __item_{v}.clone(); ", v = var))
        };

        if let Some((cond, val)) = Self::filter_parts(body) {
            self.push_scope();
            self.declare(var, elem.clone());
            let c = self.expr(cond);
            let c = self.guard_condition(cond, c);
            let mut v = self.expr(val);
            let mut vt = self.tt(val);
            if self.makes_node(val) {
                v = format!("({}).to_html()", v);
                vt = CrowType::Str;
            }
            self.pop_scope();
            return format!("{}.{}().filter_map({} {{ {}if {} {{ Some({}) }} else {{ None }} }}).collect::<Vec<{}>>()", iter_str, method, param, rebind, c, v, vt.to_rust_type());
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
            let list_ty = crow_type(&list);
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
            return format!("{}.{}().flat_map_iter({} {{\n{}{}}}).collect::<{}>()", iter_str, method, param, closure_body, pad, list_ty.to_rust_type());
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
        let mut last_type = CrowType::Unknown;
        if let Some(Stmt::Do(e)) = last_expr {
            let mut s = self.expr(e);
            last_type = self.tt(e);
            if self.makes_node(e) {
                s = format!("({}).to_html()", s);
                last_type = CrowType::Str;
            }
            let pad = "    ".repeat(self.indent);
            self.out.push_str(&format!("{}{}\n", pad, s));
        }
        self.pop_scope();
        let closure_body = std::mem::replace(&mut self.out, saved_out);
        self.indent = saved_indent;
        let pad = "    ".repeat(self.indent);
        format!("{}.{}().map({} {{\n{}{}}}).collect::<Vec<{}>>()", iter_str, method, param, closure_body, pad, last_type.to_rust_type())
    }

    /// A collecting loop as a loop that gathers into a list: each value
    /// the body yields is pushed, and the loop is the statement form's.
    fn gathering_loop(&mut self, var: &str, iter: &Expr, body: &[Stmt], whole: &Expr) -> String {
        let list = match whole.ty() {
            Ty::List(item) => Ty::List(item),
            _ => Ty::List(Box::new(Ty::Unknown)),
        };
        let list_ty = crow_type(&list);
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
            Target::User(n) => self.fns.get(n).map_or(false, |f| matches!(f.ret, Ty::Result(_))),
            Target::Module(m, f) => self.registry.type_of_in(m, f, &raw_args.iter().map(|a| self.tt(a)).collect::<Vec<_>>()).map_or(false, |t| matches!(t, CrowType::Result(_))),
            Target::Runtime(_) if self.row_for(name, raw_args).is_some() => {
                matches!(self.row_for(name, raw_args).map(|r| r.fails), Some(crate::table::Fails::Value(_)))
            }
            Target::Runtime(_) => {
                if name == "set" {
                    return false;
                }
                if self.registry.has_function(name) && !matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round" | "min" | "max") {
                    if let Some(t) = self.registry.type_of(name, &raw_args.iter().map(|a| self.tt(a)).collect::<Vec<_>>()) {
                        return matches!(t, CrowType::Result(_));
                    }
                }
                FALLIBLE_BUILTINS.contains(&name)
            }
        }
    }

    fn call(&mut self, e: &Expr) -> String {
        let (mut target, args, propagates) = match e {
            Expr::Call { target, args, propagates, .. } => (target.clone(), args.clone(), *propagates),
            Expr::Effect { name, args, propagates, .. } => {
                let t = match name.split_once('.') {
                    Some((m, f)) => Target::Module(m.to_string(), f.to_string()),
                    None => Target::Runtime(name.clone()),
                };
                (t, args.clone(), *propagates)
            }
            _ => unreachable!(),
        };
        // a qualified call to a registry module is its bare call, pinned
        let mut pinned: Option<String> = None;
        if let Target::Module(m, f) = &target {
            if matches!(m.as_str(), "math" | "io" | "frame" | "doc" | "chart" | "math.stats" | "math.nn") && self.registry.has_function(f) {
                pinned = Some(m.clone());
                target = Target::Runtime(f.clone());
            }
        }
        let name = match &target {
            Target::Runtime(p) => RUNTIME_TABLE.iter().find(|(_, r)| *r == p.as_str()).map(|(b, _)| b.to_string()).unwrap_or_else(|| p.clone()),
            Target::User(n) => n.clone(),
            Target::Module(_, f) => f.clone(),
        };
        let raw_args: Vec<Expr> = args.iter().map(|a| Emitter::raw(a).clone()).collect();
        let current_pin = self.pinned_module.clone();
        // a module's function named like an element (`frame.stack`) is the
        // module's: it propagates, where the element renders a failed child
        let from_module = pinned.is_some();
        let saved_pin = std::mem::replace(&mut self.pinned_module, pinned.or(current_pin));

        let user = matches!(target, Target::User(_));
        let ref_flags: Vec<bool> = match &target {
            Target::User(n) => match (self.by_ref_params.get(n), self.fns.get(n)) {
                (Some(set), Some(f)) => f.params.iter().map(|(p, _)| set.contains(p)).collect(),
                _ => vec![false; raw_args.len()],
            },
            _ => vec![false; raw_args.len()],
        };
        let mut arg_strs: Vec<String> = Vec::with_capacity(raw_args.len());
        for (i, a) in raw_args.iter().enumerate() {
            if ref_flags.get(i).copied().unwrap_or(false) {
                if let Some(s) = self.optbox_ref(a) {
                    arg_strs.push(s);
                } else if let Expr::Local(v, _) | Expr::Move(v, _) = a {
                    if self.ref_params.contains(v) {
                        arg_strs.push(v.clone());
                    } else {
                        arg_strs.push(format!("&{}", v));
                    }
                } else {
                    let s = self.expr(a);
                    arg_strs.push(format!("&{}", s));
                }
            } else if let Some(s) = self.optbox_value(a) {
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
                let needs_clone = user
                    && local_read
                    && matches!(
                        Self::inner(&self.tt(a)),
                        CrowType::NumArray(_) | CrowType::Array(_) | CrowType::Map(_) | CrowType::Str | CrowType::Markup | CrowType::Frame | CrowType::Struct(_) | CrowType::DbHandle
                    );
                let s = self.expr(a);
                arg_strs.push(if needs_clone && !(matches!(a, Expr::Load(n, _) if self.static_handle_vars.contains(n))) { format!("{}.clone()", s) } else { s });
            }
        }

        // an argument's error is the call's value (spec, auto-propagation),
        // unless the IR says the call takes it as it is (a collecting loop
        // gathering what an iteration yielded, errors included)
        let skip = (!propagates && matches!(target, Target::Runtime(_)))
            || matches!(name.as_str(), "log" | "assert" | "cpu" | "gpu" | "nodes" | "type" | "error" | "captured" | "running")
            || (self.needs_ui && !from_module && (name == "chart" || name == "table" || crate::lower::UI_ELEMENTS.contains(&name.as_str())))
            || (self.needs_crow_web && name == "arrow");
        if !skip {
            let result_idx: Vec<usize> = raw_args.iter().enumerate().filter(|(_, a)| matches!(self.tt(a), CrowType::Result(_))).map(|(i, _)| i).collect();
            if !result_idx.is_empty() {
                let orig: Vec<String> = result_idx.iter().map(|&i| arg_strs[i].clone()).collect();
                // a view, or a control, renders a failed argument as its reason
                let renders = match &target {
                    Target::User(v) => self.views.contains(v),
                    Target::Runtime(_) => {
                        self.in_view() && crate::lower::is_ui_control(&name, raw_args.first().map_or(false, |x| matches!(Self::inner(&self.tt(x)), CrowType::Frame)))
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
                            checks.push(format!("if !_va{k}.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _va{k}.reason)) }} else", k = k));
                            bound[i] = format!("_va{}.value.clone()", k);
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
                    unwrapped[i] = if matches!(Self::inner(&self.tt(&raw_args[i])), CrowType::Void) {
                        "unreachable!()".to_string()
                    } else if ref_flags.get(i).copied().unwrap_or(false) {
                        // a struct the callee borrows is lent, not copied
                        format!("&_prop_{}.value", i)
                    } else {
                        format!("_prop_{}.value.clone()", i)
                    };
                }
                let inner_expr = self.dispatch(&target, &name, &raw_args, &unwrapped, e);
                let full = self.tt(e);
                let inner_type = match Self::inner(&full) {
                    CrowType::Map(v) => CrowType::Map(Box::new(Self::inner(&v))),
                    CrowType::Array(v) => CrowType::Array(Box::new(Self::inner(&v))),
                    o => o,
                };
                let already = self.base_is_result(&name, &target, &raw_args);
                const HELD: &[&str] = &["query", "exec", "insert", "transact", "read", "write", "fetch", "shell", "mail", "later", "notice", "sign_in", "download"];
                let stop_as = (self.reactive_epilogue.is_some() && HELD.contains(&name.as_str())).then(|| self.shown_fn_name());
                self.pinned_module = saved_pin;
                return self.propagation_block(&result_idx, &orig, &inner_expr, &inner_type, already, stop_as);
            }
        }
        let out = self.dispatch(&target, &name, &raw_args, &arg_strs, e);
        self.pinned_module = saved_pin;
        out
    }

    fn propagation_block(&self, idx: &[usize], orig: &[String], inner: &str, inner_type: &CrowType, already: bool, stop_as: Option<String>) -> String {
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
        let mut result = if already {
            inner.to_string()
        } else {
            format!("CrowResult {{ ok: true, value: {}, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}", inner)
        };
        for (pos, &i) in idx.iter().enumerate().rev() {
            result = format!("{{ let _prop_{i} = {}.clone(); if !_prop_{i}.ok {{ {} }} else {{ {} }} }}", orig[pos], error_result(i), result, i = i);
        }
        result
    }

    /// A struct field that is a recursive link, handed by reference.
    fn optbox_ref(&mut self, e: &Expr) -> Option<String> {
        if let Expr::Field { base, name, .. } = e {
            if let CrowType::Struct(s) = self.tt(base) {
                if self.structs.get(&s).map_or(false, |d| d.fields.iter().any(|(f, t)| f == name && matches!(t, Ty::OptRecord(_)))) {
                    let obj = self.expr(base);
                    return Some(format!("{}.{}.as_ref().unwrap()", obj, Self::rust_field(name)));
                }
            }
        }
        None
    }

    /// A struct field that is a recursive link, handed by value.
    fn optbox_value(&mut self, e: &Expr) -> Option<String> {
        if let Expr::Field { base, name, .. } = e {
            if let CrowType::Struct(s) = self.tt(base) {
                if self.structs.get(&s).map_or(false, |d| d.fields.iter().any(|(f, t)| f == name && matches!(t, Ty::OptRecord(_)))) {
                    let obj = self.expr(base);
                    return Some(format!("*{}.{}.as_ref().unwrap().clone()", obj, Self::rust_field(name)));
                }
            }
        }
        None
    }

    /// A core builtin's row for a call: the one its arguments' types pick,
    /// as the interpreter picks it by their values (crate::table).
    fn row_for(&self, name: &str, args: &[Expr]) -> Option<&'static crate::table::Row> {
        let kinds: Vec<crate::table::Arg> = args.iter().map(|x| crate::table::Arg::of_type(&self.tt(x))).collect();
        crate::table::core_row(name, &kinds)
    }

    /// A call, by what it names.
    fn dispatch(&mut self, target: &Target, name: &str, args: &[Expr], arg_strs: &[String], whole: &Expr) -> String {
        match target {
            Target::User(n) => self.user_call(n, args, arg_strs.to_vec()),
            Target::Module(m, f) => self.module_call(m, f, args, arg_strs, whole),
            // (a web queue's push is the queue's; math on the dual numbers a
            // function grad differentiates is the derivative's own form)
            Target::Runtime(_) => match self
                .row_for(name, args)
                .filter(|_| !(name == "push" && self.queue_push(args)) && !(self.emitting_dual && matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round")))
            {
                Some(row) => self.table_call(row, args, arg_strs, whole),
                None => match self.builtin(name, args, arg_strs, whole) {
                    Some(s) => s,
                    None => format!("{}({})", name, arg_strs.join(", ")),
                },
            },
        }
    }

    /// `push[queue, task]`: a web queue's own push, no collection's.
    pub(crate) fn queue_push(&self, args: &[Expr]) -> bool {
        self.needs_crow_web
            && self.web_features.contains("queue")
            && !matches!(args.first().map(|x| Self::inner(&self.tt(x))), Some(CrowType::Array(_) | CrowType::NumArray(_)))
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
        // a parsed JSON value is serde's, which the program then names
        if row.ret == Kind::Json || row.ret == Kind::JsonList || row.params.contains(&Kind::Json) {
            self.needs_serde_json = true;
        }
        let mut strs = arg_strs.to_vec();
        if let Some(kinds) = crate::table::kinds_of(row.params, strs.len()) {
            let items = kinds.iter().zip(args).find_map(|(k, a)| match (k, Self::inner(&self.tt(a))) {
                (Kind::List | Kind::Seq, CrowType::Array(it)) | (Kind::MapOf | Kind::MapRef, CrowType::Map(it)) => Some(*it),
                _ => None,
            });
            let gathered = matches!(whole, Expr::Call { propagates: false, .. });
            for (i, (k, a)) in kinds.iter().zip(args).enumerate() {
                let s = strs[i].clone();
                strs[i] = match k {
                    k if k.owned() => self.moved_or_cloned(a, &s),
                    Kind::Item | Kind::ItemRef => {
                        let at = self.tt(a);
                        let s = if *k == Kind::Item { Self::kept(a, &s) } else { s };
                        // a function kept is the collection's function type
                        let s = match &at {
                            CrowType::Fn(..) => format!("({} as {})", s, at.to_rust_type()),
                            _ => s,
                        };
                        match &items {
                            Some(it @ CrowType::Result(_)) => {
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
        let trace = self.trace_format_expr();
        crate::table::print(row, &strs, &self.current_line.to_string(), Some(&trace)).unwrap_or_else(|e| format!("compile_error!({:?})", e))
    }

    /// A user function's call: a view's or an element's tree handed to it
    /// is its markup; a handler calling a helper hands it the state and
    /// reads back what it wrote; a view calling a view hands it the state.
    pub(crate) fn user_call(&mut self, name: &str, args: &[Expr], mut arg_strs: Vec<String>) -> String {
        let params: Vec<Ty> = self.fns.get(name).map(|f| f.params.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
        for (i, a) in args.iter().enumerate() {
            if self.makes_node(a) {
                if let Some(s) = arg_strs.get_mut(i) {
                    *s = format!("({}).to_html()", s);
                }
            }
            // an empty list where the function takes numbers as a tensor (a
            // numeric literal at another call typed the parameter)
            if matches!(Emitter::raw(a), Expr::ListLit(items, _) if items.is_empty()) && matches!(params.get(i), Some(Ty::Tensor(_))) {
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
            let flush = self.flush_before();
            let reloads = self.reload_after(name);
            let stop = self.stop_check();
            if !reloads.is_empty() || !flush.is_empty() || !stop.is_empty() {
                return format!("{{ {} let _r = {}({}); {}{} _r }}", flush, Self::rust_fn_name(name), all.join(", "), stop, reloads);
            }
        }
        // a helper outside the handler class that a render calls with no state
        let _ = map_literal_pairs;
        format!("{}({})", Self::rust_fn_name(name), all.join(", "))
    }
}

/// Whether a collecting loop's body can leave the function: a return, an
/// error handed back from a condition, a handler's stop.
fn leaves(body: &[Stmt]) -> bool {
    let mut out = false;
    visit_stmts(body, &mut |s| out |= matches!(s, Stmt::Return(_) | Stmt::Propagate { .. } | Stmt::Stop { .. }));
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
