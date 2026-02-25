/// Crow -> Rust transpiler: emits clean Rust source from typed AST.

use crate::ast::*;
use crate::type_infer::{CrowType, TypeContext};

pub struct Transpiler {
    ctx: TypeContext,
    indent: usize,
    output: String,
    /// Track which variables have been declared with `let mut`
    declared: std::collections::HashSet<String>,
    /// Current cpu thread count (set by cpu[n] statements)
    cpu_count: Option<usize>,
    /// Whether the transpiled code needs rayon for parallel iteration
    pub needs_rayon: bool,
    /// Map of function name -> set of param names that should be passed by reference
    by_ref_params: std::collections::BTreeMap<String, std::collections::HashSet<String>>,
    /// By-ref params in the current function being emitted
    current_ref_params: std::collections::HashSet<String>,
}

impl Transpiler {
    pub fn new(ctx: TypeContext) -> Self {
        Transpiler {
            ctx,
            indent: 0,
            output: String::new(),
            declared: std::collections::HashSet::new(),
            cpu_count: None,
            needs_rayon: false,
            by_ref_params: std::collections::BTreeMap::new(),
            current_ref_params: std::collections::HashSet::new(),
        }
    }

    /// Check if a statement is a cpu[n] call. Returns the count if so.
    fn extract_cpu_call(stmt: &Stmt) -> Option<i64> {
        if let Stmt::ExprStmt(Expr::Call { callee, args }) = stmt {
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "cpu" && args.len() == 1 {
                    if let Expr::Int(n) = &args[0] {
                        return Some(*n);
                    }
                }
            }
        }
        // Also handle Stmt::Resource (in case parser ever emits it)
        if let Stmt::Resource { kind, count } = stmt {
            if kind == "cpu" {
                if let Expr::Int(n) = count {
                    return Some(*n);
                }
            }
        }
        None
    }

    /// Compute which function parameters should be passed by reference.
    /// A struct param is by-ref if: the struct is non-Copy AND the param is not mutated in the body.
    fn compute_by_ref_params(&mut self) {
        let fn_names: Vec<String> = self.ctx.functions.keys().cloned().collect();
        for name in &fn_names {
            let sig = self.ctx.functions.get(name).unwrap().clone();
            let mut ref_params = std::collections::HashSet::new();
            for (pname, ptype) in &sig.params {
                if let CrowType::Struct(sname) = ptype {
                    if !self.is_struct_copy(sname) && !Self::is_param_mutated(pname, &sig.body) {
                        ref_params.insert(pname.clone());
                    }
                }
            }
            if !ref_params.is_empty() {
                self.by_ref_params.insert(name.clone(), ref_params);
            }
        }
    }

    /// Check if a struct type derives Copy (all fields are primitive).
    fn is_struct_copy(&self, struct_name: &str) -> bool {
        if let Some(sig) = self.ctx.structs.get(struct_name) {
            sig.fields.iter().all(|(_, ftype)| {
                matches!(ftype, CrowType::Int | CrowType::Float | CrowType::Bool)
            })
        } else {
            false
        }
    }

    /// Check if a parameter is assigned to (mutated) anywhere in the body.
    fn is_param_mutated(param_name: &str, body: &[Stmt]) -> bool {
        for stmt in body {
            match stmt {
                Stmt::Assign { name, .. } if name == param_name => return true,
                Stmt::If { then_body, else_body, .. } => {
                    if Self::is_param_mutated(param_name, then_body) { return true; }
                    if let Some(eb) = else_body {
                        if Self::is_param_mutated(param_name, eb) { return true; }
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    if Self::is_param_mutated(param_name, body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    /// Like maybe_unwrap_optbox, but returns a reference instead of cloning.
    /// Used when passing to a by-ref parameter.
    fn maybe_unwrap_optbox_ref(&mut self, expr: &Expr) -> Option<String> {
        if let Expr::FieldAccess { object, field } = expr {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                let obj_str = self.expr_to_string(object);
                                return Some(format!("{}.{}.as_ref().unwrap()", obj_str, field));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Transpile a complete program to Rust source.
    pub fn transpile(&mut self, stmts: &[Stmt]) -> String {
        // Scan for rayon need (any cpu[n] with n > 1)
        self.needs_rayon = stmts.iter().any(|s| {
            Self::extract_cpu_call(s).map_or(false, |n| n > 1)
        });

        // Compute which params should be passed by reference
        self.compute_by_ref_params();

        // Emit header
        self.emit_line("use std::collections::BTreeMap;");
        if self.needs_rayon {
            self.emit_line("use rayon::prelude::*;");
        }
        self.emit_line("");

        // Emit struct definitions
        let struct_names: Vec<String> = self.ctx.structs.keys().cloned().collect();
        for name in &struct_names {
            self.emit_struct(name);
        }

        // Emit function definitions
        let fn_names: Vec<String> = self.ctx.functions.keys().cloned().collect();
        for name in &fn_names {
            self.emit_function(name);
        }

        // Emit main function with top-level statements
        self.emit_line("fn main() {");
        self.indent += 1;
        for stmt in stmts {
            match stmt {
                Stmt::FnDecl { .. } | Stmt::StructDecl { .. } => {} // already emitted
                _ => {
                    // Check if this is a cpu[n] call
                    if let Some(n) = Self::extract_cpu_call(stmt) {
                        self.cpu_count = Some(n as usize);
                    } else {
                        self.emit_stmt(stmt);
                    }
                }
            }
        }
        self.indent -= 1;
        self.emit_line("}");

        self.output.clone()
    }

    fn emit_line(&mut self, line: &str) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
        self.output.push_str(line);
        self.output.push('\n');
    }

    fn emit(&mut self, text: &str) {
        self.output.push_str(text);
    }

    fn indent_str(&self) -> String {
        "    ".repeat(self.indent)
    }

    fn emit_struct(&mut self, name: &str) {
        let sig = self.ctx.structs.get(name).unwrap().clone();
        // Use Copy+Clone for simple structs (all fields are primitive types)
        let all_copy = sig.fields.iter().all(|(_, ftype)| {
            matches!(ftype, CrowType::Int | CrowType::Float | CrowType::Bool)
        });
        if all_copy {
            self.emit_line("#[derive(Debug, Clone, Copy)]");
        } else {
            self.emit_line("#[derive(Debug, Clone)]");
        }
        self.emit_line(&format!("struct {} {{", name));
        self.indent += 1;
        for (fname, ftype) in &sig.fields {
            self.emit_line(&format!("{}: {},", fname, ftype.to_rust_type()));
        }
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_function(&mut self, name: &str) {
        let sig = self.ctx.functions.get(name).unwrap().clone();
        let ref_set = self.by_ref_params.get(name).cloned().unwrap_or_default();

        // Build param list (use &StructName for by-ref params)
        let params: Vec<String> = sig.params.iter()
            .map(|(pname, ptype)| {
                if ref_set.contains(pname) {
                    format!("{}: &{}", pname, ptype.to_rust_type())
                } else {
                    format!("{}: {}", pname, ptype.to_rust_type())
                }
            })
            .collect();

        let ret_str = match &sig.return_type {
            CrowType::Void => String::new(),
            t => format!(" -> {}", t.to_rust_type()),
        };

        self.emit_line(&format!("fn {}({}){} {{", name, params.join(", "), ret_str));
        self.indent += 1;

        // Save and reset declared vars for function scope
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_ref_params = self.current_ref_params.clone();
        self.declared.clear();
        self.current_ref_params = ref_set;
        // Mark params as declared and add to vars for type inference
        for (pname, ptype) in &sig.params {
            self.declared.insert(pname.clone());
            self.ctx.vars.insert(pname.clone(), ptype.clone());
        }

        for stmt in &sig.body {
            self.emit_stmt(stmt);
        }

        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.current_ref_params = saved_ref_params;
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                let expr_str = self.expr_to_string(value);
                if self.declared.contains(name) {
                    self.emit_line(&format!("{} = {};", name, expr_str));
                } else {
                    let ty = self.ctx.infer_expr(value);
                    self.emit_line(&format!("let mut {}: {} = {};", name, ty.to_rust_type(), expr_str));
                    self.declared.insert(name.clone());
                    self.ctx.vars.insert(name.clone(), ty);
                }
            }
            Stmt::MultiAssign { names, values } => {
                for (name, value) in names.iter().zip(values.iter()) {
                    let expr_str = self.expr_to_string(value);
                    if self.declared.contains(name) {
                        self.emit_line(&format!("{} = {};", name, expr_str));
                    } else {
                        let ty = self.ctx.infer_expr(value);
                        self.emit_line(&format!("let mut {}: {} = {};", name, ty.to_rust_type(), expr_str));
                        self.declared.insert(name.clone());
                        self.ctx.vars.insert(name.clone(), ty);
                    }
                }
            }
            Stmt::Return(Some(e)) => {
                let expr_str = self.expr_to_string(e);
                self.emit_line(&format!("return {};", expr_str));
            }
            Stmt::Return(None) => {
                self.emit_line("return;");
            }
            Stmt::ExprStmt(_) => {
                // Check if this is a cpu[n] call - update cpu_count instead of emitting code
                if let Some(n) = Self::extract_cpu_call(stmt) {
                    self.cpu_count = Some(n as usize);
                    return;
                }
                if let Stmt::ExprStmt(e) = stmt {
                    let expr_str = self.expr_to_string(e);
                    self.emit_line(&format!("{};", expr_str));
                }
            }
            Stmt::If { condition, then_body, else_body } => {
                let cond_str = self.expr_to_string(condition);
                self.emit_line(&format!("if {} {{", cond_str));
                self.indent += 1;
                for s in then_body { self.emit_stmt(s); }
                self.indent -= 1;
                if let Some(eb) = else_body {
                    self.emit_line("} else {");
                    self.indent += 1;
                    for s in eb { self.emit_stmt(s); }
                    self.indent -= 1;
                }
                self.emit_line("}");
            }
            Stmt::While { condition, body } => {
                let cond_str = self.expr_to_string(condition);
                self.emit_line(&format!("while {} {{", cond_str));
                self.indent += 1;
                for s in body { self.emit_stmt(s); }
                self.indent -= 1;
                self.emit_line("}");
            }
            Stmt::For { var, iter, body } => {
                let iter_str = self.expr_to_string(iter);
                let use_par = self.cpu_count.map_or(false, |n| n > 1);

                // Determine element type for closure pattern
                let iter_type = self.ctx.infer_expr(iter);
                let elem_type = match &iter_type {
                    CrowType::List(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };
                let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

                if use_par {
                    let param = if is_copy { format!("|&{}|", var) } else { format!("|{}|", var) };
                    self.emit_line(&format!("{}.par_iter().for_each({} {{", iter_str, param));
                    self.indent += 1;
                    self.declared.insert(var.clone());
                    self.ctx.vars.insert(var.clone(), elem_type);
                    for s in body { self.emit_stmt(s); }
                    self.indent -= 1;
                    self.emit_line("});");
                } else {
                    let param = if is_copy { format!("&{}", var) } else { var.clone() };
                    self.emit_line(&format!("for {} in {}.iter() {{", param, iter_str));
                    self.indent += 1;
                    self.declared.insert(var.clone());
                    self.ctx.vars.insert(var.clone(), elem_type);
                    for s in body { self.emit_stmt(s); }
                    self.indent -= 1;
                    self.emit_line("}");
                }
            }
            Stmt::Match { value, arms } => {
                let val_str = self.expr_to_string(value);
                self.emit_line(&format!("match {} {{", val_str));
                self.indent += 1;
                for arm in arms {
                    let pattern = self.pattern_to_string(&arm.pattern);
                    self.emit_line(&format!("{} => {{", pattern));
                    self.indent += 1;
                    for s in &arm.body { self.emit_stmt(s); }
                    self.indent -= 1;
                    self.emit_line("}");
                }
                self.indent -= 1;
                self.emit_line("}");
            }
            Stmt::Resource { kind, count } => {
                // Update cpu thread count at runtime
                if kind == "cpu" {
                    if let Expr::Int(n) = count {
                        self.cpu_count = Some(*n as usize);
                    }
                }
            }
            _ => {}
        }
    }

    fn pattern_to_string(&mut self, pattern: &Pattern) -> String {
        match pattern {
            Pattern::Literal(e) => self.expr_to_string(e),
            Pattern::Range(a, b) => format!("{}..={}", self.expr_to_string(a), self.expr_to_string(b)),
            Pattern::Wildcard => "_".to_string(),
        }
    }

    fn expr_to_string(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Int(n) => format!("{}_i64", n),
            Expr::Float(n) => {
                let s = format!("{}", n);
                if s.contains('.') { format!("{}_f64", s) }
                else { format!("{}.0_f64", s) }
            }
            Expr::Bool(b) => format!("{}", b),
            Expr::Str(s) => self.string_to_rust(s),
            Expr::Ident(name) => name.clone(),
            Expr::BinOp { left, op, right } => {
                // Check for OptBoxStruct comparison with sentinel: field eq 0 => field.is_none()
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    if let Some(result) = self.try_optbox_comparison(left, op, right) {
                        return result;
                    }
                }
                let l = self.expr_to_string(left);
                let r = self.expr_to_string(right);
                let lt = self.ctx.infer_expr(left);
                let rt = self.ctx.infer_expr(right);
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Gt => ">",
                    BinOp::Lt => "<",
                    BinOp::Gte => ">=",
                    BinOp::Lte => "<=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                };
                // Handle mixed int/float: cast int side to f64
                match (&lt, &rt) {
                    (CrowType::Int, CrowType::Float) => format!("(({} as f64) {} {})", l, op_str, r),
                    (CrowType::Float, CrowType::Int) => format!("({} {} ({} as f64))", l, op_str, r),
                    _ => format!("({} {} {})", l, op_str, r),
                }
            }
            Expr::UnaryOp { op, expr: e } => {
                let inner = self.expr_to_string(e);
                match op {
                    UnaryOp::Neg => format!("(-{})", inner),
                    UnaryOp::Not => format!("(!{})", inner),
                }
            }
            Expr::Call { callee, args } => {
                // Determine which params are by-ref for the callee
                let callee_name = if let Expr::Ident(n) = callee.as_ref() { Some(n.clone()) } else { None };
                let ref_flags: Vec<bool> = if let Some(ref cname) = callee_name {
                    if let Some(ref_set) = self.by_ref_params.get(cname) {
                        if let Some(sig) = self.ctx.functions.get(cname) {
                            sig.params.iter().map(|(pn, _)| ref_set.contains(pn)).collect()
                        } else { vec![false; args.len()] }
                    } else { vec![false; args.len()] }
                } else { vec![false; args.len()] };

                // Build arg strings with by-ref awareness
                let mut arg_strs: Vec<String> = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let is_by_ref = ref_flags.get(i).copied().unwrap_or(false);
                    if is_by_ref {
                        // By-ref param: pass reference, no clone
                        if let Some(unwrapped) = self.maybe_unwrap_optbox_ref(a) {
                            arg_strs.push(unwrapped);
                        } else if let Expr::Ident(vname) = a {
                            if self.current_ref_params.contains(vname) {
                                arg_strs.push(vname.clone()); // already a reference
                            } else {
                                arg_strs.push(format!("&{}", vname));
                            }
                        } else {
                            let expr_str = self.expr_to_string(a);
                            arg_strs.push(format!("&{}", expr_str));
                        }
                    } else {
                        // By-value param: existing behavior
                        if let Some(unwrapped) = self.maybe_unwrap_optbox(a) {
                            arg_strs.push(unwrapped);
                        } else {
                            arg_strs.push(self.expr_to_string(a));
                        }
                    }
                }

                if let Expr::Ident(name) = callee.as_ref() {
                    // Builtins
                    if name == "log" {
                        return self.emit_log_call(&arg_strs);
                    }

                    // Struct constructor
                    if self.ctx.structs.contains_key(name) {
                        let sig = self.ctx.structs.get(name).unwrap().clone();
                        let field_inits: Vec<String> = sig.fields.iter()
                            .zip(args.iter())
                            .zip(arg_strs.iter())
                            .map(|(((fname, ftype), orig_expr), val)| {
                                if let CrowType::OptBoxStruct(_) = ftype {
                                    // Check if the arg is a sentinel (literal 0)
                                    if matches!(orig_expr, Expr::Int(0)) {
                                        format!("{}: None", fname)
                                    } else {
                                        format!("{}: Some(Box::new({}))", fname, val)
                                    }
                                } else {
                                    format!("{}: {}", fname, val)
                                }
                            })
                            .collect();
                        return format!("{} {{ {} }}", name, field_inits.join(", "));
                    }

                    // Regular function call
                    return format!("{}({})", name, arg_strs.join(", "));
                }

                // Method-style or indexed call
                let callee_str = self.expr_to_string(callee);
                format!("{}({})", callee_str, arg_strs.join(", "))
            }
            Expr::FieldAccess { object, field } => {
                let obj = self.expr_to_string(object);
                // Check if this field is OptBoxStruct — if so, the caller
                // will handle unwrapping when it's used as a function argument.
                // Here we just emit the raw field access.
                format!("{}.{}", obj, field)
            }
            Expr::List(items) => {
                let vals: Vec<String> = items.iter()
                    .map(|i| self.expr_to_string(i))
                    .collect();
                format!("vec![{}]", vals.join(", "))
            }
            Expr::Map(pairs) => {
                if pairs.is_empty() {
                    "BTreeMap::new()".to_string()
                } else {
                    let entries: Vec<String> = pairs.iter()
                        .map(|(k, v)| format!("({}, {})", self.expr_to_string(k), self.expr_to_string(v)))
                        .collect();
                    format!("BTreeMap::from([{}])", entries.join(", "))
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.emit_for_expr(var, iter, body)
            }
        }
    }

    /// Emit a ForExpr as .iter().map(|var| { body }).collect()
    /// Uses par_iter() when cpu_count > 1.
    fn emit_for_expr(&mut self, var: &str, iter: &Expr, body: &[Stmt]) -> String {
        let iter_str = self.expr_to_string(iter);
        let use_par = self.cpu_count.map_or(false, |n| n > 1);
        let iter_method = if use_par { "par_iter" } else { "iter" };

        // Determine element type of the iterator for the closure pattern
        let iter_type = self.ctx.infer_expr(iter);
        let elem_type = match &iter_type {
            CrowType::List(inner) => *inner.clone(),
            _ => CrowType::Unknown,
        };
        let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

        // Determine the return type of each iteration (last expression)
        let last_type = if let Some(last_stmt) = body.last() {
            match last_stmt {
                Stmt::ExprStmt(e) => self.ctx.infer_expr(e),
                _ => CrowType::Unknown,
            }
        } else {
            CrowType::Unknown
        };

        // Save transpiler state for closure body
        let saved_output = std::mem::take(&mut self.output);
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_indent = self.indent;
        self.indent = saved_indent + 1;
        self.declared.clear();

        // Add loop variable to context
        self.declared.insert(var.to_string());
        self.ctx.vars.insert(var.to_string(), elem_type.clone());

        // Split body into init statements and last expression
        let has_last_expr = body.last().map_or(false, |s| matches!(s, Stmt::ExprStmt(_)));
        let (init_stmts, last_expr) = if has_last_expr && body.len() > 1 {
            (&body[..body.len() - 1], body.last())
        } else if has_last_expr {
            (&body[..0], body.last())
        } else {
            (body, None)
        };

        for stmt in init_stmts {
            self.emit_stmt(stmt);
        }

        // Emit last expression without semicolon (it's the return value)
        if let Some(Stmt::ExprStmt(e)) = last_expr {
            let expr_str = self.expr_to_string(e);
            let indent = "    ".repeat(self.indent);
            self.output.push_str(&format!("{}{}\n", indent, expr_str));
        }

        let closure_body = std::mem::take(&mut self.output);

        // Restore state
        self.output = saved_output;
        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.indent = saved_indent;

        // Build the closure parameter pattern
        let param_pattern = if is_copy {
            format!("|&{}|", var)
        } else {
            format!("|{}|", var)
        };

        let indent = "    ".repeat(self.indent);
        let collect_type = last_type.to_rust_type();

        format!(
            "{}.{}().map({} {{\n{}{}}}).collect::<Vec<{}>>()",
            iter_str, iter_method, param_pattern,
            closure_body,
            indent,
            collect_type
        )
    }

    /// Convert a Crow interpolated string to Rust format!() macro.
    fn string_to_rust(&mut self, s: &str) -> String {
        // Crow strings use {expr} for interpolation
        let mut fmt_str = String::new();
        let mut args: Vec<String> = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '{' {
                // Find matching closing brace (handle nesting)
                let start = i + 1;
                let mut depth = 1;
                i += 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' { depth += 1; }
                    else if chars[i] == '}' { depth -= 1; }
                    if depth > 0 { i += 1; }
                }
                let expr_str: String = chars[start..i].iter().collect();
                i += 1; // skip closing }

                // Parse the expression as Crow code and transpile to Rust
                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                if let Ok(tokens) = lexer.tokenize() {
                    let mut parser = crate::parser::Parser::new(tokens);
                    if let Ok(stmts) = parser.parse() {
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.first() {
                            let rust_expr = self.expr_to_string(expr);
                            // Use {:?} for collection types (Vec, Map) since they don't impl Display
                            let expr_type = self.ctx.infer_expr(expr);
                            match expr_type {
                                CrowType::List(_) | CrowType::Map(_) => fmt_str.push_str("{:?}"),
                                _ => fmt_str.push_str("{}"),
                            }
                            args.push(rust_expr);
                        } else {
                            // Fallback: use raw text
                            fmt_str.push_str("{}");
                            args.push(expr_str);
                        }
                    } else {
                        fmt_str.push_str("{}");
                        args.push(expr_str);
                    }
                } else {
                    fmt_str.push_str("{}");
                    args.push(expr_str);
                }
            } else {
                fmt_str.push(chars[i]);
                i += 1;
            }
        }

        if args.is_empty() {
            format!("\"{}\".to_string()", fmt_str)
        } else {
            format!("format!(\"{}\", {})", fmt_str, args.join(", "))
        }
    }

    /// Emit a log[] call as println!()
    fn emit_log_call(&mut self, args: &[String]) -> String {
        if args.is_empty() {
            "println!()".to_string()
        } else {
            // The arg is already a string expression from string_to_rust
            let arg = &args[0];
            if arg.starts_with("format!(") {
                // Replace format!(...) with println!(...)
                format!("println!{}", &arg[7..])
            } else {
                format!("println!(\"{{}}\", {})", arg)
            }
        }
    }

    /// Check if a binary comparison involves an OptBoxStruct field compared to a sentinel (0).
    /// e.g. `tree.left eq 0` => `tree.left.is_none()`
    fn try_optbox_comparison(&mut self, left: &Expr, op: &BinOp, right: &Expr) -> Option<String> {
        // Check left is a field access on a struct with OptBoxStruct field
        if let Expr::FieldAccess { object, field } = left {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                // Check if right side is sentinel 0
                                if matches!(right, Expr::Int(0)) {
                                    let obj_str = self.expr_to_string(object);
                                    return match op {
                                        BinOp::Eq => Some(format!("{}.{}.is_none()", obj_str, field)),
                                        BinOp::Ne => Some(format!("{}.{}.is_some()", obj_str, field)),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
        // Check right side too (0 eq tree.left)
        if let Expr::FieldAccess { object, field } = right {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                if matches!(left, Expr::Int(0)) {
                                    let obj_str = self.expr_to_string(object);
                                    return match op {
                                        BinOp::Eq => Some(format!("{}.{}.is_none()", obj_str, field)),
                                        BinOp::Ne => Some(format!("{}.{}.is_some()", obj_str, field)),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if an expression refers to an OptBoxStruct field, and if so
    /// return the unwrapped version for use as a function argument.
    fn maybe_unwrap_optbox(&mut self, expr: &Expr) -> Option<String> {
        if let Expr::FieldAccess { object, field } = expr {
            let obj_type = self.ctx.infer_expr(object);
            if let CrowType::Struct(struct_name) = obj_type {
                if let Some(sig) = self.ctx.structs.get(&struct_name) {
                    for (fname, ftype) in &sig.fields {
                        if fname == field {
                            if let CrowType::OptBoxStruct(_) = ftype {
                                let obj_str = self.expr_to_string(object);
                                return Some(format!("*{}.{}.as_ref().unwrap().clone()", obj_str, field));
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

