use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

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
    Break,
    Continue,
}



#[derive(Clone)]
pub struct Interpreter {
    env: Env,
    cpu_count: Option<usize>,
    gpu_count: Option<usize>,
    source_dir: PathBuf,
    module_paths: Vec<PathBuf>,
    current_line: usize,
    current_function: String,
    current_file: String,
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
            current_function: String::new(),
            current_file: String::new(),
        }
    }

    pub fn set_source_file(&mut self, file: &str) {
        self.current_file = file.to_string();
    }

    /// Format a trace entry like "math.divide:5" or "main:3"
    fn trace_entry(&self) -> String {
        let file_stem = if self.current_file.is_empty() {
            "unknown"
        } else {
            self.current_file.strip_suffix(".crow").unwrap_or(&self.current_file)
        };
        if self.current_function.is_empty() {
            format!("{}:{}", file_stem, self.current_line)
        } else {
            format!("{}.{}:{}", file_stem, self.current_function, self.current_line)
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
                    Signal::Break | Signal::Continue | Signal::None => {}
                }
            } else {
                // multiple independent statements - execute in parallel
                let pairs: Vec<(usize, &Stmt)> = wave.iter().map(|&i| (i, &stmts[i])).collect();

                let run_parallel = |pairs: &[(usize, &Stmt)]| -> Result<Vec<(usize, Signal, Vec<(String, CrowValue)>)>, String> {
                    pairs.par_iter().map(|&(idx, stmt)| {
                        let mut interp = self.clone();
                        // Find the Line marker preceding this specific statement
                        for j in (0..idx).rev() {
                            if let Stmt::Line(n) = &stmts[j] {
                                interp.current_line = *n;
                                break;
                            }
                        }
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
                        Signal::Break | Signal::Continue | Signal::None => {}
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
        // set source file for trace entries
        if let Some(fname) = file_path.file_name() {
            module_interp.set_source_file(&fname.to_string_lossy());
        }
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
                // Check for type-changing reassignment
                if let Some(existing) = self.env.get(name) {
                    if !existing.value.type_compatible(&val.value)
                        && !matches!(existing.value, Value::Done)
                        && !matches!(val.value, Value::Done)
                    {
                        return Err(self.line_err(format!(
                            "cannot reassign '{}' from {} to {} — variable type cannot change",
                            name, existing.value.type_name(), val.value.type_name()
                        )));
                    }
                }
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
                    // Check for type-changing reassignment
                    if let Some(existing) = self.env.get(name) {
                        if !existing.value.type_compatible(&val.value)
                            && !matches!(existing.value, Value::Done)
                            && !matches!(val.value, Value::Done)
                        {
                            return Err(self.line_err(format!(
                                "cannot reassign '{}' from {} to {} — variable type cannot change",
                                name, existing.value.type_name(), val.value.type_name()
                            )));
                        }
                    }
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
            Stmt::Break => Ok(Signal::Break),
            Stmt::Continue => Ok(Signal::Continue),
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
                Self::assign_field(&mut var.value, object, field, new_val)?;
                Ok(Signal::None)
            }
            Stmt::IndexAssign { object, index, value } => {
                let new_val = self.eval_expr(value)?;
                let idx_val = self.eval_expr(index)?;
                let var_name = Self::root_var_name(object)
                    .ok_or_else(|| "index assignment target must be a variable".to_string())?;
                let var = self.env.get_mut(&var_name)
                    .ok_or_else(|| format!("undefined variable '{}'", var_name))?;
                Self::assign_index(&mut var.value, object, idx_val, new_val)?;
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
                    Self::assign_field(&mut inner.value, object, field, new_val)
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
                        if let Value::Int(i) = &idx_val.value {
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
                        if let Value::Str(key) = &idx_val.value {
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
                    Self::assign_index(&mut inner.value, object, idx_val, new_val)
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
                    Signal::Break => {
                        self.env.pop_scope();
                        return Ok(Signal::Break);
                    }
                    Signal::Continue => {
                        self.env.pop_scope();
                        return Ok(Signal::Continue);
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
                    Signal::Break => {
                        self.env.pop_scope();
                        return Ok(Signal::Break);
                    }
                    Signal::Continue => {
                        self.env.pop_scope();
                        return Ok(Signal::Continue);
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
        let items = match &iter_val.value {
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
                            Signal::None | Signal::Break | Signal::Continue => {}
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
            let mut should_break = false;
            for stmt in body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(val) => {
                        self.env.pop_scope();
                        return Ok(Signal::Return(val));
                    }
                    Signal::Break => { should_break = true; break; }
                    Signal::Continue => break,
                    Signal::Value(_) | Signal::None => {}
                }
            }
            self.env.pop_scope();
            if should_break { break; }
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
                        Signal::Break => {
                            self.env.pop_scope();
                            return Ok(Signal::Break);
                        }
                        Signal::Continue => {
                            self.env.pop_scope();
                            return Ok(Signal::Continue);
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
        match &val.value {
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
        match (&a.value, &b.value) {
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
                    .filter(|v| !matches!(v.value, Value::Done))
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
            let mut err = callee_val;
            err.append_trace(&self.trace_entry());
            return Ok(err);
        }

        let arg_vals: Vec<CrowValue> = args
            .iter()
            .map(|a| self.eval_expr(a))
            .collect::<Result<_, _>>()?;

        // Auto-propagation: if any argument is an error-value, return the first error
        for arg in &arg_vals {
            if arg.is_error() {
                let mut err = arg.clone();
                err.append_trace(&self.trace_entry());
                return Ok(err);
            }
        }

        match &callee_val.value {
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
                let saved_function = self.current_function.clone();
                self.current_function = func.name.clone();
                self.env.push_call_scope();
                for (param, val) in func.params.iter().zip(arg_vals) {
                    self.env.set(param, val);
                }
                let mut result = CrowValue::done();
                for stmt in func.body.iter() {
                    match self.exec_stmt(stmt)? {
                        Signal::Return(val) => {
                            self.env.pop_call_scope();
                            self.current_function = saved_function;
                            return Ok(val);
                        }
                        Signal::Value(val) => { result = val; }
                        Signal::Break | Signal::Continue | Signal::None => {}
                    }
                }
                self.env.pop_call_scope();
                self.current_function = saved_function;
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
            | "assert" | "error"
            | "rand" | "time" | "sleep" | "env" | "exit"
            | "json" | "encode"
        )
    }

    fn call_builtin(
        &mut self,
        name: &str,
        args: &[CrowValue],
    ) -> Result<CrowValue, String> {
        // Auto-propagation: if any argument is an error-value, return the first error.
        // Exception: `log` intentionally prints error-values, `type` inspects them, `error` creates them.
        if !matches!(name, "log" | "type" | "error") {
            for arg in args {
                if arg.is_error() {
                    let mut err = arg.clone();
                    err.append_trace(&self.trace_entry());
                    return Ok(err);
                }
            }
        }
        match name {
            "log" => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    if arg.is_error() {
                        print!("error: {}", arg.reason);
                    } else {
                        print!("{}", arg.value);
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
                        Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to int", s), "parse", &self.trace_entry())),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Int(if *b { 1 } else { 0 }))),
                    _ => Ok(CrowValue::error_with_kind("cannot convert to int", "parse", &self.trace_entry())),
                }
            }
            "float" => {
                if args.len() != 1 { return Err("float[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(n) => Ok(CrowValue::ok(Value::Float(*n as f64))),
                    Value::Float(n) => Ok(CrowValue::ok(Value::Float(*n))),
                    Value::Str(s) => match s.parse::<f64>() {
                        Ok(n) => Ok(CrowValue::ok(Value::Float(n))),
                        Err(_) => Ok(CrowValue::error_with_kind(&format!("cannot convert '{}' to float", s), "parse", &self.trace_entry())),
                    },
                    Value::Bool(b) => Ok(CrowValue::ok(Value::Float(if *b { 1.0 } else { 0.0 }))),
                    _ => Ok(CrowValue::error_with_kind("cannot convert to float", "parse", &self.trace_entry())),
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
                            match &item.value {
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
            // --- Immutable collection builtins (return new values) ---
            "push" => {
                if args.len() != 2 { return Err("push[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        let mut new_items = items.clone();
                        new_items.push(args[1].clone());
                        Ok(CrowValue::ok(Value::List(new_items)))
                    }
                    _ => Err("push[] requires a list".into()),
                }
            }
            "pop" => {
                if args.len() != 1 { return Err("pop[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        if items.is_empty() {
                            return Ok(CrowValue::error("pop from empty list", &self.trace_entry()));
                        }
                        let new_items = items[..items.len() - 1].to_vec();
                        Ok(CrowValue::ok(Value::List(new_items)))
                    }
                    _ => Err("pop[] requires a list".into()),
                }
            }
            "sort" => {
                if args.len() < 1 || args.len() > 2 { return Err("sort[] takes 1 or 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        let mut new_items = items.clone();
                        new_items.sort_by(|a, b| {
                            let va = a.unwrap_or_propagate().ok();
                            let vb = b.unwrap_or_propagate().ok();
                            match (va, vb) {
                                (Some(Value::Int(x)), Some(Value::Int(y))) => x.cmp(y),
                                (Some(Value::Float(x)), Some(Value::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                                (Some(Value::Str(x)), Some(Value::Str(y))) => x.cmp(y),
                                _ => std::cmp::Ordering::Equal,
                            }
                        });
                        Ok(CrowValue::ok(Value::List(new_items)))
                    }
                    _ => Err("sort[] requires a list".into()),
                }
            }
            "reverse" => {
                if args.len() != 1 { return Err("reverse[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::List(items) => {
                        let mut new_items = items.clone();
                        new_items.reverse();
                        Ok(CrowValue::ok(Value::List(new_items)))
                    }
                    _ => Err("reverse[] requires a list".into()),
                }
            }
            "append" => {
                if args.len() != 2 { return Err("append[] takes exactly 2 arguments".into()); }
                match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                    (Value::List(items), Value::List(other)) => {
                        let mut new_items = items.clone();
                        new_items.extend(other.clone());
                        Ok(CrowValue::ok(Value::List(new_items)))
                    }
                    _ => Err("append[] requires two lists".into()),
                }
            }
            "set" => {
                if args.len() != 3 { return Err("set[] takes exactly 3 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("set[] key must be a string".into()),
                        };
                        let mut new_map = m.clone();
                        new_map.insert(key_str, args[2].clone());
                        Ok(CrowValue::ok(Value::Map(new_map)))
                    }
                    _ => Err("set[] requires a map".into()),
                }
            }
            "del" => {
                if args.len() != 2 { return Err("del[] takes exactly 2 arguments".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Map(m) => {
                        let key_str = match args[1].unwrap_or_propagate()? {
                            Value::Str(s) => s.clone(),
                            _ => return Err("del[] key must be a string".into()),
                        };
                        let mut new_map = m.clone();
                        new_map.remove(&key_str);
                        Ok(CrowValue::ok(Value::Map(new_map)))
                    }
                    _ => Err("del[] requires a map".into()),
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
                    Err(e) => Ok(CrowValue::error_with_kind(&format!("input error: {}", e), "io", &self.trace_entry())),
                }
            }
            // --- System builtins ---
            "rand" => {
                if args.len() > 2 { return Err("rand[] takes 0, 1, or 2 arguments".into()); }
                if args.is_empty() {
                    // rand[] — random float between 0.0 and 1.0
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut hasher);
                    std::thread::current().id().hash(&mut hasher);
                    let hash = hasher.finish();
                    let val = (hash as f64) / (u64::MAX as f64);
                    Ok(CrowValue::ok(Value::Float(val)))
                } else if args.len() == 1 {
                    // rand[n] — random int between 0 and n-1
                    match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => {
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut hasher = DefaultHasher::new();
                            std::time::SystemTime::now().hash(&mut hasher);
                            std::thread::current().id().hash(&mut hasher);
                            let hash = hasher.finish();
                            let val = (hash % (*n as u64)) as i64;
                            Ok(CrowValue::ok(Value::Int(val)))
                        }
                        _ => Err("rand[n] requires an integer".into()),
                    }
                } else {
                    // rand[min, max] — random int between min and max (inclusive)
                    match (args[0].unwrap_or_propagate()?, args[1].unwrap_or_propagate()?) {
                        (Value::Int(min), Value::Int(max)) => {
                            use std::collections::hash_map::DefaultHasher;
                            use std::hash::{Hash, Hasher};
                            let mut hasher = DefaultHasher::new();
                            std::time::SystemTime::now().hash(&mut hasher);
                            std::thread::current().id().hash(&mut hasher);
                            let hash = hasher.finish();
                            let range = (max - min + 1) as u64;
                            let val = min + (hash % range) as i64;
                            Ok(CrowValue::ok(Value::Int(val)))
                        }
                        _ => Err("rand[min, max] requires two integers".into()),
                    }
                }
            }
            "time" => {
                if !args.is_empty() { return Err("time[] takes no arguments".into()); }
                match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                    Ok(d) => Ok(CrowValue::ok(Value::Float(d.as_secs_f64()))),
                    Err(_) => Ok(CrowValue::error("system time before unix epoch", &self.trace_entry())),
                }
            }
            "sleep" => {
                if args.len() != 1 { return Err("sleep[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Int(ms) => {
                        std::thread::sleep(std::time::Duration::from_millis(*ms as u64));
                        Ok(CrowValue::done())
                    }
                    Value::Float(s) => {
                        std::thread::sleep(std::time::Duration::from_secs_f64(*s));
                        Ok(CrowValue::done())
                    }
                    _ => Err("sleep[] requires a number (milliseconds as int, or seconds as float)".into()),
                }
            }
            "env" => {
                if args.len() != 1 { return Err("env[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(name) => {
                        match std::env::var(name) {
                            Ok(val) => Ok(CrowValue::ok(Value::Str(val))),
                            Err(_) => Ok(CrowValue::ok(Value::Str(String::new()))),
                        }
                    }
                    _ => Err("env[] requires a string".into()),
                }
            }
            "exit" => {
                if args.len() > 1 { return Err("exit[] takes 0 or 1 arguments".into()); }
                let code = if args.is_empty() {
                    0
                } else {
                    match args[0].unwrap_or_propagate()? {
                        Value::Int(n) => *n as i32,
                        _ => return Err("exit[] requires an integer".into()),
                    }
                };
                std::process::exit(code);
            }
            // --- File I/O builtins ---
            "read" => {
                if args.len() != 1 { return Err("read[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(path) => {
                        match std::fs::read_to_string(path) {
                            Ok(content) => Ok(CrowValue::ok(Value::Str(content))),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot read '{}': {}", path, e), "io", &self.trace_entry())),
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
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("cannot write '{}': {}", path, e), "io", &self.trace_entry())),
                        }
                    }
                    _ => Err("write[] requires a string path and string content".into()),
                }
            }
            // --- JSON builtins ---
            "json" => {
                if args.len() != 1 { return Err("json[] takes exactly 1 argument".into()); }
                match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => {
                        match serde_json::from_str::<serde_json::Value>(s) {
                            Ok(jval) => Ok(self.json_to_crow(&jval)),
                            Err(e) => Ok(CrowValue::error_with_kind(&format!("invalid JSON: {}", e), "parse", &self.trace_entry())),
                        }
                    }
                    _ => Err("json[] requires a string argument".into()),
                }
            }
            "encode" => {
                if args.len() != 1 { return Err("encode[] takes exactly 1 argument".into()); }
                let jval = self.crow_to_json(&args[0]);
                match jval {
                    Ok(v) => Ok(CrowValue::ok(Value::Str(v.to_string()))),
                    Err(e) => Ok(CrowValue::error(&e, &self.trace_entry())),
                }
            }
            "assert" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("assert[] takes 1 or 2 arguments".to_string());
                }
                let cond = &args[0];
                let passed = match &cond.value {
                    Value::Bool(b) => *b,
                    Value::Int(n) => *n != 0,
                    Value::Str(s) => !s.is_empty(),
                    Value::List(l) => !l.is_empty(),
                    Value::Map(m) => !m.is_empty(),
                    _ => true,
                };
                if !passed {
                    let msg = if args.len() == 2 {
                        format!("assertion failed: {}", args[1].value)
                    } else {
                        "assertion failed".to_string()
                    };
                    return Err(msg);
                }
                Ok(CrowValue::done())
            }
            "error" => {
                // error[reason] or error[reason, kind]
                if args.is_empty() || args.len() > 2 {
                    return Err("error[] takes 1 or 2 arguments".to_string());
                }
                let reason = match args[0].unwrap_or_propagate()? {
                    Value::Str(s) => s.clone(),
                    other => format!("{}", other),
                };
                let kind = if args.len() == 2 {
                    match args[1].unwrap_or_propagate()? {
                        Value::Str(s) => s.clone(),
                        other => format!("{}", other),
                    }
                } else {
                    String::new()
                };
                Ok(CrowValue::error_with_kind(&reason, &kind, &self.trace_entry()))
            }
            _ => Err(format!("undefined function '{}'", name)),
        }
    }

    /// Convert a serde_json::Value to a CrowValue.
    /// JSON null becomes an error-value with reason "null".
    fn json_to_crow(&self, jval: &serde_json::Value) -> CrowValue {
        match jval {
            serde_json::Value::Null => CrowValue::error_with_kind("null", "parse", &self.trace_entry()),
            serde_json::Value::Bool(b) => CrowValue::ok(Value::Bool(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    CrowValue::ok(Value::Int(i))
                } else if let Some(f) = n.as_f64() {
                    CrowValue::ok(Value::Float(f))
                } else {
                    CrowValue::ok(Value::Int(0))
                }
            }
            serde_json::Value::String(s) => CrowValue::ok(Value::Str(s.clone())),
            serde_json::Value::Array(arr) => {
                let items: Vec<CrowValue> = arr.iter().map(|v| self.json_to_crow(v)).collect();
                CrowValue::ok(Value::List(items))
            }
            serde_json::Value::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k.clone(), self.json_to_crow(v));
                }
                CrowValue::ok(Value::Map(map))
            }
        }
    }

    /// Convert a CrowValue to a serde_json::Value for encoding.
    fn crow_to_json(&self, val: &CrowValue) -> Result<serde_json::Value, String> {
        if val.is_error() {
            // Error-values encode as JSON null (round-trip with json[])
            return Ok(serde_json::Value::Null);
        }
        match &val.value {
            Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
            Value::Float(f) => {
                match serde_json::Number::from_f64(*f) {
                    Some(n) => Ok(serde_json::Value::Number(n)),
                    None => Err(format!("cannot encode float {} to JSON", f)),
                }
            }
            Value::Str(s) => Ok(serde_json::Value::String(s.clone())),
            Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
            Value::List(items) => {
                let arr: Result<Vec<serde_json::Value>, String> = items.iter().map(|v| self.crow_to_json(v)).collect();
                Ok(serde_json::Value::Array(arr?))
            }
            Value::Map(map) => {
                let mut obj = serde_json::Map::new();
                for (k, v) in map {
                    obj.insert(k.clone(), self.crow_to_json(v)?);
                }
                Ok(serde_json::Value::Object(obj))
            }
            Value::Done => Ok(serde_json::Value::Null),
            _ => Err(format!("cannot encode {} to JSON", val.value.type_name())),
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
        // Note: trace is appended at the call site (eval_call / call_builtin), not here

        let l = &left.value;
        let r = &right.value;

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
                        Ok(CrowValue::error_with_kind("division by zero", "math", &self.trace_entry()))
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
                        Ok(CrowValue::error_with_kind("modulo by zero", "math", &self.trace_entry()))
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

        let v = &val.value;
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

        // special fields: .ok, .reason, .kind, .trace — always work, even on errors
        match field {
            "ok" => {
                return Ok(CrowValue::ok(Value::Bool(obj.ok)));
            }
            "reason" => {
                return Ok(CrowValue::ok(Value::Str(obj.reason.clone())));
            }
            "kind" => {
                return Ok(CrowValue::ok(Value::Str(obj.kind.clone())));
            }
            "trace" => {
                let trace_list: Vec<CrowValue> = obj.trace.iter()
                    .map(|s| CrowValue::ok(Value::Str(s.clone())))
                    .collect();
                return Ok(CrowValue::ok(Value::List(trace_list)));
            }
            _ => {}
        }

        // soft error propagation for other field access
        if obj.is_error() {
            return Ok(obj);
        }

        // .length property for lists and strings
        if field == "len" {
            match &obj.value {
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
        match &obj.value {
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
                    result.push_str(&format!("{}", val.value));
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_program(src: &str) -> CrowValue {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap()
    }

    fn get_var(src: &str, var: &str) -> CrowValue {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        interp.run(&stmts).unwrap();
        interp.env.get(var).unwrap().clone()
    }

    // ===== §1: Variables & Types =====

    #[test]
    fn test_var_int() {
        let val = get_var("x = 42\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_var_negative_int() {
        let val = get_var("x = -7\n", "x");
        assert!(matches!(val.value, Value::Int(-7)));
    }

    #[test]
    fn test_var_float() {
        let val = get_var("x = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_var_string() {
        let val = get_var("x = 'hello'\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_var_bool_true() {
        let val = get_var("x = true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_var_bool_false() {
        let val = get_var("x = false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_var_list() {
        let val = get_var("x = [1, 2, 3]\n", "x");
        assert!(matches!(val.value, Value::List(ref items) if items.len() == 3));
    }

    #[test]
    fn test_var_empty_list() {
        let val = get_var("x = []\n", "x");
        assert!(matches!(val.value, Value::List(ref items) if items.is_empty()));
    }

    #[test]
    fn test_string_interpolation() {
        let val = get_var("name = 'crow'\nx = 'hello, {name}'\n", "x");
        assert_eq!(format!("{}", val.value), "hello, crow");
    }

    #[test]
    fn test_string_interpolation_expr() {
        let val = get_var("x = '1 + 2 = {1 + 2}'\n", "x");
        assert_eq!(format!("{}", val.value), "1 + 2 = 3");
    }

    #[test]
    fn test_multi_assign() {
        let val_x = get_var("x, y, z = 1, 2, 3\n", "x");
        let val_z = get_var("x, y, z = 1, 2, 3\n", "z");
        assert!(matches!(val_x.value, Value::Int(1)));
        assert!(matches!(val_z.value, Value::Int(3)));
    }

    #[test]
    fn test_swap() {
        let val_a = get_var("a = 1\nb = 2\na, b = b, a\n", "a");
        let val_b = get_var("a = 1\nb = 2\na, b = b, a\n", "b");
        assert!(matches!(val_a.value, Value::Int(2)));
        assert!(matches!(val_b.value, Value::Int(1)));
    }

    #[test]
    fn test_reassignment_same_type() {
        let val = get_var("x = 10\nx = 20\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_reassignment_int_float_compatible() {
        let val = get_var("x = 10\nx = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    // ===== §3: Functions =====

    #[test]
    fn test_fn_implicit_return() {
        let val = get_var("fn add[a, b]\n    a + b\nresult = add[3, 4]\n", "result");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_fn_explicit_return() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 0]\n", "result");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_fn_explicit_return_nonzero() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 2]\n", "result");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_fn_no_args() {
        let val = get_var("fn greet[]\n    return 'hello'\nresult = greet[]\n", "result");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_fn_first_class() {
        let val = get_var("fn double[x]\n    return x * 2\nfn apply[f, x]\n    return f[x]\nresult = apply[double, 5]\n", "result");
        assert!(matches!(val.value, Value::Int(10)));
    }

    // ===== §4: Control Flow =====

    #[test]
    fn test_if_true() {
        let val = get_var("x = 0\nif true\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_if_false() {
        let val = get_var("x = 0\nif false\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_if_else() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse\n    x = 2\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    #[test]
    fn test_if_else_if() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse if true\n    x = 2\nelse\n    x = 3\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    // ===== §5: Loops =====

    #[test]
    fn test_while_basic() {
        let val = get_var("x = 5\nwhile x gt 0\n    x = x - 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_while_break() {
        let val = get_var("x = 0\nwhile true\n    x = x + 1\n    if x eq 5\n        break\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_while_continue() {
        // Sum 1-10 skipping 5
        let val = get_var("total = 0\ni = 0\nwhile i lt 10\n    i = i + 1\n    if i eq 5\n        continue\n    total = total + i\n", "total");
        assert!(matches!(val.value, Value::Int(50))); // 55 - 5 = 50
    }

    #[test]
    fn test_for_as_expression() {
        let val = get_var("doubled = for x in [1, 2, 3]\n    x * 2\n", "doubled");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert!(matches!(items[0].value, Value::Int(2)));
            assert!(matches!(items[1].value, Value::Int(4)));
            assert!(matches!(items[2].value, Value::Int(6)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_for_as_expression_filter() {
        let val = get_var("evens = for x in [1, 2, 3, 4, 5]\n    if x % 2 eq 0\n        x\n", "evens");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 2);
            assert!(matches!(items[0].value, Value::Int(2)));
            assert!(matches!(items[1].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    // ===== §7: Structs =====

    #[test]
    fn test_struct_creation() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\n", "b");
        if let Value::StructInstance(si) = &val.value {
            assert_eq!(si.name, "Bird");
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn test_struct_field_access() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.name\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_struct_field_access_int() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.speed\n", "x");
        assert!(matches!(val.value, Value::Int(50)));
    }

    // ===== §8: Operators =====

    #[test]
    fn test_arithmetic_add() {
        let val = get_var("x = 3 + 4\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_sub() {
        let val = get_var("x = 10 - 3\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_mul() {
        let val = get_var("x = 6 * 7\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_arithmetic_div_int() {
        let val = get_var("x = 7 / 2\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_arithmetic_div_float() {
        let val = get_var("x = 7.0 / 2\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.5).abs() < 0.001));
    }

    #[test]
    fn test_arithmetic_modulo() {
        let val = get_var("x = 10 % 3\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_string_concat() {
        let val = get_var("x = 'hello' + ' world'\n", "x");
        assert_eq!(format!("{}", val.value), "hello world");
    }

    #[test]
    fn test_comparison_eq() {
        let val = get_var("x = 5 eq 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_ne() {
        let val = get_var("x = 5 ne 3\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gt() {
        let val = get_var("x = 10 gt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lt() {
        let val = get_var("x = 3 lt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gte() {
        let val = get_var("x = 5 gte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lte() {
        let val = get_var("x = 5 lte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_and() {
        let val = get_var("x = true and false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_logical_or() {
        let val = get_var("x = false or true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_not() {
        let val = get_var("x = not true\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_expression_grouping() {
        let val = get_var("x = (1 + 2) * 3\n", "x");
        assert!(matches!(val.value, Value::Int(9)));
    }

    // ===== Existing error tests =====

    #[test]
    fn test_error_line_int_parse() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_error_line_second_line() {
        let val = get_var("y = 10\nx = int['bad']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("2")));
    }

    #[test]
    fn test_error_line_read() {
        let val = get_var("x = read['nonexistent_test_file.txt']\n", "x");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_ok_value_line_zero() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.ok);
        assert!(val.trace.is_empty());
    }

    #[test]
    fn test_raw_value_line_field() {
        // raw values (non-error-value) should have empty trace
        let val = get_var("x = 5\n", "x");
        assert!(val.trace.is_empty());
    }

    #[test]
    fn test_type_change_reassign_error() {
        let mut lexer = crate::lexer::Lexer::new("x = 1\nx = 'hello'\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut interp = Interpreter::new();
        let result = interp.run(&stmts);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot reassign"));
    }

    #[test]
    fn test_same_type_reassign_ok() {
        let val = get_var("x = 1\nx = 42\n", "x");
        match val.value {
            Value::Int(n) => assert_eq!(n, 42),
            _ => panic!("expected int"),
        }
    }

    #[test]
    fn test_int_float_reassign_ok() {
        let val = get_var("x = 1\nx = 3.14\n", "x");
        match val.value {
            Value::Float(f) => assert!((f - 3.14).abs() < 0.001),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn test_auto_propagation_builtin() {
        // Error should propagate through len[]
        let val = get_var("x = int['bad']\ny = len[x]\n", "y");
        assert!(!val.ok);
        assert!(val.reason.contains("convert"));
    }

    #[test]
    fn test_auto_propagation_user_fn() {
        // Error should propagate through user functions
        let val = get_var("fn double[n]\n    return n * 2\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.ok);
        assert!(val.reason.contains("convert"));
    }

    #[test]
    fn test_auto_propagation_chain() {
        // Error propagates through a chain of calls
        let val = get_var("fn a[x]\n    return x + 1\nfn b[x]\n    return a[x]\ne = int['bad']\nr = b[e]\n", "r");
        assert!(!val.ok);
    }

    #[test]
    fn test_auto_propagation_preserves_line() {
        // The original error trace should contain the original line
        let val = get_var("fn double[n]\n    return n * 2\nz = 10\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.ok);
        assert!(val.trace.iter().any(|t| t.contains("4")));
    }

    #[test]
    fn test_binop_propagates_left_error() {
        // Error in left operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = bad + 10\n", "x");
        assert!(!val.ok, "binop with error left operand should propagate error");
        assert!(val.reason.contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_propagates_right_error() {
        // Error in right operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = 10 + bad\n", "x");
        assert!(!val.ok, "binop with error right operand should propagate error");
        assert!(val.reason.contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_success_unwraps() {
        // Successful CrowResult should auto-unwrap in arithmetic
        let val = get_var("good = int['42']\nx = good + 10\n", "x");
        assert!(val.ok, "binop with successful result should produce ok value");
        assert!(matches!(val.value, Value::Int(52)), "expected 52, got {:?}", val.value);
    }

    #[test]
    fn test_log_no_propagation() {
        // log should NOT propagate — it should print and return done
        let val = get_var("x = int['bad']\ny = log[x]\n", "y");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Done));
    }

    #[test]
    fn test_then_simple() {
        // x then f desugars to f[x]
        let val = get_var("x = '  hello  '\ny = x then trim\n", "y");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_then_chain() {
        // chaining multiple then
        let val = get_var("x = '  hello  '\ny = x then trim then upper\n", "y");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_then_with_args() {
        // x then f[a] desugars to f[x, a]
        let val = get_var("x = 'hello world'\ny = x then split[' '] then join['-']\n", "y");
        assert_eq!(format!("{}", val.value), "hello-world");
    }

    #[test]
    fn test_then_with_conversion() {
        // then works with conversion builtins
        let val = get_var("x = 42\ny = x then str\n", "y");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: range =====

    #[test]
    fn test_range_single_arg() {
        let val = get_var("x = range[5]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 5);
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[4].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_two_args() {
        let val = get_var("x = range[2, 5]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert!(matches!(items[0].value, Value::Int(2)));
            assert!(matches!(items[2].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_three_args() {
        let val = get_var("x = range[0, 10, 3]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 4); // 0, 3, 6, 9
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[3].value, Value::Int(9)));
        } else {
            panic!("expected list");
        }
    }

    // ===== Builtins: math =====

    #[test]
    fn test_abs() {
        let val = get_var("x = abs[-5]\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_sqrt() {
        let val = get_var("x = sqrt[16]\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 4.0).abs() < 0.001));
    }

    #[test]
    fn test_floor() {
        let val = get_var("x = floor[3.7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_ceil() {
        let val = get_var("x = ceil[3.2]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_round() {
        let val = get_var("x = round[3.5]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_pow() {
        let val = get_var("x = pow[2, 10]\n", "x");
        assert!(matches!(val.value, Value::Int(1024)));
    }

    #[test]
    fn test_min() {
        let val = get_var("x = min[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_max() {
        let val = get_var("x = max[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== Builtins: strings =====

    #[test]
    fn test_split() {
        let val = get_var("x = split['a,b,c', ',']\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert_eq!(format!("{}", items[0].value), "a");
            assert_eq!(format!("{}", items[2].value), "c");
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_trim() {
        let val = get_var("x = trim['  hello  ']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_replace() {
        let val = get_var("x = replace['hello world', 'world', 'crow']\n", "x");
        assert_eq!(format!("{}", val.value), "hello crow");
    }

    #[test]
    fn test_upper() {
        let val = get_var("x = upper['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_lower() {
        let val = get_var("x = lower['HELLO']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_starts() {
        let val = get_var("x = starts['hello', 'hel']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_ends() {
        let val = get_var("x = ends['hello', 'llo']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_join() {
        let val = get_var("x = join[['a', 'b', 'c'], '-']\n", "x");
        assert_eq!(format!("{}", val.value), "a-b-c");
    }

    // ===== Builtins: conversion =====

    #[test]
    fn test_int_from_string_success() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_int_from_string_fail() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.ok);
    }

    #[test]
    fn test_int_from_float() {
        let val = get_var("x = int[3.14]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_float_from_string_success() {
        let val = get_var("x = float['3.14']\n", "x");
        assert!(val.ok);
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_str_conversion() {
        let val = get_var("x = str[42]\n", "x");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: collections =====

    #[test]
    fn test_len_list() {
        let val = get_var("x = len[[1, 2, 3]]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_len_string() {
        let val = get_var("x = len['hello']\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_push() {
        let val = get_var("x = push[[1, 2], 3]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert!(matches!(items[2].value, Value::Int(3)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_pop() {
        let val = get_var("x = pop[[1, 2, 3]]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_append() {
        let val = get_var("x = append[[1, 2], [3, 4]]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 4);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_sort() {
        let val = get_var("x = sort[[3, 1, 2]]\n", "x");
        if let Value::List(items) = &val.value {
            assert!(matches!(items[0].value, Value::Int(1)));
            assert!(matches!(items[1].value, Value::Int(2)));
            assert!(matches!(items[2].value, Value::Int(3)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_reverse() {
        let val = get_var("x = reverse[[1, 2, 3]]\n", "x");
        if let Value::List(items) = &val.value {
            assert!(matches!(items[0].value, Value::Int(3)));
            assert!(matches!(items[2].value, Value::Int(1)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_contains() {
        let val = get_var("x = contains[[1, 2, 3], 2]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_contains_false() {
        let val = get_var("x = contains[[1, 2, 3], 5]\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_slice() {
        let val = get_var("x = slice[[10, 20, 30, 40, 50], 1, 3]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 2);
            assert!(matches!(items[0].value, Value::Int(20)));
            assert!(matches!(items[1].value, Value::Int(30)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_sum_int() {
        let val = get_var("x = sum[[1, 2, 3, 4]]\n", "x");
        assert!(matches!(val.value, Value::Int(10)));
    }

    #[test]
    fn test_list_index() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[1]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    // ===== Builtins: map operations =====

    #[test]
    fn test_map_creation() {
        let val = get_var("x = ['name' = 'crow', 'age' = 3]\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_map_access() {
        let val = get_var("m = ['name' = 'crow']\nx = m['name']\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_map_empty() {
        let val = get_var("x = map[]\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_keys() {
        let val = get_var("m = ['a' = 1, 'b' = 2]\nx = keys[m]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_values() {
        let val = get_var("m = ['a' = 1, 'b' = 2]\nx = values[m]\n", "x");
        if let Value::List(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_has() {
        let val = get_var("m = ['a' = 1]\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_has_false() {
        let val = get_var("m = ['a' = 1]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_set() {
        let val = get_var("m = ['a' = 1]\nm = set[m, 'b', 2]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_del() {
        let val = get_var("m = ['a' = 1, 'b' = 2]\nm = del[m, 'a']\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    // ===== Builtins: error[] =====

    #[test]
    fn test_error_builtin() {
        let val = get_var("x = error['something went wrong']\n", "x");
        assert!(!val.ok);
        assert_eq!(val.reason, "something went wrong");
    }

    #[test]
    fn test_error_builtin_with_kind() {
        let val = get_var("x = error['bad input', 'validation']\n", "x");
        assert!(!val.ok);
        assert_eq!(val.reason, "bad input");
        assert_eq!(val.kind, "validation");
    }

    // ===== §11: Error value status on normal values =====

    #[test]
    fn test_ok_on_normal_value() {
        let val = get_var("x = 5\n", "x");
        assert!(val.ok);
        assert_eq!(val.reason, "");
        assert_eq!(val.kind, "");
        assert!(val.trace.is_empty());
    }

    // ===== Scope =====

    #[test]
    fn test_scope_inner_accesses_outer() {
        let val = get_var("x = 10\nif true\n    y = x + 5\n    x = y\n", "x");
        assert!(matches!(val.value, Value::Int(15)));
    }
}
