//! The IR interpreter (docs/ir-design.md §5): walks the IR and decides
//! nothing. Every operation goes through the modules both walkers share,
//! `builtins`, `ops` and the module registry; values are the interpreter's
//! `CrowValue` for now, and the typed, compact values of the design are
//! the step after this walker is the default. `CROW_IR=1 crow run -i
//! file.co` runs a program here; `crow test --parity --ir` counts the
//! corpus programs it already runs identically.

use crate::ast::{BinOp, UnaryOp};
use crate::ir::*;
use crate::modules::ModuleRegistry;
use crate::value::*;
use std::collections::HashMap;

const READER_FNS: &[&str] = &["query", "read", "fetch", "recv", "input", "load", "env", "shell", "ls", "csv", "lines"];
/// Scalar and tensor names both core and math answer: scalars go to the
/// core builtin, as the AST walker routes them.
const OVERLAPPING: &[&str] = &["abs", "sqrt", "floor", "ceil", "round", "min", "max"];

enum Flow {
    Next,
    Return(CrowValue),
    Break,
    Continue,
}

struct Frame {
    scopes: Vec<HashMap<String, CrowValue>>,
    fn_name: String,
}

pub struct IrInterp {
    fns: HashMap<String, Fn>,
    derives: HashMap<String, Derive>,
    structs: HashMap<String, crate::ir::StructDef>,
    registry: ModuleRegistry,
    frames: Vec<Frame>,
    line: usize,
    file: String,
    /// The value of the last expression statement, what a collecting loop yields.
    last_value: Option<CrowValue>,
    /// Inside a view being rendered: bound inputs render, elements are markup.
    in_view: usize,
    /// One per render, for derivations that read the clock.
    tick: u64,
    engine_loaded: bool,
}

/// Run a program: its facts get their defaults, then its main block runs.
pub fn run(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<(), String> {
    let mut it = IrInterp::new(registry, source_file);
    it.load(program);
    it.run_main(program)
}

/// A test block's one input, run as the checker runs it: the program's
/// declarations first (a failure there is named as such), then the block
/// with its inputs bound. The block is lowered as the test it is, so its
/// statements keep their order — lowered at the top of a ui program they
/// would become facts, evaluated before main.
pub fn run_test(decls: &[crate::ast::Stmt], params: &[(String, CrowValue)], body: &[crate::ast::Stmt]) -> Result<(), String> {
    let mut all = decls.to_vec();
    all.push(crate::ast::Stmt::TestBlock {
        name: None,
        params: params.iter().map(|(p, _)| p.clone()).collect(),
        body: body.to_vec(),
    });
    let stmts = crate::lower::lower_program(&all)?;
    let mut ctx = crate::type_infer::TypeContext::new();
    ctx.describe_queries = false;
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "test")?;
    // CROW_IR_SHOW=1 prints the lowered test program, as `crow build --ir` does for a file
    if std::env::var("CROW_IR_SHOW").is_ok() {
        eprintln!("{}", crate::ir::print(&program));
    }
    let mut it = IrInterp::new(&ctx.registry, "test");
    it.load(&program);
    it.run_main(&program).map_err(|e| format!("a declaration failed before the test ran: {}", e))?;
    let test = program.parts.iter().flat_map(|p| p.tests.iter()).last().ok_or("lowering: the test block is missing")?;
    let mut scope = HashMap::new();
    for (p, v) in params {
        scope.insert(p.clone(), v.clone());
    }
    it.frames.push(Frame { scopes: vec![scope], fn_name: "test".to_string() });
    match it.exec_stmts(&test.body) {
        Ok(_) => Ok(()),
        Err(e) => Err(it.with_line(e)),
    }
}

impl IrInterp {
    fn new(registry: &ModuleRegistry, source_file: &str) -> Self {
        let file = std::path::Path::new(source_file)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| source_file.to_string());
        IrInterp {
            fns: HashMap::new(),
            derives: HashMap::new(),
            structs: HashMap::new(),
            registry: registry.clone(),
            frames: vec![Frame { scopes: vec![HashMap::new()], fn_name: String::new() }],
            line: 0,
            file,
            last_value: None,
            in_view: 0,
            tick: 0,
            engine_loaded: false,
        }
    }

    /// The facts' defaults, then main, part by part.
    fn run_main(&mut self, program: &Program) -> Result<(), String> {
        for part in &program.parts {
            for fact in &part.facts {
                let v = self.eval(&fact.default).map_err(|e| self.with_line(e))?;
                self.frames[0].scopes[0].insert(fact.name.clone(), v);
            }
            if let Err(e) = self.exec_stmts(&part.main) {
                return Err(self.with_line(e));
            }
        }
        Ok(())
    }

    fn load(&mut self, program: &Program) {
        for part in &program.parts {
            for f in &part.fns {
                self.fns.insert(f.name.clone(), f.clone());
            }
            for d in &part.derives {
                self.derives.insert(d.name.clone(), d.clone());
            }
            for s in &part.structs {
                self.structs.insert(s.name.clone(), s.clone());
            }
        }
    }

    /// The layout engine, Crow code lowered through the same pipeline and
    /// added to this walker's functions on first use by box, fits or style.
    fn ensure_engine(&mut self) -> Result<(), String> {
        if self.engine_loaded {
            return Ok(());
        }
        let stmts = crate::build(crate::interpreter::LAYOUT_ENGINE_SRC).map_err(|e| format!("layout engine: {}", e))?;
        let stmts = crate::lower::lower_program(&stmts).map_err(|e| format!("layout engine: {}", e))?;
        let mut ctx = crate::type_infer::TypeContext::new();
        ctx.describe_queries = false;
        ctx.infer_program(&stmts);
        let program = crate::lower_ir::lower(&stmts, &mut ctx, "layout").map_err(|e| format!("layout engine: {}", e))?;
        self.load(&program);
        self.engine_loaded = true;
        Ok(())
    }

    fn with_line(&self, e: String) -> String {
        if self.line > 0 && !e.starts_with("line ") {
            format!("line {}: {}", self.line, e)
        } else {
            e
        }
    }

    fn trace_entry(&self) -> String {
        let f = &self.frames.last().unwrap().fn_name;
        if f.is_empty() {
            format!("{}:{}", self.file, self.line)
        } else {
            format!("{}.{}:{}", self.file, f, self.line)
        }
    }

    fn in_function(&self) -> bool {
        self.frames.len() > 1
    }

    // ── scopes ──────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.frames.last_mut().unwrap().scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.frames.last_mut().unwrap().scopes.pop();
    }

    fn lookup(&self, name: &str) -> Option<&CrowValue> {
        let frame = self.frames.last().unwrap();
        for scope in frame.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn slot(&mut self, name: &str) -> Option<&mut CrowValue> {
        let frame = self.frames.last_mut().unwrap();
        for scope in frame.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                return scope.get_mut(name);
            }
        }
        None
    }

    fn bind(&mut self, name: &str, v: CrowValue) {
        self.frames.last_mut().unwrap().scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn assign(&mut self, name: &str, v: CrowValue) {
        if let Some(s) = self.slot(name) {
            *s = v;
        } else {
            self.bind(name, v);
        }
    }

    fn fact(&self, name: &str) -> Result<CrowValue, String> {
        self.frames[0].scopes[0].get(name).cloned().ok_or_else(|| format!("undefined variable '{}'", name))
    }

    // ── statements ──────────────────────────────────────────────────

    fn exec_stmts(&mut self, b: &[Stmt]) -> Result<Flow, String> {
        for s in b {
            match self.exec_stmt(s)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn exec_block(&mut self, b: &[Stmt]) -> Result<Flow, String> {
        self.push_scope();
        let r = self.exec_stmts(b);
        self.pop_scope();
        r
    }

    /// An error where a decision is needed: a function hands it to its
    /// caller, the top level halts with the reason (spec §10).
    fn refuse(&mut self, err: CrowValue) -> Result<Flow, String> {
        if self.in_function() {
            let mut e = err;
            e.append_trace(&self.trace_entry());
            Ok(Flow::Return(e))
        } else {
            Err(format!("cannot branch on an error value — {}", err.reason))
        }
    }

    fn exec_stmt(&mut self, s: &Stmt) -> Result<Flow, String> {
        match s {
            Stmt::Line(n) => {
                self.line = *n;
                Ok(Flow::Next)
            }
            Stmt::Let { var, expr, .. } => {
                let v = self.eval(expr)?;
                self.bind(var, v);
                Ok(Flow::Next)
            }
            Stmt::Set { var, expr } => {
                let v = self.eval(expr)?;
                self.assign(var, v);
                Ok(Flow::Next)
            }
            Stmt::Store { fact, expr } => {
                let v = self.eval(expr)?;
                self.frames[0].scopes[0].insert(fact.clone(), v);
                Ok(Flow::Next)
            }
            Stmt::SetPath { var, path, expr } => {
                let new = self.eval(expr)?;
                let mut idx = Vec::new();
                for p in path {
                    if let PathStep::Index(i) = p {
                        idx.push(self.eval(i)?);
                    }
                }
                let root = self
                    .slot(var)
                    .ok_or_else(|| format!("undefined variable '{}'", var))?;
                set_path(&mut root.value, path, &idx, new)?;
                Ok(Flow::Next)
            }
            Stmt::If { cond, then, els } => {
                let c = self.eval(cond)?;
                if c.is_error() {
                    return self.refuse(c);
                }
                if crate::ops::is_truthy(&c) {
                    self.exec_block(then)
                } else if let Some(e) = els {
                    self.exec_block(e)
                } else {
                    Ok(Flow::Next)
                }
            }
            Stmt::While { cond, body } => {
                loop {
                    let c = self.eval(cond)?;
                    if c.is_error() {
                        return self.refuse(c);
                    }
                    if !crate::ops::is_truthy(&c) {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::For { var, iter, body, .. } => {
                let it = self.eval(iter)?;
                if it.is_error() {
                    return Ok(Flow::Next);
                }
                let items = crate::ops::iter_items(&it)?;
                for item in items {
                    self.push_scope();
                    self.bind(var, item);
                    let r = self.exec_stmts(body);
                    self.pop_scope();
                    match r? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::Return(None) => Ok(Flow::Return(CrowValue::done())),
            Stmt::Return(Some(e)) => {
                let v = self.eval(e)?;
                Ok(Flow::Return(v))
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Do(e) => {
                let v = self.eval(e)?;
                self.last_value = Some(v);
                Ok(Flow::Next)
            }
            Stmt::Propagate { values } => {
                for v in values {
                    let val = self.lookup(v).cloned().ok_or_else(|| format!("undefined variable '{}'", v))?;
                    if val.is_error() {
                        return self.refuse(val);
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::Stop { value } => {
                let val = self.lookup(value).cloned().ok_or_else(|| format!("undefined variable '{}'", value))?;
                Err(format!("stopped: {}", val.reason))
            }
        }
    }

    // ── expressions ─────────────────────────────────────────────────

    fn eval(&mut self, e: &Expr) -> Result<CrowValue, String> {
        Ok(match e {
            Expr::Const(c, _) => match c {
                Const::Int(n) => CrowValue::ok(Value::Int(*n)),
                Const::Float(f) => CrowValue::ok(Value::Float(*f)),
                Const::Bool(b) => CrowValue::ok(Value::Bool(*b)),
                Const::Str(s) => CrowValue::ok(Value::Str(s.clone())),
                Const::Unit => CrowValue::done(),
            },
            Expr::Local(v, _) => self.lookup(v).cloned().ok_or_else(|| format!("undefined variable '{}'", v))?,
            Expr::Move(v, _) => {
                let slot = self.slot(v).ok_or_else(|| format!("undefined variable '{}'", v))?;
                std::mem::replace(slot, CrowValue::done())
            }
            Expr::Load(f, _) => self.fact(f)?,
            Expr::Derived(d, _) => self.derived(d)?,
            Expr::Constant(name, _) => {
                let bare = name.rsplit('.').next().unwrap_or(name);
                self.registry.get_constant(bare).ok_or_else(|| format!("unknown constant '{}'", name))?
            }
            Expr::FnRef(name, _) => {
                let params = self.fns.get(name).map(|f| f.params.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
                CrowValue::ok(Value::Fn(FnValue { name: name.clone(), params, body: std::sync::Arc::new(vec![]) }))
            }
            Expr::Deferred { name, args } => {
                let mut items = vec![self.eval(&Expr::FnRef(name.clone(), Ty::Unknown))?];
                items.extend(self.eval_args(args)?);
                CrowValue::ok(Value::Array(items))
            }
            Expr::Tree { element, attrs, children } => {
                let mut map = std::collections::BTreeMap::new();
                for (k, v) in attrs {
                    map.insert(k.clone(), self.eval(v)?);
                }
                let mut vals = vec![CrowValue::ok(Value::Map(map))];
                for c in children {
                    vals.push(self.eval(c)?);
                }
                CrowValue::ok(Value::Str(crate::ops::render_element(element, &vals)?))
            }
            // assert[a op b]: a failed comparison reports both operands
            Expr::Call { target: Target::Runtime(name), args, .. }
                if name == "assert" && args.len() == 1 && matches!(&args[0], Expr::Binary { op, .. } if matches!(op.as_str(), "eq" | "ne" | "gt" | "lt" | "gte" | "lte")) =>
            {
                let (op, l, r) = match &args[0] {
                    Expr::Binary { op, l, r, .. } => (op, l, r),
                    _ => unreachable!(),
                };
                let full = self.eval(&args[0])?;
                if !full.ok {
                    return Err(format!("cannot branch on an error value — {}", full.reason));
                }
                if matches!(full.value, Value::Bool(false)) {
                    let lv = self.eval(l)?;
                    let rv = self.eval(r)?;
                    return Err(format!("assert failed: {}\n    left:  {}\n    right: {}", op, lv.value, rv.value));
                }
                CrowValue::done()
            }
            Expr::Call { target: Target::Runtime(name), args, .. }
                if self.in_view > 0
                    && (matches!(name.as_str(), "field" | "slider" | "checkbox" | "toggle" | "radio" | "tabs")
                        || (name == "select" && args.len() == 2 && !matches!(args.first(), Some(Expr::MapLit(..)))))
                    && args.len() == match name.as_str() { "field" => 4, "slider" => 5, _ => 2 } =>
            {
                // a bound input: the variable's name is the binding's key
                let key = match args.get(1) {
                    Some(Expr::Load(n, _)) | Some(Expr::Local(n, _)) | Some(Expr::Move(n, _)) => n.clone(),
                    Some(Expr::Field { name, .. }) => name.clone(),
                    _ => "unknown".to_string(),
                };
                let vals = self.eval_args(args)?;
                CrowValue::ok(Value::Str(crate::ops::render_input(name, &key, &vals)?))
            }
            Expr::Call { target, args, propagates, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                match target {
                    Target::Runtime(path) => self.runtime_call(path, vals)?,
                    Target::User(name) => self.call_user(name, vals)?,
                    Target::Module(m, f) => self.module_call(m, f, vals)?,
                }
            }
            Expr::Effect { name, args, propagates, .. } => {
                let vals = self.eval_args(args)?;
                if *propagates {
                    if let Some(err) = self.first_error(&vals) {
                        return Ok(err);
                    }
                }
                self.effect(name, vals)?
            }
            Expr::Convert { how, expr, .. } => {
                let v = self.eval(expr)?;
                match how {
                    Convert::IntToFloat => match &v.value {
                        Value::Int(n) if v.ok => CrowValue::ok(Value::Float(*n as f64)),
                        _ => v,
                    },
                    // a CrowValue is its own payload; an error reached here is
                    // the caller's to propagate
                    Convert::Unwrap | Convert::ToStr | Convert::ToTensor | Convert::ToList => v,
                }
            }
            Expr::Binary { op, l, r, .. } => {
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                crate::ops::binop(&lv, &binop_of(op)?, &rv, &self.trace_entry())?
            }
            Expr::Unary { op, expr, .. } => {
                let v = self.eval(expr)?;
                let o = match op.as_str() {
                    "neg" => UnaryOp::Neg,
                    "not" => UnaryOp::Not,
                    other => return Err(format!("unknown unary operator '{}'", other)),
                };
                crate::ops::unaryop(&o, &v)?
            }
            Expr::Index { base, indices, .. } => {
                let b = self.eval(base)?;
                if b.is_error() {
                    return Ok(b);
                }
                let mut idx = Vec::with_capacity(indices.len());
                for i in indices {
                    idx.push(self.eval(i)?);
                }
                // a function value applied: f[3]; a gradient applied: df[3.0]
                match &b.value {
                    Value::Fn(fv) => {
                        let name = fv.name.clone();
                        if let Some(err) = self.first_error(&idx) {
                            return Ok(err);
                        }
                        return self.call_user(&name, idx);
                    }
                    Value::GradFn(fv) => {
                        let name = fv.name.clone();
                        return self.grad_call(&name, idx);
                    }
                    _ => {}
                }
                crate::ops::index(&b, idx)?
            }
            Expr::Field { base, name, .. } => {
                let b = self.eval(base)?;
                // an app session reads the reactive state, which only this walker holds
                if let Value::App(..) = &b.value {
                    return self.frames[0].scopes[0].get(name).cloned().ok_or_else(|| format!("no reactive state named '{}'", name));
                }
                crate::ops::field(b, name)?
            }
            Expr::Meta { base, which } => {
                let b = self.eval(base)?;
                let name = match which {
                    Meta::Ok => "ok",
                    Meta::Reason => "reason",
                    Meta::Kind => "kind",
                    Meta::Trace => "trace",
                };
                crate::ops::field(b, name)?
            }
            Expr::Interp(pieces) => {
                let mut out = String::new();
                for p in pieces {
                    match p {
                        Piece::Text(t) => out.push_str(t),
                        Piece::Expr(x) => {
                            let v = self.eval(x)?;
                            if v.is_error() {
                                let mut err = v;
                                err.append_trace(&self.trace_entry());
                                return Ok(err);
                            }
                            out.push_str(&format!("{}", v.value));
                        }
                    }
                }
                CrowValue::ok(Value::Str(out))
            }
            Expr::ListLit(items, _) => {
                let mut vals = Vec::with_capacity(items.len());
                for i in items {
                    vals.push(self.eval(i)?);
                }
                crate::ops::array_literal(vals)?
            }
            Expr::MapLit(pairs, _) => {
                let mut map = std::collections::BTreeMap::new();
                for (k, v) in pairs {
                    map.insert(k.clone(), self.eval(v)?);
                }
                CrowValue::ok(Value::Map(map))
            }
            Expr::RecordLit { name, fields } => {
                let def = self.structs.get(name).cloned().ok_or_else(|| format!("unknown struct '{}'", name))?;
                if def.fields.len() != fields.len() {
                    return Err(format!("{}[] expects {} fields, got {}", name, def.fields.len(), fields.len()));
                }
                let mut vals = std::collections::BTreeMap::new();
                for ((fname, _), e) in def.fields.iter().zip(fields) {
                    vals.insert(fname.clone(), self.eval(e)?);
                }
                CrowValue::ok(Value::StructInstance(StructInstance { name: name.clone(), fields: vals }))
            }
            Expr::ForExpr { var, iter, body, .. } => {
                let it = self.eval(iter)?;
                if it.is_error() {
                    return Ok(CrowValue::ok(Value::Array(vec![it])));
                }
                let items = crate::ops::iter_items(&it)?;
                let mut results = Vec::with_capacity(items.len());
                for item in items {
                    self.push_scope();
                    self.bind(var, item);
                    self.last_value = None;
                    let r = self.exec_stmts(body);
                    let yielded = self.last_value.take();
                    self.pop_scope();
                    match r? {
                        Flow::Return(_) => return Err("cannot return from inside a for loop".to_string()),
                        Flow::Break => break,
                        _ => {}
                    }
                    results.push(yielded.unwrap_or_else(CrowValue::done));
                }
                crate::ops::collect_loop(results)
            }
        })
    }

    fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<CrowValue>, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a)?);
        }
        Ok(vals)
    }

    fn first_error(&self, vals: &[CrowValue]) -> Option<CrowValue> {
        for v in vals {
            if v.is_error() {
                let mut err = v.clone();
                err.append_trace(&self.trace_entry());
                return Some(err);
            }
        }
        None
    }

    fn stamp(name: &str, v: CrowValue) -> CrowValue {
        if READER_FNS.contains(&name) && v.is_ok() {
            v.stamp_captured()
        } else {
            v
        }
    }

    /// A runtime builtin by its table name (`core.push`) or its own name.
    fn runtime_call(&mut self, path: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let name = RUNTIME_TABLE.iter().find(|(_, r)| *r == path).map(|(b, _)| *b).unwrap_or(path).to_string();
        self.bare_call(&name, vals)
    }

    /// A bare name, resolved as the interpreter resolves it: a loaded
    /// module's function wins over a builtin of the same name, except that
    /// a frame function given something other than a frame is the builtin
    /// (sort[xs], join[parts, sep]); then the builtins; then what only a
    /// walker can do.
    fn bare_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if name == "transact" {
            return self.transact(vals);
        }
        if matches!(name, "serve" | "page" | "fetch" | "poll" | "app" | "html" | "elements" | "change" | "click" | "box" | "fits" | "style") {
            return self.harness(name, vals);
        }
        // a scalar name both core and math answer goes to the core builtin
        // unless a tensor is among the arguments
        let scalar_overlap = OVERLAPPING.contains(&name) && !vals.iter().any(|v| matches!(v.value, Value::NumArray(_)));
        if self.registry.has_function(name) && !scalar_overlap {
            let frame_fn = self.registry.module_name_for(name) == Some("frame");
            let first_is_frame = vals.first().map_or(false, |a| matches!(a.value, Value::Frame(_)));
            let core_too = crate::checker::BUILTINS.contains(&name);
            if !(frame_fn && core_too && !first_is_frame) {
                return self.registry.call(name, &vals).map(|v| Self::stamp(name, v));
            }
        }
        if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
            return r.map(|v| Self::stamp(name, v));
        }
        match name {
            "any" | "all" => {
                let want = name == "any";
                let items = crate::ops::iter_items(&vals[0])?;
                for item in items {
                    let hit = self.call_value(&vals[1], vec![item])?;
                    if crate::ops::is_truthy(&hit) == want {
                        return Ok(CrowValue::ok(Value::Bool(want)));
                    }
                }
                Ok(CrowValue::ok(Value::Bool(!want)))
            }
            "find" => {
                if vals.len() != 2 {
                    return Err("find[] takes exactly 2 arguments".into());
                }
                // find[text, pattern]: a record with the groups, or an error value
                if let Value::Str(s) = &vals[0].value {
                    let pattern = match &vals[1].value {
                        Value::Str(p) => p.clone(),
                        _ => return Err("find[] second argument must be a string pattern".into()),
                    };
                    return Ok(match crow_runtime::text::find(s, &pattern) {
                        Ok(Some(groups)) => {
                            let mut map = std::collections::BTreeMap::new();
                            map.insert("groups".to_string(), CrowValue::ok(Value::Array(groups.into_iter().map(|m| CrowValue::ok(Value::Str(m))).collect())));
                            CrowValue::ok(Value::Map(map))
                        }
                        Ok(None) => CrowValue::error("no match", &self.trace_entry()),
                        Err(e) => CrowValue::error_with_kind(&e, "regex", &self.trace_entry()),
                    });
                }
                let items = crate::ops::iter_items(&vals[0])?;
                for item in items {
                    let hit = self.call_value(&vals[1], vec![item.clone()])?;
                    if crate::ops::is_truthy(&hit) {
                        return Ok(item);
                    }
                }
                Ok(CrowValue::error("no match", "find"))
            }
            "captured" => {
                if vals.len() != 1 {
                    return Err("captured[] takes exactly 1 argument".into());
                }
                if vals[0].is_error() {
                    let mut err = vals[0].clone();
                    err.append_trace(&self.trace_entry());
                    return Ok(err);
                }
                Ok(match vals[0].captured {
                    Some(t) => CrowValue::ok(Value::Float(t)),
                    None => CrowValue::error_with_kind(
                        "this value was computed here, not read from outside - it has no capture moment",
                        "no_capture",
                        &self.trace_entry(),
                    ),
                })
            }
            "grad" => {
                if vals.len() != 1 {
                    return Err("grad[] takes exactly 1 argument".into());
                }
                match &vals[0].value {
                    Value::Fn(fv) => Ok(CrowValue::ok(Value::GradFn(fv.clone()))),
                    other => Err(format!("grad[] takes a function, got {}", other.type_name())),
                }
            }
            "later" => {
                // the interpreter runs the work inline: immediate is a legal
                // schedule (spec §15)
                if vals.is_empty() {
                    return Err("later[] needs a function: later[work] or later[work, args…]".into());
                }
                let f = vals[0].clone();
                let rest: Vec<CrowValue> = vals[1..].to_vec();
                let _ = self.call_value(&f, rest)?;
                Ok(CrowValue::ok(Value::Bool(true)))
            }
            "running" => Ok(CrowValue::ok(Value::Bool(false))),
            // progress[value, max] (lowered)
            "progress" if vals.len() == 2 => {
                if self.in_view == 0 {
                    return Ok(CrowValue::done());
                }
                let num = |v: &CrowValue| match &v.value {
                    Value::Int(n) => *n as f64,
                    Value::Float(f) => *f,
                    _ => 0.0,
                };
                let fmt_num = |x: f64| if (x - x.round()).abs() < 1e-9 { format!("{}", x.round() as i64) } else { format!("{}", x) };
                let max = num(&vals[1]);
                Ok(CrowValue::ok(Value::Str(format!(
                    "<progress class=\"crow-progress\" max=\"{}\" value=\"{}\"></progress>",
                    fmt_num(if max > 0.0 { max } else { 1.0 }),
                    fmt_num(num(&vals[0]))
                ))))
            }
            // icon[name, size, color] (lowered): inline SVG from the kernel's set
            "icon" if vals.len() == 3 => {
                if self.in_view == 0 {
                    return Ok(CrowValue::done());
                }
                let s = |v: &CrowValue| format!("{}", v.value);
                crow_runtime::layout::icon_html(&s(&vals[0]), &s(&vals[1]), &s(&vals[2]))
                    .map(|svg| CrowValue::ok(Value::Str(svg)))
                    .map_err(|e| format!("icon: {}", e))
            }
            "cpu" | "gpu" | "nodes" => Ok(CrowValue::done()),
            _ => Err(format!("the IR interpreter does not run '{}' yet", name)),
        }
    }

    /// `transact[conn, f]`: BEGIN, the function on the same connection,
    /// COMMIT, or ROLLBACK when it fails, as the interpreter does.
    fn transact(&mut self, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if vals.len() != 2 {
            return Err("transact[] takes 2 arguments: transact[conn, fn_name]".into());
        }
        let conn = match &vals[0].value {
            Value::DbConn(h) => h.clone(),
            other => return Err(format!("transact[] first argument must be a db connection, got {}", other.type_name())),
        };
        let f = vals[1].clone();
        conn.exec("BEGIN", &[]).map_err(|e| format!("transact[]: {}", e))?;
        let tx_val = CrowValue::ok(Value::DbConn(conn.clone()));
        match self.call_value(&f, vec![tx_val]) {
            Ok(val) => {
                conn.exec("COMMIT", &[]).map_err(|e| format!("transact[]: commit failed: {}", e))?;
                Ok(val)
            }
            Err(e) => {
                let _ = conn.exec("ROLLBACK", &[]);
                Err(format!("transact[]: rolled back: {}", e))
            }
        }
    }

    /// Forward-mode differentiation: the function once per parameter with
    /// that parameter a dual (x, 1) and the others (x, 0).
    fn grad_call(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let nparams = self.fns.get(name).map(|f| f.params.len()).ok_or_else(|| format!("unknown function '{}'", name))?;
        if nparams != vals.len() {
            return Err(format!("grad[{}][] expects {} args, got {}", name, nparams, vals.len()));
        }
        let xs: Vec<f64> = vals
            .iter()
            .map(|a| match &a.value {
                Value::Int(n) => Ok(*n as f64),
                Value::Float(n) => Ok(*n),
                Value::Dual(n, _) => Ok(*n),
                _ => Err(format!("grad[{}][] requires numeric arguments", name)),
            })
            .collect::<Result<_, _>>()?;
        let mut derivatives = Vec::with_capacity(xs.len());
        for i in 0..xs.len() {
            let args: Vec<CrowValue> = xs.iter().enumerate().map(|(j, &x)| CrowValue::ok(Value::Dual(x, if j == i { 1.0 } else { 0.0 }))).collect();
            let result = self.call_user(name, args)?;
            let d = match &result.value {
                Value::Dual(_, d) => *d,
                Value::Float(_) | Value::Int(_) => 0.0,
                other => return Err(format!("grad[{}][] requires function to return a number, got {}", name, other.type_name())),
            };
            derivatives.push(d);
        }
        if derivatives.len() == 1 {
            Ok(CrowValue::ok(Value::Float(derivatives[0])))
        } else {
            Ok(CrowValue::ok(Value::Array(derivatives.into_iter().map(|d| CrowValue::ok(Value::Float(d))).collect())))
        }
    }

    fn module_call(&mut self, module: &str, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if name == "transact" && module == "db" {
            return self.transact(vals);
        }
        if OVERLAPPING.contains(&name) && !vals.iter().any(|v| matches!(v.value, Value::NumArray(_))) {
            if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
                return r;
            }
        }
        if self.registry.is_loaded(module) {
            return self.registry.call_in(module, name, &vals).map(|v| Self::stamp(name, v));
        }
        Err(format!("the IR interpreter does not run '{}.{}' yet", module, name))
    }

    /// An effect by name: qualified (`frame.load`) through its module,
    /// bare as any bare call.
    fn effect(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some((m, f)) = name.split_once('.') {
            let (m, f) = (m.to_string(), f.to_string());
            return self.module_call(&m, &f, vals);
        }
        self.bare_call(name, vals)
    }

    fn call_value(&mut self, f: &CrowValue, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        match &f.value {
            Value::Fn(fv) => self.call_user(&fv.name.clone(), args),
            other => Err(format!("expected a function, got {}", other.type_name())),
        }
    }

    fn call_user(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        // a specialised name (first__1) is the one body to this walker
        let f = match self.fns.get(name) {
            Some(f) => f.clone(),
            None => {
                let base = name.split("__").next().unwrap_or(name);
                self.fns.get(base).cloned().ok_or_else(|| format!("unknown function '{}'", name))?
            }
        };
        if f.params.len() != vals.len() {
            return Err(format!("{}[] expects {} arguments, got {}", name, f.params.len(), vals.len()));
        }
        // a view renders what it is given: a failed argument is its text
        if f.kind == FnKind::View {
            if let Some(bad) = vals.iter().find(|v| v.is_error()) {
                return Ok(CrowValue::ok(Value::Str(format!("error: {}", crate::interpreter::html_escape_text(&bad.reason)))));
            }
        }
        let mut scope = HashMap::new();
        for ((p, _), v) in f.params.iter().zip(vals) {
            scope.insert(p.clone(), v);
        }
        let saved_line = self.line;
        let is_view = f.kind == FnKind::View;
        if is_view {
            self.in_view += 1;
        }
        self.frames.push(Frame { scopes: vec![scope], fn_name: name.to_string() });
        let r = self.exec_stmts(&f.body);
        self.frames.pop();
        if is_view {
            self.in_view -= 1;
        }
        self.line = saved_line;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(CrowValue::done()),
        }
    }

    /// A derivation, recomputed on every read for now (the memoized,
    /// versioned form comes with the session layer), stamped with its moment.
    fn derived(&mut self, name: &str) -> Result<CrowValue, String> {
        let d = self.derives.get(name).cloned().ok_or_else(|| format!("unknown derivation '{}'", name))?;
        self.frames.push(Frame { scopes: vec![HashMap::new()], fn_name: name.to_string() });
        let r = self.exec_stmts(&d.body);
        self.frames.pop();
        let mut v = match r? {
            Flow::Return(v) => v,
            _ => CrowValue::done(),
        };
        v.captured = Some(crow_runtime::moments::now());
        Ok(v)
    }

    // ── the harness and the live layer's mocks ─────────────────────

    /// A session's view rendered to markup.
    fn render_view(&mut self, view: &str) -> Result<String, String> {
        self.tick += 1;
        let r = self.call_user(view, Vec::new())?;
        Ok(match &r.value {
            Value::Str(s) => s.clone(),
            other => format!("{}", other),
        })
    }

    fn harness(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let app_view = |v: &CrowValue| -> Option<(FnValue, f64, bool)> {
            if let Value::App(f, w, wide) = &v.value { Some((f.clone(), *w, *wide)) } else { None }
        };
        Ok(match name {
            "serve" | "page" => CrowValue::done(),
            "fetch" | "poll" => CrowValue::ok(Value::Map(std::collections::BTreeMap::new())),
            "app" => {
                let f = match vals.first().map(|v| &v.value) {
                    Some(Value::Fn(f)) => f.clone(),
                    _ => return Err("app[] takes a view name: app[render] (and an optional width = 390, wide = true)".into()),
                };
                let mut width = 1280.0;
                let mut wide = false;
                for a in &vals[1..] {
                    match &a.value {
                        Value::Int(n) => width = *n as f64,
                        Value::Float(x) => width = *x,
                        Value::Bool(b) => wide = *b,
                        _ => return Err("app[] takes a view name, then width = 390 and wide = true".into()),
                    }
                }
                CrowValue::ok(Value::App(f, width, wide))
            }
            "html" => {
                let (f, _, _) = vals.first().and_then(app_view).ok_or("html[] takes an app session: html[a]")?;
                CrowValue::ok(Value::Str(self.render_view(&f.name)?))
            }
            "elements" => {
                let (f, _, _) = vals.first().and_then(app_view).ok_or("elements[] takes an app session and a selector: elements[a, 'button']")?;
                let selector = match vals.get(1).map(|v| &v.value) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => return Err("elements[] selector must be a string like 'button', '.card', or 'div.stats .card'".into()),
                };
                let html = self.render_view(&f.name)?;
                let tree = crate::interpreter::harness_parse_html(&html);
                let steps = selector_steps(&selector);
                if steps.is_empty() {
                    return Err("elements[] selector is empty".into());
                }
                let mut hits = Vec::new();
                crate::interpreter::harness_select(&tree, &steps, 0, &mut hits);
                CrowValue::ok(Value::Array(hits))
            }
            "change" => {
                if vals.len() != 3 || !matches!(vals[0].value, Value::App(..)) {
                    return Err("change[] first argument must be an app session: change[a, 'min_qty', 5]".into());
                }
                let var = match &vals[1].value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("change[] names the reactive variable as a string: change[a, 'min_qty', 5]".into()),
                };
                let value = vals[2].clone();
                let current = self.frames[0].scopes[0].get(&var).cloned().ok_or_else(|| format!("change[]: no reactive variable named '{}' (reactive state is a top-level assignment)", var))?;
                let compatible = match (&current.value, &value.value) {
                    (Value::Int(_), Value::Int(_)) | (Value::Float(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) | (Value::Int(_), Value::Float(_)) | (Value::Str(_), Value::Str(_)) | (Value::Bool(_), Value::Bool(_)) => true,
                    (a, b) => a.type_name() == b.type_name(),
                };
                if !compatible {
                    return Err(format!("change[]: '{}' holds {} — cannot set it to {}", var, current.value.type_name(), value.value.type_name()));
                }
                let value = match (&current.value, &value.value) {
                    (Value::Float(_), Value::Int(i)) => CrowValue::ok(Value::Float(*i as f64)),
                    _ => value,
                };
                self.frames[0].scopes[0].insert(var, value);
                CrowValue::done()
            }
            "click" => {
                if vals.len() != 2 || !matches!(vals[0].value, Value::App(..)) {
                    return Err("click[] first argument must be an app session: click[a, go]".into());
                }
                let saved_view = self.in_view;
                self.in_view = 0;
                let r = match &vals[1].value {
                    Value::Fn(f) => {
                        let name = f.name.clone();
                        self.call_user(&name, Vec::new())
                    }
                    Value::Array(items) if matches!(items.first().map(|v| &v.value), Some(Value::Fn(_))) => {
                        let name = if let Value::Fn(f) = &items[0].value { f.name.clone() } else { unreachable!() };
                        self.call_user(&name, items[1..].to_vec())
                    }
                    _ => Err("click[] second argument must be a handler name or call: click[a, go] or click[a, go[x]]".to_string()),
                };
                self.in_view = saved_view;
                r?
            }
            "box" | "fits" | "style" => {
                if vals.len() != 2 {
                    return Err(format!("{}[] takes an app session and a selector or width", name));
                }
                let (f, session_width, session_wide) = app_view(&vals[0]).ok_or_else(|| format!("{}[] takes an app session first", name))?;
                let arg1 = vals[1].clone();
                let html = self.render_view(&f.name)?;
                let tree = crate::interpreter::harness_parse_html(&html);
                if name == "style" {
                    let selector = match &arg1.value {
                        Value::Str(s) => s.clone(),
                        _ => return Err("style[] selector must be a string".into()),
                    };
                    let steps = selector_steps(&selector);
                    let mut hits: Vec<CrowValue> = Vec::new();
                    crate::interpreter::harness_select(&tree, &steps, 0, &mut hits);
                    let mut out = Vec::new();
                    for h in hits {
                        if let Value::Map(m) = &h.value {
                            out.push(style_tokens(m));
                        }
                    }
                    return Ok(CrowValue::ok(Value::Array(out)));
                }
                self.ensure_engine()?;
                let viewport = if name == "fits" {
                    match &arg1.value {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => return Err("fits[] takes a width: fits[s, 390]".into()),
                    }
                } else {
                    session_width
                };
                let root = crate::interpreter::harness_to_lnode(&tree);
                let boxes = self.call_user("crow_layout", vec![root, CrowValue::ok(Value::Float(viewport)), CrowValue::ok(Value::Bool(session_wide))])?;
                let list = match &boxes.value {
                    Value::Array(items) => items.clone(),
                    _ => Vec::new(),
                };
                if name == "fits" {
                    let any_overflow = list.iter().any(|b| match &b.value {
                        Value::StructInstance(si) => matches!(si.fields.get("overflow").map(|v| &v.value), Some(Value::Bool(true))),
                        _ => false,
                    });
                    return Ok(CrowValue::ok(Value::Bool(!any_overflow)));
                }
                let selector = match &arg1.value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("box[] selector must be a string like 'button' or 'div.stats .card'".into()),
                };
                let steps = selector_steps(&selector);
                let hits: Vec<CrowValue> = list
                    .into_iter()
                    .filter(|b| {
                        if let Value::StructInstance(si) = &b.value {
                            let path = si.fields.get("path").map(|v| format!("{}", v.value)).unwrap_or_default();
                            crate::interpreter::harness_path_matches(&path, &steps)
                        } else {
                            false
                        }
                    })
                    .collect();
                CrowValue::ok(Value::Array(hits))
            }
            _ => return Err(format!("the IR interpreter does not run '{}' yet", name)),
        })
    }
}

/// A selector's steps: `tag`, `.class`, `tag.class`, space-separated.
fn selector_steps(selector: &str) -> Vec<(String, String)> {
    selector
        .split_whitespace()
        .map(|step| match step.find('.') {
            Some(i) => (step[..i].to_string(), step[i + 1..].to_string()),
            None => (step.to_string(), String::new()),
        })
        .collect()
}

/// The resolved tokens of a rendered element, from its class and style.
fn style_tokens(m: &std::collections::BTreeMap<String, CrowValue>) -> CrowValue {
    let cls = m.get("class").map(|v| format!("{}", v.value)).unwrap_or_default();
    let style = m.get("style").map(|v| format!("{}", v.value)).unwrap_or_default();
    let mut background = String::new();
    let mut color = String::from("--crow-text");
    let mut size = String::from("m");
    let mut weight = String::from("regular");
    for c in cls.split_whitespace() {
        match c {
            "card" => background = "--crow-surface".into(),
            "primary" => {
                background = "--crow-accent".into();
                color = "--crow-bg".into();
            }
            "muted" => color = "--crow-muted".into(),
            "title" => {
                size = "xl".into();
                weight = "bold".into();
            }
            "label" => {
                size = "s".into();
                color = "--crow-muted".into();
            }
            _ => {}
        }
    }
    for decl in style.split(';') {
        if let Some((k, v)) = decl.split_once(':') {
            let v = v.trim();
            match k.trim() {
                "color" => color = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                "background" => background = v.trim_start_matches("var(").trim_end_matches(')').to_string(),
                "font-size" => {
                    size = match v {
                        "12px" => "xs",
                        "14px" => "s",
                        "16px" => "m",
                        "20px" => "l",
                        "24px" => "xl",
                        "32px" => "xxl",
                        _ => "m",
                    }
                    .to_string()
                }
                "font-weight" => weight = match v { "700" => "bold", "500" => "medium", _ => "regular" }.to_string(),
                _ => {}
            }
        }
    }
    let mut r = std::collections::BTreeMap::new();
    r.insert("background".to_string(), CrowValue::ok(Value::Str(background)));
    r.insert("color".to_string(), CrowValue::ok(Value::Str(color)));
    r.insert("size".to_string(), CrowValue::ok(Value::Str(size)));
    r.insert("weight".to_string(), CrowValue::ok(Value::Str(weight)));
    CrowValue::ok(Value::Map(r))
}

fn binop_of(op: &str) -> Result<BinOp, String> {
    Ok(match op {
        "add" => BinOp::Add,
        "sub" => BinOp::Sub,
        "mul" => BinOp::Mul,
        "div" => BinOp::Div,
        "mod" => BinOp::Mod,
        "pow" => BinOp::Pow,
        "elemmul" => BinOp::ElemMul,
        "elemdiv" => BinOp::ElemDiv,
        "elempow" => BinOp::ElemPow,
        "eq" => BinOp::Eq,
        "ne" => BinOp::Ne,
        "gt" => BinOp::Gt,
        "lt" => BinOp::Lt,
        "gte" => BinOp::Gte,
        "lte" => BinOp::Lte,
        "and" => BinOp::And,
        "or" => BinOp::Or,
        other => return Err(format!("unknown operator '{}'", other)),
    })
}

/// `x.f = v`, `xs[i] = v`, and their nestings, applied to a value in place.
fn set_path(root: &mut Value, path: &[PathStep], idx: &[CrowValue], new: CrowValue) -> Result<(), String> {
    fn step<'v>(cur: &'v mut Value, p: &PathStep, idx: &mut std::slice::Iter<'_, CrowValue>) -> Result<&'v mut CrowValue, String> {
        match p {
            PathStep::Field(f) => match cur {
                Value::StructInstance(si) => si.fields.get_mut(f).ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, f)),
                Value::Map(m) => m.get_mut(f).ok_or_else(|| format!("map has no key '{}'", f)),
                other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
            },
            PathStep::Index(_) => {
                let i = idx.next().ok_or("missing index")?;
                match cur {
                    Value::Array(items) => match &i.value {
                        Value::Int(n) => {
                            let len = items.len() as i64;
                            let r = if *n < 0 { len + n } else { *n };
                            if r < 0 || r >= len {
                                return Err(format!("index {} out of bounds (len {})", n, items.len()));
                            }
                            Ok(&mut items[r as usize])
                        }
                        _ => Err("array index must be an integer".to_string()),
                    },
                    Value::Map(m) => match &i.value {
                        Value::Str(k) => m.get_mut(k).ok_or_else(|| format!("map has no key '{}'", k)),
                        _ => Err("map key must be a string".to_string()),
                    },
                    other => Err(format!("cannot index-assign into {}", other.type_name())),
                }
            }
        }
    }
    let mut it = idx.iter();
    let (last, front) = path.split_last().ok_or("empty path")?;
    let mut cur: &mut Value = root;
    for p in front {
        cur = &mut step(cur, p, &mut it)?.value;
    }
    match last {
        PathStep::Field(f) => match cur {
            Value::StructInstance(si) => {
                if !si.fields.contains_key(f) {
                    return Err(format!("struct '{}' has no field '{}'", si.name, f));
                }
                si.fields.insert(f.clone(), new);
                Ok(())
            }
            Value::Map(m) => {
                crate::builtins::check_map_homogeneous(m, &new)?;
                m.insert(f.clone(), new);
                Ok(())
            }
            other => Err(format!("cannot assign field '{}' on {}", f, other.type_name())),
        },
        PathStep::Index(_) => {
            let i = it.next().ok_or("missing index")?;
            match cur {
                Value::Array(items) => match &i.value {
                    Value::Int(n) => {
                        let len = items.len() as i64;
                        let r = if *n < 0 { len + n } else { *n };
                        if r < 0 || r >= len {
                            return Err(format!("index {} out of bounds (len {})", n, items.len()));
                        }
                        items[r as usize] = new;
                        Ok(())
                    }
                    _ => Err("array index must be an integer".to_string()),
                },
                Value::NumArray(t) => match (&i.value, &new.value) {
                    (Value::Int(n), Value::Int(_) | Value::Float(_)) => {
                        let mut data = t.to_vec();
                        let len = data.len() as i64;
                        let r = if *n < 0 { len + n } else { *n };
                        if r < 0 || r >= len {
                            return Err(format!("index {} out of bounds (len {})", n, data.len()));
                        }
                        data[r as usize] = match &new.value {
                            Value::Int(x) => *x as f64,
                            Value::Float(x) => *x,
                            _ => unreachable!(),
                        };
                        let shape = t.shape().to_vec();
                        *t = crow_runtime::tensor::CrowTensor::from_vec(data, shape);
                        Ok(())
                    }
                    _ => Err("a numeric array takes a number at an integer index".to_string()),
                },
                Value::Map(m) => match &i.value {
                    Value::Str(k) => {
                        crate::builtins::check_map_homogeneous(m, &new)?;
                        m.insert(k.clone(), new);
                        Ok(())
                    }
                    _ => Err("map key must be a string".to_string()),
                },
                other => Err(format!("cannot index-assign into {}", other.type_name())),
            }
        }
    }
}
