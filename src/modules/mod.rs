pub mod db;
pub mod frame;
pub mod io;
pub mod math;
pub mod nn;
pub mod stats;

use std::collections::HashMap;
use std::sync::Arc;
use crate::value::CrowValue;
use crate::type_infer::CrowType;

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

    /// Emit Rust code for a function call. Called by the transpiler.
    /// `arg_strs` are the already-transpiled argument expressions.
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
    fn_map: HashMap<String, usize>,
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
            self.fn_map.insert(fname.to_string(), idx);
        }
        for (cname, _) in module.constants() {
            self.const_map.insert(cname.to_string(), idx);
        }
        self.modules.push(module);
    }

    /// Check if a function name belongs to any loaded module.
    pub fn has_function(&self, name: &str) -> bool {
        self.fn_map.contains_key(name)
    }

    /// Which loaded module provides this function (for reservation messages).
    pub fn module_name_of(&self, name: &str) -> Option<&str> {
        let idx = self.fn_map.get(name)?;
        Some(self.modules[*idx].name())
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

    /// Call a module function (interpreter).
    pub fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        let idx = self.fn_map.get(name)
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.modules[*idx].call(name, args)
    }

    /// Emit Rust for a module function (transpiler).
    pub fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        let idx = self.fn_map.get(name)
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.modules[*idx].emit_rust(name, arg_strs, arg_types)
    }

    /// Infer return type of a module function (type inference).
    pub fn type_of(&self, name: &str, arg_count: usize) -> Option<CrowType> {
        let idx = self.fn_map.get(name)?;
        Some(self.modules[*idx].type_of(name, arg_count))
    }

    /// Get all constant (name, value) pairs from all loaded modules.
    pub fn all_constants(&self) -> Vec<(&str, CrowValue)> {
        let mut result = Vec::new();
        for module in &self.modules {
            result.extend(module.constants());
        }
        result
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
    pub fn module_name_for(&self, fn_name: &str) -> Option<&str> {
        let idx = self.fn_map.get(fn_name)?;
        Some(self.modules[*idx].name())
    }
}

