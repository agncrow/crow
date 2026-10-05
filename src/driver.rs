//! The interpreter's side of a database driver written in Crow
//! (docs/db-in-crow-design.md): `crow_db::driven` calls the driver through
//! a table of calls, and the interpreter's table calls the driver's
//! functions in a child interpreter over the program's functions — a
//! child, since the call arrives inside a builtin the interpreter is
//! evaluating. A thread without a child (a parallel loop's worker, the
//! compiler's tooling) makes one from the program the interpreter loaded.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crow_db::driven::{Calls, Failure};
use crow_runtime::frame::CrowFrame;
use crow_runtime::sys::CrowStream;

use crate::ir_interp::IrInterp;
use crate::value::{CrowValue, Value};

/// What a child interpreter is made from: the loaded program's functions.
pub struct Template {
    pub(crate) fns: HashMap<String, Arc<crate::ir::Fn>, crate::ir_interp::Names>,
    pub(crate) structs: HashMap<String, Arc<crate::ir::StructDef>>,
    pub(crate) registry: crate::modules::ModuleRegistry,
    pub(crate) file: String,
}

static TEMPLATE: Mutex<Option<Arc<Template>>> = Mutex::new(None);

/// The backends whose driver the program carries, by the functions it has.
fn backends_in(fns: &HashMap<String, Arc<crate::ir::Fn>, crate::ir_interp::Names>) -> Vec<&'static str> {
    [("clickhouse", "_db_clickhouse_ch_query")].iter().filter(|(_, f)| fns.contains_key(*f)).map(|(b, _)| *b).collect()
}

/// The interpreter loaded a program: what it carries is what a child is
/// made from, on any thread that needs one.
pub(crate) fn set_template(t: Template) {
    if backends_in(&t.fns).is_empty() {
        return;
    }
    *TEMPLATE.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(t));
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crow_db::driven::register_maker(Arc::new(|backend: &str| {
            let t = TEMPLATE.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
            if !backends_in(&t.fns).contains(&backend) {
                return None;
            }
            let prefix = match backend {
                "clickhouse" => "_db_clickhouse_ch",
                _ => return None,
            };
            let child = IrInterp::from_template(&t);
            Some(Rc::new(InterpCalls { interp: RefCell::new(child), prefix }) as Rc<dyn Calls>)
        }));
    });
}

/// The compiler's tooling (`crow db diff`, `crow db describe`) has no
/// program: a child over `use db` alone, which carries every driver.
pub(crate) fn tooling_ready() -> Result<(), String> {
    if TEMPLATE.lock().unwrap_or_else(|p| p.into_inner()).is_some() {
        return Ok(());
    }
    let stmts = crate::build("use db\n").map_err(|e| e.to_string())?;
    let stmts = crate::lower::lower_program(&stmts)?;
    let (ctx, program) = crate::infer_and_lower(&stmts, false, "db")?;
    let mut it = IrInterp::new(&ctx.registry, "db.co");
    it.load(&program);
    Ok(())
}

/// The driver's calls, each the driver's function in the child.
struct InterpCalls {
    interp: RefCell<IrInterp>,
    prefix: &'static str,
}

impl InterpCalls {
    fn call(&self, name: &str, vals: Vec<CrowValue>) -> Result<CrowValue, Failure> {
        let mut it = self.interp.try_borrow_mut().map_err(|_| Failure::new("the driver is busy on this thread", false))?;
        let r = it.call_user(&format!("{}_{}", self.prefix, name), vals).map_err(|e| Failure::new(e, false))?;
        if r.is_error() {
            return Err(Failure::new(r.reason(), r.kind() == "sys"));
        }
        Ok(r)
    }

    fn text(s: &str) -> CrowValue {
        CrowValue::ok(Value::Str(s.to_string()))
    }
}

impl Calls for InterpCalls {
    fn open(&self, parts: &str) -> Result<CrowStream, Failure> {
        match self.call("open", vec![Self::text(parts)])?.value {
            Value::Stream(s) => Ok(s),
            other => Err(Failure::new(format!("the driver's open answered {}", other.type_name()), false)),
        }
    }

    fn check(&self, parts: &str, s: &CrowStream) -> Result<(), Failure> {
        self.call("check", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone()))]).map(|_| ())
    }

    fn query(&self, parts: &str, s: &CrowStream, sql: &str, params: &str) -> Result<CrowFrame, Failure> {
        match self.call("query", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone())), Self::text(sql), Self::text(params)])?.value {
            Value::Frame(f) => Ok(*f),
            other => Err(Failure::new(format!("the driver's query answered {}", other.type_name()), false)),
        }
    }

    fn exec(&self, parts: &str, s: &CrowStream, sql: &str, params: &str) -> Result<i64, Failure> {
        match self.call("exec", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone())), Self::text(sql), Self::text(params)])?.value {
            Value::Int(n) => Ok(n),
            other => Err(Failure::new(format!("the driver's exec answered {}", other.type_name()), false)),
        }
    }

    fn insert(&self, parts: &str, s: &CrowStream, table: &str, frame: &CrowFrame) -> Result<i64, Failure> {
        match self.call("insert", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone())), Self::text(table), CrowValue::ok(Value::Frame(Box::new(frame.clone())))])?.value {
            Value::Int(n) => Ok(n),
            other => Err(Failure::new(format!("the driver's insert answered {}", other.type_name()), false)),
        }
    }

    fn describe(&self, parts: &str, s: &CrowStream, sql: &str) -> Result<CrowFrame, Failure> {
        match self.call("describe", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone())), Self::text(sql)])?.value {
            Value::Frame(f) => Ok(*f),
            other => Err(Failure::new(format!("the driver's describe answered {}", other.type_name()), false)),
        }
    }

    fn cancel(&self, parts: &str, s: &CrowStream) -> Result<(), Failure> {
        self.call("cancel", vec![Self::text(parts), CrowValue::ok(Value::Stream(s.clone()))]).map(|_| ())
    }
}
