//! Frames' columns in their type (second-half-design.md §2, columns in the
//! type): a frame operation's result has the columns its arguments' types
//! give it, and a frame variable reassigned is bound anew to the columns of
//! what it is given — where ways through join, it has the columns they
//! share. A tensor's shape is rebound and joined the same way (a local's
//! detail, `Ty::detail`; shapes.rs computes it).

use super::*;
use crate::type_infer::{Cols, Detail};
use std::sync::Arc;

/// A frame or tensor local reassigned: its columns or its shape are the
/// value's (a new binding), and how it is held — a frame, or one that can
/// fail; a tensor of its element type — is what reassignment settles it
/// to, so the variable is held the same way throughout.
pub(super) fn rebound(settled: &Ty, value: &Ty) -> Ty {
    match (settled, value.detail()) {
        (Ty::Frame(_), Some(Detail::Columns(cols))) => Ty::Frame(cols.clone()),
        // (a tensor of ints given floats holds floats from then on, as a
        // variable ever assigned a float is a float — spec §1: the binding
        // had kept the ints while its value's row answered floats)
        (Ty::Tensor(elem, _), Some(Detail::Shape(dims))) => {
            let given = match value.payload() {
                Ty::Tensor(e, _) => e.as_ref().clone(),
                _ => elem.as_ref().clone(),
            };
            Ty::Tensor(Box::new(elem.unify(&given)), dims.cloned())
        }
        (Ty::Result(inner), Some(_)) => Ty::Result(Box::new(rebound(inner, value))),
        _ => settled.clone(),
    }
}

/// The names a condition says have failed: in its body (`not x.ok`) and in
/// its else (`x.ok`).
pub(super) fn guarded_failure(condition: &A) -> (Vec<String>, Vec<String>) {
    let ok_of = |e: &A| match e {
        A::FieldAccess { object, field } if field == "ok" => match object.as_ref() {
            A::Ident(v) => Some(v.clone()),
            _ => None,
        },
        _ => None,
    };
    match condition {
        A::UnaryOp { op: crate::ast::UnaryOp::Not, expr } => (ok_of(expr).into_iter().collect(), Vec::new()),
        e => (Vec::new(), ok_of(e).into_iter().collect()),
    }
}

/// A frame's columns as a message names them.
pub(super) fn names(cols: &Cols) -> String {
    if cols.cols.is_empty() {
        return "none".to_string();
    }
    cols.cols.keys().map(|k| format!("'{}'", k)).collect::<Vec<_>>().join(", ")
}

/// A column's cell type from the values `with` is given: a list's items, a
/// tensor's numbers, a value repeated for every row.
fn cell_of(t: &Ty) -> Ty {
    let t = match t {
        Ty::Result(inner) => inner.as_ref().clone(),
        t => t.clone(),
    };
    match t {
        // a list of lists of numbers: a vector per row
        Ty::List(inner) if matches!(inner.as_ref(), Ty::List(i) if matches!(i.as_ref(), Ty::Int | Ty::Float)) => Ty::vector(Ty::Float),
        Ty::List(inner) => *inner,
        // a matrix's rows are a column of vectors, each of the matrix's
        // row shape (its width, where the type knows it)
        Ty::Tensor(elem, Some(dims)) if dims.len() >= 2 => Ty::Tensor(Box::new(match elem.as_ref() { Ty::Int => Ty::Float, _ => Ty::Float }), Some(dims[1..].to_vec())),
        // (`[1, 2]`, numbers held as a tensor of ints, had made a column of
        // floats)
        Ty::Tensor(elem, _) => match elem.as_ref() {
            Ty::Int => Ty::Int,
            _ => Ty::Float,
        },
        Ty::Int => Ty::Int,
        Ty::Float => Ty::Float,
        Ty::Bool => Ty::Bool,
        Ty::Moment => Ty::Moment,
        // a column written whole: its cells (one of cells nothing knows
        // yet is read as the program runs, never guessed text)
        Ty::Column(cells) => match cells.payload() {
            Ty::Unknown => Ty::Json,
            _ => *cells,
        },
        _ => Ty::Str,
    }
}

impl<'a> Lowerer<'a> {
    /// A frame local given a frame, or a tensor local a tensor: bound anew
    /// to its columns or its shape (a fact, which a view reads at any time,
    /// keeps what every assignment to it shares, as reassignment settles it).
    pub(super) fn rebind(&mut self, name: &str, value: &Ty) {
        if self.is_fact(name) && self.sees_facts() {
            return;
        }
        if let Some(settled) = self.ctx.vars.get(name) {
            if settled.detail().zip(value.detail()).is_some_and(|(a, b)| a.same_kind(&b)) {
                let t = rebound(settled, value);
                self.ctx.vars.insert(name.to_string(), t);
            }
        }
    }

    /// The frame and tensor locals in view: their types now, to join with
    /// what a block leaves them.
    pub(super) fn detailed_in_view(&self) -> Vec<(String, Ty)> {
        let mut out: Vec<(String, Ty)> = self
            .ctx
            .vars
            .iter()
            .filter(|(n, t)| t.detail().is_some() && self.declared(n) && !(self.is_fact(n) && self.sees_facts()))
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The same locals' types as they stand now.
    pub(super) fn detailed_now(&self, before: &[(String, Ty)]) -> Vec<(String, Ty)> {
        before.iter().map(|(n, t)| (n.clone(), self.ctx.vars.get(n).cloned().unwrap_or_else(|| t.clone()))).collect()
    }

    /// Where ways through a block meet: each frame local has the columns
    /// they share, each tensor local the dimensions they agree on.
    pub(super) fn join_detailed(&mut self, a: &[(String, Ty)], b: &[(String, Ty)]) {
        for ((n, ta), (_, tb)) in a.iter().zip(b) {
            // (the details joined, never whether the value can fail: a
            // variable whose value could not fail refuses a failure where
            // it is assigned (spec, Reassignment) — a loop pushing a value
            // that can fail onto a tensor had made the tensor one that can,
            // and the lowered program disagreed with itself; Rosetta Code,
            // colorful numbers)
            let tb = if ta.is_result() { tb.clone() } else { tb.payload().clone() };
            // (a tensor given a list in the loop stays the tensor its
            // declaration made, its sizes not known: unified, the list form
            // had won, and `last = level` was lowered unconverted into the
            // tensor `last = [0]` declared — an internal error; Rosetta Code,
            // Pancake numbers)
            let joined = match (ta.payload(), tb.payload()) {
                (Ty::Tensor(e, d), Ty::List(_)) => {
                    let open = Ty::Tensor(e.clone(), d.as_ref().map(|d| d.iter().map(|_| None).collect()));
                    if ta.is_result() { Ty::Result(Box::new(open)) } else { open }
                }
                _ => ta.unify(&tb),
            };
            self.ctx.vars.insert(n.clone(), joined);
        }
    }

    /// A loop's body read once, provisionally, for the frames and tensors
    /// it leaves:
    /// each is joined with what it was on entry, so the body's top reads
    /// what either gives (the body is then lowered for good).
    pub(super) fn pre_join_loop(&mut self, before: &[(String, Ty)], body: &[S]) {
        let saved = self.ctx.vars.clone();
        let _ = self.provisional_block(Pass::Settle, Some(HashSet::new()), body);
        let end = self.detailed_now(before);
        self.ctx.vars = saved;
        self.join_detailed(before, &end);
    }

    /// Whether a body assigns one of these names, in any block within it.
    pub(super) fn body_assigns(body: &[S], names: &[(String, Ty)]) -> bool {
        body.iter().any(|s| match s {
            S::Assign { name, .. } => names.iter().any(|(n, _)| n == name),
            S::MultiAssign { names: assigned, .. } => assigned.iter().any(|a| names.iter().any(|(n, _)| n == a)),
            S::If { then_body, else_body, .. } => Self::body_assigns(then_body, names) || else_body.as_ref().is_some_and(|b| Self::body_assigns(b, names)),
            S::While { body, .. } | S::For { body, .. } => Self::body_assigns(body, names),
            _ => false,
        })
    }

    /// The builtin a call names, when it is no function of the program's:
    /// its module, when the call is qualified, and its name.
    fn builtin_named(&self, callee: &A) -> Option<(Option<String>, String)> {
        match callee {
            A::Ident(n) if !self.calls_fn(n) && !self.ctx.specializations.contains_key(n) => Some((None, n.clone())),
            A::FieldAccess { object, field } => match object.as_ref() {
                A::Ident(m) if self.is_module(m) && !self.declared(m) => Some((Some(self.ctx.module_aliases.get(m).cloned().unwrap_or_else(|| m.clone())), field.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    /// A builtin's column, typed by what it computes. `window[df, col,
    /// how, …]`: a `sum`, `min` or `max` in the column's kind (an int over
    /// ints, a float over floats), a float for `mean`, `median`, `std`,
    /// `var` and a percentile, an int for `count`, `distinct` and `rank`,
    /// the column's own cells for `lag` and `lead`, which can be missing at
    /// a partition's edge, and for `first` and `last` — as can every cell
    /// when a `by` column, or the column a span is measured on, can have
    /// missing cells (a row in no partition). `unique[df, col]`: the
    /// column's distinct present cells, in its kind. `bucket[col, s]`: a
    /// column of moments. `where[mask, a, b]`: the kind both sides share.
    /// `int`, `float`, `str` and `moment` of a column: cells of that kind,
    /// missing where one could not convert. `lower`, `upper`, `trim`,
    /// `replace` and `sub` of a column: text. A column nothing typed gives
    /// dynamic cells, as a loaded file's are.
    pub(super) fn column_typed(&mut self, callee: &A, args: &[A], arg_types: &[Ty], lowered: &[Expr], ty: Ty) -> Ty {
        if !matches!(ty.payload(), Ty::Column(_)) {
            return ty;
        }
        let Some((module, name)) = self.builtin_named(callee) else { return ty };
        if !matches!(module.as_deref(), None | Some("frame")) {
            return ty;
        }
        let wrap = |cell: Ty, ty: &Ty| -> Ty {
            let column = Ty::Column(Box::new(cell.clone()));
            match ty {
                Ty::Result(_) => Ty::Result(Box::new(column)),
                _ => column,
            }
        };
        let plain = |t: &Ty| match t {
            Ty::Result(inner) => inner.as_ref().clone(),
            t => t.clone(),
        };
        let fallible = |t: &Ty| matches!(t, Ty::Result(_));
        // the cells of a value handed whole — a column, a list, one value —
        // as its lowered type says them
        let cell_of_arg = |i: usize| -> Option<Ty> {
            match lowered.get(i).map(|x| x.ty()) {
                Some(Ty::Column(cell)) if *cell != Ty::Unknown => Some(as_inferred(&cell)),
                Some(Ty::List(item)) => Some(as_inferred(&item)),
                Some(Ty::Tensor(..)) => Some(Ty::Float),
                Some(t @ (Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment | Ty::Json)) => Some(as_inferred(&t)),
                Some(Ty::Result(inner)) if matches!(*inner, Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Moment) => Some(Ty::Result(Box::new(as_inferred(&inner)))),
                _ => None,
            }
        };
        match name.as_str() {
            "bucket" => match cell_of_arg(0) {
                Some(t) if matches!(plain(&t), Ty::Moment | Ty::Json) => wrap(t, &ty),
                _ => ty,
            },
            "where" => match (cell_of_arg(1), cell_of_arg(2)) {
                (Some(a), Some(b)) => {
                    let kind = match (plain(&a), plain(&b)) {
                        (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
                        (x, y) if x == y => x,
                        _ => Ty::Json,
                    };
                    let kind = if fallible(&a) || fallible(&b) { Ty::Result(Box::new(kind)) } else { kind };
                    wrap(kind, &ty)
                }
                _ => ty,
            },
            "int" | "float" | "str" | "moment" => {
                let Some(source) = cell_of_arg(0) else { return ty };
                let target = match name.as_str() {
                    "int" => Ty::Int,
                    "float" => Ty::Float,
                    "str" => Ty::Str,
                    _ => Ty::Moment,
                };
                let src = plain(&source);
                let can_fail = match name.as_str() {
                    "str" => false,
                    "float" => !matches!(src, Ty::Int | Ty::Float | Ty::Bool | Ty::Moment),
                    "int" => !matches!(src, Ty::Int | Ty::Bool | Ty::Moment),
                    _ => !matches!(src, Ty::Moment),
                };
                let kind = if can_fail || fallible(&source) { Ty::Result(Box::new(target)) } else { target };
                wrap(kind, &ty)
            }
            "lower" | "upper" | "trim" | "replace" | "sub" => match cell_of_arg(0) {
                Some(source) => wrap(if fallible(&source) { Ty::Result(Box::new(Ty::Str)) } else { Ty::Str }, &ty),
                None => ty,
            },
            "window" | "unique" => {
                let base = arg_types.first().and_then(|t| t.frame_cols()).cloned().flatten();
                let lit = |i: usize| match args.get(i) {
                    Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
                    _ => None,
                };
                if let Some(b) = &base {
                    if let Some(col) = lit(1) {
                        self.refuse_missing(&name, &Some(b.clone()), &col);
                    }
                    for key in Self::names_in(args.get(4)).into_iter().chain(Self::names_in(args.get(6))) {
                        self.refuse_missing("window", &Some(b.clone()), &key);
                    }
                }
                let column = |name: &str| base.as_ref().and_then(|b| b.cols.get(name).cloned());
                let keys_fallible = Self::names_in(args.get(4)).iter().chain(Self::names_in(args.get(6)).iter()).any(|k| column(k).is_some_and(|t| fallible(&t)));
                let source = lit(1).and_then(|c| column(&c));
                if name == "unique" {
                    let cell = match &source {
                        Some(t) => plain(t),
                        None => Ty::Json,
                    };
                    return wrap(cell, &ty);
                }
                let how = lit(2);
                let percentile = how.as_deref().is_some_and(|h| crow_runtime::frame::AggFn::from_str(h).is_some_and(|a| matches!(a, crow_runtime::frame::AggFn::Percentile(_))));
                let cell = match how.as_deref() {
                    // (every window had been a float; a sum over ints is an int,
                    // and over a column nothing typed its kind shows as it runs)
                    Some("sum" | "min" | "max") => match source.as_ref().map(plain) {
                        Some(Ty::Int) => Ty::Int,
                        Some(_) => Ty::Float,
                        None => Ty::Json,
                    },
                    Some("mean" | "median" | "std" | "var") => Ty::Float,
                    Some(_) if percentile => Ty::Float,
                    Some("count" | "distinct") => Ty::Int,
                    Some("rank") => match source {
                        Some(t) if fallible(&t) => Ty::Result(Box::new(Ty::Int)),
                        Some(_) => Ty::Int,
                        None if base.is_some() => Ty::Int,
                        None => Ty::Json,
                    },
                    Some("lag" | "lead") => match source {
                        Some(Ty::Result(t)) => Ty::Result(t),
                        Some(t) => Ty::Result(Box::new(t)),
                        None => Ty::Json,
                    },
                    Some("first" | "last") => match source {
                        Some(t) => t,
                        None => Ty::Json,
                    },
                    _ => return ty,
                };
                let cell = match cell {
                    Ty::Json | Ty::Result(_) => cell,
                    t if keys_fallible => Ty::Result(Box::new(t)),
                    t => t,
                };
                wrap(cell, &ty)
            }
            _ => ty,
        }
    }

    /// The column names an argument writes: one, or a list of them.
    fn names_in(arg: Option<&A>) -> Vec<String> {
        match arg {
            Some(A::Str(s)) if !s.contains('`') && !s.is_empty() => vec![s.clone()],
            Some(A::Array(items)) => items.iter().filter_map(|i| if let A::Str(s) = i { Some(s.clone()) } else { None }).collect(),
            _ => Vec::new(),
        }
    }

    /// A call's type with the columns of the frame it answers, when it is a
    /// frame operation: its result's columns are its arguments'.
    pub(super) fn frame_typed(&mut self, callee: &A, args: &[A], arg_types: &[Ty], lowered: &[Expr], ty: Ty) -> Ty {
        if !matches!(ty.payload(), Ty::Frame(None)) {
            return ty;
        }
        let Some((module, name)) = self.builtin_named(callee) else { return ty };
        // a builtin that answers a frame no rule here reads the columns of
        // answers one whose columns show as it runs (`load`, `stats`)
        // (a column handed on whole is its cells, as its type says them: the
        // lowered argument's, which a column's Ty does not carry)
        let arg_types: Vec<Ty> = arg_types
            .iter()
            .zip(lowered.iter().map(Some).chain(std::iter::repeat(None)))
            .map(|(t, x)| match x.map(|x| x.ty()) {
                Some(Ty::Column(cell)) if *cell != Ty::Unknown => Ty::List(Box::new(as_inferred(&cell))),
                _ => t.clone(),
            })
            .collect();
        let cols = self.frame_columns(module.as_deref(), &name, args, &arg_types).unwrap_or_else(|| Some(Cols::dynamic()));
        let frame = (&Ty::Frame(cols)).clone();
        match ty {
            Ty::Result(_) => Ty::Result(Box::new(frame)),
            _ => frame,
        }
    }

    /// The literal column names an operation is given, each checked
    /// against the frame it is given.
    fn check_columns(&mut self, name: &str, args: &[A], arg_types: &[Ty]) {
        let base = |i: usize| -> Option<Arc<Cols>> { arg_types.get(i).and_then(|t| t.frame_cols()).cloned().flatten() };
        let lit = |i: usize| -> Option<String> {
            match args.get(i) {
                Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
                _ => None,
            }
        };
        match name {
            "sort" | "fill" | "rename" | "complete" => {
                for col in Self::names_in(args.get(1)) {
                    self.refuse_missing(name, &base(0), &col);
                }
            }
            "unpivot" => {
                for col in Self::names_in(args.get(1)).into_iter().chain(Self::names_in(args.get(2))) {
                    self.refuse_missing(name, &base(0), &col);
                }
            }
            "select" => {
                if let Some(A::Array(items)) = args.get(1) {
                    for item in items {
                        if let A::Str(col) = item {
                            self.refuse_missing(name, &base(0), col);
                        }
                    }
                }
            }
            "group" => {
                for key in Self::names_in(args.get(1)) {
                    self.refuse_missing(name, &base(0), &key);
                }
                for a in args.iter().skip(2) {
                    if let A::Array(pair) = a {
                        if let Some(A::Str(col)) = pair.first() {
                            self.refuse_missing(name, &base(0), col);
                        }
                    }
                }
            }
            "join" => {
                for key in Self::names_in(args.get(2)) {
                    self.refuse_missing(name, &base(0), &key);
                    self.refuse_missing(name, &base(1), &key);
                }
            }
            "pivot" => {
                for i in 1..=3 {
                    if let Some(col) = lit(i) {
                        self.refuse_missing(name, &base(0), &col);
                    }
                }
            }
            _ => {}
        }
    }

    /// A column read (`df.x`) of a frame whose columns are all known, that
    /// it does not have: refused before the program runs, in the runtime's
    /// words, with the columns it has.
    pub(super) fn refuse_column_read(&mut self, frame: &Expr, field: &str) {
        if let Some(Some(cols)) = read(frame).frame_cols() {
            if !cols.open && !cols.cols.contains_key(field) && self.pass == Pass::Final {
                let handed = self.handed(frame, "frame");
                self.refuse(format!("frame has no property or column '{}' — its columns are {}{}", field, names(cols), handed));
            }
        }
    }

    /// Where a frame a function reads came from, when a call handed it: the
    /// function and the parameter it came as (each call's columns are its
    /// own copy's, so the call to look at is one handing these).
    pub(super) fn handed(&self, value: &Expr, what: &str) -> String {
        match self.handed_as(value) {
            Some((f, p)) => format!(" (the {} {}[] is handed as `{}`)", what, super::checks::plain_name(&f), super::checks::plain_name(&p)),
            None => String::new(),
        }
    }

    /// The function and the parameter a value came as, when it is a
    /// parameter of the function being lowered, as the program names them.
    pub(super) fn handed_as(&self, value: &Expr) -> Option<(String, String)> {
        let mut e = value;
        while let Expr::Convert { expr, .. } = e {
            e = expr;
        }
        let (Expr::Local(p, _) | Expr::Move(p, _)) = e else {
            return None;
        };
        let is_param = self.ctx.functions.get(&self.current_fn).is_some_and(|s| s.params.iter().any(|(n, _)| n == p));
        if !is_param {
            return None;
        }
        let base = crate::ir::copy_base(&self.current_fn);
        Some((crate::lower::shown(base), crate::lower::shown(p)))
    }

    /// The columns a table or a chart names (`key = 'id'`, `lines = […]`),
    /// each checked against the frame it draws.
    pub(super) fn check_tree_columns(&mut self, element: &str, attrs: &[(String, Expr)], children: &[Expr]) {
        let named: &[&str] = match element {
            "table" => &["key", "mark", "delta", "hide"],
            "chart" => &["x", "candles", "lines", "bars_y"],
            _ => return,
        };
        let Some(frame) = children.iter().find_map(|c| c.ty().frame_cols_ty()) else {
            return;
        };
        for (k, v) in attrs {
            if !named.contains(&k.as_str()) {
                continue;
            }
            let names: Vec<&str> = match v {
                Expr::Const(Const::Str(n), _) => vec![n.as_str()],
                Expr::ListLit(items, _) => items.iter().filter_map(|i| if let Expr::Const(Const::Str(n), _) = i { Some(n.as_str()) } else { None }).collect(),
                _ => Vec::new(),
            };
            for n in names {
                self.refuse_missing(element, &frame, n);
            }
        }
    }

    /// The columns a chart names — its candles, lines and volume pane —
    /// each checked against the frame it draws (its x axis's default, `ts`,
    /// is not told apart from one given, and is left to the chart).
    pub(super) fn check_chart_columns(&mut self, lowered: &[Expr]) {
        let Some(frame) = lowered.first().and_then(|f| f.ty().frame_cols_ty()) else {
            return;
        };
        let strings = |x: Option<&Expr>| -> Vec<String> {
            match x {
                Some(Expr::Const(Const::Str(n), _)) if !n.is_empty() => vec![n.clone()],
                Some(Expr::ListLit(items, _)) => items.iter().filter_map(|i| if let Expr::Const(Const::Str(n), _) = i { Some(n.clone()) } else { None }).collect(),
                _ => Vec::new(),
            }
        };
        for i in 2..=4 {
            for n in strings(lowered.get(i)) {
                self.refuse_missing("chart", &frame, &n);
            }
        }
    }

    /// A column an operation names that a frame whose columns are all known
    /// does not have: refused before the program runs, in the runtime's
    /// words, with the columns it has.
    fn refuse_missing(&mut self, op: &str, base: &Option<Arc<Cols>>, name: &str) {
        if let Some(cols) = base {
            if !cols.open && !cols.cols.contains_key(name) && self.pass == Pass::Final {
                self.refuse(format!("{}: column '{}' not found — the frame's columns are {}", op, name, names(cols)));
            }
        }
    }

    /// The columns a frame operation's result has, from its arguments'
    /// types and the names it is given; `None` when the call is no frame
    /// operation, `Some(None)` when nothing is known of the columns.
    fn frame_columns(&mut self, module: Option<&str>, name: &str, args: &[A], arg_types: &[Ty]) -> Option<Option<Arc<Cols>>> {
        let fixed = |cols: &[(&str, Ty)]| Some(Some(Cols::closed(cols.iter().map(|(n, t)| (n.to_string(), t.clone())).collect())));
        // a row that declares its frame's columns (a kernel that fixes them:
        // doc's chapters, chart's macd), or a provider of them — its
        // database's description of a literal statement
        if let Some(row) = crate::table::frame_declared(module, name) {
            if !row.columns.is_empty() {
                let cols: Vec<(&str, Ty)> = row.columns.iter().map(|(n, k)| (*n, k.ty())).collect();
                return fixed(&cols);
            }
            if let Some(provider) = row.provider {
                let Some(A::Str(sql)) = args.get(provider.statement) else {
                    return Some(Some(Cols::dynamic()));
                };
                let handle = match args.get(provider.connection) {
                    Some(A::Ident(v)) => Some(v.clone()),
                    _ => None,
                };
                // (no database answered: its cells show as it runs)
                return Some(Some(self.ctx.describe_sql(provider, sql, handle.as_deref()).map(Cols::closed).unwrap_or_else(Cols::dynamic)));
            }
        }
        match (module, name) {
            (None | Some("frame"), _) => {
                self.check_columns(name, args, arg_types);
                self.frame_rule(name, args, arg_types)
            }
            _ => None,
        }
    }

    /// The frame module's rules for its operations' result columns.
    fn frame_rule(&mut self, name: &str, args: &[A], arg_types: &[Ty]) -> Option<Option<Arc<Cols>>> {
        let base = |i: usize| -> Option<Arc<Cols>> { arg_types.get(i).and_then(|t| t.frame_cols()).cloned().flatten() };
        let lit = |i: usize| -> Option<String> {
            match args.get(i) {
                Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
                _ => None,
            }
        };
        Some(match name {
                "new" => Some(Cols::closed(BTreeMap::new())),
                "sort" | "head" | "tail" | "sample" => base(0),
                // unique[df, ['a', 'b']]: the first row of each combination,
                // every column as it was
                "unique" => {
                    for key in Self::names_in(args.get(1)) {
                        self.refuse_missing(name, &base(0), &key);
                    }
                    base(0)
                }
                // complete[df], complete[df, cols]: the rows present in every
                // column, or in those named — none of which can be missing
                "complete" => base(0).map(|b| {
                    let mut c = (*b).clone();
                    let named = Self::names_in(args.get(1));
                    let all = args.len() == 1;
                    for (k, t) in c.cols.iter_mut() {
                        if all || named.contains(k) {
                            if let Ty::Result(inner) = t {
                                *t = (**inner).clone();
                            }
                        }
                    }
                    Arc::new(c)
                }),
                // unpivot[df, keys, cols]: the keys as they were, `variable`
                // the column's name, `value` the kind the columns share
                "unpivot" => base(0).map(|b| {
                    let literal = |i: usize| match args.get(i) {
                        Some(A::Str(s)) => !s.contains('`'),
                        Some(A::Array(items)) => items.iter().all(|x| matches!(x, A::Str(s) if !s.contains('`'))),
                        _ => false,
                    };
                    let mut cols = BTreeMap::new();
                    let mut open = !literal(1) || !literal(2);
                    for k in Self::names_in(args.get(1)) {
                        cols.insert(k.clone(), b.cols.get(&k).cloned().unwrap_or(Ty::Json));
                    }
                    cols.insert("variable".to_string(), Ty::Str);
                    let kinds: Vec<Ty> = Self::names_in(args.get(2)).iter().map(|c| b.cols.get(c).cloned().unwrap_or(Ty::Json)).collect();
                    let plain = |t: &Ty| match t {
                        Ty::Result(inner) => inner.as_ref().clone(),
                        t => t.clone(),
                    };
                    let value = if kinds.is_empty() {
                        open = true;
                        Ty::Json
                    } else {
                        let kinds_plain: Vec<Ty> = kinds.iter().map(plain).collect();
                        let kind = if kinds_plain.iter().all(|k| *k == Ty::Int) {
                            Ty::Int
                        } else if kinds_plain.iter().all(|k| matches!(k, Ty::Int | Ty::Float)) {
                            Ty::Float
                        } else if kinds_plain.iter().all(|k| *k == kinds_plain[0]) {
                            kinds_plain[0].clone()
                        } else {
                            Ty::Json
                        };
                        if kinds.iter().any(|k| matches!(k, Ty::Result(_))) { Ty::Result(Box::new(kind)) } else { kind }
                    };
                    cols.insert("value".to_string(), value);
                    Arc::new(Cols { cols, open })
                }),
                // fill[df, 'col', value]: that column has no missing cell
                // (filled forward or backward, its first or last cells can
                // still be)
                "fill" => base(0).map(|b| {
                    let mut c = (*b).clone();
                    let directed = args.get(3).is_some_and(|h| !matches!(h, A::Str(s) if s.is_empty()));
                    if let (Some(col), false) = (lit(1), directed) {
                        if let Some(Ty::Result(inner)) = c.cols.get(&col).cloned() {
                            c.cols.insert(col, *inner);
                        }
                    }
                    Arc::new(c)
                }),
                // stack[a, b]: the same columns, missing where either's can be
                "stack" => base(0).map(|b| {
                    let mut c = (*b).clone();
                    for i in 1..args.len() {
                        if let Some(other) = base(i) {
                            for (k, t) in &other.cols {
                                if let (Ty::Result(_), Some(mine)) = (t, c.cols.get_mut(k)) {
                                    if !matches!(mine, Ty::Result(_)) {
                                        *mine = Ty::Result(Box::new(mine.clone()));
                                    }
                                }
                            }
                            c.open |= other.open;
                        }
                    }
                    Arc::new(c)
                }),
                // `df.col = values` (the column write's row): one more column,
                // typed by what it is given; named by a value the program
                // computes (`df.`name` = values`), a column the type cannot
                // name — the frame is open from there
                n if n == crate::table::column_write().name => {
                    let value = arg_types.get(2).map(cell_of).unwrap_or(Ty::Str);
                    match (base(0), lit(1)) {
                        (Some(b), Some(col)) => {
                            let mut c = (*b).clone();
                            c.cols.insert(col, value);
                            Some(Arc::new(c))
                        }
                        (Some(b), None) => Some(Arc::new(Cols { cols: b.cols.clone(), open: true })),
                        // (the frame's columns not worked out yet)
                        (None, _) => None,
                    }
                }
                // select[df, ['a', 'b']]: exactly the named columns
                "select" => match (base(0), args.get(1)) {
                    (Some(b), Some(A::Array(items))) => {
                        let b = Some(b);
                        let mut cols = BTreeMap::new();
                        let mut open = false;
                        for item in items {
                            match item {
                                A::Str(n) if !n.contains('`') => match b.as_ref().and_then(|b| b.cols.get(n)) {
                                    Some(t) => {
                                        cols.insert(n.clone(), t.clone());
                                    }
                                    None => open = true,
                                },
                                _ => open = true,
                            }
                        }
                        Some(Arc::new(Cols { cols, open }))
                    }
                    // names the program computes: the columns show as it runs
                    (Some(_), _) => Some(Cols::dynamic()),
                    (None, _) => None,
                },
                // join[a, b, 'key', how]: a's columns, then b's but its key
                // (one a has too is named `_right`); the side a row can have
                // no match on has missing cells — b's in a left join, a's in
                // a right one, both in an outer one
                // (on several keys, every key is left out of b's; a semi or
                // anti join has a's columns alone; an as-of join is a left one)
                "join" => base(0).map(|left| {
                    let keys = Self::names_in(args.get(2));
                    let how = lit(3).unwrap_or_else(|| "inner".to_string());
                    let missing = |t: Ty, can: bool| match t {
                        Ty::Result(_) => t,
                        t if can => Ty::Result(Box::new(t)),
                        t => t,
                    };
                    let left_can_miss = matches!(how.as_str(), "right" | "outer");
                    let right_can_miss = matches!(how.as_str(), "left" | "outer" | "asof");
                    let mut cols: BTreeMap<String, Ty> = left.cols.iter().map(|(k, t)| (k.clone(), missing(t.clone(), left_can_miss))).collect();
                    let mut open = left.open || keys.is_empty() || (args.len() > 3 && lit(3).is_none());
                    if matches!(how.as_str(), "semi" | "anti") {
                        return Arc::new(Cols { cols, open });
                    }
                    match base(1) {
                        Some(right) => {
                            for (k, t) in &right.cols {
                                if keys.contains(k) {
                                    continue;
                                }
                                let n = if left.cols.contains_key(k) { format!("{}_right", k) } else { k.clone() };
                                cols.insert(n, missing(t.clone(), right_can_miss));
                            }
                            open |= right.open;
                        }
                        None => open = true,
                    }
                    Arc::new(Cols { cols, open })
                }),
                // rename[f, 'old', 'new']: the column under its new name
                "rename" => base(0).map(|b| {
                    let mut c = (*b).clone();
                    match (lit(1), lit(2)) {
                        (Some(old), Some(new)) => {
                            if let Some(t) = c.cols.remove(&old) {
                                c.cols.insert(new, t);
                            }
                        }
                        _ => c.open = true,
                    }
                    Arc::new(c)
                }),
                // group[f, 'key', ['col', 'agg'], …] or group[f, ['a', 'b'],
                // …]: each key as it was, and each aggregate named col_agg in
                // the kind it has — a count an int; a sum an int over ints or
                // bools and a float over floats; a mean a float; a min or max
                // a cell of the column's kind — where a mean, a min or a max
                // of a column whose cells can be missing can be missing too
                // (a group with no present cell). A column nothing typed
                // gives dynamic cells, as a loaded file's are. (Every
                // aggregate had been a float.)
                "group" => base(0).map(|b| {
                    let mut cols = BTreeMap::new();
                    let mut open = false;
                    let keys = Self::names_in(args.get(1));
                    if keys.is_empty() {
                        open = true;
                    }
                    for key in keys {
                        let t = b.cols.get(&key).cloned().unwrap_or(Ty::Json);
                        cols.insert(key, t);
                    }
                    for a in args.iter().skip(2) {
                        match a {
                            A::Array(pair) => match (pair.first(), pair.get(1)) {
                                (Some(A::Str(col)), Some(A::Str(agg))) => {
                                    let (plain, fallible) = match b.cols.get(col) {
                                        Some(Ty::Result(t)) => (Some(t.as_ref().clone()), true),
                                        Some(t) => (Some(t.clone()), false),
                                        None => (None, false),
                                    };
                                    let t = match (agg.as_str(), plain) {
                                        ("count" | "distinct", _) => Ty::Int,
                                        (_, None) => Ty::Json,
                                        ("sum", Some(Ty::Int | Ty::Bool)) => Ty::Int,
                                        ("sum", Some(_)) => Ty::Float,
                                        ("mean" | "median" | "std" | "var", Some(_)) => Ty::Float,
                                        ("min" | "max" | "first" | "last", Some(t)) => t,
                                        // (a percentile; a word that is none stops the program)
                                        (_, Some(_)) => Ty::Float,
                                    };
                                    let t = if fallible && !matches!(agg.as_str(), "count" | "distinct" | "sum") { Ty::Result(Box::new(t)) } else { t };
                                    cols.insert(format!("{}_{}", col, agg), t);
                                }
                                _ => open = true,
                            },
                            _ => open = true,
                        }
                    }
                    Arc::new(Cols { cols, open })
                }),
                // pivot[f, 'rows', 'cols', 'values']: the rows column as it
                // was; the rest are named by the cells of 'cols' as it runs
                "pivot" => base(0).map(|b| {
                    let mut cols = BTreeMap::new();
                    if let Some(rows) = lit(1) {
                        let t = b.cols.get(&rows).cloned().unwrap_or(Ty::Str);
                        cols.insert(rows, t);
                    }
                    Arc::new(Cols { cols, open: true })
                }),
                _ => return None,
        })
    }
}
