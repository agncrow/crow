use crate::value::{CrowValue, Value};
use crate::type_infer::CrowType;
use crate::modules::CrowModule;
use crow_runtime::frame::{CrowFrame, Column, AggFn, parse_datetime, format_datetime};

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
        call_frame(name, args)
    }

    fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
        emit_frame_rust(name, arg_strs, arg_types)
    }

    fn type_of(&self, name: &str, _arg_count: usize) -> CrowType {
        frame_type_of(name)
    }
}

fn to_str<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("{}: expected string argument", fname)),
    }
}

fn to_frame<'a>(v: &'a CrowValue, fname: &str) -> Result<&'a CrowFrame, String> {
    match &v.value {
        Value::Frame(f) => Ok(f),
        _ => Err(format!("{}: expected frame argument, got {}", fname, v.value.type_name())),
    }
}

fn to_int(v: &CrowValue, fname: &str) -> Result<i64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("{}: expected int argument", fname)),
    }
}

/// A Crow value as a frame column: an array of one scalar kind, a
/// tensor, another frame's column, or a scalar (repeated by the caller).
fn value_to_column(v: &CrowValue, fname: &str) -> Result<(Column, bool), String> {
    match &v.value {
        Value::Array(items) => {
            if items.is_empty() { return Ok((Column::Str(vec![]), false)); }
            match &items[0].value {
                Value::Int(_) => Ok((Column::Int(items.iter().map(|x| match &x.value { Value::Int(n) => *n, Value::Float(f) => *f as i64, _ => 0 }).collect()), false)),
                Value::Float(_) => Ok((Column::Float(items.iter().map(|x| match &x.value { Value::Float(f) => *f, Value::Int(n) => *n as f64, _ => f64::NAN }).collect()), false)),
                Value::Bool(_) => Ok((Column::Bool(items.iter().map(|x| matches!(x.value, Value::Bool(true))).collect()), false)),
                Value::Str(_) => Ok((Column::Str(items.iter().map(|x| format!("{}", x.value)).collect()), false)),
                other => Err(format!("{}: cannot make a column of {}", fname, other.type_name())),
            }
        }
        Value::NumArray(t) => Ok((Column::Float(t.to_vec()), false)),
        Value::Int(n) => Ok((Column::Int(vec![*n]), true)),
        Value::Float(x) => Ok((Column::Float(vec![*x]), true)),
        Value::Bool(b) => Ok((Column::Bool(vec![*b]), true)),
        Value::Str(s) => Ok((Column::Str(vec![s.clone()]), true)),
        other => Err(format!("{}: cannot make a column of {}", fname, other.type_name())),
    }
}

fn call_frame(name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
    match name {
        "new" => {
            if !args.is_empty() { return Err("new: takes no arguments — frame.new[] is the empty frame; add columns with with[frame, name, values]".into()); }
            Ok(CrowValue::ok(Value::Frame(Box::new(CrowFrame::new(vec![], vec![], vec![])))))
        }
        "with" => {
            if args.len() != 3 { return Err("with: expected 3 arguments (frame, column name, values)".into()); }
            let f = to_frame(&args[0], "with")?;
            let name = to_str(&args[1], "with")?;
            let (col, scalar) = value_to_column(&args[2], "with")?;
            let out = if scalar { f.with_scalar(name, col) } else { f.with_column(name, col) };
            match out {
                Ok(fr) => Ok(CrowValue::ok(Value::Frame(Box::new(fr)))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
            }
        }
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
        "select" => {
            if args.len() != 2 { return Err("select: expected 2 arguments (frame, columns)".into()); }
            let frame = to_frame(&args[0], "select")?;
            let col_names = extract_str_list(&args[1], "select")?;
            let refs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
            match frame.select(&refs) {
                Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
            }
        }
        "filter" => {
            if args.len() != 2 { return Err("filter: expected 2 arguments (frame, mask)".into()); }
            let frame = to_frame(&args[0], "filter")?;
            let mask = extract_bool_list(&args[1], "filter")?;
            match frame.filter(&mask) {
                Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
                Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
            }
        }
        "sort" => call_sort(args),
        "group" => call_group(args),
        "join" => call_join(args),
        "stack" => call_stack(args),
        "head" => call_head_tail(args, true),
        "tail" => call_head_tail(args, false),
        "fill" => call_fill(args),
        "unique" => call_unique(args),
        "rename" => call_rename(args),
        "stats" => call_stats(args),
        "datetime" => call_datetime(args),
        "tz" => call_tz(args),
        _ => Err(format!("{}: unknown frame function", name)),
    }
}

fn call_sort(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() < 2 || args.len() > 3 { return Err("sort: expected 2-3 arguments (frame, column[, desc])".into()); }
    let frame = to_frame(&args[0], "sort")?;
    let col = to_str(&args[1], "sort")?;
    let desc = if args.len() == 3 {
        match &args[2].value {
            Value::Bool(b) => *b,
            _ => return Err("sort: desc must be a bool — use sort[df, 'col', desc = true]".into()),
        }
    } else { false };
    match frame.sort(col, desc) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_group(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() < 3 { return Err("group: expected at least 3 arguments (frame, group_col, [col, agg]...)".into()); }
    let frame = to_frame(&args[0], "group")?;
    let group_col = to_str(&args[1], "group")?;
    let mut aggs: Vec<(String, AggFn)> = Vec::new();
    for i in 2..args.len() {
        // Each agg arg should be an array of [col_name, agg_name]
        match &args[i].value {
            Value::Array(pair) => {
                if pair.len() != 2 { return Err("group: each agg must be [column, agg]".into()); }
                let col_name = match &pair[0].value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("group: column name must be string".into()),
                };
                let agg_name = match &pair[1].value {
                    Value::Str(s) => s.clone(),
                    _ => return Err("group: agg function must be string".into()),
                };
                let agg_fn = AggFn::from_str(&agg_name)
                    .ok_or_else(|| format!("group: unknown aggregation '{}'", agg_name))?;
                aggs.push((col_name, agg_fn));
            }
            _ => return Err("group: each agg must be [column, agg]".into()),
        }
    }
    let agg_refs: Vec<(&str, AggFn)> = aggs.iter().map(|(s, a)| (s.as_str(), *a)).collect();
    match frame.group(group_col, &agg_refs) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_join(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() < 3 || args.len() > 4 { return Err("join: expected 3-4 arguments (frame1, frame2, key[, type])".into()); }
    let left = to_frame(&args[0], "join")?;
    let right = to_frame(&args[1], "join")?;
    let key = to_str(&args[2], "join")?;
    let jtype = if args.len() == 4 { to_str(&args[3], "join")? } else { "inner" };
    match left.join(right, key, jtype) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_stack(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 2 { return Err("stack: expected 2 arguments (frame1, frame2)".into()); }
    let a = to_frame(&args[0], "stack")?;
    let b = to_frame(&args[1], "stack")?;
    match a.stack(b) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_head_tail(args: &[CrowValue], is_head: bool) -> Result<CrowValue, String> {
    let fname = if is_head { "head" } else { "tail" };
    if args.len() != 2 { return Err(format!("{}: expected 2 arguments (frame, n)", fname)); }
    let frame = to_frame(&args[0], fname)?;
    let n = to_int(&args[1], fname)? as usize;
    let result = if is_head { frame.head(n) } else { frame.tail(n) };
    Ok(CrowValue::ok(Value::Frame(Box::new(result))))
}

fn call_fill(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 3 { return Err("fill: expected 3 arguments (frame, column, value)".into()); }
    let frame = to_frame(&args[0], "fill")?;
    let col = to_str(&args[1], "fill")?;
    let fill_val = format!("{}", args[2].value);
    match frame.fill_column(col, &fill_val) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_unique(args: &[CrowValue]) -> Result<CrowValue, String> {
    // unique[df.col] — receives a column array or a frame+col_name
    // For now: unique[df, col_name]
    if args.len() != 2 { return Err("unique: expected 2 arguments (frame, column)".into()); }
    let frame = to_frame(&args[0], "unique")?;
    let col = to_str(&args[1], "unique")?;
    match frame.unique_col(col) {
        Ok(vals) => {
            let arr: Vec<CrowValue> = vals.into_iter()
                .map(|s| CrowValue::ok(Value::Str(s)))
                .collect();
            Ok(CrowValue::ok(Value::Array(arr)))
        }
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_rename(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 3 { return Err("rename: expected 3 arguments (frame, old, new)".into()); }
    let frame = to_frame(&args[0], "rename")?;
    let old = to_str(&args[1], "rename")?;
    let new = to_str(&args[2], "rename")?;
    match frame.rename(old, new) {
        Ok(f) => Ok(CrowValue::ok(Value::Frame(Box::new(f)))),
        Err(e) => Ok(CrowValue::error_with_kind(&e, "frame", "")),
    }
}

fn call_stats(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 1 { return Err("stats: expected 1 argument (frame)".into()); }
    let frame = to_frame(&args[0], "stats")?;
    let summary = frame.stats_summary();
    Ok(CrowValue::ok(Value::Str(summary)))
}

fn call_datetime(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 1 { return Err("datetime: expected 1 argument (string)".into()); }
    let s = to_str(&args[0], "datetime")?;
    match parse_datetime(s) {
        Some((millis, offset)) => {
            let formatted = format_datetime(millis, offset);
            Ok(CrowValue::ok(Value::Str(formatted)))
        }
        None => Ok(CrowValue::error_with_kind(&format!("datetime: cannot parse '{}'", s), "frame", "")),
    }
}

fn call_tz(args: &[CrowValue]) -> Result<CrowValue, String> {
    if args.len() != 2 { return Err("tz: expected 2 arguments (datetime_str, offset)".into()); }
    let dt_str = to_str(&args[0], "tz")?;
    let offset_str = to_str(&args[1], "tz")?;
    // Parse the datetime
    let (millis, _old_offset) = parse_datetime(dt_str)
        .ok_or_else(|| format!("tz: cannot parse '{}'", dt_str))?;
    // Parse the new offset
    let new_offset: i16 = if offset_str == "UTC" {
        0
    } else if offset_str.len() == 6 && (offset_str.starts_with('+') || offset_str.starts_with('-')) {
        let sign: i16 = if offset_str.starts_with('-') { -1 } else { 1 };
        let oh: i16 = offset_str[1..3].parse().map_err(|_| "tz: invalid offset".to_string())?;
        let om: i16 = offset_str[4..6].parse().map_err(|_| "tz: invalid offset".to_string())?;
        sign * (oh * 60 + om)
    } else {
        return Err(format!("tz: invalid offset '{}', use 'UTC' or '+HH:MM'/'-HH:MM'", offset_str));
    };
    let formatted = format_datetime(millis, new_offset);
    Ok(CrowValue::ok(Value::Str(formatted)))
}


fn extract_str_list(v: &CrowValue, fname: &str) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => {
            let mut result = Vec::new();
            for item in items {
                match &item.value {
                    Value::Str(s) => result.push(s.clone()),
                    _ => return Err(format!("{}: expected array of strings", fname)),
                }
            }
            Ok(result)
        }
        _ => Err(format!("{}: expected array argument", fname)),
    }
}

fn extract_bool_list(v: &CrowValue, fname: &str) -> Result<Vec<bool>, String> {
    match &v.value {
        Value::Array(items) => {
            let mut result = Vec::new();
            for item in items {
                match &item.value {
                    Value::Bool(b) => result.push(*b),
                    _ => return Err(format!("{}: expected array of booleans", fname)),
                }
            }
            Ok(result)
        }
        _ => Err(format!("{}: expected array argument", fname)),
    }
}

fn emit_frame_rust(name: &str, arg_strs: &[String], arg_types: &[CrowType]) -> Result<String, String> {
    // For now, emit simple Rust calls to crow_runtime::frame functions
    let result = match name {
        "new" => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])".to_string(),
        "with" => {
            use crow_runtime::frame::Column as _C;
            let _ = _C::Int(vec![]);
            let vt = arg_types.get(2).cloned().unwrap_or(CrowType::Unknown);
            let vt = if let CrowType::Result(i) = vt { *i } else { vt };
            let (col, scalar) = match &vt {
                CrowType::Array(inner) => match inner.as_ref() {
                    CrowType::Int => (format!("crow_runtime::frame::Column::Int(({}).clone())", arg_strs[2]), false),
                    CrowType::Float => (format!("crow_runtime::frame::Column::Float(({}).clone())", arg_strs[2]), false),
                    CrowType::Bool => (format!("crow_runtime::frame::Column::Bool(({}).clone())", arg_strs[2]), false),
                    _ => (format!("crow_runtime::frame::Column::Str(({}).iter().map(|x| format!(\"{{}}\", x)).collect())", arg_strs[2]), false),
                },
                CrowType::NumArray(_) => (format!("crow_runtime::frame::Column::Float(({}).to_vec())", arg_strs[2]), false),
                CrowType::FrameCol => (format!("(*({}).data).clone()", arg_strs[2]), false),
                CrowType::Int => (format!("crow_runtime::frame::Column::Int(vec![({}) as i64])", arg_strs[2]), true),
                CrowType::Float => (format!("crow_runtime::frame::Column::Float(vec![({}) as f64])", arg_strs[2]), true),
                CrowType::Bool => (format!("crow_runtime::frame::Column::Bool(vec![{}])", arg_strs[2]), true),
                _ => (format!("crow_runtime::frame::Column::Str(vec![format!(\"{{}}\", {})])", arg_strs[2]), true),
            };
            let method = if scalar { "with_scalar" } else { "with_column" };
            format!("{{ match ({}).{}(&{}, {}) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: 0.0 }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"frame\".to_string(), trace: vec![], captured: 0.0 }} }} }}", arg_strs[0], method, arg_strs[1], col)
        }
        // load[path] is a reader: an error value on failure (kind 'frame'),
        // stamped with a capture moment on success — same shape as query[].
        "load" => format!(
            "{{ match crow_runtime::frame::load_csv(&{}) {{ Ok(_f) => CrowResult {{ ok: true, value: _f, reason: String::new(), kind: String::new(), trace: vec![], captured: crow_runtime::moments::now() }}, Err(_e) => CrowResult {{ ok: false, value: crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![]), reason: _e, kind: \"frame\".to_string(), trace: vec![], captured: 0.0 }} }} }}",
            arg_strs[0]
        ),
        "save" => format!(
            "{{ {}.save(&{}).unwrap_or_else(|e| panic!(\"save: {{}}\", e)); }}",
            arg_strs[0], arg_strs[1]
        ),
        "select" => format!(
            "{{ let _strs = {}; let _refs: Vec<&str> = _strs.iter().map(|s: &String| s.as_str()).collect(); {}.select(&_refs).unwrap_or_else(|e| panic!(\"select: {{}}\", e)) }}",
            arg_strs[1], arg_strs[0]
        ),
        "head" => format!("{}.head({} as usize)", arg_strs[0], arg_strs[1]),
        "tail" => format!("{}.tail({} as usize)", arg_strs[0], arg_strs[1]),
        "sort" => {
            if arg_strs.len() == 3 {
                // desc is a bool (lowered from `desc = true`)
                format!("{}.sort(&{}, {}).unwrap_or_else(|e| panic!(\"sort: {{}}\", e))", arg_strs[0], arg_strs[1], arg_strs[2])
            } else {
                format!("{}.sort(&{}, false).unwrap_or_else(|e| panic!(\"sort: {{}}\", e))", arg_strs[0], arg_strs[1])
            }
        }
        "rename" => format!(
            "{}.rename(&{}, &{}).unwrap_or_else(|e| panic!(\"rename: {{}}\", e))",
            arg_strs[0], arg_strs[1], arg_strs[2]
        ),
        "stats" => format!("{}.stats_summary()", arg_strs[0]),
        "stack" => format!(
            "{}.stack(&{}).unwrap_or_else(|e| panic!(\"stack: {{}}\", e))",
            arg_strs[0], arg_strs[1]
        ),
        "filter" => format!(
            "{}.filter(&{}).unwrap_or_else(|e| panic!(\"filter: {{}}\", e))",
            arg_strs[0], arg_strs[1]
        ),
        "fill" => format!(
            "{}.fill_column(&{}, &format!(\"{{}}\", {})).unwrap_or_else(|e| panic!(\"fill: {{}}\", e))",
            arg_strs[0], arg_strs[1], arg_strs[2]
        ),
        "unique" => {
            if arg_strs.len() == 1 {
                // unique[col] — col is already a CrowColumn, call .unique() directly
                format!("{}.unique()", arg_strs[0])
            } else {
                format!(
                    "{}.unique_col(&{}).unwrap_or_else(|e| panic!(\"unique: {{}}\", e))",
                    arg_strs[0], arg_strs[1]
                )
            }
        }
        "join" => {
            let jtype = if arg_strs.len() == 4 {
                format!("&{}", arg_strs[3])
            } else {
                "\"inner\"".to_string()
            };
            format!(
                "{}.join(&{}, &{}, {}).unwrap_or_else(|e| panic!(\"join: {{}}\", e))",
                arg_strs[0], arg_strs[1], arg_strs[2], jtype
            )
        }
        "group" => {
            // group[df, col, [agg_col, agg_fn], ...]
            // Each pair arg (index 2+) is a Vec<String> — extract col name and agg fn
            let mut agg_lines = String::new();
            for (j, i) in (2..arg_strs.len()).enumerate() {
                agg_lines.push_str(&format!(
                    "let _p{j} = {}; _aggs.push((_p{j}[0].as_str(), crow_runtime::frame::AggFn::from_str(&_p{j}[1]).expect(\"invalid agg\"))); ",
                    arg_strs[i]
                ));
            }
            format!(
                "{{ let mut _aggs: Vec<(&str, crow_runtime::frame::AggFn)> = Vec::new(); {} {}.group(&{}, &_aggs).unwrap_or_else(|e| panic!(\"group: {{}}\", e)) }}",
                agg_lines, arg_strs[0], arg_strs[1]
            )
        }
        "datetime" => format!(
            "{{ let (_m, _o) = crow_runtime::frame::parse_datetime(&{}).expect(\"invalid datetime\"); crow_runtime::frame::format_datetime(_m, _o) }}",
            arg_strs[0]
        ),
        "tz" => format!(
            "{{ let (_m, _old_o) = crow_runtime::frame::parse_datetime(&{}).expect(\"tz: invalid datetime\"); \
            let _new_o: i16 = if {} == \"UTC\" {{ 0 }} else {{ \
                let _s = &{}; let _sign: i16 = if _s.starts_with('-') {{ -1 }} else {{ 1 }}; \
                let _parts: Vec<&str> = _s.trim_start_matches('+').trim_start_matches('-').split(':').collect(); \
                let _h: i16 = _parts[0].parse().expect(\"invalid offset hours\"); \
                let _mm: i16 = if _parts.len() > 1 {{ _parts[1].parse().expect(\"invalid offset minutes\") }} else {{ 0 }}; \
                _sign * (_h * 60 + _mm) \
            }}; crow_runtime::frame::format_datetime(_m, _new_o) }}",
            arg_strs[0], arg_strs[1], arg_strs[1]
        ),
        _ => return Err(format!("{}: the Rust backend has no printing for frame function", name)),
    };
    Ok(result)
}

fn frame_type_of(name: &str) -> CrowType {
    match name {
        "load" => CrowType::Result(Box::new(CrowType::Frame)),
        "new" => CrowType::Frame,
        "with" => CrowType::Result(Box::new(CrowType::Frame)),
        "save" => CrowType::Void,
        "select" | "filter" | "sort" | "group" | "join" | "stack"
        | "head" | "tail" | "fill" | "rename" => CrowType::Frame,
        "unique" => CrowType::Array(Box::new(CrowType::Str)),
        "stats" => CrowType::Str,
        "datetime" | "tz" => CrowType::Str,
        _ => CrowType::Unknown,
    }
}
