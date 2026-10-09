//! The vocabularies are total: every function that maps a variant of the
//! one type (crate::ir::Ty), of the interpreter's value (crate::value::Value)
//! or of a row's kind (crate::table::Kind) answers for every variant, by
//! name — a wildcard arm hides the variant that was never thought about
//! (a list of moments compared whole had fallen to `_ => false`). The
//! samples below name every variant; a variant added fails to compile
//! here until it is placed, and each test says what each must answer.

use crate::ir::Ty;
use crate::table::Arg;
use crate::value::{CrowValue, Value};

/// One value of every variant of the type.
fn every_ty() -> Vec<Ty> {
    use std::collections::BTreeMap;
    let cols = crate::type_infer::Cols::closed(BTreeMap::from([("a".to_string(), Ty::Int)]));
    vec![
        Ty::Int, Ty::Float, Ty::Bool, Ty::Str, Ty::Moment, Ty::Bytes, Ty::Unit, Ty::Markup,
        Ty::List(Box::new(Ty::Int)), Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)), Ty::Tensor(Box::new(Ty::Float), None),
        Ty::Frame(None), Ty::Frame(Some(cols)), Ty::Column(Box::new(Ty::Int)), Ty::Json,
        Ty::Record("P".to_string()), Ty::OptRecord("P".to_string()), Ty::Result(Box::new(Ty::Int)),
        Ty::Fn(vec![Ty::Int], Box::new(Ty::Bool)), Ty::Db, Ty::Tx, Ty::Sparse, Ty::WebApp, Ty::WebRequest,
        Ty::WebResponse, Ty::Mailer, Ty::Socket, Ty::Stream, Ty::App, Ty::Node, Ty::Unknown,
    ]
}

/// The variant of a type, by name: a match with no wildcard, so a variant
/// added to the enum fails here until `every_ty` names it too.
fn variant_of(t: &Ty) -> &'static str {
    match t {
        Ty::Int => "Int", Ty::Float => "Float", Ty::Bool => "Bool", Ty::Str => "Str", Ty::Moment => "Moment",
        Ty::Bytes => "Bytes", Ty::Unit => "Unit", Ty::Markup => "Markup", Ty::List(_) => "List", Ty::Map(..) => "Map",
        Ty::Tensor(..) => "Tensor", Ty::Frame(_) => "Frame", Ty::Column(_) => "Column", Ty::Json => "Json",
        Ty::Record(_) => "Record", Ty::OptRecord(_) => "OptRecord", Ty::Result(_) => "Result", Ty::Fn(..) => "Fn",
        Ty::Db => "Db", Ty::Tx => "Tx", Ty::Sparse => "Sparse", Ty::WebApp => "WebApp", Ty::WebRequest => "WebRequest",
        Ty::WebResponse => "WebResponse", Ty::Mailer => "Mailer", Ty::Socket => "Socket", Ty::Stream => "Stream",
        Ty::App => "App", Ty::Node => "Node", Ty::Unknown => "Unknown",
    }
}

/// One of every class of argument a row's kind is asked about.
pub(crate) fn every_arg() -> Vec<Arg> {
    vec![
        Arg::Int, Arg::Float, Arg::Moment, Arg::Moments, Arg::Bytes, Arg::Stream, Arg::Socket, Arg::Mailer, Arg::Chunks,
        Arg::Json, Arg::Tensor, Arg::Ints, Arg::Floats, Arg::Tensors, Arg::Text, Arg::Texts, Arg::Truth, Arg::Truths,
        Arg::Records, Arg::Maps, Arg::Items, Arg::FallibleNumbers, Arg::Fallible, Arg::Column, Arg::Map, Arg::Record,
        Arg::Sparse, Arg::Frame, Arg::Fn, Arg::AnyList, Arg::Unknown,
    ]
}

/// The class, by name, with no wildcard: a class added fails here until
/// `every_arg` names it too.
fn arg_variant(a: Arg) -> &'static str {
    match a {
        Arg::Int => "Int", Arg::Float => "Float", Arg::Moment => "Moment", Arg::Moments => "Moments", Arg::Bytes => "Bytes",
        Arg::Stream => "Stream", Arg::Socket => "Socket", Arg::Mailer => "Mailer", Arg::Chunks => "Chunks", Arg::Json => "Json",
        Arg::Tensor => "Tensor", Arg::Ints => "Ints", Arg::Floats => "Floats", Arg::Tensors => "Tensors", Arg::Text => "Text",
        Arg::Texts => "Texts", Arg::Truth => "Truth", Arg::Truths => "Truths", Arg::Records => "Records", Arg::Maps => "Maps",
        Arg::Items => "Items", Arg::FallibleNumbers => "FallibleNumbers", Arg::Fallible => "Fallible", Arg::Column => "Column",
        Arg::Map => "Map", Arg::Record => "Record", Arg::Sparse => "Sparse", Arg::Frame => "Frame", Arg::Fn => "Fn",
        Arg::AnyList => "AnyList", Arg::Unknown => "Unknown",
    }
}

/// One value of every variant of the interpreter's value that a test can
/// make without a connection, a socket or a session.
fn every_value() -> Vec<Value> {
    let tensor = crow_runtime::tensor::CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
    vec![
        Value::Int(1), Value::Float(1.5), Value::Str("a".into()), Value::Bool(true),
        Value::Moment(crow_runtime::moments::Moment::of_seconds(1.5)),
        Value::Array(vec![CrowValue::ok(Value::Int(1))]), Value::NumArray(Box::new(tensor.clone())),
        Value::Map(std::collections::BTreeMap::from([("k".to_string(), CrowValue::ok(Value::Int(1)))])),
        Value::StructInstance(Box::new(crate::value::StructInstance { name: "P".into(), shown: "P".into(), fields: std::collections::BTreeMap::from([("x".to_string(), CrowValue::ok(Value::Int(1)))]) })),
        Value::Frame(Box::new(crow_runtime::frames::new())),
        Value::Bytes(crow_runtime::sys::Bytes(vec![1u8, 2])),
        Value::Json(Box::new(serde_json::json!({"a": 1}))),
        Value::Dual(1.5, 0.5),
        Value::Done,
    ]
}

/// The variant of a value, by name, with no wildcard.
fn value_variant(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int", Value::Float(_) => "Float", Value::Str(_) => "Str", Value::Bool(_) => "Bool",
        Value::Moment(_) => "Moment", Value::Array(_) => "Array", Value::NumArray(_) => "NumArray", Value::Map(_) | Value::IntMap(_) => "Map",
        Value::Fn(_) => "Fn", Value::StructInstance(_) => "StructInstance", Value::Sparse(_) => "Sparse",
        Value::Frame(_) => "Frame", Value::DbConn(_) => "DbConn", Value::Mailer(_) => "Mailer", Value::Socket(_) => "Socket",
        Value::Bytes(_) => "Bytes", Value::Stream(_) => "Stream", Value::Response(_) => "Response", Value::Json(_) => "Json",
        Value::Dual(..) => "Dual", Value::Traced(_) => "Traced", Value::App(_) => "App", Value::GradFn(_) => "GradFn",
        Value::Done => "Done",
    }
}

/// Every variant of the type has a Rust type, a default and a Crow name;
/// the names tell the variants apart where the programmer could meet them.
#[test]
fn every_type_has_a_rust_type_a_default_and_a_name() {
    let mut names = std::collections::BTreeMap::new();
    for t in every_ty() {
        assert!(!t.to_rust_type().is_empty(), "{} has no Rust type", variant_of(&t));
        assert!(!t.default_rust_value().is_empty(), "{} has no default", variant_of(&t));
        let name = t.crow_name();
        assert!(!name.is_empty(), "{} has no name", variant_of(&t));
        names.entry(name).or_insert_with(Vec::new).push(variant_of(&t));
    }
    // (text and markup share a name, as do a node and markup, a frame and a frame with columns)
    for (name, variants) in &names {
        assert!(variants.len() <= 3, "the name {} is {} variants'", name, variants.join(", "));
    }
}

/// Every type joins with itself to itself, with nothing to itself, and is
/// compatible with itself.
#[test]
fn every_type_joins_with_itself_and_with_nothing() {
    for t in every_ty() {
        assert_eq!(t.unify(&t), t, "{} joined with itself", variant_of(&t));
        assert_eq!(t.unify(&Ty::Unknown), t, "{} joined with nothing", variant_of(&t));
        assert!(t.compatible(&t), "{} is not compatible with itself", variant_of(&t));
    }
}

/// Every type is a dispatch class of the row table, by name: a value a row
/// takes has its class, a handle no row takes is read as the program runs.
#[test]
fn every_type_has_a_dispatch_class() {
    for t in every_ty() {
        let class = Arg::of_type(&t);
        let takes_none = matches!(t, Ty::Db | Ty::Tx | Ty::WebApp | Ty::WebRequest | Ty::WebResponse | Ty::App | Ty::Node | Ty::Unit | Ty::Unknown);
        assert_eq!(class == Arg::Unknown, takes_none, "{} is {:?}", variant_of(&t), class);
    }
}

/// The projection inference reads a type through forgets a column's cells
/// and a view's node and nothing else.
#[test]
fn the_inferred_view_of_a_type_forgets_only_cells_and_nodes() {
    for t in every_ty() {
        let seen = crate::lower_ir::as_inferred(&t);
        match t {
            Ty::Column(_) => assert_eq!(seen, Ty::Column(Box::new(Ty::Unknown))),
            Ty::Node => assert_eq!(seen, Ty::Unknown),
            _ => assert_eq!(seen, t, "{} changed", variant_of(&t)),
        }
    }
}

/// Every value a test can make is equal to itself — a data value by its
/// content; a handle, decided by name in `values_equal`, is equal to
/// nothing — and has a type name and a comparison cell.
#[test]
fn every_value_is_equal_to_itself_and_has_a_name() {
    for v in every_value() {
        let a = CrowValue::ok(v.clone());
        assert!(crate::ops::values_equal(&a, &a), "{} is not equal to itself", value_variant(&v));
        let name = crate::builtins::call("type", &[a.clone()], "t:1").expect("type[] answers").expect("a name");
        assert!(matches!(name.value, Value::Str(ref s) if !s.is_empty()), "{} has no type name", value_variant(&v));
        let _ = crate::ops::field(a.clone(), "ok").expect("every value has .ok");
    }
}

/// Every class of argument is named once.
#[test]
fn every_class_of_argument_is_named() {
    let all = every_arg();
    let names: std::collections::BTreeSet<&str> = all.iter().map(|a| arg_variant(*a)).collect();
    assert_eq!(names.len(), all.len());
}

/// Whether a type holds an unknown, and whether it is concrete, are
/// answered by the type — the same answers the type's debug text had
/// given at the sites that read it.
#[test]
fn unknown_inside_and_concrete_are_what_the_text_said() {
    let mut types = every_ty();
    types.push(Ty::List(Box::new(Ty::Unknown)));
    types.push(Ty::Column(Box::new(Ty::Unknown)));
    types.push(Ty::Result(Box::new(Ty::List(Box::new(Ty::Frame(None))))));
    types.push(Ty::Fn(vec![Ty::Unknown], Box::new(Ty::Int)));
    types.push(Ty::Tensor(Box::new(Ty::Unknown), Some(vec![None])));
    let open = crate::type_infer::Cols::closed(std::collections::BTreeMap::from([("a".to_string(), Ty::Unknown)]));
    types.push(Ty::Frame(Some(open)));
    for t in &types {
        let shown = format!("{:?}", t);
        assert_eq!(t.has_unknown(), shown.contains("Unknown"), "{}", shown);
        assert_eq!(t.is_concrete(), !shown.contains("Unknown") && !shown.contains("Frame(None)"), "{}", shown);
    }
}
