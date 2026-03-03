use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crow_runtime::tensor::CrowTensor;
use crow_runtime::sparse::CrowSparse;

/// Every value in Crow carries error metadata: ok, reason, kind, trace.
/// Values auto-unwrap — you use them directly.
#[derive(Debug, Clone)]
pub struct CrowValue {
    pub value: Value,              // the actual value
    pub ok: bool,                  // true for normal values, false for errors
    pub reason: String,            // error message ("" if not error)
    pub kind: String,              // error category ("" if not error)
    pub trace: Vec<String>,        // call trace, built during propagation
}

impl CrowValue {
    pub fn ok(value: Value) -> Self {
        CrowValue {
            value,
            ok: true,
            reason: String::new(),
            kind: String::new(),
            trace: Vec::new(),
        }
    }

    pub fn done() -> Self {
        CrowValue {
            value: Value::Done,
            ok: true,
            reason: String::new(),
            kind: String::new(),
            trace: Vec::new(),
        }
    }

    pub fn error(reason: &str, trace_entry: &str) -> Self {
        CrowValue {
            value: Value::Done,
            ok: false,
            reason: reason.to_string(),
            kind: String::new(),
            trace: vec![trace_entry.to_string()],
        }
    }

    pub fn error_with_kind(reason: &str, kind: &str, trace_entry: &str) -> Self {
        CrowValue {
            value: Value::Done,
            ok: false,
            reason: reason.to_string(),
            kind: kind.to_string(),
            trace: vec![trace_entry.to_string()],
        }
    }

    #[inline(always)]
    pub fn is_error(&self) -> bool {
        !self.ok
    }

    #[inline(always)]
    #[allow(dead_code)]
    pub fn is_ok(&self) -> bool {
        self.ok
    }

    /// Auto-unwrap: get the inner value, propagating errors
    pub fn unwrap_or_propagate(&self) -> Result<&Value, String> {
        if !self.ok {
            Err(if self.reason.is_empty() { "unknown error".to_string() } else { self.reason.clone() })
        } else {
            Ok(&self.value)
        }
    }

    /// Append a trace entry during auto-propagation
    pub fn append_trace(&mut self, location: &str) {
        self.trace.push(location.to_string());
    }
}

/// The actual runtime values
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Array(Vec<CrowValue>),
    NumArray(CrowTensor),
    Map(BTreeMap<String, CrowValue>),
    Fn(FnValue),
    StructDef(StructDef),
    StructInstance(StructInstance),
    Module(ModuleValue),
    Sparse(CrowSparse), // sparse matrix (CSR format)
    Dual(f64, f64), // dual number for autograd: (real, derivative)
    GradFn(FnValue), // gradient function wrapping a user function
    Done, // represents no value (from 'done' status)
}

#[derive(Debug, Clone)]
pub struct FnValue {
    pub name: String,
    pub params: Vec<String>,
    pub body: Arc<Vec<crate::ast::Stmt>>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StructInstance {
    pub name: String,
    pub fields: BTreeMap<String, CrowValue>,
}

#[derive(Debug, Clone)]
pub struct ModuleValue {
    pub name: String,
    pub bindings: BTreeMap<String, CrowValue>,
}

impl Value {
    /// Format a value with quotes around strings (for display inside containers)
    fn fmt_repr(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "'{}'", s),
            other => write!(f, "{}", other),
        }
    }

    /// Return the Crow type name for this value
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Bool(_) => "bool",
            Value::Array(_) => "array",
            Value::NumArray(_) => "array",
            Value::Map(_) => "map",
            Value::Fn(_) => "fn",
            Value::StructDef(_) => "struct",
            Value::StructInstance(_) => "struct_instance",
            Value::Module(_) => "module",
            Value::Sparse(_) => "sparse",
            Value::Dual(_, _) => "float", // Dual appears as float to user
            Value::GradFn(_) => "fn",
            Value::Done => "done",
        }
    }

    /// Check if two values have compatible types for reassignment
    pub fn type_compatible(&self, other: &Value) -> bool {
        match (self, other) {
            // int and float are compatible (Dual acts like float)
            (Value::Int(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) => true,
            (Value::Dual(_, _), Value::Float(_)) | (Value::Float(_), Value::Dual(_, _)) => true,
            (Value::Dual(_, _), Value::Int(_)) | (Value::Int(_), Value::Dual(_, _)) => true,
            (Value::Dual(_, _), Value::Dual(_, _)) => true,
            // Array and NumArray are compatible (both are "array")
            (Value::Array(_), Value::NumArray(_)) | (Value::NumArray(_), Value::Array(_)) => true,
            // same variant is compatible
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    item.value.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::NumArray(tensor) => {
                // Display 1D numeric arrays like regular arrays
                let data = tensor.to_vec();
                write!(f, "[")?;
                for (i, v) in data.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    // Display as int if it's a whole number
                    if *v == (*v as i64) as f64 && v.is_finite() {
                        write!(f, "{}", *v as i64)?;
                    } else {
                        write!(f, "{}", v)?;
                    }
                }
                write!(f, "]")
            }
            Value::Map(map) => {
                write!(f, "[")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = ", k)?;
                    v.value.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::Fn(func) => write!(f, "<fn {}>", func.name),
            Value::StructDef(sd) => write!(f, "<struct {}>", sd.name),
            Value::StructInstance(si) => {
                write!(f, "{}[", si.name)?;
                for (i, (k, v)) in si.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = ", k)?;
                    v.value.fmt_repr(f)?;
                }
                write!(f, "]")
            }
            Value::Sparse(s) => write!(f, "{}", s),
            Value::GradFn(func) => write!(f, "<grad {}>", func.name),
            Value::Module(m) => write!(f, "<module {}>", m.name),
            Value::Dual(real, _) => {
                // Display as float (user never sees the derivative)
                if *real == (*real as i64) as f64 && real.is_finite() {
                    write!(f, "{}", *real as i64)
                } else {
                    write!(f, "{}", real)
                }
            }
            Value::Done => write!(f, "done"),
        }
    }
}

impl fmt::Display for CrowValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ok {
            write!(f, "{}", self.value)
        } else {
            write!(f, "error: {}", self.reason)
        }
    }
}

