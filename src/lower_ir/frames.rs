//! Frames' columns in their type (second-half-design.md §2, columns in the
//! type): a frame operation's result has the columns its arguments' types
//! give it, and a frame variable reassigned is bound anew to the columns of
//! what it is given — where ways through join, it has the columns they
//! share. A tensor's shape is rebound and joined the same way (a local's
//! detail, `CrowType::detail`; shapes.rs computes it).

use super::*;
use crate::type_infer::{Cols, Detail};
use std::sync::Arc;

/// A frame or tensor local reassigned: its columns or its shape are the
/// value's (a new binding), and how it is held — a frame, or one that can
/// fail; a tensor of its element type — is what reassignment settles it
/// to, so the variable is held the same way throughout.
pub(super) fn rebound(settled: &CrowType, value: &CrowType) -> CrowType {
    match (settled, value.detail()) {
        (CrowType::Frame(_), Some(Detail::Columns(cols))) => CrowType::Frame(cols.clone()),
        (CrowType::NumArray(elem, _), Some(Detail::Shape(dims))) => CrowType::NumArray(elem.clone(), dims.cloned()),
        (CrowType::Result(inner), Some(_)) => CrowType::Result(Box::new(rebound(inner, value))),
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
fn cell_of(t: &CrowType) -> CrowType {
    let t = match t {
        CrowType::Result(inner) => inner.as_ref().clone(),
        t => t.clone(),
    };
    match t {
        CrowType::Array(inner) => *inner,
        CrowType::NumArray(..) => CrowType::Float,
        CrowType::Int => CrowType::Int,
        CrowType::Float => CrowType::Float,
        CrowType::Bool => CrowType::Bool,
        _ => CrowType::Str,
    }
}

impl<'a> Lowerer<'a> {
    /// A frame local given a frame, or a tensor local a tensor: bound anew
    /// to its columns or its shape (a fact, which a view reads at any time,
    /// keeps what every assignment to it shares, as reassignment settles it).
    pub(super) fn rebind(&mut self, name: &str, value: &CrowType) {
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
    pub(super) fn detailed_in_view(&self) -> Vec<(String, CrowType)> {
        let mut out: Vec<(String, CrowType)> = self
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
    pub(super) fn detailed_now(&self, before: &[(String, CrowType)]) -> Vec<(String, CrowType)> {
        before.iter().map(|(n, t)| (n.clone(), self.ctx.vars.get(n).cloned().unwrap_or_else(|| t.clone()))).collect()
    }

    /// Where ways through a block meet: each frame local has the columns
    /// they share, each tensor local the dimensions they agree on.
    pub(super) fn join_detailed(&mut self, a: &[(String, CrowType)], b: &[(String, CrowType)]) {
        for ((n, ta), (_, tb)) in a.iter().zip(b) {
            self.ctx.vars.insert(n.clone(), ta.unify(tb));
        }
    }

    /// A loop's body read once, provisionally, for the frames and tensors
    /// it leaves:
    /// each is joined with what it was on entry, so the body's top reads
    /// what either gives (the body is then lowered for good).
    pub(super) fn pre_join_loop(&mut self, before: &[(String, CrowType)], body: &[S]) {
        let saved = self.ctx.vars.clone();
        let _ = self.provisional_block(Pass::Settle, Some(HashSet::new()), body);
        let end = self.detailed_now(before);
        self.ctx.vars = saved;
        self.join_detailed(before, &end);
    }

    /// Whether a body assigns one of these names, in any block within it.
    pub(super) fn body_assigns(body: &[S], names: &[(String, CrowType)]) -> bool {
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

    /// `window[df, col, how, n = …, by = …]`'s column, typed by what it
    /// computes: a float for `sum`, `mean`, `min` and `max`, an int for
    /// `count` and `rank`, the column's own cells for `lag` and `lead`,
    /// which can be missing at a partition's edge — as can every cell when
    /// a `by` column can have missing cells (a row in no partition). A
    /// column nothing typed gives dynamic cells, as a loaded file's are.
    pub(super) fn column_typed(&mut self, callee: &A, args: &[A], arg_types: &[CrowType], ty: Ty) -> Ty {
        if !matches!(ty.payload(), Ty::Column(_)) {
            return ty;
        }
        let Some((module, name)) = self.builtin_named(callee) else { return ty };
        if name != "window" || !matches!(module.as_deref(), None | Some("frame")) {
            return ty;
        }
        let base = arg_types.first().and_then(|t| t.frame_cols()).cloned().flatten();
        let lit = |i: usize| match args.get(i) {
            Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
            _ => None,
        };
        if let Some(b) = &base {
            if let Some(col) = lit(1) {
                self.refuse_missing("window", &Some(b.clone()), &col);
            }
            for key in Self::names_in(args.get(4)) {
                self.refuse_missing("window", &Some(b.clone()), &key);
            }
        }
        let column = |name: &str| base.as_ref().and_then(|b| b.cols.get(name).cloned());
        let fallible = |t: &CrowType| matches!(t, CrowType::Result(_));
        let keys_fallible = Self::names_in(args.get(4)).iter().any(|k| column(k).is_some_and(|t| fallible(&t)));
        let source = lit(1).and_then(|c| column(&c));
        let cell = match lit(2).as_deref() {
            Some("sum" | "mean" | "min" | "max") => CrowType::Float,
            Some("count") => CrowType::Int,
            Some("rank") => match source {
                Some(t) if fallible(&t) => CrowType::Result(Box::new(CrowType::Int)),
                Some(_) => CrowType::Int,
                None if base.is_some() => CrowType::Int,
                None => CrowType::Json,
            },
            Some("lag" | "lead") => match source {
                Some(CrowType::Result(t)) => CrowType::Result(t),
                Some(t) => CrowType::Result(Box::new(t)),
                None => CrowType::Json,
            },
            _ => return ty,
        };
        let cell = match cell {
            CrowType::Json | CrowType::Result(_) => cell,
            t if keys_fallible => CrowType::Result(Box::new(t)),
            t => t,
        };
        let column = Ty::Column(Box::new(ty_of(&cell)));
        match ty {
            Ty::Result(_) => Ty::Result(Box::new(column)),
            _ => column,
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
    pub(super) fn frame_typed(&mut self, callee: &A, args: &[A], arg_types: &[CrowType], lowered: &[Expr], ty: Ty) -> Ty {
        if !matches!(ty.payload(), Ty::Frame(None)) {
            return ty;
        }
        let Some((module, name)) = self.builtin_named(callee) else { return ty };
        // a builtin that answers a frame no rule here reads the columns of
        // answers one whose columns show as it runs (`load`, `stats`)
        // (a column handed on whole is its cells, as its type says them: the
        // lowered argument's, which a column's CrowType does not carry)
        let arg_types: Vec<CrowType> = arg_types
            .iter()
            .zip(lowered.iter().map(Some).chain(std::iter::repeat(None)))
            .map(|(t, x)| match x.map(|x| x.ty()) {
                Some(Ty::Column(cell)) if *cell != Ty::Unknown => CrowType::Array(Box::new(crow_type(&cell))),
                _ => t.clone(),
            })
            .collect();
        let cols = self.frame_columns(module.as_deref(), &name, args, &arg_types).unwrap_or_else(|| Some(Cols::dynamic()));
        let frame = ty_of(&CrowType::Frame(cols));
        match ty {
            Ty::Result(_) => Ty::Result(Box::new(frame)),
            _ => frame,
        }
    }

    /// The literal column names an operation is given, each checked
    /// against the frame it is given.
    fn check_columns(&mut self, name: &str, args: &[A], arg_types: &[CrowType]) {
        let base = |i: usize| -> Option<Arc<Cols>> { arg_types.get(i).and_then(|t| t.frame_cols()).cloned().flatten() };
        let lit = |i: usize| -> Option<String> {
            match args.get(i) {
                Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
                _ => None,
            }
        };
        match name {
            "sort" | "fill" | "rename" => {
                if let Some(col) = lit(1) {
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
                if let Some(key) = lit(2) {
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
            Some((f, p)) => format!(" (the {} {}[] is handed as `{}`)", what, f, p),
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
        let base = self.current_fn.split("__").next().unwrap_or(&self.current_fn);
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
    fn frame_columns(&mut self, module: Option<&str>, name: &str, args: &[A], arg_types: &[CrowType]) -> Option<Option<Arc<Cols>>> {
        let fixed = |cols: &[(&str, CrowType)]| Some(Some(Cols::closed(cols.iter().map(|(n, t)| (n.to_string(), t.clone())).collect())));
        match (module, name) {
            // the frames the standard modules build have fixed columns:
            // doc.chapters as crow_runtime::markdown::chapters_frame builds
            // it, chart.macd as crow_runtime::ta::macd_frame does
            (Some("doc"), "chapters") => fixed(&[("title", CrowType::Str), ("path", CrowType::Str), ("depth", CrowType::Int), ("part", CrowType::Str)]),
            (Some("chart"), "macd") => fixed(&[("line", CrowType::Float), ("signal", CrowType::Float), ("hist", CrowType::Float)]),
            // a literal query's columns, as its database describes them
            (None | Some("db"), "query") => {
                let Some(A::Str(sql)) = args.get(1) else {
                    return Some(Some(Cols::dynamic()));
                };
                let handle = match args.first() {
                    Some(A::Ident(v)) => Some(v.clone()),
                    _ => None,
                };
                // (no database answered: its cells show as it runs)
                Some(Some(self.ctx.describe_sql(sql, handle.as_deref()).map(Cols::closed).unwrap_or_else(Cols::dynamic)))
            }
            (None | Some("frame"), _) => {
                self.check_columns(name, args, arg_types);
                self.frame_rule(name, args, arg_types)
            }
            _ => None,
        }
    }

    /// The frame module's rules for its operations' result columns.
    fn frame_rule(&mut self, name: &str, args: &[A], arg_types: &[CrowType]) -> Option<Option<Arc<Cols>>> {
        let base = |i: usize| -> Option<Arc<Cols>> { arg_types.get(i).and_then(|t| t.frame_cols()).cloned().flatten() };
        let lit = |i: usize| -> Option<String> {
            match args.get(i) {
                Some(A::Str(s)) if !s.contains('`') => Some(s.clone()),
                _ => None,
            }
        };
        Some(match name {
                "new" => Some(Cols::closed(BTreeMap::new())),
                "filter" | "sort" | "head" | "tail" => base(0),
                // fill[df, 'col', value]: that column has no missing cell
                "fill" => base(0).map(|b| {
                    let mut c = (*b).clone();
                    if let Some(col) = lit(1) {
                        if let Some(CrowType::Result(inner)) = c.cols.get(&col).cloned() {
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
                                if let (CrowType::Result(_), Some(mine)) = (t, c.cols.get_mut(k)) {
                                    if !matches!(mine, CrowType::Result(_)) {
                                        *mine = CrowType::Result(Box::new(mine.clone()));
                                    }
                                }
                            }
                            c.open |= other.open;
                        }
                    }
                    Arc::new(c)
                }),
                // with[df, 'col', values]: one more column, typed by what it
                // is given; named by a variable, a column the type cannot name
                "with" => {
                    let value = arg_types.get(2).map(cell_of).unwrap_or(CrowType::Str);
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
                "join" => base(0).map(|left| {
                    let key = lit(2).unwrap_or_default();
                    let how = lit(3).unwrap_or_else(|| "inner".to_string());
                    let missing = |t: CrowType, can: bool| match t {
                        CrowType::Result(_) => t,
                        t if can => CrowType::Result(Box::new(t)),
                        t => t,
                    };
                    let mut cols: BTreeMap<String, CrowType> = left.cols.iter().map(|(k, t)| (k.clone(), missing(t.clone(), how == "right" || how == "outer"))).collect();
                    let mut open = left.open || lit(2).is_none() || (args.len() > 3 && lit(3).is_none());
                    match base(1) {
                        Some(right) => {
                            for (k, t) in &right.cols {
                                if *k == key {
                                    continue;
                                }
                                let n = if left.cols.contains_key(k) { format!("{}_right", k) } else { k.clone() };
                                cols.insert(n, missing(t.clone(), how == "left" || how == "outer"));
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
                // …]: each key as it was, and each aggregate a float named
                // col_agg
                "group" => base(0).map(|b| {
                    let mut cols = BTreeMap::new();
                    let mut open = false;
                    let keys = Self::names_in(args.get(1));
                    if keys.is_empty() {
                        open = true;
                    }
                    for key in keys {
                        let t = b.cols.get(&key).cloned().unwrap_or(CrowType::Str);
                        cols.insert(key, t);
                    }
                    for a in args.iter().skip(2) {
                        match a {
                            A::Array(pair) => match (pair.first(), pair.get(1)) {
                                (Some(A::Str(col)), Some(A::Str(agg))) => {
                                    cols.insert(format!("{}_{}", col, agg), CrowType::Float);
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
                        let t = b.cols.get(&rows).cloned().unwrap_or(CrowType::Str);
                        cols.insert(rows, t);
                    }
                    Arc::new(Cols { cols, open: true })
                }),
                _ => return None,
        })
    }
}
