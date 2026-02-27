/// Crow -> Rust transpiler: emits clean Rust source from typed AST.

use crate::ast::*;
use crate::parallel::group_into_waves;
use crate::type_infer::{CrowType, TypeContext};

pub struct Transpiler {
    ctx: TypeContext,
    indent: usize,
    output: String,
    /// Track which variables have been declared with `let`
    declared: std::collections::HashSet<String>,
    /// Variables that are reassigned after initial declaration (need `let mut`)
    mutated_vars: std::collections::HashSet<String>,
    /// Current cpu thread count (set by cpu[n] statements)
    cpu_count: Option<usize>,
    /// Whether the transpiled code needs rayon for parallel iteration
    pub needs_rayon: bool,
    /// Whether the transpiled code needs serde_json for JSON parsing/encoding
    pub needs_serde_json: bool,
    /// Map of function name -> set of param names that should be passed by reference
    by_ref_params: std::collections::BTreeMap<String, std::collections::HashSet<String>>,
    /// By-ref params in the current function being emitted
    current_ref_params: std::collections::HashSet<String>,
    /// Module names from `use` imports (for resolving module.field → module_field)
    modules: std::collections::HashSet<String>,
    /// Current source line number (from Stmt::Line markers)
    current_line: usize,
    /// Flag: when true, CrowResult args are replaced with _prop_N.result.clone()
    unwrap_result_args: bool,
    /// Indices of CrowResult args being unwrapped (used with unwrap_result_args)
    propagation_indices: Option<Vec<usize>>,
}

impl Transpiler {
    pub fn new(ctx: TypeContext) -> Self {
        Transpiler {
            ctx,
            indent: 0,
            output: String::new(),
            declared: std::collections::HashSet::new(),
            mutated_vars: std::collections::HashSet::new(),
            cpu_count: None,
            needs_rayon: false,
            needs_serde_json: false,
            by_ref_params: std::collections::BTreeMap::new(),
            current_ref_params: std::collections::HashSet::new(),
            modules: std::collections::HashSet::new(),
            current_line: 0,
            unwrap_result_args: false,
            propagation_indices: None,
        }
    }

    pub fn set_cpu_count(&mut self, n: usize) {
        self.cpu_count = Some(n);
    }

    /// Map Crow function names to valid Rust names.
    /// User-defined `main` becomes `crow_main` to avoid clashing with Rust's entry point.
    fn rust_fn_name(name: &str) -> String {
        if name == "main" {
            "crow_main".to_string()
        } else {
            name.to_string()
        }
    }

    /// Check if a list of statements contains any parallel waves (2+ independent stmts).
    fn has_parallel_waves(stmts: &[Stmt]) -> bool {
        let waves = group_into_waves(stmts);
        waves.iter().any(|w| w.len() > 1)
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

    /// Operator precedence (higher = binds tighter).
    fn op_precedence(op: &BinOp) -> u8 {
        match op {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => 3,
            BinOp::Add | BinOp::Sub => 4,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 5,
        }
    }

    /// Get the precedence of an expression (only returns Some for BinOps).
    fn expr_precedence(expr: &Expr) -> Option<u8> {
        if let Expr::BinOp { op, .. } = expr {
            Some(Self::op_precedence(op))
        } else {
            None
        }
    }

    /// Check if a CrowType contains a Map anywhere.
    fn type_uses_map(ty: &CrowType) -> bool {
        match ty {
            CrowType::Map(_) => true,
            CrowType::List(inner) | CrowType::Result(inner) => Self::type_uses_map(inner),
            _ => false,
        }
    }

    /// Check if the program uses any Map types (to decide whether to import BTreeMap).
    fn program_uses_map(&self) -> bool {
        for ty in self.ctx.vars.values() {
            if Self::type_uses_map(ty) { return true; }
        }
        for sig in self.ctx.functions.values() {
            for (_, ty) in &sig.params {
                if Self::type_uses_map(ty) { return true; }
            }
            if Self::type_uses_map(&sig.return_type) { return true; }
        }
        for sig in self.ctx.structs.values() {
            for (_, ty) in &sig.fields {
                if Self::type_uses_map(ty) { return true; }
            }
        }
        false
    }

    /// Check if a CrowType contains a Result anywhere.
    fn type_uses_result(ty: &CrowType) -> bool {
        match ty {
            CrowType::Result(_) => true,
            CrowType::List(inner) => Self::type_uses_result(inner),
            _ => false,
        }
    }

    /// Check if a CrowType contains Json anywhere.
    fn type_uses_json(ty: &CrowType) -> bool {
        match ty {
            CrowType::Json => true,
            CrowType::Result(inner) => Self::type_uses_json(inner),
            CrowType::List(inner) => Self::type_uses_json(inner),
            _ => false,
        }
    }

    /// Check if the program uses any CrowResult types.
    fn program_uses_result(&self) -> bool {
        for ty in self.ctx.vars.values() {
            if Self::type_uses_result(ty) { return true; }
        }
        for sig in self.ctx.functions.values() {
            if Self::type_uses_result(&sig.return_type) { return true; }
        }
        false
    }

    /// Collect all variable names that are reassigned (assigned more than once) in a list of statements.
    fn collect_reassigned_vars(stmts: &[Stmt], seen: &mut std::collections::HashSet<String>, mutated: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, .. } => {
                    if !seen.insert(name.clone()) {
                        mutated.insert(name.clone());
                    }
                }
                Stmt::MultiAssign { names, .. } => {
                    for name in names {
                        if !seen.insert(name.clone()) {
                            mutated.insert(name.clone());
                        }
                    }
                }
                Stmt::If { then_body, else_body, .. } => {
                    Self::collect_reassigned_vars(then_body, seen, mutated);
                    if let Some(eb) = else_body {
                        Self::collect_reassigned_vars(eb, seen, mutated);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    // Any variable assigned inside a loop body is effectively mutated
                    // because the loop may execute multiple times
                    let mut loop_seen = std::collections::HashSet::new();
                    Self::collect_loop_assigned_vars(body, &mut loop_seen);
                    for name in &loop_seen {
                        mutated.insert(name.clone());
                    }
                    Self::collect_reassigned_vars(body, seen, mutated);
                }
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        Self::collect_reassigned_vars(&arm.body, seen, mutated);
                    }
                }

                // Field assignment and index assignment mutate the root variable
                Stmt::FieldAssign { object, .. } | Stmt::IndexAssign { object, .. } => {
                    if let Some(var_name) = Self::extract_root_var(object) {
                        mutated.insert(var_name);
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract the root variable name from a nested expression (for mutation tracking)
    fn extract_root_var(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => Some(name.clone()),
            Expr::FieldAccess { object, .. } => Self::extract_root_var(object),
            Expr::Call { callee, .. } => Self::extract_root_var(callee),
            _ => None,
        }
    }

    /// Collect all variable names assigned inside a loop body (they need mut).
    fn collect_loop_assigned_vars(stmts: &[Stmt], assigned: &mut std::collections::HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign { name, .. } => { assigned.insert(name.clone()); }
                Stmt::MultiAssign { names, .. } => {
                    for name in names { assigned.insert(name.clone()); }
                }
                Stmt::If { then_body, else_body, .. } => {
                    Self::collect_loop_assigned_vars(then_body, assigned);
                    if let Some(eb) = else_body {
                        Self::collect_loop_assigned_vars(eb, assigned);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    Self::collect_loop_assigned_vars(body, assigned);
                }
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        Self::collect_loop_assigned_vars(&arm.body, assigned);
                    }
                }

                _ => {}
            }
        }
    }

    /// Check if a for loop body mutates variables from the outer scope.
    /// If it does, the loop must run sequentially (not with par_iter).
    /// "Outer scope" means any variable that was declared before the loop,
    /// excluding the loop variable itself.
    fn for_body_mutates_outer(body: &[Stmt], loop_var: &str, outer_vars: &std::collections::HashSet<String>) -> bool {
        let mut body_writes = std::collections::HashSet::new();
        Self::collect_loop_assigned_vars(body, &mut body_writes);
        // Check if any written variable is from the outer scope (and not the loop var)
        for name in &body_writes {
            if name != loop_var && outer_vars.contains(name) {
                return true;
            }
        }
        false
    }

    /// Recursively check if any for loop exists in the statements (for rayon detection).
    fn has_for_loop(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::For { .. } => return true,
                Stmt::If { then_body, else_body, .. } => {
                    if Self::has_for_loop(then_body) { return true; }
                    if let Some(eb) = else_body {
                        if Self::has_for_loop(eb) { return true; }
                    }
                }
                Stmt::While { body, .. } => {
                    if Self::has_for_loop(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    /// Transpile a complete program to Rust source.
    pub fn transpile(&mut self, stmts: &[Stmt]) -> String {
        // Register all module names upfront so function bodies can resolve module references
        for stmt in stmts {
            if let Stmt::Use { path } = stmt {
                let module_name = path.rsplit('/').next().unwrap_or(path).to_string();
                self.modules.insert(module_name);
            }
        }

        // Scan for rayon need
        let needs_par_iter = stmts.iter().any(|s| {
            Self::extract_cpu_call(s).map_or(false, |n| n > 1)
        });
        // For loops are parallel by default — any for loop means we need rayon
        let has_for = Self::has_for_loop(stmts) || self.ctx.functions.values().any(|sig| Self::has_for_loop(&sig.body));
        // Check for parallel waves (uses rayon::join, no prelude needed)
        let needs_parallel_waves = Self::has_parallel_waves(stmts) || self.ctx.functions.values().any(|sig| Self::has_parallel_waves(&sig.body));
        self.needs_rayon = needs_par_iter || needs_parallel_waves || has_for;

        // Compute which params should be passed by reference
        self.compute_by_ref_params();

        // Emit header — only import BTreeMap if actually used
        if self.program_uses_map() {
            self.emit_line("use std::collections::BTreeMap;");
        }
        if needs_par_iter || has_for {
            self.emit_line("use rayon::prelude::*;");
        }
        self.emit_line("");

        // Emit CrowResult<T> struct if any fallible builtins are used
        if self.program_uses_result() {
            self.emit_line("#[derive(Debug, Clone)]");
            self.emit_line("struct CrowResult<T: Clone> {");
            self.indent += 1;
            self.emit_line("status: String,");
            self.emit_line("result: T,");
            self.emit_line("reason: String,");
            self.emit_line("line: usize,");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("impl<T: Clone + std::fmt::Display> std::fmt::Display for CrowResult<T> {");
            self.indent += 1;
            self.emit_line("fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {");
            self.indent += 1;
            self.emit_line("if self.status == \"error\" { write!(f, \"error: {}\", self.reason) } else { write!(f, \"{}\", self.result) }");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Pre-scan: check if any variable uses Json type (sets needs_serde_json)
        for ty in self.ctx.vars.values() {
            if Self::type_uses_json(ty) {
                self.needs_serde_json = true;
                break;
            }
        }

        // Emit JSON display helper if serde_json is used
        if self.needs_serde_json {
            self.emit_line("fn crow_json_display(v: &serde_json::Value) -> String {");
            self.indent += 1;
            self.emit_line("match v {");
            self.indent += 1;
            self.emit_line("serde_json::Value::String(s) => s.clone(),");
            self.emit_line("serde_json::Value::Null => \"null\".to_string(),");
            self.emit_line("other => other.to_string(),");
            self.indent -= 1;
            self.emit_line("}");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("");
        }

        // Emit module-level variable assigns as top-level statics so functions can access them
        let mut module_vars = std::collections::HashSet::new();
        for stmt in stmts {
            if let Stmt::Assign { name, value } = stmt {
                // Check if this variable is prefixed with a known module name
                for mod_name in &self.modules {
                    if name.starts_with(&format!("{}_", mod_name)) {
                        let expr_str = self.expr_to_string(value);
                        let ty = self.ctx.infer_expr(value);
                        let rust_ty = ty.to_rust_type();
                        if rust_ty == "String" {
                            // For string literals, emit as &str const
                            let str_val = expr_str.trim_end_matches(".to_string()");
                            self.emit_line("#[allow(non_upper_case_globals)]");
                            self.emit_line(&format!("const {}: &str = {};", name, str_val));
                        } else {
                            self.emit_line("#[allow(non_upper_case_globals)]");
                            self.emit_line(&format!("const {}: {} = {};", name, rust_ty, expr_str));
                        }
                        module_vars.insert(name.clone());
                        break;
                    }
                }
            }
        }
        if !module_vars.is_empty() {
            self.emit_line("");
        }

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

        // Compute which top-level variables need `mut`
        let top_level_stmts: Vec<&Stmt> = stmts.iter()
            .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. } | Stmt::Line(_)))
            .collect();
        let top_stmts_owned: Vec<Stmt> = top_level_stmts.iter().map(|s| (*s).clone()).collect();
        let mut seen = std::collections::HashSet::new();
        let mut mutated = std::collections::HashSet::new();
        Self::collect_reassigned_vars(&top_stmts_owned, &mut seen, &mut mutated);
        self.mutated_vars = mutated;

        // Collect main-body statements (excluding already-emitted items)
        let main_stmts: Vec<Stmt> = stmts.iter()
            .filter(|s| !matches!(s, Stmt::FnDecl { .. } | Stmt::StructDecl { .. }))
            .filter(|s| {
                if let Stmt::Assign { name, .. } = s {
                    !module_vars.contains(name)
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        // Emit main function with automatic parallelism
        self.emit_line("fn main() {");
        self.indent += 1;
        self.emit_stmts_parallel(&main_stmts);
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

        let rust_name = Self::rust_fn_name(name);
        self.emit_line(&format!("fn {}({}){} {{", rust_name, params.join(", "), ret_str));
        self.indent += 1;

        // Save and reset declared vars for function scope
        let saved_declared = self.declared.clone();
        let saved_vars = self.ctx.vars.clone();
        let saved_ref_params = self.current_ref_params.clone();
        let saved_mutated = self.mutated_vars.clone();
        self.declared.clear();
        self.current_ref_params = ref_set;

        // Compute which local variables need `mut` in this function
        let mut seen = std::collections::HashSet::new();
        let mut mutated = std::collections::HashSet::new();
        // Params count as first "assignment"
        for (pname, _) in &sig.params {
            seen.insert(pname.clone());
        }
        Self::collect_reassigned_vars(&sig.body, &mut seen, &mut mutated);
        self.mutated_vars = mutated;

        // Mark params as declared and add to vars for type inference
        for (pname, ptype) in &sig.params {
            self.declared.insert(pname.clone());
            self.ctx.vars.insert(pname.clone(), ptype.clone());
        }

        self.emit_stmts_parallel(&sig.body);

        self.declared = saved_declared;
        self.ctx.vars = saved_vars;
        self.current_ref_params = saved_ref_params;
        self.mutated_vars = saved_mutated;
        self.indent -= 1;
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emit a block of statements with automatic parallelism.
    /// Groups independent assignments into waves and emits rayon::join for parallel waves.
    fn emit_stmts_parallel(&mut self, stmts: &[Stmt]) {
        let waves = group_into_waves(stmts);
        for wave in &waves {
            // Update current_line from nearest preceding Line marker for each wave
            if let Some(&first_idx) = wave.first() {
                for j in (0..first_idx).rev() {
                    if let Stmt::Line(n) = &stmts[j] {
                        self.current_line = *n;
                        break;
                    }
                }
            }
            if wave.len() <= 1 {
                // Single statement — emit normally
                for &i in wave {
                    // Update line for this specific statement
                    for j in (0..i).rev() {
                        if let Stmt::Line(n) = &stmts[j] {
                            self.current_line = *n;
                            break;
                        }
                    }
                    if let Some(n) = Self::extract_cpu_call(&stmts[i]) {
                        self.cpu_count = Some(n as usize);
                    } else {
                        self.emit_stmt(&stmts[i]);
                    }
                }
            } else {
                // Multiple independent assignments — emit in parallel
                self.emit_parallel_wave(wave, stmts);
            }
        }
    }

    /// Emit a wave of independent assignments using rayon::join.
    /// For 2 stmts: let (a, b) = rayon::join(|| expr1, || expr2);
    /// For 3+: nested rayon::join
    fn emit_parallel_wave(&mut self, wave: &[usize], stmts: &[Stmt]) {
        self.needs_rayon = true;

        // Collect (name, expr, type) for each assignment in the wave
        let mut assigns: Vec<(String, String, CrowType)> = vec![];
        for &i in wave {
            // Update current_line for this specific statement
            for j in (0..i).rev() {
                if let Stmt::Line(n) = &stmts[j] {
                    self.current_line = *n;
                    break;
                }
            }
            match &stmts[i] {
                Stmt::Assign { name, value } => {
                    let expr_str = self.expr_to_string(value);
                    let mut ty = self.ctx.infer_expr(value);
                    if format!("{:?}", ty).contains("Unknown") {
                        if let Some(resolved) = self.ctx.vars.get(name) {
                            ty = resolved.clone();
                        }
                    }
                    assigns.push((name.clone(), expr_str, ty));
                }
                Stmt::MultiAssign { names, values } => {
                    for (name, value) in names.iter().zip(values.iter()) {
                        let expr_str = self.expr_to_string(value);
                        let ty = self.ctx.infer_expr(value);
                        assigns.push((name.clone(), expr_str, ty));
                    }
                }
                _ => {
                    // Shouldn't happen (barriers are filtered), but emit normally
                    self.emit_stmt(&stmts[i]);
                }
            }
        }

        if assigns.is_empty() {
            return;
        }

        // Check which vars are new declarations vs reassignments
        let all_new = assigns.iter().all(|(name, _, _)| !self.declared.contains(name));

        // Build the nested rayon::join expression
        let join_expr = self.build_rayon_join(&assigns, 0);

        // Build the destructuring pattern
        let pattern = self.build_join_pattern(&assigns, 0);

        if all_new {
            // All new declarations: let (a, b, ...) = rayon::join(...)
            // Build type annotation
            let type_pattern = self.build_join_type_pattern(&assigns, 0);
            self.emit_line(&format!("let {}: {} = {};", pattern, type_pattern, join_expr));
            for (name, _, ty) in &assigns {
                self.declared.insert(name.clone());
                self.ctx.vars.insert(name.clone(), ty.clone());
            }
        } else {
            // Mixed or all reassignments: use temp names
            let temp_pattern = self.build_join_temp_pattern(&assigns, 0);
            let type_pattern = self.build_join_type_pattern(&assigns, 0);
            self.emit_line(&format!("let {}: {} = {};", temp_pattern, type_pattern, join_expr));
            for (idx, (name, _, ty)) in assigns.iter().enumerate() {
                let temp = format!("_par_{}", idx);
                if self.declared.contains(name) {
                    self.emit_line(&format!("{} = {};", name, temp));
                } else {
                    let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                    self.emit_line(&format!("{} {} = {};", kw, name, temp));
                    self.declared.insert(name.clone());
                    self.ctx.vars.insert(name.clone(), ty.clone());
                }
            }
        }
    }

    /// Build nested rayon::join expression from assignments.
    /// 2 items: rayon::join(|| e1, || e2)
    /// 3+ items: rayon::join(|| e1, || rayon::join(|| e2, || e3))
    fn build_rayon_join(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            assigns[start].1.clone()
        } else if remaining == 2 {
            format!("rayon::join(|| {}, || {})", assigns[start].1, assigns[start + 1].1)
        } else {
            let rest = self.build_rayon_join(assigns, start + 1);
            format!("rayon::join(|| {}, || {})", assigns[start].1, rest)
        }
    }

    /// Build destructuring pattern: (mut a, (b, mut c)) for nested join
    fn build_join_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        let name_with_mut = |idx: usize| -> String {
            let name = &assigns[idx].0;
            if self.mutated_vars.contains(name) {
                format!("mut {}", name)
            } else {
                name.clone()
            }
        };
        if remaining == 1 {
            name_with_mut(start)
        } else if remaining == 2 {
            format!("({}, {})", name_with_mut(start), name_with_mut(start + 1))
        } else {
            let rest = self.build_join_pattern(assigns, start + 1);
            format!("({}, {})", name_with_mut(start), rest)
        }
    }

    /// Build temp name pattern: (_par_0, (_par_1, _par_2))
    fn build_join_temp_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            format!("_par_{}", start)
        } else if remaining == 2 {
            format!("(_par_{}, _par_{})", start, start + 1)
        } else {
            let rest = self.build_join_temp_pattern(assigns, start + 1);
            format!("(_par_{}, {})", start, rest)
        }
    }

    /// Build type pattern: (i64, (String, f64))
    fn build_join_type_pattern(&self, assigns: &[(String, String, CrowType)], start: usize) -> String {
        let remaining = assigns.len() - start;
        if remaining == 1 {
            assigns[start].2.to_rust_type()
        } else if remaining == 2 {
            format!("({}, {})", assigns[start].2.to_rust_type(), assigns[start + 1].2.to_rust_type())
        } else {
            let rest = self.build_join_type_pattern(assigns, start + 1);
            format!("({}, {})", assigns[start].2.to_rust_type(), rest)
        }
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { name, value } => {
                let expr_str = self.expr_to_string(value);
                if self.declared.contains(name) {
                    // Check for type-changing reassignment
                    let new_ty = self.ctx.infer_expr(value);
                    if let Some(existing_ty) = self.ctx.vars.get(name).cloned() {
                        if !existing_ty.compatible(&new_ty) {
                            panic!("error: cannot reassign '{}' from {} to {} — variable type cannot change", name, existing_ty.crow_name(), new_ty.crow_name());
                        }
                    }
                    self.emit_line(&format!("{} = {};", name, expr_str));
                } else {
                    let mut ty = self.ctx.infer_expr(value);
                    // If the expression type has Unknown (e.g. map[]), use the
                    // resolved variable type from the inference pass instead
                    if format!("{:?}", ty).contains("Unknown") {
                        if let Some(resolved) = self.ctx.vars.get(name) {
                            ty = resolved.clone();
                        }
                    }
                    let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                    self.emit_line(&format!("{} {}: {} = {};", kw, name, ty.to_rust_type(), expr_str));
                    self.declared.insert(name.clone());
                    self.ctx.vars.insert(name.clone(), ty);
                }
            }
            Stmt::MultiAssign { names, values } => {
                // Evaluate all RHS into temporaries first (needed for swap: a, b = b, a)
                let mut exprs: Vec<String> = Vec::new();
                let mut types: Vec<CrowType> = Vec::new();
                for value in values.iter() {
                    exprs.push(self.expr_to_string(value));
                    types.push(self.ctx.infer_expr(value));
                }
                // Check for type-changing reassignment
                for (i, name) in names.iter().enumerate() {
                    if self.declared.contains(name) {
                        if let Some(existing_ty) = self.ctx.vars.get(name).cloned() {
                            if !existing_ty.compatible(&types[i]) {
                                panic!("error: cannot reassign '{}' from {} to {} — variable type cannot change", name, existing_ty.crow_name(), types[i].crow_name());
                            }
                        }
                    }
                }
                // Check if any RHS reads a variable that a previous LHS writes (swap pattern)
                let rhs_reads: std::collections::HashSet<String> = values.iter().flat_map(|v| crate::parallel::expr_reads(v)).collect();
                let lhs_names: std::collections::HashSet<String> = names.iter().cloned().collect();
                let needs_temps = !rhs_reads.is_disjoint(&lhs_names) && names.iter().any(|n| self.declared.contains(n));
                if needs_temps {
                    // Use temporaries for safe swap
                    for (i, (expr, ty)) in exprs.iter().zip(types.iter()).enumerate() {
                        self.emit_line(&format!("let _multi_{}: {} = {};", i, ty.to_rust_type(), expr));
                    }
                    for (i, name) in names.iter().enumerate() {
                        if self.declared.contains(name) {
                            self.emit_line(&format!("{} = _multi_{};", name, i));
                        } else {
                            let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                            self.emit_line(&format!("{} {}: {} = _multi_{};", kw, name, types[i].to_rust_type(), i));
                            self.declared.insert(name.clone());
                            self.ctx.vars.insert(name.clone(), types[i].clone());
                        }
                    }
                } else {
                    // No swap conflict — emit directly
                    for (i, name) in names.iter().enumerate() {
                        if self.declared.contains(name) {
                            self.emit_line(&format!("{} = {};", name, exprs[i]));
                        } else {
                            let kw = if self.mutated_vars.contains(name) { "let mut" } else { "let" };
                            self.emit_line(&format!("{} {}: {} = {};", kw, name, types[i].to_rust_type(), exprs[i]));
                            self.declared.insert(name.clone());
                            self.ctx.vars.insert(name.clone(), types[i].clone());
                        }
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
            Stmt::Break => {
                self.emit_line("break;");
            }
            Stmt::Continue => {
                self.emit_line("continue;");
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
                let saved = self.declared.clone();
                for s in then_body { self.emit_stmt(s); }
                self.declared = saved;
                self.indent -= 1;
                if let Some(eb) = else_body {
                    self.emit_line("} else {");
                    self.indent += 1;
                    let saved = self.declared.clone();
                    for s in eb { self.emit_stmt(s); }
                    self.declared = saved;
                    self.indent -= 1;
                }
                self.emit_line("}");
            }
            Stmt::While { condition, body } => {
                let cond_str = self.expr_to_string(condition);
                self.emit_line(&format!("while {} {{", cond_str));
                self.indent += 1;
                let saved = self.declared.clone();
                for s in body { self.emit_stmt(s); }
                self.declared = saved;
                self.indent -= 1;
                self.emit_line("}");
            }
            Stmt::For { var, iter, body } => {
                let iter_str = self.expr_to_string(iter);

                // Determine element type for closure pattern
                let iter_type = self.ctx.infer_expr(iter);
                let elem_type = match &iter_type {
                    CrowType::List(inner) => *inner.clone(),
                    _ => CrowType::Unknown,
                };
                let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

                // Parallel by default. Fall back to sequential if:
                // 1. cpu[1] was explicitly set, OR
                // 2. The loop body mutates variables from the outer scope, OR
                // 3. The loop body contains break or continue
                let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
                let mutates_outer = Self::for_body_mutates_outer(body, var, &self.declared);
                let use_par = !forced_sequential && !mutates_outer;

                let saved = self.declared.clone();
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
                self.declared = saved;
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
            Stmt::FieldAssign { object, field, value } => {
                let obj_str = self.expr_to_string(object);
                let val_str = self.expr_to_string(value);
                self.emit_line(&format!("{}.{} = {};", obj_str, field, val_str));
            }
            Stmt::IndexAssign { object, index, value } => {
                let obj_str = self.expr_to_string(object);
                let val_str = self.expr_to_string(value);
                // Determine if object is a map or list
                let obj_type = self.ctx.infer_expr(object);
                match obj_type {
                    CrowType::Map(_) => {
                        let key_str = self.expr_to_string(index);
                        self.emit_line(&format!("{}.insert({}, {});", obj_str, key_str, val_str));
                    }
                    _ => {
                        let idx_str = self.expr_to_string(index);
                        self.emit_line(&format!("{}[{} as usize] = {};", obj_str, idx_str, val_str));
                    }
                }
            }
            Stmt::Use { path } => {
                self.emit_use(path);
            }
            Stmt::Resource { kind, count } => {
                // Update cpu thread count at runtime
                if kind == "cpu" {
                    if let Expr::Int(n) = count {
                        self.cpu_count = Some(*n as usize);
                    }
                }
            }
            Stmt::Line(n) => {
                self.current_line = *n;
            }
            _ => {}
        }
    }

    /// Handle a `use` import. Module declarations have already been expanded and
    /// type-inferred as prefixed names (e.g., mymath_double) by expand_module_imports.
    /// We just need to record the module name so FieldAccess can resolve module.var.
    fn emit_use(&mut self, path: &str) {
        let module_name = path.rsplit('/').next().unwrap_or(path).to_string();
        self.modules.insert(module_name);
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
            Expr::Ident(name) => {
                // If this ident refers to a user-defined function (used as a value),
                // apply the same renaming to avoid Rust collisions
                if self.ctx.functions.contains_key(name) {
                    Self::rust_fn_name(name)
                } else {
                    name.clone()
                }
            }
            Expr::BinOp { left, op, right } => {
                // Check for OptBoxStruct comparison with sentinel: field eq 0 => field.is_none()
                if matches!(op, BinOp::Eq | BinOp::Ne) {
                    if let Some(result) = self.try_optbox_comparison(left, op, right) {
                        return result;
                    }
                }
                let parent_prec = Self::op_precedence(op);
                // Wrap left child in parens if it has strictly lower precedence
                let l = self.expr_to_string(left);
                let l = if Self::expr_precedence(left).map_or(false, |p| p < parent_prec) {
                    format!("({})", l)
                } else { l };
                // Wrap right child in parens if it has lower or equal precedence (left-associative)
                let r = self.expr_to_string(right);
                let r = if Self::expr_precedence(right).map_or(false, |p| p <= parent_prec) {
                    format!("({})", r)
                } else { r };

                let lt = self.ctx.infer_expr(left);
                let rt = self.ctx.infer_expr(right);
                // Auto-unwrap CrowResult in BinOp: access .result
                let (l, lt) = if let CrowType::Result(inner) = &lt {
                    (format!("{}.result", l), *inner.clone())
                } else { (l, lt) };
                let (r, rt) = if let CrowType::Result(inner) = &rt {
                    (format!("{}.result", r), *inner.clone())
                } else { (r, rt) };
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
                // Wrap cast target in parens because `as` binds tighter than arithmetic
                match (&lt, &rt) {
                    (CrowType::Int, CrowType::Float) => format!("({}) as f64 {} {}", l, op_str, r),
                    (CrowType::Float, CrowType::Int) => format!("{} {} ({}) as f64", l, op_str, r),
                    _ => format!("{} {} {}", l, op_str, r),
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
                    // When unwrapping CrowResult args, replace with temp var .result
                    if self.unwrap_result_args {
                        if let Some(ref indices) = self.propagation_indices {
                            if indices.contains(&i) {
                                arg_strs.push(format!("_prop_{}.result.clone()", i));
                                continue;
                            }
                        }
                    }
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

                // --- Auto-propagation: detect CrowResult args ---
                if !self.unwrap_result_args {
                    let skip_propagation = callee_name.as_ref()
                        .map(|n| matches!(n.as_str(), "log" | "assert" | "cpu" | "gpu" | "type"))
                        .unwrap_or(true);
                    if !skip_propagation {
                        let result_arg_indices: Vec<usize> = args.iter().enumerate()
                            .filter(|(_, a)| matches!(self.ctx.infer_expr(a), CrowType::Result(_)))
                            .map(|(i, _)| i)
                            .collect();
                        if !result_arg_indices.is_empty() {
                            // Save original arg expressions for temp var bindings
                            let orig_arg_strs: Vec<String> = result_arg_indices.iter()
                                .map(|&i| arg_strs[i].clone())
                                .collect();
                            // Re-enter with unwrapping enabled
                            self.unwrap_result_args = true;
                            self.propagation_indices = Some(result_arg_indices.clone());
                            let inner_expr = self.expr_to_string(expr);
                            self.unwrap_result_args = false;
                            self.propagation_indices = None;
                            // Determine return type for error default
                            let full_type = self.ctx.infer_expr(expr);
                            let inner_type = match &full_type {
                                CrowType::Result(inner) => inner.as_ref().clone(),
                                other => other.clone(),
                            };
                            // Check if the inner call already returns CrowResult
                            let base_name = callee_name.as_deref().unwrap_or("");
                            let base_type = self.ctx.infer_call_base(base_name, args, callee);
                            let already_result = matches!(base_type, CrowType::Result(_));
                            return self.emit_propagation_block(
                                &result_arg_indices,
                                &orig_arg_strs,
                                &inner_expr,
                                &inner_type,
                                already_result,
                            );
                        }
                    }
                }

                if let Expr::Ident(name) = callee.as_ref() {
                    // Builtins
                    if name == "log" {
                        return self.emit_log_call(&arg_strs, args);
                    }

                    // range[stop], range[start, stop], range[start, stop, step]
                    if name == "range" {
                        return match arg_strs.len() {
                            1 => format!("(0..{}).collect::<Vec<i64>>()", arg_strs[0]),
                            2 => format!("({}..{}).collect::<Vec<i64>>()", arg_strs[0], arg_strs[1]),
                            3 => format!("({}..{}).step_by(({}) as usize).collect::<Vec<i64>>()", arg_strs[0], arg_strs[1], arg_strs[2]),
                            _ => "vec![]".to_string(),
                        };
                    }

                    // map[] — empty map constructor
                    if name == "map" && arg_strs.is_empty() {
                        return "BTreeMap::new()".to_string();
                    }

                    // --- Math builtins ---
                    if name == "abs" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).abs()", arg_strs[0]),
                            _ => format!("({}).abs()", arg_strs[0]),
                        };
                    }
                    if name == "sqrt" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).sqrt()", arg_strs[0]),
                            _ => format!("(({}) as f64).sqrt()", arg_strs[0]),
                        };
                    }
                    if name == "floor" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).floor() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    if name == "ceil" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).ceil() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    if name == "round" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => format!("({}).round() as i64", arg_strs[0]),
                            _ => arg_strs[0].clone(),
                        };
                    }
                    if name == "pow" && arg_strs.len() == 2 {
                        let t1 = self.ctx.infer_expr(&args[0]);
                        let t2 = self.ctx.infer_expr(&args[1]);
                        return if t1 == CrowType::Float || t2 == CrowType::Float {
                            format!("(({}) as f64).powf(({}) as f64)", arg_strs[0], arg_strs[1])
                        } else {
                            format!("({}).pow(({}) as u32)", arg_strs[0], arg_strs[1])
                        };
                    }
                    if name == "min" && arg_strs.len() == 2 {
                        let t1 = self.ctx.infer_expr(&args[0]);
                        let t2 = self.ctx.infer_expr(&args[1]);
                        return if t1 == CrowType::Float || t2 == CrowType::Float {
                            format!("(({}) as f64).min(({}) as f64)", arg_strs[0], arg_strs[1])
                        } else {
                            format!("std::cmp::min({}, {})", arg_strs[0], arg_strs[1])
                        };
                    }
                    if name == "max" && arg_strs.len() == 2 {
                        let t1 = self.ctx.infer_expr(&args[0]);
                        let t2 = self.ctx.infer_expr(&args[1]);
                        return if t1 == CrowType::Float || t2 == CrowType::Float {
                            format!("(({}) as f64).max(({}) as f64)", arg_strs[0], arg_strs[1])
                        } else {
                            format!("std::cmp::max({}, {})", arg_strs[0], arg_strs[1])
                        };
                    }
                    // --- String builtins ---
                    if name == "split" && arg_strs.len() == 2 {
                        return format!("({}).split(&{}).map(|s| s.to_string()).collect::<Vec<String>>()", arg_strs[0], arg_strs[1]);
                    }
                    if name == "trim" && arg_strs.len() == 1 {
                        return format!("({}).trim().to_string()", arg_strs[0]);
                    }
                    if name == "replace" && arg_strs.len() == 3 {
                        return format!("({}).replace(&{}, &{})", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }
                    if name == "upper" && arg_strs.len() == 1 {
                        return format!("({}).to_uppercase()", arg_strs[0]);
                    }
                    if name == "lower" && arg_strs.len() == 1 {
                        return format!("({}).to_lowercase()", arg_strs[0]);
                    }
                    if name == "join" && arg_strs.len() == 2 {
                        return format!("{}.iter().map(|s| s.to_string()).collect::<Vec<String>>().join(&{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "starts" && arg_strs.len() == 2 {
                        return format!("({}).starts_with(&*{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "ends" && arg_strs.len() == 2 {
                        return format!("({}).ends_with(&*{})", arg_strs[0], arg_strs[1]);
                    }
                    // --- Conversion builtins ---
                    if name == "int" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Int => arg_strs[0].clone(),
                            CrowType::Float => format!("({}) as i64", arg_strs[0]),
                            CrowType::Str => format!("{{ let _int_arg = {}; match _int_arg.parse::<i64>() {{ Ok(v) => CrowResult {{ status: \"ok\".to_string(), result: v, reason: \"\".to_string(), line: 0 }}, Err(_) => CrowResult {{ status: \"error\".to_string(), result: 0_i64, reason: format!(\"cannot convert '{{}}' to int\", _int_arg), line: {} }} }} }}", arg_strs[0], self.current_line),
                            CrowType::Bool => format!("if {} {{ 1_i64 }} else {{ 0_i64 }}", arg_strs[0]),
                            _ => format!("({}) as i64", arg_strs[0]),
                        };
                    }
                    if name == "float" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Float => arg_strs[0].clone(),
                            CrowType::Int => format!("({}) as f64", arg_strs[0]),
                            CrowType::Str => format!("{{ let _float_arg = {}; match _float_arg.parse::<f64>() {{ Ok(v) => CrowResult {{ status: \"ok\".to_string(), result: v, reason: \"\".to_string(), line: 0 }}, Err(_) => CrowResult {{ status: \"error\".to_string(), result: 0.0_f64, reason: format!(\"cannot convert '{{}}' to float\", _float_arg), line: {} }} }} }}", arg_strs[0], self.current_line),
                            CrowType::Bool => format!("if {} {{ 1.0_f64 }} else {{ 0.0_f64 }}", arg_strs[0]),
                            _ => format!("({}) as f64", arg_strs[0]),
                        };
                    }
                    if name == "str" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Str => arg_strs[0].clone(),
                            _ => format!("format!(\"{{}}\", {})", arg_strs[0]),
                        };
                    }
                    // --- Collection builtins ---
                    if name == "len" && arg_strs.len() == 1 {
                        return format!("({}).len() as i64", arg_strs[0]);
                    }
                    if name == "sum" && arg_strs.len() == 1 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::List(inner) => match *inner {
                                CrowType::Float => format!("({}).iter().sum::<f64>()", arg_strs[0]),
                                _ => format!("({}).iter().sum::<i64>()", arg_strs[0]),
                            },
                            _ => format!("({}).iter().sum::<i64>()", arg_strs[0]),
                        };
                    }
                    if name == "contains" && arg_strs.len() == 2 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Str => format!("({}).contains(&*{})", arg_strs[0], arg_strs[1]),
                            _ => format!("({}).contains(&{})", arg_strs[0], arg_strs[1]),
                        };
                    }
                    if name == "slice" && arg_strs.len() == 3 {
                        let ty = self.ctx.infer_expr(&args[0]);
                        return match ty {
                            CrowType::Str => format!("({})[({}) as usize..({}) as usize].to_string()", arg_strs[0], arg_strs[1], arg_strs[2]),
                            _ => format!("({})[({}) as usize..({}) as usize].to_vec()", arg_strs[0], arg_strs[1], arg_strs[2]),
                        };
                    }
                    // Immutable collection builtins (return new values)
                    if name == "push" && arg_strs.len() == 2 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.push({}); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    if name == "pop" && arg_strs.len() == 1 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.pop(); tmp }}", arg_strs[0]);
                    }
                    if name == "sort" {
                        return format!("{{ let mut tmp = {}.clone(); tmp.sort(); tmp }}", arg_strs[0]);
                    }
                    if name == "reverse" && arg_strs.len() == 1 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.reverse(); tmp }}", arg_strs[0]);
                    }
                    if name == "append" && arg_strs.len() == 2 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.extend({}.iter().cloned()); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    // --- Map builtins ---
                    if name == "keys" && arg_strs.len() == 1 {
                        return format!("{}.keys().cloned().collect::<Vec<String>>()", arg_strs[0]);
                    }
                    if name == "values" && arg_strs.len() == 1 {
                        return format!("{}.values().cloned().collect::<Vec<_>>()", arg_strs[0]);
                    }
                    if name == "has" && arg_strs.len() == 2 {
                        return format!("{}.contains_key(&{})", arg_strs[0], arg_strs[1]);
                    }
                    if name == "set" && arg_strs.len() == 3 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.insert({}, {}); tmp }}", arg_strs[0], arg_strs[1], arg_strs[2]);
                    }
                    if name == "del" && arg_strs.len() == 2 {
                        return format!("{{ let mut tmp = {}.clone(); tmp.remove(&{}); tmp }}", arg_strs[0], arg_strs[1]);
                    }
                    // --- CLI / IO builtins ---
                    if name == "args" && arg_strs.is_empty() {
                        return "std::env::args().skip(1).collect::<Vec<String>>()".to_string();
                    }
                    if name == "input" {
                        let ln = self.current_line;
                        if arg_strs.len() == 1 {
                            return format!("{{ print!(\"{{}}\", {}); use std::io::Write; std::io::stdout().flush().unwrap(); let mut _line = String::new(); match std::io::stdin().read_line(&mut _line) {{ Ok(_) => CrowResult {{ status: \"ok\".to_string(), result: _line.trim_end().to_string(), reason: \"\".to_string(), line: 0 }}, Err(e) => CrowResult {{ status: \"error\".to_string(), result: String::new(), reason: e.to_string(), line: {} }} }} }}", arg_strs[0], ln);
                        }
                        return format!("{{ let mut _line = String::new(); match std::io::stdin().read_line(&mut _line) {{ Ok(_) => CrowResult {{ status: \"ok\".to_string(), result: _line.trim_end().to_string(), reason: \"\".to_string(), line: 0 }}, Err(e) => CrowResult {{ status: \"error\".to_string(), result: String::new(), reason: e.to_string(), line: {} }} }} }}", ln);
                    }
                    // --- System builtins ---
                    if name == "rand" {
                        if arg_strs.is_empty() {
                            return "{ use std::collections::hash_map::DefaultHasher; use std::hash::{Hash, Hasher}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); (h.finish() as f64) / (u64::MAX as f64) }".to_string();
                        } else if arg_strs.len() == 1 {
                            return format!("{{ use std::collections::hash_map::DefaultHasher; use std::hash::{{Hash, Hasher}}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); (h.finish() % ({} as u64)) as i64 }}", arg_strs[0]);
                        } else if arg_strs.len() == 2 {
                            return format!("{{ use std::collections::hash_map::DefaultHasher; use std::hash::{{Hash, Hasher}}; let mut h = DefaultHasher::new(); std::time::SystemTime::now().hash(&mut h); std::thread::current().id().hash(&mut h); let _min = {}; let _max = {}; _min + (h.finish() % ((_max - _min + 1) as u64)) as i64 }}", arg_strs[0], arg_strs[1]);
                        }
                    }
                    if name == "time" && arg_strs.is_empty() {
                        return "std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()".to_string();
                    }
                    if name == "sleep" && arg_strs.len() == 1 {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        return match arg_type {
                            CrowType::Int => format!("std::thread::sleep(std::time::Duration::from_millis({} as u64))", arg_strs[0]),
                            _ => format!("std::thread::sleep(std::time::Duration::from_secs_f64({}))", arg_strs[0]),
                        };
                    }
                    if name == "env" && arg_strs.len() == 1 {
                        return format!("std::env::var({}).unwrap_or_default()", arg_strs[0]);
                    }
                    if name == "exit" {
                        if arg_strs.is_empty() {
                            return "std::process::exit(0)".to_string();
                        } else if arg_strs.len() == 1 {
                            return format!("std::process::exit({} as i32)", arg_strs[0]);
                        }
                    }
                    // --- Assert ---
                    if name == "assert" && (arg_strs.len() == 1 || arg_strs.len() == 2) {
                        let ty = self.ctx.infer_expr(&args[0]);
                        let cond = match ty {
                            CrowType::Bool => arg_strs[0].clone(),
                            CrowType::Int => format!("({}) != 0", arg_strs[0]),
                            CrowType::Str => format!("!({}).is_empty()", arg_strs[0]),
                            _ => format!("({}) != 0", arg_strs[0]),
                        };
                        if arg_strs.len() == 2 {
                            return format!("if !({}) {{ panic!(\"assertion failed: {{}}\", {}); }}", cond, arg_strs[1]);
                        } else {
                            return format!("assert!({})", cond);
                        }
                    }
                    // --- File I/O builtins ---
                    if name == "read" && arg_strs.len() == 1 {
                        let ln = self.current_line;
                        return format!("match std::fs::read_to_string(&*{}) {{ Ok(v) => CrowResult {{ status: \"ok\".to_string(), result: v, reason: \"\".to_string(), line: 0 }}, Err(e) => CrowResult {{ status: \"error\".to_string(), result: String::new(), reason: e.to_string(), line: {} }} }}", arg_strs[0], ln);
                    }
                    if name == "write" && arg_strs.len() == 2 {
                        let ln = self.current_line;
                        return format!("match std::fs::write(&*{}, &*{}) {{ Ok(_) => CrowResult {{ status: \"ok\".to_string(), result: \"\".to_string(), reason: \"\".to_string(), line: 0 }}, Err(e) => CrowResult {{ status: \"error\".to_string(), result: \"\".to_string(), reason: e.to_string(), line: {} }} }}", arg_strs[0], arg_strs[1], ln);
                    }
                    // --- JSON builtins ---
                    if name == "json" && arg_strs.len() == 1 {
                        self.needs_serde_json = true;
                        let ln = self.current_line;
                        return format!("match serde_json::from_str::<serde_json::Value>(&*{}) {{ Ok(v) => if v.is_null() {{ CrowResult {{ status: \"error\".to_string(), result: serde_json::Value::Null, reason: \"null\".to_string(), line: {} }} }} else {{ CrowResult {{ status: \"ok\".to_string(), result: v, reason: \"\".to_string(), line: 0 }} }}, Err(e) => CrowResult {{ status: \"error\".to_string(), result: serde_json::Value::Null, reason: format!(\"invalid JSON: {{}}\", e), line: {} }} }}", arg_strs[0], ln, ln);
                    }
                    if name == "encode" && arg_strs.len() == 1 {
                        self.needs_serde_json = true;
                        return format!("CrowResult {{ status: \"ok\".to_string(), result: serde_json::to_string(&{}).unwrap_or_default(), reason: \"\".to_string(), line: 0 }}", arg_strs[0]);
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

                    // Known function → direct call
                    if self.ctx.functions.contains_key(name) {
                        return format!("{}({})", Self::rust_fn_name(name), arg_strs.join(", "));
                    }

                    // Check variable type for list/map/json indexing vs indirect function call
                    let var_type = self.ctx.vars.get(name).cloned()
                        .unwrap_or(CrowType::Unknown);
                    // Unwrap CrowResult for indexing purposes
                    let inner_type = match &var_type {
                        CrowType::Result(inner) => inner.as_ref(),
                        other => other,
                    };
                    let is_result = matches!(&var_type, CrowType::Result(_));
                    let base_name = if is_result {
                        format!("{}.result", name)
                    } else {
                        name.to_string()
                    };
                    match inner_type {
                        CrowType::List(_) => {
                            // List indexing: list[i] → list[i as usize]
                            return format!("{}[{} as usize]", base_name, arg_strs.join(", "));
                        }
                        CrowType::Map(_) => {
                            // Map lookup: map[key] → map[&key]
                            return format!("{}[&{}].clone()", base_name, arg_strs.join(", "));
                        }
                        CrowType::Json => {
                            // JSON indexing: data["key"] or data[0] → data[key].clone()
                            let idx = &arg_strs[0];
                            // serde_json::Value indexes by &str or usize
                            let arg_type = self.ctx.infer_expr(&args[0]);
                            let idx_expr = match arg_type {
                                CrowType::Int => format!("{} as usize", idx),
                                _ => format!("&*{}", idx),
                            };
                            return format!("{}[{}].clone()", base_name, idx_expr);
                        }
                        _ => {
                            // Function pointer call or unknown: name(args)
                            return format!("{}({})", name, arg_strs.join(", "));
                        }
                    }
                }

                // Module function call: module.func[args] → module_func(args)
                if let Expr::FieldAccess { object, field } = callee.as_ref() {
                    if let Expr::Ident(mod_name) = object.as_ref() {
                        let prefixed = format!("{}_{}", mod_name, field);
                        if self.ctx.functions.contains_key(&prefixed) {
                            return format!("{}({})", prefixed, arg_strs.join(", "));
                        }
                    }
                }

                // Non-ident callee (e.g., list[0][3] or expr returning a function)
                let callee_type = self.ctx.infer_expr(callee);
                let callee_str = self.expr_to_string(callee);
                match &callee_type {
                    CrowType::List(_) => format!("{}[{} as usize]", callee_str, arg_strs.join(", ")),
                    CrowType::Map(_) => format!("{}[&{}].clone()", callee_str, arg_strs.join(", ")),
                    CrowType::Json => {
                        let arg_type = self.ctx.infer_expr(&args[0]);
                        let idx_expr = match arg_type {
                            CrowType::Int => format!("{} as usize", arg_strs[0]),
                            _ => format!("&*{}", arg_strs[0]),
                        };
                        format!("{}[{}].clone()", callee_str, idx_expr)
                    }
                    CrowType::Fn(_, _) => format!("{}({})", callee_str, arg_strs.join(", ")),
                    _ => format!("{}({})", callee_str, arg_strs.join(", ")),
                }
            }
            Expr::FieldAccess { object, field } => {
                // Module variable access: module.var → module_var
                if let Expr::Ident(mod_name) = object.as_ref() {
                    if self.modules.contains(mod_name) {
                        let prefixed = format!("{}_{}", mod_name, field);
                        return prefixed;
                    }
                }
                let obj = self.expr_to_string(object);
                // Map Crow's .len to Rust's .len()
                if field == "len" {
                    let obj_type = self.ctx.infer_expr(object);
                    if matches!(obj_type, CrowType::List(_) | CrowType::Str) {
                        return format!("{}.len() as i64", obj);
                    }
                }
                // Error-value struct field access: .status, .result, .reason, .line
                if field == "status" || field == "result" || field == "reason" || field == "line" {
                    let obj_type = self.ctx.infer_expr(object);
                    if let CrowType::Result(_) = &obj_type {
                        // Real CrowResult<T> — access the field directly
                        if field == "line" {
                            return format!("({}.line as i64)", obj);
                        }
                        return format!("{}.{}.clone()", obj, field);
                    } else if !matches!(obj_type, CrowType::Struct(_)) {
                        // Raw type — return synthetic defaults
                        return match field.as_str() {
                            "status" => "\"ok\".to_string()".to_string(),
                            "result" => obj,
                            "reason" => "\"\".to_string()".to_string(),
                            "line" => "0_i64".to_string(),
                            _ => unreachable!(),
                        };
                    }
                }
                // Check if this field is OptBoxStruct — if so, the caller
                // will handle unwrapping when it's used as a function argument.
                // Here we just emit the raw field access.
                format!("{}.{}", obj, field)
            }
            Expr::List(items) => {
                // Check if this is a list of functions — need explicit fn pointer cast
                let inner_type = if !items.is_empty() {
                    let mut t = CrowType::Unknown;
                    for item in items {
                        t = t.unify(&self.ctx.infer_expr(item));
                    }
                    t
                } else {
                    CrowType::Unknown
                };
                let vals: Vec<String> = if let CrowType::Fn(_, _) = &inner_type {
                    // Cast each item to fn pointer type to avoid Rust's distinct fn item types
                    let fn_type = inner_type.to_rust_type();
                    items.iter()
                        .map(|i| format!("{} as {}", self.expr_to_string(i), fn_type))
                        .collect()
                } else {
                    items.iter()
                        .map(|i| self.expr_to_string(i))
                        .collect()
                };
                format!("vec![{}]", vals.join(", "))
            }
            Expr::Map(pairs) => {
                if pairs.is_empty() {
                    "BTreeMap::new()".to_string()
                } else {
                    // Check individual value types to detect mixed-type maps
                    let val_types: Vec<CrowType> = pairs.iter()
                        .map(|(_, v)| self.ctx.infer_expr(v))
                        .collect();
                    let val_type = {
                        let mut t = CrowType::Unknown;
                        for vt in &val_types {
                            t = t.unify(vt);
                        }
                        t
                    };
                    // Detect mixed types: if any value type differs from the unified type,
                    // convert everything to String
                    let has_mixed = val_types.iter().any(|vt| vt != &val_type && *vt != CrowType::Unknown);
                    let entries: Vec<String> = if let CrowType::Fn(_, _) = &val_type {
                        let fn_type = val_type.to_rust_type();
                        pairs.iter()
                            .map(|(k, v)| format!("({}, {} as {})", self.expr_to_string(k), self.expr_to_string(v), fn_type))
                            .collect()
                    } else if has_mixed {
                        // Mixed-type map: convert all values to String
                        pairs.iter()
                            .zip(val_types.iter())
                            .map(|((k, v), vt)| {
                                let v_str = self.expr_to_string(v);
                                let v_as_str = match vt {
                                    CrowType::Str => v_str,
                                    _ => format!("{}.to_string()", v_str),
                                };
                                format!("({}, {})", self.expr_to_string(k), v_as_str)
                            })
                            .collect()
                    } else {
                        pairs.iter()
                            .map(|(k, v)| format!("({}, {})", self.expr_to_string(k), self.expr_to_string(v)))
                            .collect()
                    };
                    format!("BTreeMap::from([{}])", entries.join(", "))
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.emit_for_expr(var, iter, body)
            }
        }
    }

    /// Check if a for-expr body is a filter pattern:
    /// The last non-line stmt is an `if` without `else`, whose then_body's last stmt
    /// is an ExprStmt (the value to keep).
    fn is_filter_pattern(body: &[Stmt]) -> bool {
        let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
        if let Some(Stmt::If { else_body: None, then_body, .. }) = last {
            // Check that then_body ends with an ExprStmt
            let last_then = then_body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
            matches!(last_then, Some(Stmt::ExprStmt(_)))
        } else {
            false
        }
    }

    /// Extract the filter condition and value expression from a filter pattern.
    fn extract_filter_parts(body: &[Stmt]) -> Option<(&Expr, &Expr)> {
        let last = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
        if let Some(Stmt::If { condition, else_body: None, then_body }) = last {
            let last_then = then_body.iter().rev().find(|s| !matches!(s, Stmt::Line(_)));
            if let Some(Stmt::ExprStmt(val_expr)) = last_then {
                return Some((condition, val_expr));
            }
        }
        None
    }

    /// Emit a ForExpr as .iter().map(|var| { body }).collect()
    /// Parallel by default. Falls back to sequential if body mutates outer state or cpu[1].
    /// Detects filter patterns (if without else) and uses filter_map instead.
    fn emit_for_expr(&mut self, var: &str, iter: &Expr, body: &[Stmt]) -> String {
        let iter_str = self.expr_to_string(iter);
        let forced_sequential = self.cpu_count.map_or(false, |n| n <= 1);
        let mutates_outer = Self::for_body_mutates_outer(body, var, &self.declared);
        let use_par = !forced_sequential && !mutates_outer;
        let iter_method = if use_par { "par_iter" } else { "iter" };

        // Determine element type of the iterator for the closure pattern
        let iter_type = self.ctx.infer_expr(iter);
        let elem_type = match &iter_type {
            CrowType::List(inner) => *inner.clone(),
            _ => CrowType::Unknown,
        };
        let is_copy = matches!(elem_type, CrowType::Int | CrowType::Float | CrowType::Bool);

        // Build the closure parameter pattern
        let param_pattern = if is_copy {
            format!("|&{}|", var)
        } else {
            format!("|{}|", var)
        };

        // Check for filter pattern: for x in list \n if cond \n x
        if Self::is_filter_pattern(body) {
            if let Some((cond, val_expr)) = Self::extract_filter_parts(body) {
                // Save state for closure body
                let saved_vars = self.ctx.vars.clone();
                self.ctx.vars.insert(var.to_string(), elem_type.clone());

                let cond_str = self.expr_to_string(cond);
                let val_str = self.expr_to_string(val_expr);
                let val_type = self.ctx.infer_expr(val_expr);

                self.ctx.vars = saved_vars;

                let collect_type = val_type.to_rust_type();
                return format!(
                    "{}.{}().filter_map({} {{ if {} {{ Some({}) }} else {{ None }} }}).collect::<Vec<{}>>()",
                    iter_str, iter_method, param_pattern, cond_str, val_str, collect_type
                );
            }
        }

        // Determine the return type of each iteration (last expression)
        let last_type = if let Some(last_stmt) = body.iter().rev().find(|s| !matches!(s, Stmt::Line(_))) {
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
        let last_non_line = body.iter().rposition(|s| !matches!(s, Stmt::Line(_)));
        let has_last_expr = last_non_line.map_or(false, |i| matches!(body[i], Stmt::ExprStmt(_)));
        let (init_stmts, last_expr) = if has_last_expr {
            let idx = last_non_line.unwrap();
            if idx > 0 {
                (&body[..idx], Some(&body[idx]))
            } else {
                (&body[..0], Some(&body[idx]))
            }
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
                        if let Some(crate::ast::Stmt::ExprStmt(expr)) = stmts.iter().find(|s| !matches!(s, Stmt::Line(_))) {
                            let rust_expr = self.expr_to_string(expr);
                            // Use {:?} for collection/struct types since they don't impl Display
                            let expr_type = self.ctx.infer_expr(expr);
                            match expr_type {
                                CrowType::List(_) | CrowType::Map(_) | CrowType::Struct(_) => fmt_str.push_str("{:?}"),
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
                // Escape special characters for Rust string literal
                match chars[i] {
                    '\n' => fmt_str.push_str("\\n"),
                    '\t' => fmt_str.push_str("\\t"),
                    '\\' => fmt_str.push_str("\\\\"),
                    '"' => fmt_str.push_str("\\\""),
                    '\r' => fmt_str.push_str("\\r"),
                    ch => fmt_str.push(ch),
                }
                i += 1;
            }
        }

        if args.is_empty() {
            format!("\"{}\".to_string()", fmt_str)
        } else {
            format!("format!(\"{}\", {})", fmt_str, args.join(", "))
        }
    }

    /// Emit a propagation block that checks CrowResult args for errors before calling.
    /// If any arg is an error, returns a CrowResult with that error.
    /// Otherwise, executes the inner call and wraps the result.
    fn emit_propagation_block(
        &self,
        result_indices: &[usize],
        orig_arg_strs: &[String],
        inner_expr: &str,
        inner_type: &CrowType,
        already_result: bool,
    ) -> String {
        let default_val = inner_type.default_rust_value();
        let error_result = |idx: usize| -> String {
            format!(
                "CrowResult {{ status: \"error\".to_string(), result: {}, reason: _prop_{}.reason.clone(), line: _prop_{}.line }}",
                default_val, idx, idx
            )
        };
        let success_expr = if already_result {
            inner_expr.to_string()
        } else {
            format!(
                "CrowResult {{ status: \"ok\".to_string(), result: {}, reason: \"\".to_string(), line: 0 }}",
                inner_expr
            )
        };
        // Build nested if-else from inside out
        let mut result = success_expr;
        for (pos, &idx) in result_indices.iter().enumerate().rev() {
            result = format!(
                "{{ let _prop_{} = {}.clone(); if _prop_{}.status == \"error\" {{ {} }} else {{ {} }} }}",
                idx, orig_arg_strs[pos], idx, error_result(idx), result
            );
        }
        result
    }

    /// Emit a log[] call as println!()
    fn emit_log_call(&mut self, args: &[String], orig_exprs: &[Expr]) -> String {
        if args.is_empty() {
            return "println!()".to_string();
        }
        if args.len() == 1 {
            let arg = &args[0];
            if arg.starts_with("format!(") {
                return format!("println!{}", &arg[7..]);
            }
            let (needs_debug, json_kind) = if let Some(expr) = orig_exprs.first() {
                let ty = self.ctx.infer_expr(expr);
                let (inner, is_result) = match &ty {
                    CrowType::Result(inner) => (inner.as_ref().clone(), true),
                    other => (other.clone(), false),
                };
                let debug = matches!(inner, CrowType::List(_) | CrowType::Map(_) | CrowType::Struct(_));
                let jk = if matches!(inner, CrowType::Json) {
                    if is_result { 2 } else { 1 }  // 2=Result<Json>, 1=Json
                } else { 0 };
                (debug, jk)
            } else {
                (false, 0)
            };
            return if needs_debug {
                format!("println!(\"{{:?}}\", {})", arg)
            } else if json_kind == 2 {
                // CrowResult<Json> — check status, display result with crow_json_display
                format!("{{ let _jv = {}.clone(); if _jv.status == \"error\" {{ println!(\"error: {{}}\", _jv.reason); }} else {{ println!(\"{{}}\", crow_json_display(&_jv.result)); }} }}", arg)
            } else if json_kind == 1 {
                format!("println!(\"{{}}\", crow_json_display(&{}))", arg)
            } else {
                format!("println!(\"{{}}\", {})", arg)
            };
        }
        // Multi-argument log: join all args with spaces
        let mut fmt_parts = Vec::new();
        let mut fmt_args = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let (needs_debug, json_kind) = if i < orig_exprs.len() {
                let ty = self.ctx.infer_expr(&orig_exprs[i]);
                let (inner, is_result) = match &ty {
                    CrowType::Result(inner) => (inner.as_ref().clone(), true),
                    other => (other.clone(), false),
                };
                let debug = matches!(inner, CrowType::List(_) | CrowType::Map(_) | CrowType::Struct(_));
                let jk = if matches!(inner, CrowType::Json) {
                    if is_result { 2 } else { 1 }
                } else { 0 };
                (debug, jk)
            } else {
                (false, 0)
            };
            if needs_debug {
                fmt_parts.push("{:?}".to_string());
            } else {
                fmt_parts.push("{}".to_string());
            }
            if json_kind == 2 {
                fmt_args.push(format!("crow_json_display(&{}.result)", arg));
            } else if json_kind == 1 {
                fmt_args.push(format!("crow_json_display(&{})", arg));
            } else {
                fmt_args.push(arg.clone());
            }
        }
        format!("println!(\"{}\", {})", fmt_parts.join(" "), fmt_args.join(", "))
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



#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// Helper: parse Crow source, run type inference, transpile to Rust
    fn transpile(source: &str) -> String {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("lexer failed");
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().expect("parser failed");
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut t = Transpiler::new(ctx);
        t.transpile(&stmts)
    }

    #[test]
    fn test_multi_assign() {
        let src = "x, y = 10, 20\nlog['{x} {y}']\n";
        let rust = transpile(src);
        assert!(rust.contains("let x: i64 = 10"), "should declare x\n{}", rust);
        assert!(rust.contains("let y: i64 = 20"), "should declare y\n{}", rust);
    }

    #[test]
    fn test_swap_uses_temporaries() {
        let src = "a = 1\nb = 2\na, b = b, a\nlog['{a} {b}']\n";
        let rust = transpile(src);
        // Swap should use temp variables
        assert!(rust.contains("_multi_"), "swap should use temp vars\n{}", rust);
    }

    #[test]
    fn test_int_parse_returns_crow_result() {
        let src = "x = int['42']\nlog['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "int[] from string should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "int[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_float_parse_returns_crow_result() {
        let src = "x = float['3.14']\nlog['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "float[] from string should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "float[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_scope_if_block() {
        // Variable declared inside if should not affect outer scope declarations
        let src = "x = 1\nif true\n    y = 2\n    x = 10\nx = 5\n";
        let rust = transpile(src);
        // x should be declared with let mut (reassigned)
        // y inside if should be a separate let
        assert!(rust.contains("let y:"), "y should be declared inside if\n{}", rust);
    }

    #[test]
    fn test_scope_while_block() {
        let src = "n = 5\nwhile n gt 0\n    step = n\n    n = n - 1\n";
        let rust = transpile(src);
        assert!(rust.contains("let") && rust.contains("step:"), "step should be declared inside while\n{}", rust);
    }

    #[test]
    fn test_scope_for_block() {
        let src = "for i in [1, 2, 3]\n    temp = i * 10\n    log['{temp}']\n";
        let rust = transpile(src);
        assert!(rust.contains("let") && rust.contains("temp:"), "temp should be declared inside for\n{}", rust);
    }

    #[test]
    fn test_cpu_count_wiring() {
        let mut lexer = Lexer::new("x = 1\nlog['{x}']\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let stmts = parser.parse().unwrap();
        let mut ctx = TypeContext::new();
        ctx.infer_program(&stmts);
        let mut t = Transpiler::new(ctx);
        t.set_cpu_count(4);
        assert_eq!(t.cpu_count, Some(4));
    }

    #[test]
    fn test_read_returns_crow_result() {
        let src = "content = read['/tmp/crow_test_read.txt']\nlog['{content}']\n";
        let rust = transpile(src);
        assert!(rust.contains("CrowResult"), "read[] should return CrowResult\n{}", rust);
        assert!(!rust.contains(".unwrap()"), "read[] should not use .unwrap()\n{}", rust);
    }

    #[test]
    fn test_parallel_independent_stmts() {
        let src = "a = 1 + 2\nb = 3 + 4\nc = 5 + 6\n";
        let rust = transpile(src);
        // Independent assignments should be grouped into a rayon::join
        assert!(rust.contains("rayon::join"), "independent stmts should use rayon::join\n{}", rust);
    }

    #[test]
    fn test_dependent_stmts_sequential() {
        let src = "a = 1\nb = a + 1\nc = b + 1\n";
        let rust = transpile(src);
        // Dependent assignments should NOT be in rayon::join
        assert!(!rust.contains("rayon::join"), "dependent stmts should be sequential\n{}", rust);
    }

    #[test]
    fn test_crow_result_struct_emitted() {
        let src = "x = int['42']\n";
        let rust = transpile(src);
        assert!(rust.contains("struct CrowResult<T: Clone>"), "should emit CrowResult struct\n{}", rust);
        assert!(rust.contains("status: String"), "CrowResult should have status field\n{}", rust);
        assert!(rust.contains("result: T"), "CrowResult should have result field\n{}", rust);
        assert!(rust.contains("reason: String"), "CrowResult should have reason field\n{}", rust);
        assert!(rust.contains("line: usize"), "CrowResult should have line field\n{}", rust);
    }

    #[test]
    fn test_crow_result_not_emitted_when_unnecessary() {
        let src = "x = 5\nlog[x]\n";
        let rust = transpile(src);
        assert!(!rust.contains("CrowResult"), "should not emit CrowResult when not needed\n{}", rust);
    }

    #[test]
    fn test_status_on_raw_type() {
        let src = "x = 5\nlog[x.status]\n";
        let rust = transpile(src);
        assert!(rust.contains("\"ok\""), "status on raw type should return 'ok'\n{}", rust);
    }

    #[test]
    fn test_result_field_access() {
        let src = "x = int['42']\nlog[x.result]\n";
        let rust = transpile(src);
        assert!(rust.contains(".result"), "should access .result field\n{}", rust);
    }

    #[test]
    fn test_status_field_access_on_result() {
        let src = "x = int['42']\nlog[x.status]\n";
        let rust = transpile(src);
        assert!(rust.contains(".status"), "should access .status field on CrowResult\n{}", rust);
    }

    #[test]
    fn test_auto_unwrap_in_binop() {
        let src = "x = int['42']\ny = x + 1\n";
        let rust = transpile(src);
        assert!(rust.contains(".result"), "should auto-unwrap CrowResult in BinOp\n{}", rust);
    }

    #[test]
    fn test_int_from_float_no_result() {
        let src = "x = 3.14\ny = int[x]\n";
        let rust = transpile(src);
        assert!(!rust.contains("CrowResult"), "int from float should not use CrowResult\n{}", rust);
    }

    #[test]
    #[should_panic(expected = "cannot reassign")]
    fn test_type_changing_reassignment_error() {
        // x = 5 (int), then x = read['f'] (CrowResult<String>) — should panic
        let src = "x = 5\nx = read['file.txt']\n";
        transpile(src);
    }

    #[test]
    fn test_same_type_reassignment_ok() {
        // x = 5, then x = 10 — same type, should be fine
        let src = "x = 5\nx = 10\n";
        let rust = transpile(src);
        assert!(rust.contains("x = 10"), "same-type reassignment should work\n{}", rust);
    }

    #[test]
    fn test_int_float_reassignment_ok() {
        // x = 5, then x = 3.14 — int/float are compatible
        let src = "x = 5\nx = 3.14\n";
        let rust = transpile(src);
        assert!(rust.contains("x = 3.14"), "int/float reassignment should work\n{}", rust);
    }

    #[test]
    fn test_sum_int_list() {
        let src = "x = [1, 2, 3]\ny = sum[x]\n";
        let rust = transpile(src);
        assert!(rust.contains(".iter().sum::<i64>()"), "sum of int list should use sum::<i64>()\n{}", rust);
    }

    #[test]
    fn test_sum_float_list() {
        let src = "x = [1.0, 2.0, 3.0]\ny = sum[x]\n";
        let rust = transpile(src);
        assert!(rust.contains(".iter().sum::<f64>()"), "sum of float list should use sum::<f64>()\n{}", rust);
    }

    #[test]
    fn test_for_loop_parallel_by_default() {
        let src = "items = [1, 2, 3]\nfor x in items\n    log['{x}']\n";
        let rust = transpile(src);
        assert!(rust.contains("par_iter()"), "for loop should use par_iter by default\n{}", rust);
        assert!(rust.contains("for_each"), "parallel for should use for_each\n{}", rust);
        assert!(rust.contains("use rayon::prelude::*"), "should import rayon prelude\n{}", rust);
    }

    #[test]
    fn test_for_loop_sequential_when_mutates_outer() {
        // Loop body assigns to `total` which is in outer scope — must be sequential
        let src = "total = 0\nitems = [1, 2, 3]\nfor x in items\n    total = total + x\nlog['{total}']\n";
        let rust = transpile(src);
        assert!(rust.contains("for") && rust.contains(".iter()"), "mutating loop should use iter()\n{}", rust);
        assert!(!rust.contains("par_iter"), "mutating loop should NOT use par_iter\n{}", rust);
    }

    #[test]
    fn test_for_loop_sequential_with_cpu_1() {
        let src = "cpu[1]\nitems = [1, 2, 3]\nfor x in items\n    log['{x}']\n";
        let rust = transpile(src);
        assert!(!rust.contains("par_iter"), "cpu[1] should force sequential for loop\n{}", rust);
    }

    #[test]
    fn test_for_expr_parallel_by_default() {
        let src = "items = [1, 2, 3]\ndoubled = for x in items\n    x * 2\nlog['{doubled}']\n";
        let rust = transpile(src);
        assert!(rust.contains("par_iter()"), "for-expr should use par_iter by default\n{}", rust);
    }

    #[test]
    fn test_error_line_field_access() {
        let src = "x = int['bad']\ny = x.line\n";
        let rust = transpile(src);
        assert!(rust.contains(".line"), "should access .line field on CrowResult\n{}", rust);
    }

    #[test]
    fn test_raw_value_line_is_zero() {
        let src = "x = 5\ny = x.line\n";
        let rust = transpile(src);
        assert!(rust.contains("0_i64"), "raw value .line should be 0\n{}", rust);
    }

    #[test]
    fn test_auto_propagation_uses_if_else() {
        let src = "fn double[x]\n    return x * 2\nbad = int['hello']\nresult = double[bad]\n";
        let rust = transpile(src);
        assert!(rust.contains("_prop_"), "should generate propagation temp vars\n{}", rust);
        assert!(rust.contains("\"error\""), "should check for error status\n{}", rust);
        assert!(rust.contains("else"), "should use if-else for propagation\n{}", rust);
    }

    #[test]
    fn test_auto_propagation_unwraps_args() {
        let src = "fn double[x]\n    return x * 2\nbad = int['hello']\nresult = double[bad]\n";
        let rust = transpile(src);
        // Function should take i64, not CrowResult<i64>
        assert!(rust.contains("fn double(x: i64)"), "function param should be base type, not CrowResult\n{}", rust);
    }

    #[test]
    fn test_int_error_message_format() {
        let src = "x = int['bad']\n";
        let rust = transpile(src);
        assert!(rust.contains("cannot convert"), "should use Crow-style error message\n{}", rust);
    }

    #[test]
    fn test_crow_result_display_shows_error() {
        let src = "x = int['bad']\nlog[x]\n";
        let rust = transpile(src);
        assert!(rust.contains("\"error\""), "Display impl should check error status\n{}", rust);
    }

    #[test]
    fn test_json_builtin_emits_serde() {
        let src = "items = json['[1,2,3]']\nlog[items[0]]\n";
        let rust = transpile(src);
        assert!(rust.contains("serde_json::from_str"), "json[] should use serde_json\n{}", rust);
        assert!(rust.contains("CrowResult"), "json[] should return CrowResult\n{}", rust);
    }

    #[test]
    fn test_json_null_returns_error() {
        let src = "n = json['null']\nlog[n.status]\n";
        let rust = transpile(src);
        assert!(rust.contains("is_null()"), "json[] should check for null\n{}", rust);
        assert!(rust.contains("\"error\""), "null should map to error status\n{}", rust);
    }

    #[test]
    fn test_encode_builtin() {
        let src = "items = json['[1,2,3]']\ns = encode[items]\nlog[s]\n";
        let rust = transpile(src);
        assert!(rust.contains("serde_json::to_string"), "encode[] should use serde_json::to_string\n{}", rust);
    }

    #[test]
    fn test_json_display_helper_emitted() {
        let src = "s = json['\"hello\"']\nlog[s]\n";
        let rust = transpile(src);
        assert!(rust.contains("crow_json_display"), "should emit crow_json_display helper\n{}", rust);
    }

    #[test]
    fn test_json_indexing_with_int() {
        let src = "items = json['[1,2,3]']\nlog[items[0]]\n";
        let rust = transpile(src);
        assert!(rust.contains("as usize"), "int index on JSON should cast to usize\n{}", rust);
    }
}