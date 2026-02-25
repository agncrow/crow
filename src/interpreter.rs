use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;

use crate::ast::*;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::value::*;

/// Environment: a stack of scopes for variable lookup
#[derive(Debug, Clone)]
struct Env {
    scopes: Vec<HashMap<String, CrowValue>>,
    /// Saved scope stacks for function call isolation.
    /// When a function is called, non-global scopes are saved here
    /// so the function can only see globals + its own scope.
    saved_stacks: Vec<Vec<HashMap<String, CrowValue>>>,
}

impl Env {
    fn new() -> Self {
        Env {
            scopes: vec![HashMap::new()],
            saved_stacks: Vec::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Push an isolated function scope.
    /// Saves all non-global scopes and creates a fresh scope for the function.
    /// This prevents function-local variables from clobbering caller variables.
    fn push_call_scope(&mut self) {
        let saved: Vec<HashMap<String, CrowValue>> = self.scopes.drain(1..).collect();
        self.saved_stacks.push(saved);
        // Now scopes = [global] only; push a fresh function scope
        self.scopes.push(HashMap::new());
    }

    /// Pop an isolated function scope and restore the caller's scopes.
    fn pop_call_scope(&mut self) {
        // Pop the function scope
        self.scopes.pop();
        // Restore saved non-global scopes
        if let Some(saved) = self.saved_stacks.pop() {
            self.scopes.extend(saved);
        }
    }

    fn set(&mut self, name: &str, value: CrowValue) {
        // set in the current (top) scope
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn get(&self, name: &str) -> Option<&CrowValue> {
        // search from innermost to outermost scope
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(val);
            }
        }
        None
    }

    fn update(&mut self, name: &str, value: CrowValue) -> bool {
        // update in the scope where the variable exists
        // When inside a function call (saved_stacks non-empty), don't
        // cross into the global scope (index 0) — this prevents
        // function-local assignments from clobbering global variables.
        let stop_at = if self.saved_stacks.is_empty() { 0 } else { 1 };
        for scope in self.scopes[stop_at..].iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }
}

/// Signal for control flow
enum Signal {
    None,
    Return(CrowValue),
    Value(CrowValue), // expression result (for collecting in for loops)
}

/// Extract variable names read by string interpolation patterns {expr}
fn str_interpolation_reads(s: &str) -> HashSet<String> {
    let mut reads = HashSet::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let start = i + 1;
            let mut depth = 1;
            i += 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '{' { depth += 1; }
                else if chars[i] == '}' { depth -= 1; }
                if depth > 0 { i += 1; }
            }
            let expr_str: String = chars[start..i].iter().collect();
            i += 1;
            // parse the interpolation expression and extract reads
            if let Ok(tokens) = crate::lexer::Lexer::new(&expr_str).tokenize() {
                if let Ok(stmts) = crate::parser::Parser::new(tokens).parse() {
                    if let Some(Stmt::ExprStmt(expr)) = stmts.first() {
                        reads.extend(expr_reads(expr));
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    reads
}

/// Extract all variable names read by an expression
fn expr_reads(expr: &Expr) -> HashSet<String> {
    let mut reads = HashSet::new();
    match expr {
        Expr::Ident(name) => { reads.insert(name.clone()); }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) => {}
        Expr::Str(s) => { reads.extend(str_interpolation_reads(s)); }
        Expr::BinOp { left, right, .. } => {
            reads.extend(expr_reads(left));
            reads.extend(expr_reads(right));
        }
        Expr::UnaryOp { expr: e, .. } => { reads.extend(expr_reads(e)); }
        Expr::Call { callee, args } => {
            reads.extend(expr_reads(callee));
            for a in args { reads.extend(expr_reads(a)); }
        }
        Expr::FieldAccess { object, .. } => { reads.extend(expr_reads(object)); }
        Expr::List(items) => {
            for item in items { reads.extend(expr_reads(item)); }
        }
        Expr::Map(entries) => {
            for (k, v) in entries {
                reads.extend(expr_reads(k));
                reads.extend(expr_reads(v));
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            reads.extend(expr_reads(iter));
            for s in body { reads.extend(stmt_reads(s)); }
        }
    }
    reads
}

/// Extract variables read by a statement
fn stmt_reads(stmt: &Stmt) -> HashSet<String> {
    match stmt {
        Stmt::Assign { value, .. } => expr_reads(value),
        Stmt::MultiAssign { values, .. } => {
            let mut reads = HashSet::new();
            for v in values { reads.extend(expr_reads(v)); }
            reads
        }
        Stmt::ExprStmt(expr) => expr_reads(expr),
        Stmt::Return(Some(expr)) => expr_reads(expr),
        _ => HashSet::new(),
    }
}

/// Extract variables written by a statement
fn stmt_writes(stmt: &Stmt) -> HashSet<String> {
    let mut writes = HashSet::new();
    match stmt {
        Stmt::Assign { name, .. } => { writes.insert(name.clone()); }
        Stmt::MultiAssign { names, .. } => {
            for n in names { writes.insert(n.clone()); }
        }
        Stmt::FnDecl { name, .. } => { writes.insert(name.clone()); }
        Stmt::StructDecl { name, .. } => { writes.insert(name.clone()); }
        _ => {}
    }
    writes
}

/// Check if a statement is a "barrier" that must execute alone and sequentially
fn is_barrier(stmt: &Stmt) -> bool {
    matches!(stmt,
        Stmt::If { .. } | Stmt::For { .. } | Stmt::While { .. } |
        Stmt::Match { .. } | Stmt::Return(_) | Stmt::Use { .. } |
        Stmt::FnDecl { .. } | Stmt::StructDecl { .. } | Stmt::Resource { .. } |
        Stmt::ExprStmt(_)
    )
}

/// Group statements into waves of independent work.
/// Each wave contains statements that can safely run in parallel.
fn group_into_waves(stmts: &[Stmt]) -> Vec<Vec<usize>> {
    let mut waves: Vec<Vec<usize>> = vec![];
    let mut current_wave: Vec<usize> = vec![];
    let mut wave_reads: HashSet<String> = HashSet::new();
    let mut wave_writes: HashSet<String> = HashSet::new();

    for (i, stmt) in stmts.iter().enumerate() {
        if is_barrier(stmt) {
            // flush current wave, then add barrier as its own wave
            if !current_wave.is_empty() {
                waves.push(current_wave);
                current_wave = vec![];
            }
            waves.push(vec![i]);
            wave_reads = HashSet::new();
            wave_writes = HashSet::new();
            continue;
        }

        let reads = stmt_reads(stmt);
        let writes = stmt_writes(stmt);

        // Check for dependencies with current wave:
        // RAW: this reads something the wave writes
        let raw = !reads.is_disjoint(&wave_writes);
        // WAW: this writes something the wave writes
        let waw = !writes.is_disjoint(&wave_writes);
        // WAR: this writes something the wave reads
        let war = !writes.is_disjoint(&wave_reads);

        if raw || waw || war {
            // flush current wave, start new one with this statement
            if !current_wave.is_empty() {
                waves.push(current_wave);
            }
            current_wave = vec![i];
            wave_reads = reads;
            wave_writes = writes;
        } else {
            current_wave.push(i);
            wave_reads.extend(reads);
            wave_writes.extend(writes);
        }
    }

    if !current_wave.is_empty() {
        waves.push(current_wave);
    }

    waves
}

#[derive(Clone)]
pub struct Interpreter {
    env: Env,
    cpu_count: Option<usize>,
    gpu_count: Option<usize>,
    source_dir: PathBuf,
    module_paths: Vec<PathBuf>,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            env: Env::new(),
            cpu_count: None,
            gpu_count: None,
            source_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            module_paths: vec![],
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = dir;
    }

    pub fn add_module_path(&mut self, dir: PathBuf) {
        self.module_paths.push(dir);
    }

    pub fn set_cpu_count(&mut self, n: usize) {
        self.cpu_count = Some(n);
    }

    pub fn set_gpu_count(&mut self, n: usize) {
        self.gpu_count = Some(n);
    }

    pub fn get_var(&self, name: &str) -> Option<CrowValue> {
        self.env.get(name).cloned()
    }

    /// Get all variables from the top-level scope (for module exports)
    pub fn get_all_vars(&self) -> BTreeMap<String, CrowValue> {
        if let Some(scope) = self.env.scopes.first() {
            scope.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            BTreeMap::new()
        }
    }

    pub fn run(&mut self, stmts: &[Stmt]) -> Result<CrowValue, String> {
        let waves = group_into_waves(stmts);
        let mut result = CrowValue::done();

        for wave in &waves {
            if wave.len() == 1 {
                // single statement - execute sequentially (no overhead)
                match self.exec_stmt(&stmts[wave[0]])? {
                    Signal::Return(val) => return Ok(val),
                    Signal::Value(val) => result = val,
                    Signal::None => {}
                }
            } else {
                // multiple independent statements - execute in parallel
                let pairs: Vec<(usize, &Stmt)> = wave.iter().map(|&i| (i, &stmts[i])).collect();

                let run_parallel = |pairs: &[(usize, &Stmt)]| -> Result<Vec<(usize, Signal, Vec<(String, CrowValue)>)>, String> {
                    pairs.par_iter().map(|&(idx, stmt)| {
                        let mut interp = self.clone();
                        let signal = interp.exec_stmt(stmt)?;
                        // collect written variables
                        let writes = stmt_writes(stmt);
                        let mut written_vars = vec![];
                        for var_name in &writes {
                            if let Some(val) = interp.env.get(var_name) {
                                written_vars.push((var_name.clone(), val.clone()));
                            }
                        }
                        Ok((idx, signal, written_vars))
                    }).collect()
                };

                let results = if let Some(n) = self.cpu_count {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(n)
                        .build()
                        .map_err(|e| format!("failed to create thread pool: {}", e))?;
                    pool.install(|| run_parallel(&pairs))?
                } else {
                    run_parallel(&pairs)?
                };

                // merge results back in statement order
                for (_idx, signal, written_vars) in results {
                    // merge written variables back into main environment
                    for (name, val) in written_vars {
                        if !self.env.update(&name, val.clone()) {
                            self.env.set(&name, val);
                        }
                    }
                    match signal {
                        Signal::Return(val) => return Ok(val),
                        Signal::Value(val) => result = val,
                        Signal::None => {}
                    }
                }
            }
        }

        Ok(result)
    }

    fn resolve_module_file(&self, module_path: &str) -> Result<PathBuf, String> {
        // 1. try relative to source_dir: <source_dir>/<module_path>.crow
        let rel_path = self.source_dir.join(format!("{}.crow", module_path));
        if rel_path.exists() {
            return Ok(rel_path);
        }

        // 2. search module_paths (e.g. crow_modules/)
        for base in &self.module_paths {
            // try <base>/<module_path>.crow
            let p = base.join(format!("{}.crow", module_path));
            if p.exists() {
                return Ok(p);
            }
            // try <base>/<module_path>/main.crow (package with main entry)
            let p = base.join(module_path).join("main.crow");
            if p.exists() {
                return Ok(p);
            }
        }

        Err(format!("cannot find module '{}'", module_path))
    }

    fn load_module(&mut self, module_path: &str) -> Result<(), String> {
        let file_path = self.resolve_module_file(module_path)?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("cannot load module '{}': {}", module_path, e))?;

        // lex and parse
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse()?;

        // run in a fresh interpreter with the module's directory as source_dir
        let mut module_interp = Interpreter::new();
        let module_dir = file_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        module_interp.set_source_dir(module_dir);
        // pass module_paths so deps can find their own deps
        module_interp.module_paths = self.module_paths.clone();
        module_interp.run(&stmts)?;

        // extract the module name (last component of path)
        let module_name = module_path
            .rsplit('/')
            .next()
            .unwrap_or(module_path)
            .to_string();

        // collect all bindings from the module
        let bindings = module_interp.get_all_vars();

        // store as a module value
        let module_val = CrowValue::ok(Value::Module(ModuleValue {
            name: module_name.clone(),
            bindings,
        }));
        self.env.set(&module_name, module_val);

        Ok(())
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Signal, String> {
        match stmt {
            Stmt::Assign { name, value } => {
                let val = self.eval_expr(value)?;
                if !self.env.update(name, val.clone()) {
                    self.env.set(name, val);
                }
                Ok(Signal::None)
            }
            Stmt::MultiAssign { names, values } => {
                let vals: Vec<CrowValue> = values
                    .iter()
                    .map(|v| self.eval_expr(v))
                    .collect::<Result<_, _>>()?;
                for (name, val) in names.iter().zip(vals) {
                    if !self.env.update(name, val.clone()) {
                        self.env.set(name, val);
                    }
                }
                Ok(Signal::None)
            }
            Stmt::FnDecl { name, params, body } => {
                let func = CrowValue::ok(Value::Fn(FnValue {
                    name: name.clone(),
                    params: params.clone(),
                    body: Arc::new(body.clone()),
                }));
                self.env.set(name, func);
                Ok(Signal::None)
            }
            Stmt::Return(expr) => {
                let val = match expr {
                    Some(e) => self.eval_expr(e)?,
                    None => CrowValue::done(),
                };
                Ok(Signal::Return(val))
            }
            Stmt::ExprStmt(expr) => {
                let val = self.eval_expr(expr)?;
                Ok(Signal::Value(val))
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => self.exec_if(condition, then_body, else_body),
            Stmt::For { var, iter, body } => {
                self.exec_for(var, iter, body)?;
                Ok(Signal::None)
            }
            Stmt::While { condition, body } => {
                self.exec_while(condition, body)
            }
            Stmt::Match { value, arms } => self.exec_match(value, arms),
            Stmt::StructDecl { name, fields } => {
                let sd = CrowValue::ok(Value::StructDef(StructDef {
                    name: name.clone(),
                    fields: fields.clone(),
                }));
                self.env.set(name, sd);
                Ok(Signal::None)
            }
            Stmt::Use { path } => {
                self.load_module(path)?;
                Ok(Signal::None)
            }
            Stmt::Resource { .. } => {
                // resource control not yet implemented
                Ok(Signal::None)
            }
        }
    }

    // --- Control flow ---

    fn exec_if(
        &mut self,
        condition: &Expr,
        then_body: &[Stmt],
        else_body: &Option<Vec<Stmt>>,
    ) -> Result<Signal, String> {
        let cond = self.eval_expr(condition)?;
        if self.is_truthy(&cond) {
            self.env.push_scope();
            let mut last_signal = Signal::None;
            for stmt in then_body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Value(val) => last_signal = Signal::Value(val),
                    Signal::None => {}
                }
            }
            self.env.pop_scope();
            return Ok(last_signal);
        } else if let Some(else_stmts) = else_body {
            self.env.push_scope();
            let mut last_signal = Signal::None;
            for stmt in else_stmts {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Value(val) => last_signal = Signal::Value(val),
                    Signal::None => {}
                }
            }
            self.env.pop_scope();
            return Ok(last_signal);
        }
        Ok(Signal::None)
    }

    fn exec_for(
        &mut self,
        var: &str,
        iter_expr: &Expr,
        body: &[Stmt],
    ) -> Result<Vec<CrowValue>, String> {
        let iter_val = self.eval_expr(iter_expr)?;
        if iter_val.is_error() {
            return Ok(vec![iter_val]);
        }
        let items = match &iter_val.result {
            Value::List(items) => items.clone(),
            _ => return Err("for loop requires a list".to_string()),
        };

        let run_parallel = |items: &[CrowValue], interp: &Self, var: &str, body: &[Stmt]| -> Result<Vec<CrowValue>, String> {
            items
                .par_iter()
                .map(|item| {
                    let mut local = interp.clone();
                    local.env.push_scope();
                    local.env.set(var, item.clone());
                    let mut last_val = CrowValue::done();
                    for stmt in body {
                        match local.exec_stmt(stmt)? {
                            Signal::Return(_) => {
                                return Err(
                                    "cannot return from inside a for loop".to_string()
                                );
                            }
                            Signal::Value(val) => last_val = val,
                            Signal::None => {}
                        }
                    }
                    Ok(last_val)
                })
                .collect()
        };

        match self.cpu_count {
            Some(n) => {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build()
                    .map_err(|e| format!("failed to create thread pool: {}", e))?;
                pool.install(|| run_parallel(&items, self, var, body))
            }
            None => run_parallel(&items, self, var, body),
        }
    }

    fn exec_while(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
    ) -> Result<Signal, String> {
        loop {
            let cond = self.eval_expr(condition)?;
            if !self.is_truthy(&cond) {
                break;
            }
            self.env.push_scope();
            for stmt in body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Value(_) | Signal::None => {}
                }
            }
            self.env.pop_scope();
        }
        Ok(Signal::None)
    }

    fn exec_match(
        &mut self,
        value: &Expr,
        arms: &[MatchArm],
    ) -> Result<Signal, String> {
        let val = self.eval_expr(value)?;
        for arm in arms {
            if self.pattern_matches(&arm.pattern, &val)? {
                self.env.push_scope();
                for stmt in &arm.body {
                    match self.exec_stmt(stmt)? {
                        Signal::Return(v) => {
                            self.env.pop_scope();
                            return Ok(Signal::Return(v));
                        }
                        Signal::Value(_) | Signal::None => {}
                    }
                }
                self.env.pop_scope();
                return Ok(Signal::None);
            }
        }
        Ok(Signal::None)
    }

    fn pattern_matches(
        &mut self,
        pattern: &Pattern,
        value: &CrowValue,
    ) -> Result<bool, String> {
        match pattern {
            Pattern::Wildcard => Ok(true),
            Pattern::Literal(expr) => {
                let pat_val = self.eval_expr(expr)?;
                Ok(self.values_equal(&pat_val, value))
            }
            Pattern::Range(start, end) => {
                let start_val = self.eval_expr(start)?;
                let end_val = self.eval_expr(end)?;
                match (
                    start_val.unwrap_or_propagate()?,
                    value.unwrap_or_propagate()?,
                    end_val.unwrap_or_propagate()?,
                ) {
                    (Value::Int(s), Value::Int(v), Value::Int(e)) => {
                        Ok(*v >= *s && *v <= *e)
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    fn is_truthy(&self, val: &CrowValue) -> bool {
        if val.is_error() {
            return false;
        }
        match &val.result {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Done => false,
            _ => true,
        }
    }

    fn values_equal(&self, a: &CrowValue, b: &CrowValue) -> bool {
        match (&a.result, &b.result) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
            (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            _ => false,
        }
    }

    // --- Expression evaluation ---

    fn eval_expr(&mut self, expr: &Expr) -> Result<CrowValue, String> {
        match expr {
            Expr::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
            Expr::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
            Expr::Str(s) => {
                let interpolated = self.interpolate_string(s)?;
                Ok(CrowValue::ok(Value::Str(interpolated)))
            }
            Expr::Bool(b) => Ok(CrowValue::ok(Value::Bool(*b))),
            Expr::Ident(name) => {
                match self.env.get(name) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!("undefined variable '{}'", name)),
                }
            }
            Expr::BinOp { left, op, right } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binop(&l, op, &r)
            }
            Expr::UnaryOp { op, expr } => {
                let val = self.eval_expr(expr)?;
                self.eval_unaryop(op, &val)
            }
            Expr::Call { callee, args } => {
                self.eval_call(callee, args)
            }
            Expr::FieldAccess { object, field } => {
                self.eval_field_access(object, field)
            }
            Expr::List(items) => {
                let vals: Vec<CrowValue> = items
                    .iter()
                    .map(|e| self.eval_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(CrowValue::ok(Value::List(vals)))
            }
            Expr::Map(pairs) => {
                let mut map = BTreeMap::new();
                for (k, v) in pairs {
                    let key = self.eval_expr(k)?;
                    let val = self.eval_expr(v)?;
                    let key_str = match key.unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        other => format!("{}", other),
                    };
                    map.insert(key_str, val);
                }
                Ok(CrowValue::ok(Value::Map(map)))
            }
            Expr::ForExpr { var, iter, body } => {
                let results = self.exec_for(var, iter, body)?;
                // filter out 'done' results (from iterations that didn't produce a value)
                let filtered: Vec<CrowValue> = results
                    .into_iter()
                    .filter(|v| v.is_ok())
                    .collect();
                Ok(CrowValue::ok(Value::List(filtered)))
            }
        }
    }

    fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<CrowValue, String> {
        // Check for builtins first (before evaluating callee as a variable)
        if let Expr::Ident(name) = callee {
            if self.is_builtin(name) {
                let arg_vals: Vec<CrowValue> = args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<_, _>>()?;
                return self.call_builtin(name, &arg_vals);
            }
        }

        let callee_val = self.eval_expr(callee)?;

        // soft error propagation: if callee is an error, propagate it
        if callee_val.is_error() {
            return Ok(callee_val);
        }

        let arg_vals: Vec<CrowValue> = args
            .iter()
            .map(|a| self.eval_expr(a))
            .collect::<Result<_, _>>()?;

        match &callee_val.result {
            Value::Fn(func) => {
                let func = func.clone();
                if func.params.len() != arg_vals.len() {
                    return Err(format!(
                        "{}[] expects {} args, got {}",
                        func.name,
                        func.params.len(),
                        arg_vals.len()
                    ));
                }
                // Use isolated call scope so function locals don't
                // clobber caller variables with the same name.
                self.env.push_call_scope();
                for (param, val) in func.params.iter().zip(arg_vals) {
                    self.env.set(param, val);
                }
                let result = CrowValue::done();
                for stmt in func.body.iter() {
                    match self.exec_stmt(stmt)? {
                        Signal::Return(val) => {
                            self.env.pop_call_scope();
                            return Ok(val);
                        }
                        Signal::Value(_) | Signal::None => {}
                    }
                }
                self.env.pop_call_scope();
                Ok(result)
            }
            Value::StructDef(sd) => {
                let sd = sd.clone();
                if sd.fields.len() != arg_vals.len() {
                    return Err(format!(
                        "{}[] expects {} fields, got {}",
                        sd.name,
                        sd.fields.len(),
                        arg_vals.len()
                    ));
                }
                let mut fields = BTreeMap::new();
                for (name, val) in sd.fields.iter().zip(arg_vals) {
                    fields.insert(name.clone(), val);
                }
                Ok(CrowValue::ok(Value::StructInstance(StructInstance {
                    name: sd.name.clone(),
                    fields,
                })))
            }
            Value::List(items) => {
                // list[index] - indexing
                if arg_vals.len() != 1 {
                    return Err("list indexing requires exactly one argument".to_string());
                }
                let index_val = arg_vals[0].unwrap_or_propagate()?;
                match index_val {
                    Value::Int(idx) => {
                        let idx = *idx;
                        if idx < 0 || idx >= items.len() as i64 {
                            return Err(format!(
                                "index {} out of bounds for list of length {}",
                                idx,
                                items.len()
                            ));
                        }
                        Ok(items[idx as usize].clone())
                    }
                    _ => Err("list index must be an integer".to_string()),
                }
            }
            Value::Map(map) => {
                // map['key'] - access
                if arg_vals.len() != 1 {
                    return Err("map access requires exactly one argument".to_string());
                }
                let key_val = arg_vals[0].unwrap_or_propagate()?;
                let key_str = match key_val {
                    Value::Str(s) => s.clone(),
                    other => format!("{}", other),
                };
                match map.get(&key_str) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!("key '{}' not found in map", key_str)),
                }
            }
            _ => {
                // check if callee is an ident for builtins
                if let Expr::Ident(name) = callee {
                    return self.call_builtin(name, &arg_vals);
                }
                Err("not callable".to_string())
            }
        }
    }

    fn is_builtin(&self, name: &str) -> bool {
        matches!(name, "log" | "map" | "cpu" | "gpu")
    }

    fn call_builtin(
        &mut self,
        name: &str,
        args: &[CrowValue],
    ) -> Result<CrowValue, String> {
        match name {
            "log" => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    if arg.is_error() {
                        print!("error: {}", arg.reason.as_deref().unwrap_or("unknown"));
                    } else {
                        print!("{}", arg.result);
                    }
                }
                println!();
                Ok(CrowValue::done())
            }
            "map" => {
                // map[] creates an empty map
                if args.is_empty() {
                    Ok(CrowValue::ok(Value::Map(BTreeMap::new())))
                } else {
                    Err("map[] takes no arguments".to_string())
                }
            }
            "cpu" => {
                if args.len() != 1 {
                    return Err("cpu[] takes exactly 1 argument".to_string());
                }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("cpu[] requires a positive integer".to_string());
                        }
                        self.cpu_count = Some(*n as usize);
                        Ok(CrowValue::done())
                    }
                    _ => Err("cpu[] requires an integer argument".to_string()),
                }
            }
            "gpu" => {
                if args.len() != 1 {
                    return Err("gpu[] takes exactly 1 argument".to_string());
                }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("gpu[] requires a positive integer".to_string());
                        }
                        self.gpu_count = Some(*n as usize);
                        Ok(CrowValue::done())
                    }
                    _ => Err("gpu[] requires an integer argument".to_string()),
                }
            }
            _ => Err(format!("undefined function '{}'", name)),
        }
    }

    fn eval_binop(
        &self,
        left: &CrowValue,
        op: &BinOp,
        right: &CrowValue,
    ) -> Result<CrowValue, String> {
        // soft error propagation: if either operand is an error, propagate it
        if left.is_error() {
            return Ok(left.clone());
        }
        if right.is_error() {
            return Ok(right.clone());
        }

        let l = &left.result;
        let r = &right.result;

        match op {
            BinOp::Add => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a + b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a + b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 + b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a + *b as f64))),
                _ => Err("cannot add these types".to_string()),
            },
            BinOp::Sub => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a - b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a - b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 - b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a - *b as f64))),
                _ => Err("cannot subtract these types".to_string()),
            },
            BinOp::Mul => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(a * b))),
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a * b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 * b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a * *b as f64))),
                _ => Err("cannot multiply these types".to_string()),
            },
            BinOp::Div => match (l, r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Ok(CrowValue::error("division by zero"))
                    } else {
                        Ok(CrowValue::ok(Value::Int(a / b)))
                    }
                }
                (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a / b))),
                (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(*a as f64 / b))),
                (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a / *b as f64))),
                _ => Err("cannot divide these types".to_string()),
            },
            BinOp::Mod => match (l, r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        Ok(CrowValue::error("modulo by zero"))
                    } else {
                        Ok(CrowValue::ok(Value::Int(a % b)))
                    }
                }
                _ => Err("modulo only works on integers".to_string()),
            },
            BinOp::Eq => Ok(CrowValue::ok(Value::Bool(self.values_equal(left, right)))),
            BinOp::Ne => Ok(CrowValue::ok(Value::Bool(!self.values_equal(left, right)))),
            BinOp::Gt => self.compare_values(l, r, |a, b| a > b),
            BinOp::Lt => self.compare_values(l, r, |a, b| a < b),
            BinOp::Gte => self.compare_values(l, r, |a, b| a >= b),
            BinOp::Lte => self.compare_values(l, r, |a, b| a <= b),
            BinOp::And => Ok(CrowValue::ok(Value::Bool(
                self.is_truthy(left) && self.is_truthy(right),
            ))),
            BinOp::Or => Ok(CrowValue::ok(Value::Bool(
                self.is_truthy(left) || self.is_truthy(right),
            ))),
        }
    }

    fn compare_values(
        &self,
        l: &Value,
        r: &Value,
        cmp: impl Fn(f64, f64) -> bool,
    ) -> Result<CrowValue, String> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b as f64))))
            }
            (Value::Float(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b))))
            }
            (Value::Int(a), Value::Float(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a as f64, *b))))
            }
            (Value::Float(a), Value::Int(b)) => {
                Ok(CrowValue::ok(Value::Bool(cmp(*a, *b as f64))))
            }
            _ => Err("cannot compare these types".to_string()),
        }
    }

    fn eval_unaryop(
        &self,
        op: &UnaryOp,
        val: &CrowValue,
    ) -> Result<CrowValue, String> {
        // soft error propagation
        if val.is_error() {
            return Ok(val.clone());
        }

        let v = &val.result;
        match op {
            UnaryOp::Neg => match v {
                Value::Int(n) => Ok(CrowValue::ok(Value::Int(-n))),
                Value::Float(n) => Ok(CrowValue::ok(Value::Float(-n))),
                _ => Err("cannot negate this type".to_string()),
            },
            UnaryOp::Not => {
                Ok(CrowValue::ok(Value::Bool(!self.is_truthy(val))))
            }
        }
    }

    fn eval_field_access(
        &mut self,
        object: &Expr,
        field: &str,
    ) -> Result<CrowValue, String> {
        let obj = self.eval_expr(object)?;

        // special fields: .status, .result, .reason — always work, even on errors
        match field {
            "status" => {
                return Ok(CrowValue::ok(Value::Str(obj.status.to_string())));
            }
            "result" => {
                return Ok(CrowValue::ok(obj.result.clone()));
            }
            "reason" => {
                return Ok(CrowValue::ok(Value::Str(
                    obj.reason.as_deref().unwrap_or("").to_string(),
                )));
            }
            _ => {}
        }

        // soft error propagation for other field access
        if obj.is_error() {
            return Ok(obj);
        }

        // .length property for lists and strings
        if field == "length" {
            match &obj.result {
                Value::List(items) => {
                    return Ok(CrowValue::ok(Value::Int(items.len() as i64)));
                }
                Value::Str(s) => {
                    return Ok(CrowValue::ok(Value::Int(s.len() as i64)));
                }
                _ => return Err("only lists and strings have .length".to_string()),
            }
        }

        // struct and module field access
        match &obj.result {
            Value::StructInstance(si) => {
                match si.fields.get(field) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!(
                        "struct '{}' has no field '{}'",
                        si.name, field
                    )),
                }
            }
            Value::Module(m) => {
                match m.bindings.get(field) {
                    Some(val) => Ok(val.clone()),
                    None => Err(format!(
                        "module '{}' has no member '{}'",
                        m.name, field
                    )),
                }
            }
            _ => Err(format!("cannot access field '{}' on this value", field)),
        }
    }

    fn interpolate_string(&mut self, s: &str) -> Result<String, String> {
        let mut result = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '{' {
                // find matching }
                let start = i + 1;
                let mut depth = 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' {
                        depth += 1;
                    } else if chars[i] == '}' {
                        depth -= 1;
                    }
                    if depth > 0 {
                        i += 1;
                    }
                }
                let expr_str: String = chars[start..i].iter().collect();
                i += 1; // skip closing }

                // parse and evaluate the expression
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                let tokens = lexer.tokenize()?;
                let mut parser = crate::parser::Parser::new(tokens);
                let stmts = parser.parse()?;

                if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.first() {
                    let val = self.eval_expr(expr)?;
                    result.push_str(&format!("{}", val.result));
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        Ok(result)
    }
}

