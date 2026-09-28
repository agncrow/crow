//! `use frame`: every function but `load` and `save`, which read and
//! write files, is a row of the builtin table (crate::table), its runtime
//! function in `crow_runtime::frames`, which both modes call.
use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;

pub struct FrameModule;

pub(crate) const FRAME_FUNCTIONS: &[&str] = &[
    "load", "save", "select", "filter", "sort", "group", "join", "stack",
    "head", "tail", "fill", "unique", "rename", "stats", "datetime", "tz",
    "new", "with",
];

impl CrowModule for FrameModule {
    fn name(&self) -> &str { "frame" }

    fn functions(&self) -> &[&str] { FRAME_FUNCTIONS }

    fn call(&self, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        // Propagate errors
        for a in args {
            if !a.is_ok() { return Ok(a.clone()); }
        }
        match name {
            "load" => {
                if args.len() != 1 { return Err("load: expected 1 argument (path)".into()); }
                let path = to_str(&args[0], "load")?;
                match crow_runtime::frame::load_csv(path) {
                    Ok(frame) => Ok(CrowValue::ok(Value::Frame(Box::new(frame)))),
                    Err(ref e) => Ok(CrowValue::error_with_kind(e, "frame", "")),
                }
            }
            "save" => {
                if args.len() != 2 { return Err("save: expected 2 arguments (frame, path)".into()); }
                let frame = to_frame(&args[0], "save")?;
                let path = to_str(&args[1], "save")?;
                match frame.save(path) {
                    Ok(()) => Ok(CrowValue::done()),
                    Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
                }
            }
            _ => Err(format!("{}: unknown frame function", name)),
        }
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], _arg_types: &[CrowType]) -> Result<String, String> {
        Ok(match name {
            // load[path] is a reader: an error value on failure (kind 'frame'),
            // stamped with a capture moment on success — same shape as query[].
            "load" => format!(
                "{{ match crow_runtime::frame::load_csv(&{}) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"frame\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
                arg_strs[0]
            ),
            "save" => format!(
                "{{ {}.save(&{}).unwrap_or_else(|e| crow_stop(__crow_line__, format!(\"save: {{}}\", e))); }}",
                arg_strs[0], arg_strs[1]
            ),
            _ => return Err(format!("{}: the Rust backend has no printing for frame function", name)),
        })
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        match name {
            "load" => CrowType::Result(Box::new(CrowType::Frame)),
            "save" => CrowType::Void,
            _ => CrowType::Unknown,
        }
    }
}

fn to_str<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("{}: expected string argument", fname)),
    }
}

fn to_frame<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a crow_runtime::frame::CrowFrame, String> {
    match &v.value {
        Value::Frame(f) => Ok(f),
        _ => Err(format!("{}: expected frame argument, got {}", fname, v.value.type_name())),
    }
}
