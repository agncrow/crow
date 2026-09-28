pub mod chart;
pub mod db;
pub mod doc;
pub mod frame;
pub mod io;
pub mod math;
pub mod nn;
pub mod stats;

use std::collections::HashMap;
use std::sync::Arc;
use crate::value::CrowValue;
use crate::type_infer::CrowType;

/// Where a module's printed code stops the program, the line it stops on:
/// the emitter puts the call's line in its place, so a module's refusal
/// names its line as the interpreter's does (`crow_stop(__crow_line__, e)`).
pub const STOP_LINE: &str = "__crow_line__";

/// Trait that every Crow module implements.
/// Each module is self-contained: it declares its functions, constants,
/// and knows how to interpret calls, emit Rust code, and infer types.
pub trait CrowModule: Send + Sync {
    /// Module name (e.g. "math", "math.stats")
    fn name(&self) -> &str;

    /// List of function names this module provides.
    fn functions(&self) -> &[&str];

    /// Constants this module provides (e.g. pi, e, inf, nan).
    /// Returns (name, value) pairs.
    fn constants(&self) -> Vec<(&str, CrowValue)> {
        vec![]
    }

    /// Interpret a function call. Called by the interpreter.
    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String>;

    /// Emit Rust code for a function call. Called by the Rust backend.
    /// `arg_strs` are the argument expressions, already printed.
    /// `arg_types` are the inferred types of each argument.
    fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String>;

    /// Infer the return type of a function call. Called by the type inference engine.
    fn type_of(&self, name: &str, arg_count: usize) -> CrowType;
}

/// Registry of loaded modules. Provides generic dispatch.
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn CrowModule>>,
    /// Maps function name → index into `modules` vec.
    /// Built when modules are loaded.
    fn_map: HashMap<String, Vec<usize>>,
    /// Maps constant name → index into `modules` vec.
    const_map: HashMap<String, usize>,
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.modules.iter().map(|m| m.name()).collect();
        f.debug_struct("ModuleRegistry")
            .field("modules", &names)
            .finish()
    }
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry {
            modules: Vec::new(),
            fn_map: HashMap::new(),
            const_map: HashMap::new(),
        }
    }

    /// Load a module, registering all its functions and constants.
    pub fn load(&mut self, module: Arc<dyn CrowModule>) {
        let idx = self.modules.len();
        for &fname in module.functions() {
            self.fn_map.entry(fname.to_string()).or_default().push(idx);
        }
        for (cname, _) in module.constants() {
            self.const_map.insert(cname.to_string(), idx);
        }
        self.modules.push(module);
    }

    /// A registry with every stdlib module loaded — for tooling (checker,
    /// LSP) that needs the full export vocabulary without a `use` context.
    pub fn with_stdlib() -> Self {
        let mut r = Self::new();
        r.load(Arc::new(crate::modules::math::MathModule));
        r.load(Arc::new(crate::modules::stats::StatsModule));
        r.load(Arc::new(crate::modules::nn::NnModule));
        r.load(Arc::new(crate::modules::io::IoModule));
        r.load(Arc::new(crate::modules::frame::FrameModule));
        r.load(Arc::new(crate::modules::db::DbModule));
        r.load(Arc::new(crate::modules::doc::DocModule));
        r.load(Arc::new(crate::modules::chart::ChartModule));
        r
    }

    /// Every function name provided by any loaded module.
    pub fn all_function_names(&self) -> Vec<&str> {
        self.modules
            .iter()
            .flat_map(|m| m.functions().iter().copied())
            .collect()
    }

    /// Check if a function name belongs to any loaded module.
    pub fn has_function(&self, name: &str) -> bool {
        self.fn_map.contains_key(name)
    }

    /// All loaded modules providing this name — >1 means a bare call is
    /// ambiguous (spec: Conflict Resolution).
    pub fn providers_of(&self, name: &str) -> Vec<&str> {
        self.fn_map
            .get(name)
            .map(|idxs| idxs.iter().map(|&i| self.modules[i].name()).collect())
            .unwrap_or_default()
    }

    /// Which loaded module provides this function (for reservation messages).
    /// Returns the first provider when several do.
    pub fn module_name_of(&self, name: &str) -> Option<&str> {
        let idx = self.fn_map.get(name)?.first()?;
        Some(self.modules[*idx].name())
    }

    /// Resolve a function within one specific module (prefixed calls).
    fn idx_in(&self, module_name: &str, fname: &str) -> Option<usize> {
        self.fn_map.get(fname)?.iter().copied().find(|&i| self.modules[i].name() == module_name)
    }

    /// Call a module function through its module prefix.
    pub fn call_in(&self, module_name: &str, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.call_at(idx, name, args)
    }

    /// Emit Rust for a function through its module prefix.
    pub fn emit_rust_in(&self, module_name: &str, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type through a module prefix.
    pub fn type_of_in(&self, module_name: &str, name: &str, arg_count: usize) -> Option<CrowType> {
        let idx = self.idx_in(module_name, name)?;
        Some(self.type_at(idx, name, arg_count))
    }

    // A module function with a row in the builtin table is its runtime
    // function, for the interpreter, the emitter and inference alike
    // (crate::table); the module's own arms answer the rest.

    fn call_at(&self, idx: usize, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        match crate::table::module(self.modules[idx].name(), name, args.len()) {
            // a module's error value carries no trace, in either mode
            Some(row) => (row.call)(args, ""),
            None => self.modules[idx].call(name, args),
        }
    }

    fn emit_at(&self, idx: usize, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        match crate::table::module(self.modules[idx].name(), name, arg_strs.len()) {
            Some(row) => crate::table::print(row, arg_strs, STOP_LINE, None),
            None => self.modules[idx].emit_rust(name, arg_strs, arg_types),
        }
    }

    fn type_at(&self, idx: usize, name: &str, arg_count: usize) -> CrowType {
        match crate::table::module(self.modules[idx].name(), name, arg_count) {
            Some(row) => row.crow_type(),
            None => self.modules[idx].type_of(name, arg_count),
        }
    }

    /// Check if a constant name belongs to any loaded module.
    pub fn has_constant(&self, name: &str) -> bool {
        self.const_map.contains_key(name)
    }

    /// Get the constant value for a name.
    pub fn get_constant(&self, name: &str) -> Option<CrowValue> {
        let idx = self.const_map.get(name)?;
        let module = &self.modules[*idx];
        module.constants().into_iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    /// Bare-call ambiguity message, or None when the name is unique.
    fn ambiguity(&self, name: &str) -> Option<String> {
        let providers = self.providers_of(name);
        if providers.len() > 1 {
            let forms: Vec<String> = providers.iter().map(|m| format!("{}.{}[...]", m, name)).collect();
            Some(format!(
                "'{}' is ambiguous — provided by {}; use {}",
                name,
                providers.join(" and "),
                forms.join(" or ")
            ))
        } else {
            None
        }
    }

    /// Call a module function (interpreter, bare call).
    pub fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        if let Some(msg) = self.ambiguity(name) {
            return Err(msg);
        }
        let idx = self.fn_map.get(name).and_then(|v| v.first().copied())
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.call_at(idx, name, args)
    }

    /// Emit Rust for a module function (the Rust backend, bare call).
    pub fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        if let Some(msg) = self.ambiguity(name) {
            return Err(msg);
        }
        let idx = self.fn_map.get(name).and_then(|v| v.first().copied())
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type of a module function (bare call; None when ambiguous).
    pub fn type_of(&self, name: &str, arg_count: usize) -> Option<CrowType> {
        let idxs = self.fn_map.get(name)?;
        if idxs.len() > 1 {
            return None;
        }
        Some(self.type_at(*idxs.first()?, name, arg_count))
    }


    /// Check if a specific module (by name) has been loaded.
    pub fn is_loaded(&self, module_name: &str) -> bool {
        self.modules.iter().any(|m| m.name() == module_name)
    }

    /// All registered function names.
    pub fn fn_names(&self) -> Vec<&str> {
        self.fn_map.keys().map(|s| s.as_str()).collect()
    }

    /// All registered constant names.
    pub fn const_names(&self) -> Vec<&str> {
        self.const_map.keys().map(|s| s.as_str()).collect()
    }

    /// Return the module name that owns a given function, if any.
    /// (First provider when several do — prefer providers_of for messages.)
    pub fn module_name_for(&self, fn_name: &str) -> Option<&str> {
        let idx = self.fn_map.get(fn_name)?.first()?;
        Some(self.modules[*idx].name())
    }
}

