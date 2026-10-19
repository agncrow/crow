//! `describe` and `apply`, the standard modules' two operations over
//! functions (docs/extending-crow.md, section 7), and the dispatch the
//! compiler writes for a function held by its name: the lowering of a
//! call by name to what inference knows, so both modes agree by
//! construction. The names this file holds are the machinery's own: the
//! two forms, the hidden rows a dispatch is written with, and the core
//! forms it writes (`json`, `encode`, `error`).

use super::*;

/// The two forms, sys's.
const DESCRIBE: &str = "describe";
const APPLY: &str = "apply";
/// The core forms a dispatch is written with.
const JSON: &str = "json";
const ENCODE: &str = "encode";
const ERROR: &str = "error";
const MAP: &str = "map";
/// The hidden rows: a function compared with one written, the arguments
/// read by name and kind, a text made one that can fail.
const SAME_FN: &str = "__same_fn";
const OK_TEXT: &str = "core.__ok_text";
const ARG_INT: &str = "core.__arg_int";
const ARG_FLOAT: &str = "core.__arg_float";
const ARG_STR: &str = "core.__arg_str";
const ARG_BOOL: &str = "core.__arg_bool";
const ARG_JSON: &str = "core.__arg_json";
const SYS: &str = "sys";

impl<'a> Lowerer<'a> {
    /// `describe[f]` and `apply[f, args]`, the standard modules' two
    /// operations over functions (sys's forms; docs/extending-crow.md,
    /// section 7), lowered to what inference knows so both modes agree by
    /// construction: for a function written, its description or its call;
    /// for a function value, a dispatch the compiler writes over every
    /// function it can be — the ones a list literal of several signatures
    /// names (`Ty::Fns`), or every function of its signature — each
    /// branch compared with the value (`__same_fn`), an error of kind
    /// `call` where none matches. `describe` answers a parsed value: name,
    /// doc, params (name, takes — the word errors use for the kind —,
    /// default), answers.
    /// `apply` reads each argument from the parsed value by the parameter's
    /// name and kind (an error of kind `call` where it is missing or of
    /// another kind), calls, and answers the value encoded as text.
    pub(super) fn described(&mut self, name: &str, args: &[A]) -> Result<Option<Expr>, String> {
        if !(name == DESCRIBE || name == APPLY) || self.declared(name) || self.calls_fn(name) {
            return Ok(None);
        }
        if crate::table::rows_named_in(SYS, name).next().is_none() || !(self.v.bare_import_lines.contains_key(SYS) || crate::modules::is_crow_fn(&self.current_fn)) {
            return Ok(None);
        }
        let wants = if name == DESCRIBE { 1 } else { 2 };
        if args.len() != wants {
            self.refuse(format!("{}[] takes {} arguments, got {}", name, wants, args.len()));
            return Ok(Some(self.placeholder(name)));
        }
        let result = Ty::Result(Box::new(if name == DESCRIBE { Ty::Json } else { Ty::Str }));
        let what = name.to_string();
        // a function written: its description, or its call — no dispatch,
        // and no function value made of it (one whose parameters nothing
        // types is still described, by what its body reads them as)
        if let A::Ident(f) = &args[0] {
            if self.calls_fn(f) && !self.declared(f) {
                if self.pass != Pass::Final {
                    if name == APPLY {
                        let _ = self.lower_expr(&args[1])?;
                    }
                    self.note_called_by_name(std::slice::from_ref(f));
                    return Ok(Some(self.placeholder(name)));
                }
                if name == DESCRIBE {
                    let text = self.description(f);
                    return Ok(Some(self.lower_expr(&A::Call { callee: Box::new(A::Ident(JSON.to_string())), args: vec![A::Str(text)] })?));
                }
                let a = self.bound_arguments(&args[1])?;
                return Ok(Some(self.applied(f, &a)?));
            }
        }
        let x = self.lower_expr(&args[0])?;
        if self.pass != Pass::Final {
            if name == APPLY {
                let _ = self.lower_expr(&args[1])?;
            }
            // (a group's functions are called by name: each is read as
            // called with its parameters' kinds, so inference types it and
            // the compiled program holds it, as `later`'s function is)
            if let Ty::Fns(names) = x.ty().payload() {
                self.note_called_by_name(&names.clone());
            }
            return Ok(Some(self.placeholder(name)));
        }
        // the candidates: a group's names, or every function of the value's
        // signature
        let candidates: Vec<String> = match x.ty().payload() {
            Ty::Fns(names) => names.clone(),
            Ty::Fn(params, ret) => {
                let mut found: Vec<String> = self
                    .ctx
                    .functions
                    .iter()
                    .filter(|(g, sig)| self.calls_fn(g) && !crate::ir::is_copy(g) && sig.params.iter().map(|(_, t)| t).eq(params.iter()) && sig.return_type == **ret)
                    .map(|(g, _)| g.clone())
                    .collect();
                found.sort();
                found
            }
            // (a subject nothing typed — a loop over the empty default of
            // a tools list — has no candidates: the dispatch answers its
            // error, which no round reaches)
            Ty::Unknown => Vec::new(),
            _ => {
                self.refuse(format!("{}[] takes a function written, or one of a list of functions, got {}", name, crate::table::Arg::of_type(x.ty().payload()).words()));
                return Ok(Some(self.placeholder(name)));
            }
        };
        let init = Self::no_candidate(&what, &result);
        if name == DESCRIBE {
            let texts: Vec<(String, String)> = candidates.iter().map(|c| (c.clone(), self.description(c))).collect();
            let out = self.dispatch(x, &candidates, result, init, &|me, cand| {
                let text = texts.iter().find(|(c, _)| c == cand).map(|(_, t)| t.clone()).unwrap_or_default();
                me.lower_expr(&A::Call { callee: Box::new(A::Ident(JSON.to_string())), args: vec![A::Str(text)] })
            })?;
            return Ok(Some(out));
        }
        let a = self.bound_arguments(&args[1])?;
        let out = self.dispatch(x, &candidates, result, init, &|me, cand| me.applied(cand, &a))?;
        Ok(Some(out))
    }

    /// A function called by name (`apply`, a group's call) is read by the
    /// scan as called with its parameters' kinds — what the body reads
    /// them as, or their defaults — so inference types it and the compiled
    /// program holds it though no call names it positionally.
    pub(super) fn note_called_by_name(&mut self, fns: &[String]) {
        if self.pass != Pass::Scan || self.unscanned > 0 {
            return;
        }
        for f in fns {
            let typed = self.typed_params(f);
            if typed.iter().any(|(_, t)| matches!(t, Ty::Unknown)) {
                continue;
            }
            let types: Vec<Ty> = typed.iter().map(|(_, t)| t.clone()).collect();
            let dummies: Vec<A> = typed.iter().map(|_| A::Str(String::new())).collect();
            self.ctx.note_call(&A::Ident(f.clone()), &dummies, &types);
        }
    }

    /// What `describe` or `apply` answers where the pass only types it, or
    /// the call was refused: a value of its kind.
    fn placeholder(&self, name: &str) -> Expr {
        if name == DESCRIBE {
            Expr::Call {
                target: Target::Runtime(ERROR.to_string()),
                args: vec![Expr::Const(Const::Str(format!("{}[]", DESCRIBE)), Ty::Str), Expr::Const(Const::Str(DESCRIBE.to_string()), Ty::Str)],
                ty: Ty::Result(Box::new(Ty::Json)),
                propagates: false,
            }
        } else {
            let ok_text = crate::table::ROWS.iter().find(|r| r.id == OK_TEXT).expect("the text row exists");
            Expr::Call { target: Target::Row(ok_text), args: vec![Expr::Const(Const::Str(String::new()), Ty::Str)], ty: Ty::Result(Box::new(Ty::Str)), propagates: false }
        }
    }

    /// `apply`'s arguments, a parsed value or a map, bound once to a
    /// hidden local the reads take them from.
    fn bound_arguments(&mut self, arg: &A) -> Result<String, String> {
        let given = self.lower_expr(arg)?;
        let given = match given.ty().payload() {
            Ty::Json => given,
            Ty::Map(..) => {
                // (a map's JSON is its parsed value)
                let t = self.fresh();
                let ty = given.ty();
                self.pre.push(Stmt::Let { var: t.clone(), ty: ty.clone(), expr: given });
                self.scopes.last_mut().unwrap().insert(t.clone());
                self.ctx.vars.insert(t.clone(), ty);
                self.lower_expr(&A::Call { callee: Box::new(A::Ident(JSON.to_string())), args: vec![A::Call { callee: Box::new(A::Ident(ENCODE.to_string())), args: vec![A::Ident(t)] }] })?
            }
            other => {
                self.refuse(format!("apply[] takes the arguments as a parsed value or a map, got {}", crate::table::Arg::of_type(other).words()));
                given
            }
        };
        let given = self.decided(given, Decision::Read("apply".to_string()));
        let a = self.fresh();
        self.pre.push(Stmt::Let { var: a.clone(), ty: Ty::Json, expr: given });
        self.scopes.last_mut().unwrap().insert(a.clone());
        self.ctx.vars.insert(a.clone(), Ty::Json);
        Ok(a)
    }

    /// A function described or called by name whose parameter nothing
    /// types — not a call of its own, not its body's read, not a default —
    /// refused with the fix: where the body reads a parsed value as
    /// nothing, how to read it; else, to read it or call the function once.
    /// Refused once per function (the list it is written in, else the
    /// dispatch that meets it).
    pub(super) fn check_describable(&mut self, f: &str) {
        if self.undescribable.contains(f) {
            return;
        }
        let shown = crate::lower::shown(f);
        let declared: Vec<(String, Ty)> = self.ctx.functions.get(f).map(|sig| sig.params.clone()).unwrap_or_default();
        for ((p, ty), (_, declared)) in self.typed_params(f).iter().zip(declared.iter()) {
            // (a parameter handed a parsed value that the body reads as
            // nothing has no kind to be named by: a model could not be told
            // one — refused as before, with how to read it)
            let read = self.ctx.param_reads.get(f).and_then(|m| m.get(p)).is_some();
            let unreadable = matches!(declared.payload(), Ty::Json) && !read;
            if !matches!(ty, Ty::Unknown) && !unreadable {
                continue;
            }
            self.undescribable.insert(f.to_string());
            if matches!(declared.payload(), Ty::Json) {
                self.refuse(format!("{0}'s parameter '{1}' is handed a parsed value and read as nothing — read it in the body as what it is (upper[{1}] reads text, {1} + 0 a number, str[{1}] text), or call {0}[…] once with a value of its kind", shown, p));
            } else {
                self.refuse(format!("{0}'s parameter '{1}' is typed by nothing — read it in the body, or call {0}[…] once with a value of its kind", shown, p));
            }
            return;
        }
    }

    /// A function's description as `describe` answers it: JSON text.
    fn description(&mut self, f: &str) -> String {
        let quoted = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let shown = crate::lower::shown(f);
        self.check_describable(f);
        let params: Vec<(String, Ty)> = self.typed_params(f);
        let defaults = self.ctx.fn_defaults.get(f).cloned().unwrap_or_default();
        let mut written: Vec<String> = Vec::new();
        for (i, (p, ty)) in params.iter().enumerate() {
            let kind = crate::table::Arg::of_type(ty.payload()).words();
            let default = match defaults.get(i).and_then(|d| d.as_ref()) {
                Some(A::Int(n)) => n.to_string(),
                Some(A::Float(x)) => x.to_string(),
                Some(A::Bool(b)) => b.to_string(),
                Some(A::Str(s)) => quoted(s),
                Some(A::Array(items)) if items.is_empty() => "[]".to_string(),
                Some(A::Call { callee, args }) if matches!(callee.as_ref(), A::Ident(n) if n == MAP) && args.is_empty() => "{}".to_string(),
                _ => "null".to_string(),
            };
            // (`takes`, not `kind`: a value's `kind` is its error's)
            written.push(format!("{{\"name\": {}, \"takes\": {}, \"default\": {}}}", quoted(p), quoted(kind), default));
        }
        let answers = self.ctx.functions.get(f).map(|sig| crate::table::Arg::of_type(sig.return_type.payload()).words()).unwrap_or("a value");
        let doc = crate::fn_doc(f).unwrap_or_default();
        format!("{{\"name\": {}, \"doc\": {}, \"params\": [{}], \"answers\": {}}}", quoted(&shown), quoted(&doc), written.join(", "), quoted(answers))
    }

    /// A function's parameters with their kinds: as typed, or — handed a
    /// parsed value, or typed by nothing, as a tool's often are — as the
    /// body reads them (`param_reads`).
    fn typed_params(&self, f: &str) -> Vec<(String, Ty)> {
        let params: Vec<(String, Ty)> = self.ctx.functions.get(f).map(|sig| sig.params.clone()).unwrap_or_default();
        let defaults = self.ctx.fn_defaults.get(f).cloned().unwrap_or_default();
        params
            .into_iter()
            .enumerate()
            .map(|(i, (p, ty))| {
                let handed = matches!(ty.payload(), Ty::Json) || matches!(ty, Ty::Unknown);
                let mut ty = if handed { self.ctx.param_reads.get(f).and_then(|m| m.get(&p)).cloned().unwrap_or(ty) } else { ty };
                // (a default literal says the kind where nothing else does)
                if matches!(ty, Ty::Unknown) {
                    ty = match defaults.get(i).and_then(|d| d.as_ref()) {
                        Some(A::Int(_)) => Ty::Int,
                        Some(A::Float(_)) => Ty::Float,
                        Some(A::Str(_)) => Ty::Str,
                        Some(A::Bool(_)) => Ty::Bool,
                        _ => Ty::Unknown,
                    };
                }
                (p, ty)
            })
            .collect()
    }

    /// One candidate's call for `apply`: each parameter read from the
    /// parsed value `a` by name and kind into a hidden local, the call
    /// written over those locals, its answer encoded, as text that can fail.
    fn applied(&mut self, f: &str, a: &str) -> Result<Expr, String> {
        let shown = crate::lower::shown(f);
        let params: Vec<(String, Ty)> = self.typed_params(f);
        let mut locals: Vec<A> = Vec::new();
        for (p, ty) in &params {
            let (id, kind) = match ty.payload() {
                Ty::Int | Ty::Big => (ARG_INT, Ty::Int),
                Ty::Float => (ARG_FLOAT, Ty::Float),
                Ty::Str | Ty::Markup => (ARG_STR, Ty::Str),
                Ty::Bool => (ARG_BOOL, Ty::Bool),
                Ty::Json => (ARG_JSON, Ty::Json),
                other => {
                    self.refuse(format!(
                        "{0}'s parameter '{1}' takes {2}, which a call by name cannot hand it: a function called by name takes an int, a number, text, a bool or a parsed value",
                        shown,
                        p,
                        crate::table::Arg::of_type(other).words()
                    ));
                    (ARG_JSON, Ty::Json)
                }
            };
            let row = crate::table::ROWS.iter().find(|r| r.id == id).expect("the argument rows exist");
            let read = Expr::Call {
                target: Target::Row(row),
                args: vec![Expr::Local(a.to_string(), Ty::Json), Expr::Const(Const::Str(p.clone()), Ty::Str), Expr::Const(Const::Str(shown.clone()), Ty::Str)],
                ty: Ty::Result(Box::new(kind.clone())),
                propagates: false,
            };
            let t = self.fresh();
            self.pre.push(Stmt::Let { var: t.clone(), ty: Ty::Result(Box::new(kind.clone())), expr: read });
            self.scopes.last_mut().unwrap().insert(t.clone());
            self.ctx.vars.insert(t.clone(), Ty::Result(Box::new(kind)));
            locals.push(A::Ident(t));
        }
        let call = A::Call { callee: Box::new(A::Ident(f.to_string())), args: locals };
        let encoded = self.lower_expr(&A::Call { callee: Box::new(A::Ident(ENCODE.to_string())), args: vec![call] })?;
        // (text that can fail, whatever the call could: the branches answer one kind)
        let ok_text = crate::table::ROWS.iter().find(|r| r.id == OK_TEXT).expect("the text row exists");
        let (arg, propagates) = if encoded.ty().is_result() {
            (Expr::Convert { how: Convert::Unwrap, expr: Box::new(encoded), ty: Ty::Str }, true)
        } else {
            (encoded, false)
        };
        Ok(Expr::Call { target: Target::Row(ok_text), args: vec![arg], ty: Ty::Result(Box::new(Ty::Str)), propagates })
    }

    /// An error of kind `call` for a dispatch no candidate matched, of the
    /// result's kind.
    fn no_candidate(what: &str, result: &Ty) -> Expr {
        Expr::Call {
            target: Target::Runtime(ERROR.to_string()),
            args: vec![Expr::Const(Const::Str(format!("{}[]: no function of the ones it can be", what)), Ty::Str), Expr::Const(Const::Str("call".to_string()), Ty::Str)],
            ty: result.clone(),
            propagates: false,
        }
    }

    /// A value of the kind, for a dispatch's local before a branch sets
    /// it; none for a kind with no literal of its own.
    pub(super) fn zero_of(t: &Ty) -> Option<Expr> {
        Some(match t {
            Ty::Int => Expr::Const(Const::Int(0), Ty::Int),
            Ty::Float => Expr::Const(Const::Float(0.0), Ty::Float),
            Ty::Bool => Expr::Const(Const::Bool(false), Ty::Bool),
            Ty::Str => Expr::Const(Const::Str(String::new()), Ty::Str),
            Ty::List(_) => Expr::ListLit(Vec::new(), t.clone()),
            Ty::Map(..) => Expr::MapLit(Vec::new(), t.clone()),
            _ => return None,
        })
    }

    /// A dispatch the compiler writes: `_t` holds `init` until a candidate
    /// matches the subject (`__same_fn`), then that branch's value.
    pub(super) fn dispatch(&mut self, subject: Expr, candidates: &[String], result: Ty, init: Expr, branch: &dyn std::ops::Fn(&mut Self, &str) -> Result<Expr, String>) -> Result<Expr, String> {
        let t = self.fresh();
        self.pre.push(Stmt::Let { var: t.clone(), ty: result.clone(), expr: init });
        self.scopes.last_mut().unwrap().insert(t.clone());
        self.ctx.vars.insert(t.clone(), result.clone());
        let subject_ty = subject.ty();
        let s = self.fresh();
        self.pre.push(Stmt::Let { var: s.clone(), ty: subject_ty.clone(), expr: subject });
        self.scopes.last_mut().unwrap().insert(s.clone());
        self.ctx.vars.insert(s.clone(), subject_ty.clone());
        for cand in candidates {
            let saved = std::mem::take(&mut self.pre);
            let value = branch(self, cand)?;
            let mut then = std::mem::replace(&mut self.pre, saved);
            then.push(Stmt::Set { var: t.clone(), expr: value });
            let cand_ty = match &subject_ty {
                Ty::Fns(_) => subject_ty.clone(),
                _ => self.ctx.ident_type(cand),
            };
            let cond = Expr::Call {
                target: Target::Runtime(SAME_FN.to_string()),
                args: vec![Expr::Local(s.clone(), subject_ty.clone()), Expr::FnRef(cand.clone(), cand_ty)],
                ty: Ty::Bool,
                propagates: false,
            };
            self.pre.push(Stmt::If { cond, then, els: None });
        }
        Ok(Expr::Local(t, result))
    }

}
