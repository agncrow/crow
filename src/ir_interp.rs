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

pub struct IrInterp<'a> {
    fns: HashMap<String, &'a Fn>,
    derives: HashMap<String, &'a Derive>,
    structs: HashMap<String, &'a crate::ir::StructDef>,
    registry: &'a ModuleRegistry,
    frames: Vec<Frame>,
    line: usize,
    file: String,
    /// The value of the last expression statement, what a collecting loop yields.
    last_value: Option<CrowValue>,
}

/// Run a program: its facts get their defaults, then its main block runs.
pub fn run(program: &Program, registry: &ModuleRegistry, source_file: &str) -> Result<(), String> {
    let file = std::path::Path::new(source_file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| source_file.to_string());
    let mut it = IrInterp {
        fns: HashMap::new(),
        derives: HashMap::new(),
        structs: HashMap::new(),
        registry,
        frames: vec![Frame { scopes: vec![HashMap::new()], fn_name: String::new() }],
        line: 0,
        file,
        last_value: None,
    };
    for part in &program.parts {
        for f in &part.fns {
            it.fns.insert(f.name.clone(), f);
        }
        for d in &part.derives {
            it.derives.insert(d.name.clone(), d);
        }
        for s in &part.structs {
            it.structs.insert(s.name.clone(), s);
        }
    }
    for part in &program.parts {
        for fact in &part.facts {
            let v = it.eval(&fact.default).map_err(|e| it.with_line(e))?;
            it.frames[0].scopes[0].insert(fact.name.clone(), v);
        }
        match it.exec_stmts(&part.main) {
            Ok(_) => {}
            Err(e) => return Err(it.with_line(e)),
        }
    }
    Ok(())
}

impl<'a> IrInterp<'a> {
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
                crate::ops::index(&b, idx)?
            }
            Expr::Field { base, name, .. } => {
                let b = self.eval(base)?;
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
                let def = self.structs.get(name).copied().ok_or_else(|| format!("unknown struct '{}'", name))?;
                if def.fields.len() != fields.len() {
                    return Err(format!("{}[] expects {} fields, got {}", name, def.fields.len(), fields.len()));
                }
                let mut vals = std::collections::BTreeMap::new();
                for ((fname, _), e) in def.fields.iter().zip(fields) {
                    vals.insert(fname.clone(), self.eval(e)?);
                }
                CrowValue::ok(Value::StructInstance(StructInstance { name: name.clone(), fields: vals }))
            }
            Expr::Tree { element, .. } => {
                return Err(format!("the IR interpreter does not run views yet (element '{}')", element));
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
        let name = RUNTIME_TABLE.iter().find(|(_, r)| *r == path).map(|(b, _)| *b).unwrap_or(path);
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
                let items = crate::ops::iter_items(&vals[0])?;
                for item in items {
                    let hit = self.call_value(&vals[1], vec![item.clone()])?;
                    if crate::ops::is_truthy(&hit) {
                        return Ok(item);
                    }
                }
                Ok(CrowValue::error("no match", "find"))
            }
            "cpu" | "gpu" | "nodes" => Ok(CrowValue::done()),
            _ => {
                if self.registry.has_function(name) {
                    return self.registry.call(name, &vals).map(|v| Self::stamp(name, v));
                }
                Err(format!("the IR interpreter does not run '{}' yet", name))
            }
        }
    }

    fn module_call(&mut self, module: &str, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
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

    fn effect(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        if let Some(r) = crate::builtins::call(name, &vals, &self.trace_entry()) {
            return r.map(|v| Self::stamp(name, v));
        }
        if let Some(m) = self.registry.module_name_of(name).map(|s| s.to_string()) {
            return self.registry.call_in(&m, name, &vals).map(|v| Self::stamp(name, v));
        }
        Err(format!("the IR interpreter does not run the effect '{}' yet", name))
    }

    fn call_value(&mut self, f: &CrowValue, args: Vec<CrowValue>) -> Result<CrowValue, String> {
        match &f.value {
            Value::Fn(fv) => self.call_user(&fv.name.clone(), args),
            other => Err(format!("expected a function, got {}", other.type_name())),
        }
    }

    fn call_user(&mut self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, String> {
        let f = *self.fns.get(name).ok_or_else(|| format!("unknown function '{}'", name))?;
        if f.params.len() != vals.len() {
            return Err(format!("{}[] expects {} arguments, got {}", name, f.params.len(), vals.len()));
        }
        let mut scope = HashMap::new();
        for ((p, _), v) in f.params.iter().zip(vals) {
            scope.insert(p.clone(), v);
        }
        let saved_line = self.line;
        self.frames.push(Frame { scopes: vec![scope], fn_name: name.to_string() });
        let r = self.exec_stmts(&f.body);
        self.frames.pop();
        self.line = saved_line;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(CrowValue::done()),
        }
    }

    fn derived(&mut self, name: &str) -> Result<CrowValue, String> {
        let d = *self.derives.get(name).ok_or_else(|| format!("unknown derivation '{}'", name))?;
        self.frames.push(Frame { scopes: vec![HashMap::new()], fn_name: name.to_string() });
        let r = self.exec_stmts(&d.body);
        self.frames.pop();
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(CrowValue::done()),
        }
    }
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
