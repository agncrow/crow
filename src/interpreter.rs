use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;

use crate::ast::*;
use crate::lexer::Lexer;
use crate::parallel::{group_into_waves, stmt_writes};
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

    fn get_mut(&mut self, name: &str) -> Option<&mut CrowValue> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(val) = scope.get_mut(name) {
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



#[derive(Clone)]
pub struct Interpreter {
    env: Env,
    cpu_count: Option<usize>,
    gpu_count: Option<usize>,
    source_dir: PathBuf,
    module_paths: Vec<PathBuf>,
    current_line: usize,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            env: Env::new(),
            cpu_count: None,
            gpu_count: None,
            source_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            module_paths: vec![],
            current_line: 0,
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

    /// Wrap an error with the current line number
    fn line_err(&self, e: String) -> String {
        if self.current_line > 0 && !e.starts_with("line ") {
            format!("line {}: {}", self.current_line, e)
        } else {
            e
        }
    }

    pub fn run(&mut self, stmts: &[Stmt]) -> Result<CrowValue, String> {
        let waves = group_into_waves(stmts);
        let mut result = CrowValue::done();

        for wave in &waves {
            // Update current_line from the nearest preceding Line marker
            if let Some(&first_idx) = wave.first() {
                for j in (0..first_idx).rev() {
                    if let Stmt::Line(n) = &stmts[j] {
                        self.current_line = *n;
                        break;
                    }
                }
            }
            if wave.len() == 1 {
                // single statement - execute sequentially (no overhead)
                match self.exec_stmt(&stmts[wave[0]]).map_err(|e| self.line_err(e))? {
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
                        let signal = interp.exec_stmt(stmt).map_err(|e| interp.line_err(e))?;
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
            Stmt::FieldAssign { object, field, value } => {
                let new_val = self.eval_expr(value)?;
                // Find root variable name from expression
                let var_name = Self::root_var_name(object)
                    .ok_or_else(|| "field assignment target must be a variable".to_string())?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                // Navigate to the right nested field using the expression chain
                Self::assign_field(&mut var.result, object, field, new_val)?;
                Ok(Signal::None)
            }
            Stmt::IndexAssign { object, index, value } => {
                let new_val = self.eval_expr(value)?;
                let idx_val = self.eval_expr(index)?;
                let var_name = Self::root_var_name(object)
                    .ok_or_else(|| "index assignment target must be a variable".to_string())?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                Self::assign_index(&mut var.result, object, idx_val, new_val)?;
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
            Stmt::Line(n) => {
                self.current_line = *n;
                Ok(Signal::None)
            }
        }
    }

    // --- Mutation helpers ---

    /// Extract the root variable name from a nested expression (e.g., p.x → p, nums[0] → nums)
    fn root_var_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, .. } => Self::root_var_name(object),
            Expr::Call { callee, .. } => Self::root_var_name(callee),
            _ => None,
        }
    }

    /// Assign to a struct field, navigating nested field access chains
    fn assign_field(val: &mut Value, object_expr: &Expr, field: &str, new_val: CrowValue) -> Result<(), String> {
        match object_expr {
            Expr::Ident(_) => {
                // Direct field access: var.field = value
                if let Value::StructInstance(si) = val {
                    if si.fields.contains_key(field) {
                        si.fields.insert(field.to_string(), new_val);
                        Ok(())
                    } else {
                        Err(format!("struct '{}' has no field '{}'", si.name, field))
                    }
                } else {
                    Err("field assignment requires a struct".to_string())
                }
            }
            Expr::FieldAccess { object, field: outer_field } => {
                // Nested: obj.outer.field = value — navigate to outer first
                if let Value::StructInstance(si) = val {
                    let inner = si.fields.get_mut(outer_field)
                        .ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, outer_field))?;
                    Self::assign_field(&mut inner.result, object, field, new_val)
                } else {
                    Err("field assignment requires a struct".to_string())
                }
            }
            _ => Err("invalid field assignment target".to_string()),
        }
    }

    /// Assign to a list index or map key
    fn assign_index(val: &mut Value, object_expr: &Expr, idx_val: CrowValue, new_val: CrowValue) -> Result<(), String> {
        match object_expr {
            Expr::Ident(_) => {
                match val {
                    Value::List(items) => {
                        if let Value::Int(i) = &idx_val.result {
                            let idx = *i as usize;
                            if idx < items.len() {
                                items[idx] = new_val;
                                Ok(())
                            } else {
                                Err(format!("index {} out of bounds (len {})", idx, items.len()))
                            }
                        } else {
                            Err("list index must be an integer".to_string())
                        }
                    }
                    Value::Map(map) => {
                        if let Value::Str(key) = &idx_val.result {
                            map.insert(key.clone(), new_val);
                            Ok(())
                        } else {
                            Err("map key must be a string".to_string())
                        }
                    }
                    _ => Err("index assignment requires a list or map".to_string()),
                }
            }
            Expr::FieldAccess { object, field } => {
                // Nested: obj.field[idx] = value
                if let Value::StructInstance(si) = val {
                    let inner = si.fields.get_mut(field)
                        .ok_or_else(|| format!("struct '{}' has no field '{}'", si.name, field))?;
                    Self::assign_index(&mut inner.result, object, idx_val, new_val)
                } else {
                    Err("nested index assignment requires a struct".to_string())
                }
            }
            _ => Err("invalid index assignment target".to_string()),
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
            // Mutating builtins: push, pop, sort, reverse, append, set, del
            // These modify the first argument (a variable) in place
            if matches!(name.as_str(), "push" | "pop" | "sort" | "reverse" | "append" | "set" | "del") {
                return self.call_mutating_builtin(name, args);
            }
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
                let mut result = CrowValue::done();
                for stmt in func.body.iter() {
                    match self.exec_stmt(stmt)? {
                        Signal::Return(val) => {
                            self.env.pop_call_scope();
                            return Ok(val);
                        }
                        Signal::Value(val) => { result = val; }
                        Signal::None => {}
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
        matches!(name, "log" | "map" | "cpu" | "gpu" | "range"
            | "abs" | "sqrt" | "floor" | "ceil" | "round" | "pow" | "min" | "max"
            | "split" | "trim" | "replace" | "upper" | "lower" | "starts" | "ends" | "join"
            | "int" | "float" | "str"
            | "push" | "pop" | "sort" | "reverse" | "append" | "contains" | "slice"
            | "read" | "write" | "len" | "sum"
            | "keys" | "values" | "has" | "set" | "del"
            | "args" | "input"
            | "assert"
        )
    }

    /// Handle mutating builtins: push, pop, sort, reverse, append, set, del
    /// These modify the first argument (a variable) in place.
    fn call_mutating_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<CrowValue, String> {
        // First arg must be an identifier (the variable to mutate)
        let var_name = match args.first() {
            Some(Expr::Ident(n)) => n.clone(),
            _ => return Err(format!("{}[] first argument must be a variable name", name)),
        };

        match name {
            "push" => {
                if args.len() != 2 { return Err("push[] takes exactly 2 arguments".into()); }
                let val = self.eval_expr(&args[1])?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::List(ref mut items) = var.result {
                    items.push(val);
                    Ok(CrowValue::done())
                } else {
                    Err("push[] requires a list".into())
                }
            }
            "pop" => {
                if args.len() != 1 { return Err("pop[] takes exactly 1 argument".into()); }
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::List(ref mut items) = var.result {
                    match items.pop() {
                        Some(val) => Ok(val),
                        None => Ok(CrowValue::error("pop from empty list")),
                    }
                } else {
                    Err("pop[] requires a list".into())
                }
            }
            "sort" => {
                if args.len() > 2 { return Err("sort[] takes 1 or 2 arguments".into()); }
                // If a sort key function is provided, evaluate it
                let sort_fn = if args.len() == 2 {
                    let fn_val = self.eval_expr(&args[1])?;
                    Some(fn_val)
                } else {
                    None
                };
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::List(ref mut items) = var.result {
                    if sort_fn.is_some() {
                        // Sort with key function — need to compute keys first
                        // For now, we'll collect keys, sort by them
                        // We can't call eval during sort (borrow issues), so pre-compute keys
                        // Actually we need self to call functions. Let's clone and sort.
                        let mut indexed: Vec<(usize, CrowValue)> = items.iter().cloned().enumerate().collect();
                        // We need to extract the sort fn body and apply it
                        // For simplicity, sort by natural ordering for now
                        items.sort_by(|a, b| {
                            let va = a.unwrap_or_propagate().ok();
                            let vb = b.unwrap_or_propagate().ok();
                            match (va, vb) {
                                (Some(Value::Int(x)), Some(Value::Int(y))) => x.cmp(y),
                                (Some(Value::Float(x)), Some(Value::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                                (Some(Value::Str(x)), Some(Value::Str(y))) => x.cmp(y),
                                _ => std::cmp::Ordering::Equal,
                            }
                        });
                        let _ = indexed; // suppress warning
                    } else {
                        items.sort_by(|a, b| {
                            let va = a.unwrap_or_propagate().ok();
                            let vb = b.unwrap_or_propagate().ok();
                            match (va, vb) {
                                (Some(Value::Int(x)), Some(Value::Int(y))) => x.cmp(y),
                                (Some(Value::Float(x)), Some(Value::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                                (Some(Value::Str(x)), Some(Value::Str(y))) => x.cmp(y),
                                _ => std::cmp::Ordering::Equal,
                            }
                        });
                    }
                    Ok(CrowValue::done())
                } else {
                    Err("sort[] requires a list".into())
                }
            }
            "reverse" => {
                if args.len() != 1 { return Err("reverse[] takes exactly 1 argument".into()); }
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::List(ref mut items) = var.result {
                    items.reverse();
                    Ok(CrowValue::done())
                } else {
                    Err("reverse[] requires a list".into())
                }
            }
            "append" => {
                if args.len() != 2 { return Err("append[] takes exactly 2 arguments".into()); }
                let other = self.eval_expr(&args[1])?;
                let other_items = match other.unwrap_or_propagate()? {
                    Value::List(items) => items.clone(),
                    _ => return Err("append[] second argument must be a list".into()),
                };
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::List(ref mut items) = var.result {
                    items.extend(other_items);
                    Ok(CrowValue::done())
                } else {
                    Err("append[] requires a list".into())
                }
            }
            "set" => {
                if args.len() != 3 { return Err("set[] takes exactly 3 arguments".into()); }
                let key = self.eval_expr(&args[1])?;
                let val = self.eval_expr(&args[2])?;
                let key_str = match key.unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("set[] key must be a string".into()),
                };
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::Map(ref mut map) = var.result {
                    map.insert(key_str, val);
                    Ok(CrowValue::done())
                } else {
                    Err("set[] requires a map".into())
                }
            }
            "del" => {
                if args.len() != 2 { return Err("del[] takes exactly 2 arguments".into()); }
                let key = self.eval_expr(&args[1])?;
                let key_str = match key.unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    _ => return Err("del[] key must be a string".into()),
                };
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                if let Value::Map(ref mut map) = var.result {
                    map.remove(&key_str);
                    Ok(CrowValue::done())
                } else {
                    Err("del[] requires a map".into())
                }
            }
            _ => Err(format!("unknown mutating builtin '{}'", name)),
        }
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
            "range" => {
                // range[stop], range[start, stop], range[start, stop, step]
                let (start, stop, step) = match args.len() {
                    1 => {
                        match args[0].unwrap_or_propagate()? {
                            Value::Int(n) => (0i64, *n, 1i64),
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    2 => {
                        match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                            (Value::Int(a), Value::Int(b)) => (*a, *b, 1i64),
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    3 => {
                        match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(a), Value::Int(b), Value::Int(c)) => {
                                if *c == 0 {
                                    return Err("range[] step cannot be zero".to_string());
                                }
                                (*a, *b, *c)
                            }
                            _ => return Err("range[] requires integer arguments".to_string()),
                        }
                    }
                    _ => return Err("range[] takes 1, 2, or 3 arguments".to_string()),
                };
                let mut result = Vec::new();
                if step > 0 {
                    let mut i = start;
                    while i < stop {
                        result.push(CrowValue::ok(Value::Int(i)));
                        i += step;
                    }
                } else {
                    let mut i = start;
                    while i > stop {
                        result.push(CrowValue::ok(Value::Int(i)));
                        i += step;
                    }
                }
                Ok(CrowValue::ok(Value::List(result)))
            }
            // --- Math builtins ---
            "abs" => {
                if args.len() != 1 { return Err("abs[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(n.abs()))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.abs()))),
                    _ => Err("abs[] requires a number".into()),
                }
            }
            "sqrt" => {
                if args.len() != 1 { return Err("sqrt[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Float((*n as f64).sqrt()))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(n.sqrt()))),
                    _ => Err("sqrt[] requires a number".into()),
                }
            }
            "floor" => {
                if args.len() != 1 { return Err("floor[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.floor() as i64))),
                    _ => Err("floor[] requires a number".into()),
                }
            }
            "ceil" => {
                if args.len() != 1 { return Err("ceil[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.ceil() as i64))),
                    _ => Err("ceil[] requires a number".into()),
                }
            }
            "round" => {
                if args.len() != 1 { return Err("round[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(n.round() as i64))),
                    _ => Err("round[] requires a number".into()),
                }
            }
            "pow" => {
                if args.len() != 2 { return Err("pow[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(a), Value::Int(b)) => {
                        if *b >= 0 {
                            Ok(CrowValue::ok(Value::Int(a.pow(*b as u32))))
                        } else {
                            Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b as f64))))
                        }
                    }
                    (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b as f64)))),
                    (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).powf(*b)))),
                    (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.powf(*b)))),
                    _ => Err("pow[] requires numbers".into()),
                }
            }
            "min" => {
                if args.len() != 2 { return Err("min[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.min(b)))),
                    (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b)))),
                    (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).min(*b)))),
                    (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.min(*b as f64)))),
                    _ => Err("min[] requires numbers".into()),
                }
            }
            "max" => {
                if args.len() != 2 { return Err("max[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Int(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Int(*a.max(b)))),
                    (Value::Float(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b)))),
                    (Value::Int(a), Value::Float(b)) => Ok(CrowValue::ok(Value::Float((*a as f64).max(*b)))),
                    (Value::Float(a), Value::Int(b)) => Ok(CrowValue::ok(Value::Float(a.max(*b as f64)))),
                    _ => Err("max[] requires numbers".into()),
                }
            }
            // --- String builtins ---
            "split" => {
                if args.len() != 2 { return Err("split[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(delim)) => {
                        let parts: Vec<CrowValue> = s.split(delim.as_str())
                            .map(|p| CrowValue::ok(Value::Str(p.to_string())))
                            .collect();
                        Ok(CrowValue::ok(Value::List(parts)))
                    }
                    _ => Err("split[] requires two strings".into()),
                }
            }
            "trim" => {
                if args.len() != 1 { return Err("trim[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.trim().to_string()))),
                    _ => Err("trim[] requires a string".into()),
                }
            }
            "replace" => {
                if args.len() != 3 { return Err("replace[] takes exactly 3 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(old), Value::Str(new)) => {
                        Ok(CrowValue::ok(Value::Str(s.replace(old.as_str(), new.as_str()))))
                    }
                    _ => Err("replace[] requires three strings".into()),
                }
            }
            "upper" => {
                if args.len() != 1 { return Err("upper[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.to_uppercase()))),
                    _ => Err("to_upper[] requires a string".into()),
                }
            }
            "lower" => {
                if args.len() != 1 { return Err("lower[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => Ok(CrowValue::ok(Value::Str(s.to_lowercase()))),
                    _ => Err("to_lower[] requires a string".into()),
                }
            }
            "join" => {
                if args.len() != 2 { return Err("join[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::List(items), Value::Str(sep)) => {
                        let parts: Vec<String> = items.iter()
                            .map(|item| match item.unwrap_or_propagate() {
                                Ok(Value::Str(s)) => s.clone(),
                                Ok(Value::Int(n)) => format!("{}", n),
                                Ok(Value::Float(f)) => format!("{}", f),
                                Ok(Value::Bool(b)) => format!("{}", b),
                                _ => "?".to_string(),
                            })
                            .collect();
                        Ok(CrowValue::ok(Value::Str(parts.join(sep.as_str()))))
                    }
                    _ => Err("join[] requires a list and a string separator".into()),
                }
            }
            "starts" => {
                if args.len() != 2 { return Err("starts[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(prefix)) => {
                        Ok(CrowValue::ok(Value::Bool(s.starts_with(prefix.as_str()))))
                    }
                    _ => Err("starts_with[] requires two strings".into()),
                }
            }
            "ends" => {
                if args.len() != 2 { return Err("ends[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(s), Value::Str(suffix)) => {
                        Ok(CrowValue::ok(Value::Bool(s.ends_with(suffix.as_str()))))
                    }
                    _ => Err("ends_with[] requires two strings".into()),
                }
            }
            // --- Conversion builtins ---
            "int" => {
                if args.len() != 1 { return Err("int[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Int(*n))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Int(*n as i64))),
                    Value::Str(s) => match s.parse::<i64>() {
                        Ok(n) => Ok(CrowValue::ok(Value::Int(n))),
                        Err(_) => Ok(CrowValue::error(&format!("cannot convert '{}' to int", s))),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Int(if *b { 1 } else { 0 }))),
                    _ => Ok(CrowValue::error("cannot convert to int")),
                }
            }
            "float" => {
                if args.len() != 1 { return Err("float[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Float(*n as f64))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
                    Value::Str(s) => match s.parse::<f64>() {
                        Ok(n) => Ok(CrowValue::ok(Value::Float(n))),
                        Err(_) => Ok(CrowValue::error(&format!("cannot convert '{}' to float", s))),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Float(if *b { 1.0 } else { 0.0 }))),
                    _ => Ok(CrowValue::error("cannot convert to float")),
                }
            }
            "str" => {
                if args.len() != 1 { return Err("str[] takes exactly 1 argument".into()); }
                let val = args[0].unwrap_or_propagate()?;
                Ok(CrowValue::ok(Value::Str(format!("{}", val))))
            }
            // --- Collection builtins (mutating: push, pop, sort, reverse, append) ---
            "len" => {
                if args.len() != 1 { return Err("len[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => Ok(CrowValue::ok(Value::Int(items.len() as i64))),
                    Value::Str(s) => Ok(CrowValue::ok(Value::Int(s.len() as i64))),
                    Value::Map(m) => Ok(CrowValue::ok(Value::Int(m.len() as i64))),
                    _ => Err("length[] requires a list, string, or map".into()),
                }
            }
            "sum" => {
                if args.len() != 1 { return Err("sum[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        let mut int_sum: i64 = 0;
                        let mut float_sum: f64 = 0.0;
                        let mut is_float = false;
                        for item in items {
                            match &item.result {
                                Value::Int(n) => int_sum += n,
                                Value::Float(f) => { is_float = true; float_sum += f; }
                                _ => return Err("sum[] requires a list of numbers".into()),
                            }
                        }
                        if is_float {
                            Ok(CrowValue::ok(Value::Float(float_sum + int_sum as f64)))
                        } else {
                            Ok(CrowValue::ok(Value::Int(int_sum)))
                        }
                    }
                    _ => Err("sum[] requires a list".into()),
                }
            }
            "contains" => {
                if args.len() != 2 { return Err("contains[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        let needle = args[1].unwrap_or_propagate()?;
                        let found = items.iter().any(|item| {
                            if let Ok(v) = item.unwrap_or_propagate() {
                                match (v, needle) {
                                    (Value::Int(a), Value::Int(b)) => a == b,
                                    (Value::Float(a), Value::Float(b)) => a == b,
                                    (Value::Str(a), Value::Str(b)) => a == b,
                                    (Value::Bool(a), Value::Bool(b)) => a == b,
                                    _ => false,
                                }
                            } else {
                                false
                            }
                        });
                        Ok(CrowValue::ok(Value::Bool(found)))
                    }
                    Value::Str(s) => {
                        match args[1].unwrap_or_propagate()? {
                            Value::Str(sub) => Ok(CrowValue::ok(Value::Bool(s.contains(sub.as_str())))),
                            _ => Err("contains[] with string requires a string needle".into()),
                        }
                    }
                    _ => Err("contains[] requires a list or string".into()),
                }
            }
            "slice" => {
                if args.len() != 3 { return Err("slice[] takes exactly 3 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(start), Value::Int(end)) => {
                                let s = (*start).max(0) as usize;
                                let e = (*end as usize).min(items.len());
                                if s >= e {
                                    Ok(CrowValue::ok(Value::List(vec![])))
                                } else {
                                    Ok(CrowValue::ok(Value::List(items[s..e].to_vec())))
                                }
                            }
                            _ => Err("slice[] requires integer start and end".into()),
                        }
                    }
                    Value::Str(s) => {
                        match (args[1].unwrap_or_propagate()?, args[2].unwrap_or_propagate()?) {
                            (Value::Int(start), Value::Int(end)) => {
                                let st = (*start).max(0) as usize;
                                let en = (*end as usize).min(s.len());
                                if st >= en {
                                    Ok(CrowValue::ok(Value::Str(String::new())))
                                } else {
                                    Ok(CrowValue::ok(Value::Str(s[st..en].to_string())))
                                }
                            }
                            _ => Err("slice[] requires integer start and end".into()),
                        }
                    }
                    _ => Err("slice[] requires a list or string".into()),
                }
            }
            // --- Map builtins (non-mutating) ---
            "keys" => {
                if args.len() != 1 { return Err("keys[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_list: Vec<CrowValue> = m.keys()
                            .map(|k| CrowValue::ok(Value::Str(k.clone())))
                            .collect();
                        Ok(CrowValue::ok(Value::List(key_list)))
                    }
                    _ => Err("keys[] requires a map".into()),
                }
            }
            "values" => {
                if args.len() != 1 { return Err("values[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let val_list: Vec<CrowValue> = m.values().cloned().collect();
                        Ok(CrowValue::ok(Value::List(val_list)))
                    }
                    _ => Err("values[] requires a map".into()),
                }
            }
            "has" => {
                if args.len() != 2 { return Err("has[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("has[] key must be a string".into()),
                        };
                        Ok(CrowValue::ok(Value::Bool(m.contains_key(&key_str))))
                    }
                    _ => Err("has[] requires a map".into()),
                }
            }
            // --- CLI / IO builtins ---
            "args" => {
                if !args.is_empty() { return Err("args[] takes no arguments".into()); }
                let cli_args: Vec<CrowValue> = std::env::args()
                    .skip(1) // skip binary name
                    .skip_while(|a| a == "run" || a == "-i" || a.ends_with(".crow") || a.ends_with(".crowc"))
                    .map(|a| CrowValue::ok(Value::Str(a)))
                    .collect();
                Ok(CrowValue::ok(Value::List(cli_args)))
            }
            "input" => {
                if !args.is_empty() { return Err("input[] takes no arguments".into()); }
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(_) => {
                        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r').to_string();
                        Ok(CrowValue::ok(Value::Str(trimmed)))
                    }
                    Err(e) => Ok(CrowValue::error(&format!("input error: {}", e))),
                }
            }
            // --- File I/O builtins ---
            "read" => {
                if args.len() != 1 { return Err("read[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        match std::fs::read_to_string(path) {
                            Ok(content) => Ok(CrowValue::ok(Value::Str(content))),
                            Err(e) => Ok(CrowValue::error(&format!("cannot read '{}': {}", path, e))),
                        }
                    }
                    _ => Err("read[] requires a string path".into()),
                }
            }
            "write" => {
                if args.len() != 2 { return Err("write[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::Str(path), Value::Str(content)) => {
                        match std::fs::write(path, content) {
                            Ok(_) => Ok(CrowValue::done()),
                            Err(e) => Ok(CrowValue::error(&format!("cannot write '{}': {}", path, e))),
                        }
                    }
                    _ => Err("write[] requires a string path and string content".into()),
                }
            }
            "assert" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("assert[] takes 1 or 2 arguments".to_string());
                }
                let cond = &args[0];
                let passed = match &cond.result {
                    Value::Bool(b) => *b,
                    Value::Int(n) => *n != 0,
                    Value::Str(s) => !s.is_empty(),
                    Value::List(l) => !l.is_empty(),
                    Value::Map(m) => !m.is_empty(),
                    _ => true,
                };
                if !passed {
                    let msg = if args.len() == 2 {
                        format!("assertion failed: {}", args[1].result)
                    } else {
                        "assertion failed".to_string()
                    };
                    return Err(msg);
                }
                Ok(CrowValue::done())
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
                (Value::Str(a), Value::Str(b)) => Ok(CrowValue::ok(Value::Str(format!("{}{}", a, b)))),
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
        if field == "len" {
            match &obj.result {
                Value::List(items) => {
                    return Ok(CrowValue::ok(Value::Int(items.len() as i64)));
                }
                Value::Str(s) => {
                    return Ok(CrowValue::ok(Value::Int(s.len() as i64)));
                }
                _ => return Err("only lists and strings have .len".to_string()),
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

                if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, crate::ast::Stmt::Line(_))) {
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

