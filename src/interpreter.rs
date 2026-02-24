use std::collections::BTreeMap;

use crate::ast::*;
use crate::value::*;

/// Environment: a stack of scopes for variable lookup
#[derive(Debug, Clone)]
struct Env {
    scopes: Vec<BTreeMap<String, CrowValue>>,
}

impl Env {
    fn new() -> Self {
        Env {
            scopes: vec![BTreeMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
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
        for scope in self.scopes.iter_mut().rev() {
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

pub struct Interpreter {
    env: Env,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter { env: Env::new() }
    }

    pub fn get_var(&self, name: &str) -> Option<CrowValue> {
        self.env.get(name).cloned()
    }

    pub fn run(&mut self, stmts: &[Stmt]) -> Result<CrowValue, String> {
        let mut result = CrowValue::done();
        for stmt in stmts {
            match self.exec_stmt(stmt)? {
                Signal::Return(val) => return Ok(val),
                Signal::Value(val) => result = val,
                Signal::None => {}
            }
        }
        Ok(result)
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
                    body: body.clone(),
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
            Stmt::Use { .. } => {
                // modules not yet implemented
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
        let items = match iter_val.unwrap_or_propagate()? {
            Value::List(items) => items.clone(),
            _ => return Err("for loop requires a list".to_string()),
        };

        let mut results = Vec::new();
        for item in &items {
            self.env.push_scope();
            self.env.set(var, item.clone());
            let mut last_val = CrowValue::done();
            for stmt in body {
                match self.exec_stmt(stmt)? {
                    Signal::Return(_) => {
                        self.env.pop_scope();
                        return Err(
                            "cannot return from inside a for loop".to_string()
                        );
                    }
                    Signal::Value(val) => last_val = val,
                    Signal::None => {}
                }
            }
            results.push(last_val);
            self.env.pop_scope();
        }
        Ok(results)
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
        if val.status == "error" {
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
                    .filter(|v| v.status == "ok")
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
        let arg_vals: Vec<CrowValue> = args
            .iter()
            .map(|a| self.eval_expr(a))
            .collect::<Result<_, _>>()?;

        match callee_val.unwrap_or_propagate()? {
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
                self.env.push_scope();
                for (param, val) in func.params.iter().zip(arg_vals) {
                    self.env.set(param, val);
                }
                let result = CrowValue::done();
                for stmt in &func.body {
                    match self.exec_stmt(stmt)? {
                        Signal::Return(val) => {
                            self.env.pop_scope();
                            return Ok(val);
                        }
                        Signal::Value(_) | Signal::None => {}
                    }
                }
                self.env.pop_scope();
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
        matches!(name, "log" | "map")
    }

    fn call_builtin(
        &self,
        name: &str,
        args: &[CrowValue],
    ) -> Result<CrowValue, String> {
        match name {
            "log" => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    print!("{}", arg.result);
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
            _ => Err(format!("undefined function '{}'", name)),
        }
    }

    fn eval_binop(
        &self,
        left: &CrowValue,
        op: &BinOp,
        right: &CrowValue,
    ) -> Result<CrowValue, String> {
        let l = left.unwrap_or_propagate()?;
        let r = right.unwrap_or_propagate()?;

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
        let v = val.unwrap_or_propagate()?;
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

        // special fields: .status, .result, .reason
        match field {
            "status" => {
                return Ok(CrowValue::ok(Value::Str(obj.status.clone())));
            }
            "result" => {
                return Ok(CrowValue::ok(obj.result.clone()));
            }
            "reason" => {
                return Ok(CrowValue::ok(Value::Str(obj.reason.clone())));
            }
            _ => {}
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

        // struct field access
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

