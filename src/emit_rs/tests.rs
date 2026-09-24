//! The Rust backend's tests: what the generated program says for each
//! shape of Crow — carried over from the transpiler it replaced, each
//! checking the same meaning in the emitter's output.

use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_infer::TypeContext;

/// A handler reads a derivation again after a write to what it reads —
/// a helper's or its own — as the interpreter does (spec, Derivations,
/// rule 1). The compiled handler read each derivation once at its
/// start, so finch's hop said "RSI crosses 70 at 09:26:49" while the
/// clock it had just moved showed 14:27:00.
#[test]
fn a_handler_reads_a_derivation_after_a_write() {
    let src = "use web, ui\n\nbase = 1.0\nmsg = ''\n\nderive twice\n    return base * 2.0\n\nfn move_to[t]\n    base = t\n\nfn go[]\n    move_to[5.0]\n    msg = 'twice is `twice`'\n\nfn go_direct[]\n    base = 7.0\n    msg = 'twice is `twice`'\n\nview render[]\n    return stack[p[msg], button['Go', click = go], button['Direct', click = go_direct]]\n\nserve[3997]\npage['/', render]\n";
    let rust = transpile(src);
    let body = |name: &str| -> String {
        let start = rust.find(&format!("fn {}(state", name)).unwrap_or_else(|| panic!("{} is a handler:\n{}", name, rust));
        let rest = &rust[start..];
        rest[..rest.find("\n}\n").unwrap()].to_string()
    };
    let go = body("go");
    let call = go.find("move_to(state").expect("the helper call");
    let reread = go.rfind("twice = __derive_twice(state);").expect("twice read again");
    let used = go.find("format!(\"twice is {}\", twice)").expect("the message");
    assert!(call < reread && reread < used, "twice is read again after the helper, before the message:\n{}", go);
    let direct = body("go_direct");
    let write = direct.find("base = 7.0").expect("the handler's own write");
    let flushed = direct.find("state.set(\"base\"").expect("the write lands before the read");
    let reread = direct.rfind("twice = __derive_twice(state);").expect("twice read again");
    let used = direct.find("format!(\"twice is {}\", twice)").expect("the message");
    assert!(write < flushed && flushed < reread && reread < used, "the own write lands, then twice is read again:\n{}", direct);
}

/// A handler's loop reads its derivations before the loop: a parallel
/// body runs in a closure, which reads nothing again (it writes no
/// fact, so nothing it reads can move), and so does a collecting
/// loop's. A read inside the closure would assign a captured local and
/// borrow the state inside it — a build error.
#[test]
fn a_handler_loop_reads_its_derivations_before_the_loop() {
    let src = "use web, ui\n\nbase = 1.0\nmsg = ''\nitems = [1, 2, 3]\n\nderive twice\n    return base * 2.0\n\nfn go[]\n    base = 5.0\n    for i in items\n        y = i * twice\n    out = for i in items\n        i * twice\n    msg = '`len[out]`'\n\nview render[]\n    return stack[p[msg], button['Go', click = go]]\n\nserve[3996]\npage['/', render]\n";
    let rust = transpile(src);
    let start = rust.find("fn go(state").expect("go is a handler");
    let rest = &rust[start..];
    let go = &rest[..rest.find("\n}\n").unwrap()];
    assert!(go.contains("par_iter().for_each("), "the loop is parallel:\n{}", go);
    let rereads: Vec<&str> = go.lines().filter(|l| l.contains("twice = __derive_twice(state);") && !l.contains("let mut")).collect();
    assert_eq!(rereads.len(), 2, "read again before the loop and before the collecting loop:\n{}", go);
    for l in &rereads {
        assert!(l.starts_with("    twice") , "read at the handler's level, never inside a closure: {:?}\n{}", l, go);
    }
}

/// A view reads its derivations into locals up front; handing one to a
/// call clones it like any local, or the second call finds it moved (finch's
/// clock tile, a build error)
#[test]
fn a_derivation_read_up_front_is_cloned_into_a_call() {
    let src = "use web, ui\n\nbase = 'x'\n\nderive label\n    return '`base`!'\n\nview stat[name, value]\n    return p['`name`: `value`']\n\nview render[]\n    tiles = [stat['a', label]]\n    if len[tiles] gt 0\n        tiles = [stat['a', label], stat['b', label]]\n    return stack[tiles]\n\nserve[3996]\npage['/', render]\n";
    let rust = transpile(src);
    let start = rust.find("fn render(state").expect("render is a view");
    let rest = &rust[start..];
    let render = &rest[..rest.find("\n}\n").unwrap()];
    assert!(render.contains("let label = __derive_label(state);"), "read up front:\n{}", render);
    assert_eq!(render.matches("\"a\".to_string(), label.clone())").count(), 2, "cloned at each call:\n{}", render);
}

/// A collecting loop whose condition can hand an error back gathers in a
/// loop inside the function, which returns the error: a closure cannot
/// return from the function (it did not build), and the function's value
/// is a result because the loop's condition can fail
#[test]
fn a_collecting_loop_that_can_return_gathers_in_a_loop() {
    let rust = transpile("fn parse_all[xs]\n    out = for x in xs\n        if int[x] gt 0\n            x\n    return out\nlog[parse_all[['1', 'a']]]\n");
    let start = rust.find("fn parse_all(").expect("parse_all");
    let rest = &rust[start..];
    let body = &rest[..rest.find("\n}\n").unwrap()];
    assert!(body.contains("-> CrowResult<Vec<String>>"), "{}", body);
    assert!(body.contains("for x in xs.iter() {"), "a loop, not a closure:\n{}", body);
    assert!(body.contains("return CrowResult { ok: false"), "the error goes back:\n{}", body);
    assert!(!body.contains(".map(|"), "{}", body);
}

/// What a list or a map keeps is its own: a variable read is cloned into
/// push[] and into set[]'s key, so the variable stays usable and a loop's
/// borrowed variable becomes a value
#[test]
fn a_collection_keeps_a_clone_of_a_variable() {
    let rust = transpile("out = []\nfor w in ['a', 'b']\n    out = push[out, w]\nm = map[]\nfor k in ['p', 'q']\n    m = set[m, k, 1]\nlog[out]\nlog[m]\n");
    assert!(rust.contains("crow_runtime::core::push(out, (w).clone())"), "{}", rust);
    assert!(rust.contains("crow_runtime::core::map_set(m, (k).clone(), 1_i64)"), "{}", rust);
}

/// A control renders what it is given, as an element and a view do: a
/// failed argument is its reason in place of the control (it was a result
/// where the page wanted a node, and the program did not build)
#[test]
fn a_control_renders_a_failed_argument_as_its_reason() {
    let src = "use web, ui\n\npick = 'a'\n\nderive options\n    r = read['missing.txt']\n    return split[r, ',']\n\nview render[]\n    return stack[select[options, pick]]\n\nserve[3996]\npage['/', render]\n";
    let rust = transpile(src);
    let start = rust.find("fn render(state").expect("render");
    let rest = &rust[start..];
    let render = &rest[..rest.find("\n}\n").unwrap()];
    assert!(render.contains("CrowVNode::text(&format!(\"error: {}\", _va0.reason))"), "{}", render);
}

/// A loop over a list that can fail is refused like a condition: the
/// function returns the error before the loop, and the loop walks the value
#[test]
fn a_loop_over_a_list_that_can_fail_returns_the_error_first() {
    let rust = transpile("fn count[s]\n    n = 0\n    for i in range[int[s]]\n        n = n + 1\n    return n\nlog[count['3']]\n");
    let start = rust.find("fn count(").expect("count");
    let rest = &rust[start..];
    let body = &rest[..rest.find("\n}\n").unwrap()];
    assert!(body.contains("-> CrowResult<i64>"), "{}", body);
    let refused = body.find("return CrowResult { ok: false").expect("refused before the loop");
    let looped = body.find(".value.iter()").expect("the loop walks the value");
    assert!(refused < looped, "{}", body);
}

/// A loop over an error at the top level halts with the interpreter's
/// words: `cannot loop over an error value`, not the branch's
#[test]
fn a_top_level_loop_over_an_error_halts_with_the_loop_message() {
    let rust = transpile("for i in range[int['x']]\n    log[i]\n");
    assert!(rust.contains("runtime error: line 1: cannot loop over an error value — {}"), "{}", rust);
}

/// A parallel collecting loop that yields on some paths only, after
/// statements of its own, gathers per iteration and joins the lists in
/// input order (a closure's `if` without `else` did not build)
#[test]
fn a_parallel_loop_that_skips_gathers_per_iteration() {
    let rust = transpile("out = for x in [1, 2, 3]\n    y = x * 2\n    if y gt 2\n        y\nlog[out]\n");
    assert!(rust.contains(".par_iter().flat_map_iter(|&x| {"), "{}", rust);
    assert!(rust.contains("= crow_runtime::core::push(_yielded_"), "{}", rust);
}

/// A guard narrows: after `if not t.ok` / `return`, t is its value, so
/// the function is not typed as able to fail and len[t] is a number read
/// from the value, with nothing to propagate
#[test]
fn a_narrowed_read_is_the_value() {
    let rust = transpile("fn count[path]\n    t = read[path]\n    if not t.ok\n        return 0\n    return len[t]\nlog[count['x.txt']]\n");
    let start = rust.find("fn count(").expect("count");
    let rest = &rust[start..];
    let body = &rest[..rest.find("\n}\n").unwrap()];
    assert!(body.contains(") -> i64 {"), "{}", body);
    assert!(body.contains("(t).value"), "{}", body);
    assert!(!body.contains("_prop_"), "nothing propagates: {}", body);
}

/// An error made by error[…] has no value: comparing it with a number
/// hands its error on, and the value path is unreachable (it was typed as
/// text, and comparing text with a number did not build)
#[test]
fn an_error_without_a_value_meets_a_number() {
    let rust = transpile("e = error['boom']\nc = e gt 0\nlog[c.ok]\n");
    assert!(rust.contains("let e: CrowResult<()> = CrowResult { ok: false, value: ()"), "{}", rust);
    assert!(rust.contains("unreachable!()"), "{}", rust);
}

/// A branch on an error at the top level halts with the interpreter's
/// words and the line (spec, Errors in Conditions); the compiled program
/// said `error: cannot branch…` and no line
#[test]
fn a_top_level_branch_on_an_error_halts_with_the_line() {
    let rust = transpile("z = int['f']\nif z gt 1\n    log['never']\n");
    assert!(rust.contains("runtime error: line 2: cannot branch on an error value — {}"), "{}", rust);
}

/// Helper: parse Crow source, infer, lower to the IR, emit Rust
fn transpile(source: &str) -> String {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("lexer failed");
    let mut parser = Parser::new(tokens);
    let stmts = parser.parse().expect("parser failed");
    // Same pipeline as the real build: lower named args before inference
    let stmts = crate::lower::lower_program(&stmts).expect("lowering failed");
    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowering to the IR failed");
    crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false).expect("emitting failed").source
}

#[test]
fn test_multi_assign() {
    let src = "x, y = 10, 20\nlog['{x} {y}']\n";
    let rust = transpile(src);
    assert!(rust.contains("let x: i64 = 10"), "should declare x\n{}", rust);
    assert!(rust.contains("let y: i64 = 20"), "should declare y\n{}", rust);
}

/// a, b = b, a reads both values before it writes either
#[test]
fn test_swap_uses_temporaries() {
    let src = "a = 1\nb = 2\na, b = b, a\nlog['{a} {b}']\n";
    let rust = transpile(src);
    let read_b = rust.find("= b;").expect("b is read into a temporary");
    let read_a = rust.find("= a;").expect("a is read into a temporary");
    let write_a = rust.find("a = t").expect("a is written from a temporary");
    assert!(read_b < write_a && read_a < write_a, "both are read before either is written\n{}", rust);
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

/// cpu[1] runs the loops that follow in order; with more, a loop that
/// writes nothing outside it and reaches no effect runs in parallel
#[test]
fn test_cpu_count_wiring() {
    let par = transpile("xs = [1, 2, 3]\nys = for x in xs\n    x * 2\nlog[len[ys]]\n");
    assert!(par.contains("par_iter"), "{}", par);
    let seq = transpile("cpu[1]\nxs = [1, 2, 3]\nys = for x in xs\n    x * 2\nlog[len[ys]]\n");
    assert!(!seq.contains("par_iter"), "{}", seq);
}

#[test]
fn test_read_returns_crow_result() {
    let src = "content = read['/tmp/crow_test_read.txt']\nlog['{content}']\n";
    let rust = transpile(src);
    assert!(rust.contains("CrowResult"), "read[] should return CrowResult\n{}", rust);
    assert!(!rust.contains(".unwrap()"), "read[] should not use .unwrap()\n{}", rust);
}

#[test]
fn independent_statements_run_in_order() {
    // Statements run in the order written (spec §14, Design Note
    // "Statements Run in Order"): no rayon::join, and `a` before `b`.
    let src = "a = 1 + 2\nb = 3 + 4\nc = 5 + 6\n";
    let rust = transpile(src);
    assert!(!rust.contains("rayon::join"), "no statement waves\n{}", rust);
    let ia = rust.find("a: i64").expect("a is declared");
    let ib = rust.find("b: i64").expect("b is declared");
    assert!(ia < ib, "a must be emitted before b\n{}", rust);
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
    assert!(rust.contains("ok: bool"), "CrowResult should have ok field\n{}", rust);
    assert!(rust.contains("value: T"), "CrowResult should have value field\n{}", rust);
    assert!(rust.contains("reason: String"), "CrowResult should have reason field\n{}", rust);
    assert!(rust.contains("kind: String"), "CrowResult should have kind field\n{}", rust);
    assert!(rust.contains("trace: Vec<String>"), "CrowResult should have trace field\n{}", rust);
}

#[test]
fn test_crow_result_not_emitted_when_unnecessary() {
    // CrowResult is now always emitted (allow(dead_code)) — flag-timing
    // made conditional emission unreliable. Assert no Result USAGE instead.
    let src = "x = 5\nlog[x]\n";
    let rust = transpile(src);
    assert!(!rust.contains("CrowResult {"), "no Result values should be constructed\n{}", rust);
}

#[test]
fn test_ok_on_raw_type() {
    let src = "x = 5\nlog[x.ok]\n";
    let rust = transpile(src);
    assert!(rust.contains("true"), "ok on raw type should return true\n{}", rust);
}

#[test]
fn test_reason_field_access() {
    let src = "x = int['42']\nlog[x.reason]\n";
    let rust = transpile(src);
    assert!(rust.contains(".reason"), "should access .reason field\n{}", rust);
}

#[test]
fn test_ok_field_access_on_result() {
    let src = "x = int['42']\nlog[x.ok]\n";
    let rust = transpile(src);
    assert!(rust.contains(".ok"), "should access .ok field on CrowResult\n{}", rust);
}

#[test]
fn test_auto_unwrap_in_binop() {
    let src = "x = int['42']\ny = x + 1\n";
    let rust = transpile(src);
    // Should generate propagation block that checks .ok and propagates errors
    assert!(rust.contains("_binl_"), "should generate binop propagation temp vars\n{}", rust);
    assert!(rust.contains(".ok"), "should check ok field for propagation\n{}", rust);
    assert!(rust.contains(".value"), "should access .value for success case\n{}", rust);
}

#[test]
fn test_binop_propagation_right_operand() {
    let src = "x = int['42']\ny = 10 + x\n";
    let rust = transpile(src);
    // Right operand is CrowResult, should generate propagation
    assert!(rust.contains("_binr_"), "should generate binop right propagation temp var\n{}", rust);
    assert!(rust.contains(".ok"), "should check ok field\n{}", rust);
}

#[test]
fn test_binop_no_propagation_plain() {
    let src = "x = 5\ny = x + 10\n";
    let rust = transpile(src);
    // No CrowResult operands, should NOT generate propagation
    assert!(!rust.contains("_binl_"), "should not generate binop propagation for plain types\n{}", rust);
    assert!(!rust.contains("_binr_"), "should not generate binop propagation for plain types\n{}", rust);
}

#[test]
fn test_binop_result_type_is_crow_result() {
    let src = "x = int['42']\ny = x + 1\nlog[y.ok]\n";
    let rust = transpile(src);
    // y should be CrowResult since x is CrowResult
    assert!(rust.contains("y.ok"), "y should be CrowResult with .ok field\n{}", rust);
}

#[test]
fn test_int_from_float_no_result() {
    let src = "x = 3.14\ny = int[x]\n";
    let rust = transpile(src);
    assert!(!rust.contains("CrowResult {"), "int from float should not construct a Result\n{}", rust);
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

/// A loop is parallel when its body writes nothing outside it and reaches
/// no effect; a body that logs runs in order, so its lines print in order
#[test]
fn test_for_loop_parallel_by_default() {
    let src = "items = [1, 2, 3]\nfor x in items\n    y = x * 2\n";
    let rust = transpile(src);
    assert!(rust.contains("par_iter()"), "for loop should use par_iter by default\n{}", rust);
    assert!(rust.contains("for_each"), "parallel for should use for_each\n{}", rust);
    assert!(rust.contains("use rayon::prelude::*"), "should import rayon prelude\n{}", rust);
    let logs = transpile("items = [1, 2, 3]\nfor x in items\n    log['{x}']\n");
    assert!(!logs.contains("par_iter()") && logs.contains("for &x in"), "a loop that logs runs in order\n{}", logs);
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
fn test_error_trace_field_access() {
    let src = "x = int['bad']\ny = x.trace\n";
    let rust = transpile(src);
    assert!(rust.contains(".trace"), "should access .trace field on CrowResult\n{}", rust);
}

#[test]
fn test_raw_value_kind_is_empty() {
    let src = "x = 5\ny = x.kind\n";
    let rust = transpile(src);
    assert!(rust.contains("String::new()"), "raw value .kind should be empty string\n{}", rust);
}

#[test]
fn test_auto_propagation_uses_if_else() {
    let src = "fn double[x]\n    return x * 2\nbad = int['hello']\nresult = double[bad]\n";
    let rust = transpile(src);
    assert!(rust.contains("_prop_"), "should generate propagation temp vars\n{}", rust);
    assert!(rust.contains("!_prop_") || rust.contains(".ok"), "should check ok field for propagation\n{}", rust);
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
    assert!(rust.contains("!self.ok"), "Display impl should check ok field\n{}", rust);
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
    let src = "n = json['null']\nlog[n.ok]\n";
    let rust = transpile(src);
    assert!(rust.contains("is_null()"), "json[] should check for null\n{}", rust);
    assert!(rust.contains("ok: false"), "null should map to ok: false\n{}", rust);
}

#[test]
fn test_encode_builtin() {
    let src = "items = json['[1,2,3]']\ns = encode[items]\nlog[s]\n";
    let rust = transpile(src);
    assert!(rust.contains(".to_json()"), "encode[] goes through the runtime's ToJson\n{}", rust);
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

// ===== §1: Variables & Types =====

#[test]
fn test_var_int_transpile() {
    let rust = transpile("x = 42\n");
    assert!(rust.contains("let x: i64 = 42"), "int var\n{}", rust);
}

#[test]
fn test_var_float_transpile() {
    let rust = transpile("x = 3.14\n");
    assert!(rust.contains("let x: f64 = 3.14"), "float var\n{}", rust);
}

#[test]
fn test_var_string_transpile() {
    let rust = transpile("x = 'hello'\n");
    assert!(rust.contains("let x: String"), "string var\n{}", rust);
}

#[test]
fn test_var_bool_transpile() {
    let rust = transpile("x = true\n");
    assert!(rust.contains("let x: bool = true"), "bool var\n{}", rust);
}

#[test]
fn test_var_list_transpile() {
    let rust = transpile("x = [1, 2, 3]\n");
    assert!(rust.contains("Vec"), "array should use Vec\n{}", rust);
}

#[test]
fn test_string_interpolation_transpile() {
    let rust = transpile("name = 'crow'\nx = 'hello {name}'\n");
    assert!(rust.contains("format!"), "interpolation should use format!\n{}", rust);
}

// ===== §3: Functions =====

#[test]
fn test_fn_declaration_transpile() {
    let rust = transpile("fn add[a, b]\n    a + b\nlog[add[1, 2]]\n");
    assert!(rust.contains("fn add("), "should declare function\n{}", rust);
}

#[test]
fn test_fn_return_transpile() {
    let rust = transpile("fn greet[]\n    return 'hello'\nx = greet[]\n");
    assert!(rust.contains("return"), "should have return\n{}", rust);
}

// ===== §4: Control Flow =====

#[test]
fn test_if_else_transpile() {
    let rust = transpile("x = 5\nif x gt 3\n    log['big']\nelse\n    log['small']\n");
    assert!(rust.contains("if"), "should have if\n{}", rust);
    assert!(rust.contains("else"), "should have else\n{}", rust);
}

// ===== §5: Loops =====

#[test]
fn test_while_transpile() {
    let rust = transpile("x = 5\nwhile x gt 0\n    x = x - 1\n");
    assert!(rust.contains("while") || rust.contains("loop"), "should have loop\n{}", rust);
}

#[test]
fn test_while_break_transpile() {
    let rust = transpile("x = 0\nwhile true\n    x = x + 1\n    if x eq 5\n        break\n");
    assert!(rust.contains("break"), "should have break\n{}", rust);
}

#[test]
fn test_for_expr_transpile() {
    let rust = transpile("doubled = for x in [1, 2, 3]\n    x * 2\n");
    assert!(rust.contains("collect"), "for-expr should collect results\n{}", rust);
}

// ===== §7: Structs =====

#[test]
fn test_struct_transpile() {
    let rust = transpile("struct Bird [name, speed]\nb = Bird['crow', 50]\n");
    assert!(rust.contains("struct Bird"), "should declare struct\n{}", rust);
}

#[test]
fn test_struct_field_transpile() {
    let rust = transpile("struct Bird [name, speed]\nb = Bird['crow', 50]\nlog[b.name]\n");
    assert!(rust.contains(".name"), "should access field\n{}", rust);
}

// ===== §8: Operators =====

#[test]
fn test_comparison_operators_transpile() {
    let rust = transpile("x = 5\ny = x gt 3\n");
    assert!(rust.contains(">"), "gt should map to >\n{}", rust);
}

#[test]
fn test_logical_operators_transpile() {
    // both sides are evaluated, the left first (spec, Logical): Rust's
    // `&` and `|` on booleans, never the short-circuiting `&&` and `||`
    let rust = transpile("x = true and false\ny = x or true\n");
    assert!(rust.contains("true & false"), "and evaluates both sides\n{}", rust);
    assert!(rust.contains("x | true"), "or evaluates both sides\n{}", rust);
    for l in rust.lines().filter(|l| l.contains("let x") || l.contains("let y")) {
        assert!(!l.contains("&&") && !l.contains("||"), "{}", l);
    }
}

#[test]
fn test_expression_grouping_transpile() {
    let rust = transpile("x = (1 + 2) * 3\n");
    assert!(rust.contains("(1") && rust.contains("* 3"), "should preserve grouping\n{}", rust);
}

// ===== Builtins =====

#[test]
fn test_range_transpile() {
    let rust = transpile("x = range[5]\n");
    assert!(rust.contains("0..5") || rust.contains("range"), "should generate range\n{}", rust);
}

#[test]
fn test_len_transpile() {
    let rust = transpile("x = [1, 2, 3]\ny = len[x]\n");
    assert!(rust.contains(".len()"), "should use .len()\n{}", rust);
}

#[test]
fn test_push_transpile() {
    let rust = transpile("x = [1, 2]\ny = push[x, 3]\n");
    assert!(rust.contains("push"), "should use push\n{}", rust);
}

#[test]
fn test_sort_transpile() {
    let rust = transpile("x = [3, 1, 2]\ny = sort[x]\n");
    assert!(rust.contains("sort"), "should use sort\n{}", rust);
}

#[test]
fn test_map_operations_transpile() {
    let rust = transpile("m = map[]\nm = set[m, 'a', 1]\n");
    assert!(rust.contains("BTreeMap") || rust.contains("HashMap"), "should use map type\n{}", rust);
}

#[test]
fn test_error_builtin_transpile() {
    let rust = transpile("x = error['something went wrong']\n");
    assert!(rust.contains("ok: false"), "error[] should create error value\n{}", rust);
}

#[test]
fn test_math_sqrt_transpile() {
    let rust = transpile("x = sqrt[16]\n");
    assert!(rust.contains("sqrt"), "should use sqrt\n{}", rust);
}

#[test]
fn test_string_upper_transpile() {
    // the one implementation both modes call
    let rust = transpile("x = upper['hello']\n");
    assert!(rust.contains("crow_runtime::text::upper("), "should call the runtime's upper\n{}", rust);
}

/// A negative count is refused by the runtime's message, as the
/// interpreter refuses it, where `repeat(n as usize)` made the count huge
#[test]
fn repeat_goes_through_the_runtime() {
    let rust = transpile("n = int['-2']\nx = repeat['ab', n]\nlog[x]\n");
    assert!(rust.contains("crow_runtime::text::repeat(") && rust.contains("as i64"), "{}", rust);
    assert!(!rust.contains("as usize"), "{}", rust);
}

#[test]
fn test_string_split_transpile() {
    let rust = transpile("x = split['a,b', ',']\n");
    assert!(rust.contains("split"), "should use split\n{}", rust);
}

#[test]
fn test_contains_transpile() {
    let rust = transpile("x = [1, 2, 3]\ny = contains[x, 2]\n");
    assert!(rust.contains("contains") || rust.contains("any"), "should check contains\n{}", rust);
}

#[test]
fn test_then_chaining_transpile() {
    let rust = transpile("x = '  hello  '\ny = x then trim\n");
    assert!(rust.contains("trim"), "then should desugar to function call\n{}", rust);
}

#[test]
fn test_negative_index_transpile() {
    let rust = transpile("nums = [10, 20, 30]\nx = nums[-1]\n");
    assert!(rust.contains("if _i < 0"), "should emit negative index handling\n{}", rust);
}

#[test]
fn test_negative_index_assign_transpile() {
    let rust = transpile("nums = [10, 20, 30]\nnums[-1] = 99\n");
    assert!(rust.contains("if _i < 0"), "should emit negative index handling for assignment\n{}", rust);
}

#[test]
fn test_type_builtin_transpile() {
    let rust = transpile("x = type[42]\n");
    assert!(rust.contains("\"int\""), "type[42] should produce 'int'\n{}", rust);
}

#[test]
fn test_type_builtin_str_transpile() {
    let rust = transpile("x = type['hello']\n");
    assert!(rust.contains("\"str\""), "type['hello'] should produce 'str'\n{}", rust);
}

#[test]
fn test_type_builtin_array_transpile() {
    let rust = transpile("x = type[[1, 2]]\n");
    assert!(rust.contains("\"array\""), "type[[1,2]] should produce 'array'\n{}", rust);
}

#[test]
fn test_string_index_transpile() {
    let rust = transpile("s = 'hello'\nx = s[0]\n");
    assert!(rust.contains(".chars().collect()"), "string indexing should use chars()\n{}", rust);
}

#[test]
fn test_string_negative_index_transpile() {
    let rust = transpile("s = 'hello'\nx = s[-1]\n");
    assert!(rust.contains("if _i < 0"), "should handle negative string index\n{}", rust);
}

// ===== NumArray tests =====

#[test]
fn test_numarray_literal_transpile() {
    let rust = transpile("x = [1, 2, 3]\n");
    assert!(rust.contains("CrowTensor::from_vec"), "numeric array should use CrowTensor::from_vec\n{}", rust);
    assert!(rust.contains("as f64"), "elements should be cast to f64\n{}", rust);
    assert!(rust.contains("vec![3]"), "shape should be [3]\n{}", rust);
}

#[test]
fn test_numarray_import_transpile() {
    let rust = transpile("x = [1, 2, 3]\n");
    assert!(rust.contains("use crow_runtime::tensor::CrowTensor"), "should import CrowTensor\n{}", rust);
}

#[test]
fn test_numarray_add_transpile() {
    let rust = transpile("a = [1, 2, 3]\nb = [4, 5, 6]\nc = a + b\n");
    assert!(rust.contains(".add(&"), "NumArray + NumArray should use .add()\n{}", rust);
}

#[test]
fn test_numarray_scalar_mul_transpile() {
    let rust = transpile("a = [1, 2, 3]\nb = a * 2\n");
    assert!(rust.contains(".mul_scalar("), "NumArray * scalar should use .mul_scalar()\n{}", rust);
}

#[test]
fn test_numarray_neg_transpile() {
    let rust = transpile("a = [1, 2, 3]\nb = -a\n");
    assert!(rust.contains(".neg()"), "negating NumArray should use .neg()\n{}", rust);
}

#[test]
fn test_numarray_indexing_transpile() {
    let rust = transpile("nums = [10, 20, 30]\nx = nums[0]\n");
    assert!(rust.contains("get_flat"), "NumArray indexing should use get_flat\n{}", rust);
}

#[test]
fn test_numarray_negative_index_transpile() {
    let rust = transpile("nums = [10, 20, 30]\nx = nums[-1]\n");
    assert!(rust.contains("get_flat"), "NumArray negative indexing should use get_flat\n{}", rust);
    assert!(rust.contains("if _i < 0"), "should handle negative index\n{}", rust);
}

#[test]
fn test_numarray_type_builtin_transpile() {
    let rust = transpile("x = type[[1, 2]]\n");
    assert!(rust.contains("\"array\""), "type[[1,2]] should produce 'array'\n{}", rust);
}

#[test]
fn test_numarray_len_field_transpile() {
    let rust = transpile("a = [1, 2, 3]\nn = a.len\n");
    assert!(rust.contains(".len() as i64"), "NumArray .len should use .len() as i64\n{}", rust);
}

#[test]
fn test_numarray_sort_transpile() {
    let rust = transpile("a = [3, 1, 2]\nb = sort[a]\n");
    assert!(rust.contains("sort_by") || rust.contains("CrowTensor::from_vec"), "NumArray sort should produce CrowTensor\n{}", rust);
}

#[test]
fn test_numarray_contains_transpile() {
    let rust = transpile("a = [1, 2, 3]\nb = contains[a, 2]\n");
    assert!(rust.contains("to_vec().contains"), "NumArray contains should use to_vec().contains\n{}", rust);
}

#[test]
fn test_numarray_for_loop_transpile() {
    let rust = transpile("cpu[1]\na = [1, 2, 3]\nfor x in a\n    log['{x}']\n");
    assert!(rust.contains("to_vec()"), "for loop over NumArray should use to_vec()\n{}", rust);
}

#[test]
fn test_gpu_call_transpile() {
    let rust = transpile("gpu[1]\n");
    assert!(rust.contains("set_gpu_enabled(true)"), "gpu[] should emit set_gpu_enabled\n{}", rust);
}

#[test]
fn test_gpu_no_args_transpile() {
    let rust = transpile("gpu[]\n");
    assert!(rust.contains("set_gpu_enabled(true)"), "gpu[] should emit set_gpu_enabled\n{}", rust);
}

/// gpu[] and gpu[n], as a statement or a resource line, turn the device on
#[test]
fn test_is_gpu_call() {
    for src in ["gpu[]\nlog[1]\n", "gpu[2]\nlog[1]\n"] {
        let rust = transpile(src);
        assert!(rust.contains("crow_runtime::gpu::set_gpu_enabled(true)"), "{}\n{}", src, rust);
    }
    assert!(!transpile("cpu[4]\nlog[1]\n").contains("set_gpu_enabled"));
}

// ===== nodes (distributed) tests =====

#[test]
fn test_nodes_call_transpile() {
    let rust = transpile("nodes[4]\n");
    assert!(rust.contains("distributed::init(4)"), "nodes[] should emit distributed::init\n{}", rust);
    assert!(rust.contains("distributed::finalize()"), "nodes[] should emit distributed::finalize\n{}", rust);
}

/// nodes[n] joins a cluster of n before anything runs and leaves it at the end
#[test]
fn test_extract_nodes_call() {
    let rust = transpile("nodes[8]\nlog[1]\n");
    assert!(rust.contains("crow_runtime::distributed::init(8)"), "{}", rust);
    assert!(rust.contains("crow_runtime::distributed::finalize()"), "{}", rust);
    assert!(!transpile("gpu[]\nlog[1]\n").contains("distributed::init"));
}

// ===== grad tests =====

#[test]
fn test_grad_emits_crow_dual() {
    let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
    assert!(rust.contains("struct CrowDual"), "should emit CrowDual struct\n{}", rust);
    assert!(rust.contains("_dual_f"), "should emit _dual_f function\n{}", rust);
}

#[test]
fn test_grad_single_param_closure() {
    let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
    assert!(rust.contains("-> f64"), "grad closure should return f64\n{}", rust);
    assert!(rust.contains("CrowDual::new"), "should use CrowDual::new\n{}", rust);
    assert!(rust.contains(".dual"), "should extract .dual\n{}", rust);
}

#[test]
fn test_grad_multi_param_closure() {
    let rust = transpile("fn g[x, y]\n  x * x + y * y\ndg = grad[g]\nlog[dg[3, 4]]\n");
    assert!(rust.contains("Vec<f64>"), "multi-param grad should return Vec<f64>\n{}", rust);
    assert!(rust.contains("_dual_g"), "should emit _dual_g\n{}", rust);
}

#[test]
fn test_grad_int_arg_cast() {
    let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
    assert!(rust.contains("as f64"), "should cast int arg to f64\n{}", rust);
}

#[test]
fn test_grad_dual_struct_has_ops() {
    let rust = transpile("fn f[x]\n  x * x\ndf = grad[f]\nlog[df[3]]\n");
    assert!(rust.contains("impl std::ops::Add"), "CrowDual should impl Add\n{}", rust);
    assert!(rust.contains("impl std::ops::Mul"), "CrowDual should impl Mul\n{}", rust);
    assert!(rust.contains("impl std::ops::Sub"), "CrowDual should impl Sub\n{}", rust);
    assert!(rust.contains("impl std::ops::Div"), "CrowDual should impl Div\n{}", rust);
}

#[test]
fn test_math_pi_transpile() {
    let rust = transpile("use math\nx = pi\n");
    assert!(rust.contains("std::f64::consts::PI"), "should emit PI constant\n{}", rust);
}

#[test]
fn test_math_e_transpile() {
    let rust = transpile("use math\nx = e\n");
    assert!(rust.contains("std::f64::consts::E"), "should emit E constant\n{}", rust);
}

#[test]
fn test_math_inf_transpile() {
    let rust = transpile("use math\nx = inf\n");
    assert!(rust.contains("f64::INFINITY"), "should emit INFINITY\n{}", rust);
}

#[test]
fn test_math_nan_transpile() {
    let rust = transpile("use math\nx = nan\n");
    assert!(rust.contains("f64::NAN"), "should emit NAN\n{}", rust);
}

#[test]
fn test_numarray_dtype_transpile() {
    let rust = transpile("use math\nx = ones[3]\ny = x.dtype\n");
    assert!(rust.contains("\"float\""), "should emit dtype as float string\n{}", rust);
}

#[test]
fn test_numarray_device_transpile() {
    let rust = transpile("use math\nx = ones[3]\ny = x.device\n");
    assert!(rust.contains("is_gpu()"), "should emit device check\n{}", rust);
}

#[test]
fn test_math_sign_transpile() {
    let rust = transpile("use math\nx = sign[ones[3]]\n");
    assert!(rust.contains(".sign()"), "should emit .sign()\n{}", rust);
}

#[test]
fn test_math_floor_tensor_transpile() {
    let rust = transpile("use math\nx = floor[ones[3]]\n");
    assert!(rust.contains(".floor()"), "should emit .floor()\n{}", rust);
}

#[test]
fn test_math_clip_transpile() {
    let rust = transpile("use math\nx = clip[ones[3], 0, 1]\n");
    assert!(rust.contains(".clip("), "should emit .clip()\n{}", rust);
}

#[test]
fn test_math_mod_transpile() {
    let rust = transpile("use math\nx = mod[ones[3], 2]\n");
    assert!(rust.contains("modulo_scalar"), "should emit modulo_scalar for scalar arg\n{}", rust);
}

#[test]
fn test_math_tan_transpile() {
    let rust = transpile("use math\nx = tan[ones[3]]\n");
    assert!(rust.contains(".tan()"), "should emit .tan()\n{}", rust);
}

#[test]
fn test_math_tanh_transpile() {
    let rust = transpile("use math\nx = tanh[ones[3]]\n");
    assert!(rust.contains(".tanh()"), "should emit .tanh()\n{}", rust);
}

#[test]
fn test_math_atan2_transpile() {
    let rust = transpile("use math\nx = atan2[ones[3], ones[3]]\n");
    assert!(rust.contains(".atan2("), "should emit .atan2()\n{}", rust);
}

// ===== Operator tests: ^, .*, ./, .^ =====

#[test]
fn test_pow_int_transpile() {
    let rust = transpile("x = 2 ^ 10\n");
    assert!(rust.contains(".pow("), "should use .pow()\n{}", rust);
}

#[test]
fn test_pow_float_transpile() {
    let rust = transpile("x = 2.0 ^ 3.0\n");
    assert!(rust.contains(".powf("), "should use .powf()\n{}", rust);
}

#[test]
fn test_elem_mul_transpile() {
    let rust = transpile("use math\nx = ones[3] .* ones[3]\n");
    assert!(rust.contains(".mul("), "should use .mul()\n{}", rust);
}

#[test]
fn test_elem_div_transpile() {
    let rust = transpile("use math\nx = ones[3] ./ ones[3]\n");
    assert!(rust.contains(".div("), "should use .div()\n{}", rust);
}

#[test]
fn test_elem_pow_transpile() {
    let rust = transpile("use math\nx = ones[3] .^ ones[3]\n");
    assert!(rust.contains(".pow("), "should use .pow()\n{}", rust);
}

#[test]
fn test_tensor_mul_is_matmul_transpile() {
    let rust = transpile("use math\nx = eye[2] * eye[2]\n");
    assert!(rust.contains(".try_matmul("), "* on tensors should emit .try_matmul()\n{}", rust);
}

// ===== Reduction tests: prod, argmax, argmin =====

#[test]
fn test_math_prod_transpile() {
    let rust = transpile("use math\nx = prod[ones[3]]\n");
    assert!(rust.contains(".prod()"), "should emit .prod()\n{}", rust);
}

#[test]
fn test_math_argmax_transpile() {
    let rust = transpile("use math\nx = argmax[ones[3]]\n");
    assert!(rust.contains(".argmax()"), "should emit .argmax()\n{}", rust);
}

#[test]
fn test_math_argmin_transpile() {
    let rust = transpile("use math\nx = argmin[ones[3]]\n");
    assert!(rust.contains(".argmin()"), "should emit .argmin()\n{}", rust);
}

#[test]
fn test_math_sum_axis_transpile() {
    let rust = transpile("use math\nx = sum[ones[2, 3], 0]\n");
    assert!(rust.contains(".sum_axis("), "should emit .sum_axis()\n{}", rust);
}

#[test]
fn test_math_max_axis_transpile() {
    let rust = transpile("use math\nx = max[ones[2, 3], 1]\n");
    assert!(rust.contains(".max_axis("), "should emit .max_axis()\n{}", rust);
}

#[test]
fn test_math_prod_axis_transpile() {
    let rust = transpile("use math\nx = prod[ones[2, 3], 0]\n");
    assert!(rust.contains(".prod_axis("), "should emit .prod_axis()\n{}", rust);
}

// ===== Sorting & selection tests =====

#[test]
fn test_math_argsort_transpile() {
    let rust = transpile("use math\nx = argsort[ones[5]]\n");
    assert!(rust.contains(".argsort()"), "should emit .argsort()\n{}", rust);
}

#[test]
fn test_math_topk_transpile() {
    let rust = transpile("use math\nx = topk[ones[5], 3]\n");
    assert!(rust.contains(".topk("), "should emit .topk()\n{}", rust);
}

#[test]
fn test_math_unique_transpile() {
    let rust = transpile("use math\nx = unique[ones[5]]\n");
    assert!(rust.contains(".unique()"), "should emit .unique()\n{}", rust);
}

#[test]
fn test_math_where_transpile() {
    let rust = transpile("use math\nx = where[ones[3], ones[3], zeros[3]]\n");
    assert!(rust.contains(".where_cond("), "should emit .where_cond()\n{}", rust);
}

// ===== Cumulative ops tests =====

#[test]
fn test_math_cumsum_transpile() {
    let rust = transpile("use math\nx = cumsum[ones[5]]\n");
    assert!(rust.contains(".cumsum()"), "should emit .cumsum()\n{}", rust);
}

#[test]
fn test_math_cumprod_transpile() {
    let rust = transpile("use math\nx = cumprod[ones[5]]\n");
    assert!(rust.contains(".cumprod()"), "should emit .cumprod()\n{}", rust);
}

// ===== Combining ops tests =====

#[test]
fn test_math_concat_transpile() {
    let rust = transpile("use math\nx = concat[ones[3], ones[2]]\n");
    assert!(rust.contains(".concat("), "should emit .concat()\n{}", rust);
}

#[test]
fn test_math_stack_transpile() {
    let rust = transpile("use math\nx = stack[[ones[3], ones[3]]]\n");
    assert!(rust.contains("CrowTensor::stack("), "should emit CrowTensor::stack()\n{}", rust);
}

// ===== Broadcasting tests =====

#[test]
fn test_broadcast_add_transpile() {
    let rust = transpile("use math\na = reshape[ones[3], 3, 1]\nb = reshape[ones[4], 1, 4]\nx = a + b\n");
    assert!(rust.contains(".add("), "should emit .add() for broadcast\n{}", rust);
}

#[test]
fn test_broadcast_elem_mul_transpile() {
    let rust = transpile("use math\na = reshape[ones[6], 3, 1]\nb = reshape[ones[2], 1, 2]\nx = a .* b\n");
    assert!(rust.contains(".mul("), "should emit .mul() for broadcast elem mul\n{}", rust);
}

// ===== Advanced indexing (gather) tests =====

#[test]
fn test_gather_transpile() {
    let rust = transpile("use math\na = linspace[10, 50, 5]\nidx = linspace[0, 2, 3]\nx = a[idx]\n");
    assert!(rust.contains(".gather("), "should emit .gather() for tensor indexing by tensor\n{}", rust);
}

// ===== Norm tests =====

#[test]
fn test_norm_l2_transpile() {
    let rust = transpile("use math\nx = norm[ones[3]]\n");
    assert!(rust.contains(".norm(2.0)"), "should emit .norm(2.0) for L2 norm\n{}", rust);
}

#[test]
fn test_norm_lp_transpile() {
    let rust = transpile("use math\nx = norm[ones[3], 1]\n");
    assert!(rust.contains(".norm("), "should emit .norm() for Lp norm\n{}", rust);
}

// ===== Layer 3: Random & Distributions =====

#[test]
fn test_math_seed_transpile() {
    let rust = transpile("use math\nseed[42]\n");
    assert!(rust.contains("set_seed(42"), "should emit set_seed\n{}", rust);
}

#[test]
fn test_math_normal_transpile() {
    let rust = transpile("use math\nx = normal[0, 1, 3, 4]\n");
    assert!(rust.contains("CrowTensor::normal("), "should emit CrowTensor::normal\n{}", rust);
}

#[test]
fn test_math_uniform_transpile() {
    let rust = transpile("use math\nx = uniform[2, 5, 6]\n");
    assert!(rust.contains("CrowTensor::rand_range("), "should emit CrowTensor::rand_range\n{}", rust);
}

#[test]
fn test_math_poisson_transpile() {
    let rust = transpile("use math\nx = poisson[3.5, 4]\n");
    assert!(rust.contains("CrowTensor::poisson("), "should emit CrowTensor::poisson\n{}", rust);
}

#[test]
fn test_math_bernoulli_transpile() {
    let rust = transpile("use math\nx = bernoulli[0.5, 8]\n");
    assert!(rust.contains("CrowTensor::bernoulli("), "should emit CrowTensor::bernoulli\n{}", rust);
}

#[test]
fn test_math_shuffle_transpile() {
    let rust = transpile("use math\nx = shuffle[ones[5]]\n");
    assert!(rust.contains(".shuffle()"), "should emit .shuffle()\n{}", rust);
}

#[test]
fn test_math_choice_transpile() {
    let rust = transpile("use math\nx = choice[ones[5], 3]\n");
    assert!(rust.contains(".choice("), "should emit .choice()\n{}", rust);
}

#[test]
fn test_math_permutation_transpile() {
    let rust = transpile("use math\nx = permutation[5]\n");
    assert!(rust.contains("CrowTensor::permutation("), "should emit CrowTensor::permutation\n{}", rust);
}

// ===== Layer 4: Signal Processing =====

#[test]
fn test_math_fft_transpile() {
    let rust = transpile("use math\nx = fft[ones[4]]\n");
    assert!(rust.contains(".fft()"), "should emit .fft()\n{}", rust);
}

#[test]
fn test_math_ifft_transpile() {
    let rust = transpile("use math\nx = ifft[ones[4], zeros[4]]\n");
    assert!(rust.contains("CrowTensor::ifft("), "should emit CrowTensor::ifft\n{}", rust);
}

#[test]
fn test_math_fft2_transpile() {
    let rust = transpile("use math\nx = fft2[ones[4]]\n");
    assert!(rust.contains(".fft2()"), "should emit .fft2()\n{}", rust);
}

#[test]
fn test_math_conv_transpile() {
    let rust = transpile("use math\nx = conv[ones[3], ones[2]]\n");
    assert!(rust.contains(".conv("), "should emit .conv()\n{}", rust);
}

#[test]
fn test_math_conv2d_transpile() {
    let rust = transpile("use math\nx = conv2d[ones[4], ones[4]]\n");
    assert!(rust.contains(".conv2d("), "should emit .conv2d()\n{}", rust);
}

#[test]
fn test_math_hamming_transpile() {
    let rust = transpile("use math\nx = hamming[5]\n");
    assert!(rust.contains("CrowTensor::hamming("), "should emit CrowTensor::hamming\n{}", rust);
}

#[test]
fn test_math_hanning_transpile() {
    let rust = transpile("use math\nx = hanning[5]\n");
    assert!(rust.contains("CrowTensor::hanning("), "should emit CrowTensor::hanning\n{}", rust);
}

// ===== Layer 5: Special Functions & Complex Numbers =====

#[test]
fn test_math_gamma_transpile() {
    let rust = transpile("use math\nx = gamma[ones[3]]\n");
    assert!(rust.contains(".gamma()"), "should emit .gamma()\n{}", rust);
}

#[test]
fn test_math_erf_transpile() {
    let rust = transpile("use math\nx = erf[ones[3]]\n");
    assert!(rust.contains(".erf()"), "should emit .erf()\n{}", rust);
}

#[test]
fn test_math_bessel_transpile() {
    let rust = transpile("use math\nx = bessel[ones[3], 0]\n");
    assert!(rust.contains(".bessel("), "should emit .bessel()\n{}", rust);
}

#[test]
fn test_math_factorial_transpile() {
    let rust = transpile("use math\nx = factorial[ones[3]]\n");
    assert!(rust.contains(".factorial()"), "should emit .factorial()\n{}", rust);
}

#[test]
fn test_math_choose_transpile() {
    let rust = transpile("use math\nx = choose[ones[3], ones[3]]\n");
    assert!(rust.contains(".choose("), "should emit .choose()\n{}", rust);
}

#[test]
fn test_math_complex_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\n");
    assert!(rust.contains("CrowTensor::complex("), "should emit CrowTensor::complex\n{}", rust);
}

#[test]
fn test_math_magnitude_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\nm = magnitude[c]\n");
    assert!(rust.contains("CrowTensor::magnitude("), "should emit CrowTensor::magnitude\n{}", rust);
}

#[test]
fn test_math_conjugate_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\ncc = conjugate[c]\n");
    assert!(rust.contains("CrowTensor::conjugate("), "should emit CrowTensor::conjugate\n{}", rust);
}

// ===== Sparse Arrays =====

#[test]
fn test_math_sparse_from_dense_transpile() {
    let rust = transpile("use math\ns = sparse[eye[3]]\n");
    assert!(rust.contains("CrowSparse::from_dense("), "should emit CrowSparse::from_dense\n{}", rust);
}

#[test]
fn test_math_sparse_from_coo_transpile() {
    let rust = transpile("use math\ns = sparse[[0, 1], [0, 1], [5, 6], [2, 2]]\n");
    assert!(rust.contains("CrowSparse::from_coo("), "should emit CrowSparse::from_coo\n{}", rust);
}

#[test]
fn test_math_dense_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\nd = dense[s]\n");
    assert!(rust.contains(".to_dense()"), "should emit .to_dense()\n{}", rust);
}

#[test]
fn test_sparse_matmul_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\nv = [1, 2]\nr = s * v\n");
    assert!(rust.contains(".matmul_dense("), "should emit .matmul_dense()\n{}", rust);
}

#[test]
fn test_sparse_add_transpile() {
    let rust = transpile("use math\na = sparse[eye[2]]\nb = sparse[eye[2]]\nc = a + b\n");
    assert!(rust.contains(".add("), "should emit .add() for sparse+sparse\n{}", rust);
}

#[test]
fn test_sparse_solve_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\nx = solve[s, [1, 2]]\n");
    assert!(rust.contains(".solve("), "should emit .solve() for sparse solve\n{}", rust);
}

#[test]
fn test_sparse_type_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\nt = type[s]\n");
    assert!(rust.contains("\"sparse\""), "should emit 'sparse' for type[]\n{}", rust);
}

#[test]
fn test_sparse_import_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\n");
    assert!(rust.contains("use crow_runtime::sparse::CrowSparse"), "should import CrowSparse\n{}", rust);
}

// ===== Stats sub-module =====

#[test]
fn test_stats_median_transpile() {
    let rust = transpile("use math\nuse math.stats\nx = median[ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::median("), "should emit stats::median\n{}", rust);
}

#[test]
fn test_stats_std_transpile() {
    let rust = transpile("use math\nuse math.stats\nx = std[ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::std("), "should emit stats::std\n{}", rust);
}

#[test]
fn test_stats_var_transpile() {
    let rust = transpile("use math\nuse math.stats\nx = var[ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::var("), "should emit stats::var\n{}", rust);
}

#[test]
fn test_stats_percentile_transpile() {
    let rust = transpile("use math\nuse math.stats\nx = percentile[ones[5], 50]\n");
    assert!(rust.contains("crow_runtime::stats::percentile("), "should emit stats::percentile\n{}", rust);
}

#[test]
fn test_stats_zscore_transpile() {
    let rust = transpile("use math\nuse math.stats\nz = zscore[ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::zscore("), "should emit stats::zscore\n{}", rust);
}

#[test]
fn test_stats_corr_transpile() {
    let rust = transpile("use math\nuse math.stats\nc = corr[ones[5], ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::corr("), "should emit stats::corr\n{}", rust);
}

#[test]
fn test_stats_cov_transpile() {
    let rust = transpile("use math\nuse math.stats\nc = cov[ones[5], ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::cov("), "should emit stats::cov\n{}", rust);
}

#[test]
fn test_stats_ttest_transpile() {
    let rust = transpile("use math\nuse math.stats\nr = ttest[ones[5], ones[5]]\n");
    assert!(rust.contains("crow_runtime::stats::ttest("), "should emit stats::ttest\n{}", rust);
}

// ===== File System Builtins =====

#[test]
fn test_fs_exists_transpile() {
    let rust = transpile("e = exists['foo.txt']\n");
    assert!(rust.contains("crow_runtime::io::fs_exists("), "exists goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_mkdir_transpile() {
    let rust = transpile("mkdir['dir']\n");
    assert!(rust.contains("crow_runtime::io::fs_mkdir("), "mkdir goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_ls_transpile() {
    let rust = transpile("f = ls['.']\n");
    assert!(rust.contains("crow_runtime::io::fs_ls("), "ls goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_rm_transpile() {
    let rust = transpile("rm['file.txt']\n");
    assert!(rust.contains("crow_runtime::io::fs_rm(&*"), "rm goes through the runtime and borrows its path\n{}", rust);
}

#[test]
fn test_fs_mv_transpile() {
    let rust = transpile("mv['a.txt', 'b.txt']\n");
    assert!(rust.contains("crow_runtime::io::fs_mv("), "mv goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_cp_transpile() {
    let rust = transpile("cp['a.txt', 'b.txt']\n");
    assert!(rust.contains("crow_runtime::io::fs_cp("), "cp goes through the runtime\n{}", rust);
}

// ===== env builtin =====

#[test]
fn test_env_one_arg_transpile() {
    let rust = transpile("x = env['PORT']\n");
    assert!(rust.contains("std::env::var("), "should emit std::env::var\n{}", rust);
    assert!(rust.contains("\"env\".to_string()"), "1-arg env is an error value (kind env) when unset\n{}", rust);
    assert!(rust.contains("captured: std::time::SystemTime"), "env is a reader: its result carries a capture moment\n{}", rust);
}

#[test]
fn test_env_two_arg_transpile() {
    let rust = transpile("x = env['PORT', 3000]\n");
    assert!(rust.contains("std::env::var("), "should emit std::env::var\n{}", rust);
    assert!(rust.contains("unwrap_or_else"), "should emit unwrap_or_else for 2-arg env\n{}", rust);
}

// ===== Web tests =====

#[test]
fn test_web_new_transpile() {
    let rust = transpile("use web\nsite = web.new[]\n");
    assert!(rust.contains("CrowApp::new()"), "should emit CrowApp::new()\n{}", rust);
}

#[test]
fn test_web_imports() {
    let rust = transpile("use web\nsite = web.new[]\n");
    assert!(rust.contains("use crow_web::server::CrowApp;"), "should import CrowApp\n{}", rust);
    assert!(rust.contains("use crow_web::core::{CrowRequest, CrowResponse};"), "should import CrowRequest/CrowResponse\n{}", rust);
    assert!(rust.contains("use std::sync::Arc;"), "should import Arc\n{}", rust);
}

#[test]
fn test_web_async_main() {
    let rust = transpile("use web\nsite = web.new[]\n");
    assert!(rust.contains("#[tokio::main]"), "should emit #[tokio::main]\n{}", rust);
    assert!(rust.contains("async fn main()"), "should emit async fn main\n{}", rust);
}

#[test]
fn test_web_route_transpile() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains(".route("), "should emit .route()\n{}", rust);
    assert!(rust.contains("Arc::new("), "should wrap handler in Arc\n{}", rust);
}

#[test]
fn test_web_serve_transpile() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nfn handle[s, e, v]\n    log[e]\n\nserve[3000]\npage['/', render, handle]\n");
    assert!(rust.contains("__crow_app.serve("), "should emit __crow_app.serve()\n{}", rust);
    assert!(rust.contains(".await"), "should emit .await\n{}", rust);
}

#[test]
fn test_web_text_response() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::text("), "should emit CrowResponse::text()\n{}", rust);
}

#[test]
fn test_web_html_response() {
    let rust = transpile("use web\nfn handler[req]\n    return html['<h1>hi</h1>']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::html("), "should emit CrowResponse::html()\n{}", rust);
}

#[test]
fn test_web_redirect_response() {
    let rust = transpile("use web\nfn handler[req]\n    return redirect['/home']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::redirect("), "should emit CrowResponse::redirect()\n{}", rust);
}

#[test]
fn test_web_respond_response() {
    let rust = transpile("use web\nfn handler[req]\n    return respond[404, 'not found']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::respond("), "should emit CrowResponse::respond()\n{}", rust);
}

#[test]
fn test_web_handler_types() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    // Handler should have CrowRequest param and CrowResponse return
    assert!(rust.contains("crow_web::core::CrowRequest"), "handler param should be CrowRequest\n{}", rust);
    assert!(rust.contains("crow_web::core::CrowResponse"), "handler return should be CrowResponse\n{}", rust);
}

#[test]
fn test_web_no_async_without_web() {
    let rust = transpile("x = 1 + 2\n");
    assert!(!rust.contains("async fn main"), "should NOT emit async main without web\n{}", rust);
    assert!(!rust.contains("#[tokio::main]"), "should NOT emit tokio::main without web\n{}", rust);
}

// --- P4: Web module codegen tests ---

#[test]
fn test_web_cors_permissive() {
    let rust = transpile("use web\nuse web.cors\nsite = web.new[]\nsite = cors[site]\n");
    assert!(rust.contains(".cors_permissive()"), "should emit cors_permissive()\n{}", rust);
}

#[test]
fn test_web_cors_with_origins() {
    let rust = transpile("use web\nuse web.cors\nsite = web.new[]\norigins = ['http://localhost']\nsite = cors[site, origins]\n");
    assert!(rust.contains(".cors(CrowCors::origins("), "should emit cors with origins\n{}", rust);
}

#[test]
fn test_web_cors_import() {
    let rust = transpile("use web\nuse web.cors\nsite = web.new[]\nsite = cors[site]\n");
    assert!(rust.contains("use crow_web::cors::CrowCors;"), "should import CrowCors\n{}", rust);
}

#[test]
fn test_web_sessions() {
    let rust = transpile("use web\nuse web.session\nstore = sessions[]\n");
    assert!(rust.contains("CrowSessionStore::new()"), "should emit CrowSessionStore::new()\n{}", rust);
}

#[test]
fn test_web_sessions_import() {
    let rust = transpile("use web\nuse web.session\nstore = sessions[]\n");
    assert!(rust.contains("use crow_web::session::CrowSessionStore;"), "should import CrowSessionStore\n{}", rust);
}

#[test]
fn test_web_templates() {
    let rust = transpile("use web\nuse web.template\nt = templates['./templates/**/*']\n");
    assert!(rust.contains("CrowTemplates::from_glob("), "should emit CrowTemplates::from_glob()\n{}", rust);
}

#[test]
fn test_web_render() {
    let rust = transpile("use web\nuse web.template\nfn handler[req]\n    t = templates['./t/**/*']\n    data = map[name = 'crow']\n    return html[render[t, 'index', data]]\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains(".render("), "should emit .render()\n{}", rust);
}

#[test]
fn test_web_cache() {
    let rust = transpile("use web\nuse web.cache\nc = cache[100]\n");
    assert!(rust.contains("CrowCache::new(100"), "should emit CrowCache::new()\n{}", rust);
}

#[test]
fn test_web_cache_with_ttl() {
    let rust = transpile("use web\nuse web.cache\nc = cache[100, 60]\n");
    assert!(rust.contains("CrowCache::with_ttl("), "should emit CrowCache::with_ttl()\n{}", rust);
}

#[test]
fn test_web_limit() {
    let rust = transpile("use web\nuse web.rate\nlimiter = limit[100, 60]\n");
    assert!(rust.contains("CrowRateLimiter::new("), "should emit CrowRateLimiter::new()\n{}", rust);
}

#[test]
fn test_web_limit_import() {
    let rust = transpile("use web\nuse web.rate\nlimiter = limit[100, 60]\n");
    assert!(rust.contains("use crow_web::rate::CrowRateLimiter;"), "should import CrowRateLimiter\n{}", rust);
}

#[test]
fn test_web_sse() {
    let rust = transpile("use web\nuse web.sse\nfn events[req]\n    return text['data']\nsite = web.new[]\nsite = sse[site, '/events', events]\n");
    assert!(rust.contains(".sse_route("), "should emit .sse_route()\n{}", rust);
}

#[test]
fn test_web_ws() {
    let rust = transpile("use web\nuse web.ws\nfn handler[req]\n    return text['ok']\nsite = web.new[]\nsite = ws[site, '/ws', handler]\n");
    assert!(rust.contains(".ws_route("), "should emit .ws_route()\n{}", rust);
}

#[test]
fn test_web_static_dir() {
    let rust = transpile("use web\nuse web.static\nsite = web.new[]\nsite = static_dir[site, '/assets', './public']\n");
    assert!(rust.contains(".static_dir("), "should emit .static_dir()\n{}", rust);
}

#[test]
fn test_web_smtp() {
    let rust = transpile("use web\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\n");
    assert!(rust.contains("CrowMailer::new("), "should emit CrowMailer::new()\n{}", rust);
}

#[test]
fn test_web_smtp_import() {
    let rust = transpile("use web\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\n");
    assert!(rust.contains("use crow_web::mail::CrowMailer;"), "should import CrowMailer\n{}", rust);
}

#[test]
fn test_web_connect() {
    let rust = transpile("use web\nuse web.ws\nurl = 'ws://localhost:3000'\nsock = connect[url]\n");
    assert!(rust.contains("ws::connect("), "should emit ws::connect()\n{}", rust);
}

#[test]
fn test_web_send_recv_close() {
    let rust = transpile("use web\nuse web.ws\nurl = 'ws://localhost:3000'\nsock = connect[url]\nsend[sock, 'hi']\nmsg = recv[sock]\nclose[sock]\n");
    assert!(rust.contains(".send("), "should emit .send()\n{}", rust);
    assert!(rust.contains(".recv()"), "should emit .recv()\n{}", rust);
    assert!(rust.contains(".close()"), "should emit .close()\n{}", rust);
}

#[test]
fn test_web_queue() {
    let rust = transpile("use web\nuse web.queue\nfn job[req]\n    return text['done']\nq = web.queue[4]\n");
    assert!(rust.contains("CrowQueue::new("), "should emit CrowQueue::new()\n{}", rust);
}

#[test]
fn test_web_docs() {
    let rust = transpile("use web\nuse web.docs\nd = web.docs['My API', '1.0']\n");
    assert!(rust.contains("crow_web::docs::CrowDocs::new("), "should emit CrowDocs::new()\n{}", rust);
}

#[test]
fn test_web_uploads() {
    let rust = transpile("use web\nuse web.upload\nfn handler[req]\n    files = uploads[req]\n    return text['ok']\nsite = web.new[]\nsite = route[site, '/upload', handler]\n");
    assert!(rust.contains(".uploads()"), "should emit .uploads()\n{}", rust);
}

// --- DB module codegen tests ---

#[test]
fn test_db_connect() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\n");
    assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect()\n{}", rust);
    assert!(rust.contains("None"), "should pass None for default pool\n{}", rust);
}

#[test]
fn test_db_connect_pool_size() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db', 5]\n");
    assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect()\n{}", rust);
    assert!(rust.contains("Some(5"), "should pass Some(5) for pool size\n{}", rust);
}

#[test]
fn test_db_connect_prefixed() {
    let rust = transpile("use db\nconn = db.connect['sqlite://test.db']\n");
    assert!(rust.contains("crow_db::connect("), "should emit crow_db::connect() for db.connect\n{}", rust);
}

#[test]
fn test_db_exec() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\nexec[conn, 'CREATE TABLE t [id INT]']\n");
    assert!(rust.contains(".exec("), "should emit .exec()\n{}", rust);
    assert!(rust.contains(".unwrap()"), "should unwrap result\n{}", rust);
}

#[test]
fn test_db_query() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\ndf = query[conn, 'SELECT * FROM t']\n");
    assert!(rust.contains(".query("), "should emit .query()\n{}", rust);
}

#[test]
fn test_db_migrate() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\nn = migrate[conn, 'migrations/']\n");
    assert!(rust.contains(".migrate("), "should emit .migrate()\n{}", rust);
}

#[test]
fn test_db_import() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\n");
    assert!(rust.contains("use crow_db::DbHandle;"), "should import DbHandle\n{}", rust);
}

#[test]
fn test_db_insert() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\ndf = query[conn, 'SELECT * FROM t']\ninsert[conn, 'backup', df]\n");
    assert!(rust.contains(".insert("), "should emit .insert()\n{}", rust);
}

// --- P4: Response format codegen tests ---

#[test]
fn test_web_msgpack_response() {
    let rust = transpile("use web\nfn handler[req]\n    m = map[]\n    return msgpack[m]\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::msgpack("), "should emit CrowResponse::msgpack()\n{}", rust);
    assert!(rust.contains("rmp_serde::to_vec"), "should use rmp_serde\n{}", rust);
}

#[test]
fn test_web_arrow_response() {
    let rust = transpile("use web\nuse frame\nfn handler[req]\n    f = load['data.csv']\n    return arrow[f]\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::arrow_ipc("), "should emit CrowResponse::arrow_ipc()\n{}", rust);
    assert!(rust.contains("to_arrow_ipc"), "should use to_arrow_ipc\n{}", rust);
}

#[test]
fn test_web_bytes_response() {
    let rust = transpile("use web\nfn handler[req]\n    return bytes['binary data']\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("CrowResponse::binary("), "should emit CrowResponse::binary()\n{}", rust);
    assert!(rust.contains("application/octet-stream"), "should set octet-stream content-type\n{}", rust);
}

#[test]
fn test_web_stream_response() {
    let rust = transpile("use web\nfn sender[emit]\n    emit['chunk1']\n\nfn handler[req]\n    return stream[sender]\n\nsite = web.new[]\nsite = route[site, '/', handler]\n");
    assert!(rust.contains("mpsc::channel"), "should use mpsc channel for streaming\n{}", rust);
    assert!(rust.contains("Body::from_stream"), "should use Body::from_stream\n{}", rust);
}

// --- UI / LiveView tests ---

/// `crow test --compiled`: a part with no page is built as a page program
/// that answers the harness — its views and derivations by name, the routes
/// merged, serving on the port the test gives; a plain build has none of it
#[test]
fn a_test_build_answers_the_harness() {
    let src = "use web, ui\nn = 0\nfn inc[]\n    n = n + 1\nderive twice\n    return n * 2\nview v[]\n    return stack[p['`n` `twice`'], button['+', click = inc]]\n";
    let stmts = crate::lower::lower_program(&crate::build(src).unwrap()).unwrap();
    let mut ctx = crate::type_infer::TypeContext::new();
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").unwrap();
    let rust = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, true).expect("emitting failed").source;
    assert!(rust.contains("fn __crow_view(") && rust.contains("\"v\" => Some(v(state)),"), "{}", rust);
    assert!(rust.contains("fn __crow_derived(") && rust.contains("\"twice\" =>"), "{}", rust);
    assert!(rust.contains("crow_web::live::test_control::router(") && rust.contains("__crow_app.serve(0)"), "{}", rust);
    let plain = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false).map(|e| e.source).unwrap_or_default();
    assert!(!plain.contains("test_control"), "{}", plain);
}

#[test]
fn test_ui_imports() {
    let rust = transpile("use web\nuse ui\nsite = web.new[]\n");
    assert!(rust.contains("use crow_web::view::CrowVNode;"), "should import CrowVNode\n{}", rust);
    assert!(rust.contains("use crow_web::live::{CrowLiveView, CrowLiveState};"), "should import CrowLiveView/CrowLiveState\n{}", rust);
}

#[test]
fn test_ui_div_text() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return div[h1['hello']]\n");
    assert!(rust.contains("CrowVNode::div(vec![CrowVNode::h1(vec![CrowVNode::text("), "should emit CrowVNode::div with h1 child\n{}", rust);
}

#[test]
fn test_ui_button_click() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return button['+', click = 'inc']\n");
    assert!(rust.contains("CrowVNode::button(vec![CrowVNode::text("), "should emit CrowVNode::button\n{}", rust);
    assert!(rust.contains(".on_click("), "should emit .on_click()\n{}", rust);
}

#[test]
fn test_ui_input_via_module() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return ui.input[type = 'text', input = 'change']\n");
    assert!(rust.contains("CrowVNode::input()"), "should emit CrowVNode::input()\n{}", rust);
    assert!(rust.contains(".attr("), "should emit .attr() for type\n{}", rust);
    assert!(rust.contains(".on_input("), "should emit .on_input()\n{}", rust);
}

#[test]
fn test_ui_page_3arg() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nfn handle[s, e, v]\n    log[e]\n\nserve[3000]\npage['/counter', render, handle]\n");
    assert!(rust.contains("CrowLiveView::new("), "should emit CrowLiveView::new\n{}", rust);
    assert!(rust.contains("let mut __crow_state = CrowLiveState::new()"), "should init __crow_state\n{}", rust);
    assert!(rust.contains("__crow_state.clone()"), "should pass __crow_state.clone() to LiveView\n{}", rust);
    assert!(rust.contains("into_router("), "should emit into_router\n{}", rust);
    assert!(rust.contains("__crow_app = __crow_app.merge("), "should merge into implicit app\n{}", rust);
}

#[test]
fn test_ui_page_2arg_noop_handler() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nserve[3000]\npage['/dash', render]\n");
    assert!(rust.contains("CrowLiveView::new("), "should emit CrowLiveView::new\n{}", rust);
    assert!(rust.contains("Option<&str>"), "no-op handler should use Option<&str> for value param\n{}", rust);
    assert!(rust.contains("__crow_state"), "should init __crow_state\n{}", rust);
}

#[test]
fn test_ui_nested_elements() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return div[span['a'], p['b']]\n");
    assert!(rust.contains("CrowVNode::div(vec![CrowVNode::span("), "should emit nested elements\n{}", rust);
    assert!(rust.contains("CrowVNode::p(vec![CrowVNode::text("), "should emit p element\n{}", rust);
}

#[test]
fn test_ui_attr_class() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return div[h1['title'], class = 'container']\n");
    assert!(rust.contains(".attr("), "should emit .attr() for class\n{}", rust);
    assert!(rust.contains("container"), "should contain class value\n{}", rust);
}

#[test]
fn f11_click_call_shape_emits_frozen_args() {
    let rust = transpile("use web, ui\nspeed = 1.0\nfn set_speed[s]\n    speed = s\nview render[]\n    return div[button['10x', click = set_speed[10.0]]]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains("on_click_args(\"set_speed\""), "call-shape should emit on_click_args\n{}", rust);
    assert!(rust.contains("json!([(10.0"), "frozen arg should serialize at render, parenthesised so a block argument is not read as a map\n{}", rust);
    assert!(rust.contains("fn set_speed(state: &mut CrowLiveState, s: f64)"), "handler keeps its declared param\n{}", rust);
    assert!(rust.contains("_a[0].as_f64()"), "dispatch should parse the frozen arg\n{}", rust);
}

#[test]
fn test_ui_state_get_int() {
    let rust = transpile("use web\nuse ui\nview render[state]\n    count = state.get['count', 0]\n    return div[h1['{count}']]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains(".get_i64("), "state.get with int default should map to get_i64\n{}", rust);
    assert!(!rust.contains("get_i64["), "should not contain Crow syntax in output\n{}", rust);
}

#[test]
fn test_ui_state_get_str() {
    let rust = transpile("use web\nuse ui\nview render[state]\n    name = state.get['name', '']\n    return div[h1['{name}']]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains(".get_str("), "state.get with str default should map to get_str\n{}", rust);
}

#[test]
fn test_ui_state_get_bool() {
    let rust = transpile("use web\nuse ui\nview render[state]\n    active = state.get['active', false]\n    return div[h1['{active}']]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains(".get_bool("), "state.get with bool default should map to get_bool\n{}", rust);
}

#[test]
fn test_ui_fetch_assignment() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\ndata = fetch['/api/data']\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains("register_fetch(\"data\""), "fetch assignment should emit register_fetch\n{}", rust);
    // no top-level local for a data source; a view reads it as a frame
    // from its preamble, and that is the only `let data` there is
    assert_eq!(rust.matches("let data").count(), rust.matches("let data = __json_to_frame(").count(), "fetch should not create a local variable\n{}", rust);
}

#[test]
fn test_ui_poll_assignment() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nprices = poll['/api/prices', 5]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains("register_poll(\"prices\""), "poll assignment should emit register_poll\n{}", rust);
    assert!(rust.contains("as u64"), "poll should pass interval as u64\n{}", rust);
}

#[test]
fn test_ui_field_codegen() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    vol = s.get['vol', 0]\n    return div[field['Min Vol', vol]]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains("CrowVNode::field_typed("), "field should emit CrowVNode::field_typed\n{}", rust);
    assert!(rust.contains("\"vol\""), "field should extract key name\n{}", rust);
}

#[test]
fn test_ui_select_codegen() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    opts = ['a', 'b']\n    val = s.get['choice', 'a']\n    return div[select[opts, val]]\nserve[3000]\npage['/', render]\n");
    assert!(rust.contains("CrowVNode::select_input("), "select should emit CrowVNode::select_input\n{}", rust);
    assert!(rust.contains("\"val\""), "select should extract key name\n{}", rust);
}

#[test]
fn test_reactive_counter() {
    let rust = transpile("use web, ui\n\ncount = 0\n\nfn inc[]\n    count = count + 1\n\nfn dec[]\n    count = count - 1\n\nview render[]\n    return div[\n        h1['Count: {count}'],\n        button['+', click = inc],\n        button['-', click = dec]\n    ]\n\nserve[3000]\npage['/', render]\n");
    // Reactive render function should get state param injected
    assert!(rust.contains("fn render(state: &CrowLiveState) -> CrowVNode"), "render should have injected state param\n{}", rust);
    // Reactive handler functions should get mut state param injected
    assert!(rust.contains("fn inc(state: &mut CrowLiveState)"), "inc should have injected mut state param\n{}", rust);
    assert!(rust.contains("fn dec(state: &mut CrowLiveState)"), "dec should have injected mut state param\n{}", rust);
    // State preamble: state.get_i64 for reactive vars
    assert!(rust.contains("state.get_i64(\"count\""), "should inject state.get_i64 preamble\n{}", rust);
    // State epilogue: state.set for reactive vars in handlers
    assert!(rust.contains("state.set(\"count\""), "should inject state.set epilogue in handlers\n{}", rust);
    // Dispatch function
    assert!(rust.contains("fn __crow_handle(state: &mut CrowLiveState, event: &str, _value: Option<&str>)"), "should generate __crow_handle dispatch\n{}", rust);
    assert!(rust.contains("\"inc\" => inc(state)"), "dispatch should route inc\n{}", rust);
    assert!(rust.contains("\"dec\" => dec(state)"), "dispatch should route dec\n{}", rust);
    // Event attributes should use string, not reference
    assert!(rust.contains(".on_click(\"inc\")"), "click = inc should emit .on_click(\"inc\")\n{}", rust);
    assert!(rust.contains(".on_click(\"dec\")"), "click = dec should emit .on_click(\"dec\")\n{}", rust);
    // Main should register defaults
    assert!(rust.contains("__crow_state.set(\"count\", serde_json::json!(0"), "should register count default in main\n{}", rust);
    // page[] should auto-inject __crow_handle
    assert!(rust.contains("__crow_handle"), "page should auto-inject __crow_handle\n{}", rust);
}

#[test]
fn test_serve_refresh_budget_emission() {
    let rust = transpile("use web, ui\n\ncount = 0\n\nview render[]\n    return div[p['{count}']]\n\npage['/', render]\nserve[3000, refresh = 5, budget = 50]\n");
    assert!(
        rust.contains("crow_web::live::configure(5_i64 as i64, 50_i64 as i64)"),
        "serve named args should configure the live runtime\n{}",
        rust
    );
    assert!(rust.contains("__crow_app.serve(3000_i64 as u16)"), "{}", rust);
}

#[test]
fn test_later_script_emission() {
    let rust = transpile("fn work[msg]\n    log[msg]\n\nlater[work, 'hi']\n");
    assert!(
        rust.contains("crow_runtime::later::spawn(\"work\""),
        "script later[] should emit kernel spawn\n{}",
        rust
    );
    assert!(
        rust.contains("crow_runtime::later::wait_all()"),
        "main should wait for stragglers at exit\n{}",
        rust
    );
}

#[test]
fn test_later_handler_emission() {
    let rust = transpile("use web, ui\n\ncount = 0\n\nfn slow_work[]\n    count = count + 1\n\nfn go[]\n    later[slow_work]\n\nview render[]\n    return div[p['{count}'], button['go', click = go]]\n\npage['/', render]\nserve[3000]\n");
    assert!(
        rust.contains("state.spawn_later(\"slow_work\", slow_work)"),
        "handler later[] should go through session state\n{}",
        rust
    );
    assert!(
        rust.contains("fn slow_work(state: &mut CrowLiveState)"),
        "later target must compile as a handler-class fn\n{}",
        rust
    );
}

#[test]
fn test_reactive_vars_skipped_in_main() {
    let rust = transpile("use web, ui\n\ncount = 0\n\nview render[]\n    return div[h1['Count: {count}']]\n\nserve[3000]\npage['/', render]\n");
    // count should NOT appear as a `let count` in main — it's registered via state.set
    let main_section = rust.split("fn main()").nth(1).unwrap_or("");
    assert!(!main_section.contains("let count"), "reactive var should not appear as local var in main\n{}", rust);
}

// ===== Boolean mask semantics (frame API: strings + expressions) =====

#[test]
fn test_mask_and_emits_elementwise_zip() {
    let rust = transpile("use frame\ndf = load['d.csv']\nm = df.age gt 5 and df.age lt 30\n");
    assert!(rust.contains(".zip("), "and on bool masks should zip element-wise\n{}", rust);
}

#[test]
fn test_mask_not_emits_elementwise_negation() {
    let rust = transpile("use frame\ndf = load['d.csv']\nm = not (df.age gt 5)\n");
    assert!(rust.contains("!*_b"), "not on bool mask should negate element-wise\n{}", rust);
}

#[test]
fn test_not_comparison_keeps_parens() {
    // Regression: `not (x eq 8)` must not compile to `!x == 8` (Rust ! binds tighter)
    let rust = transpile("x = 7\nif not (x eq 8)\n    log['ok']\n");
    assert!(rust.contains("(!("), "not must parenthesize its operand\n{}", rust);
}

#[test]
fn test_implicit_return_gives_fn_return_type() {
    // Regression: implicit last-expression return must produce a typed fn
    let rust = transpile("fn add[a, b]\n    a + b\nr = add[3, 4]\nlog[r]\n");
    assert!(rust.contains("-> i64"), "implicit return should type the function\n{}", rust);
}

#[test]
fn test_user_variable_shadows_module_constant() {
    // Regression: `e = m.eye[2]` then `log[e]` must reference the variable,
    // not Euler's e (a bare `use math` reserves the name; a qualified one
    // leaves it free)
    let rust = transpile("use math as m\ne = m.eye[2]\nlog[e]\n");
    assert!(!rust.contains("consts::E"), "user variable must shadow constant\n{}", rust);
}

#[test]
fn test_find_answers_a_record_or_an_error_value() {
    // find[text, pattern] is a record with the groups on a match and an
    // error value otherwise, so m.ok is the value's own status
    let rust = transpile("m = find['abc', '[a-z]+']\nlog[m.ok]\n");
    assert!(rust.contains("crow_runtime::text::find("), "find goes through the runtime\n{}", rust);
    assert!(rust.contains("\"no match\""), "no match is an error value\n{}", rust);
    assert!(rust.contains("struct CrowFindResult { groups: Vec<String> }"), "the record has the groups only\n{}", rust);
}

#[test]
fn test_monomorphization_clones_per_signature() {
    // F2: a function used at two types compiles as two clones
    let rust = transpile("fn first[arr]\n    return arr[0]\nlog[first[[1, 2]]]\nlog[first[['a', 'b']]]\n");
    assert!(rust.contains("fn first__1"), "should emit first clone\n{}", rust);
    assert!(rust.contains("fn first__2"), "should emit second clone\n{}", rust);
    assert!(!rust.contains("fn first("), "original should not be emitted\n{}", rust);
}

#[test]
fn test_single_signature_functions_untouched() {
    let rust = transpile("fn double[x]\n    return x * 2\nlog[double[5]]\nlog[double[7]]\n");
    assert!(rust.contains("fn double("), "single-sig fn keeps its name\n{}", rust);
    assert!(!rust.contains("double__"), "no clones for single signature\n{}", rust);
}

#[test]
fn test_search_builtins_emit_parallel_early_exit() {
    let rust = transpile("fn big[x]\n    return x gt 10\na = any[[1, 2], big]\nlog[a]\n");
    assert!(rust.contains("par_iter().any"), "any should be parallel\n{}", rust);
    let rust = transpile("fn big[x]\n    return x gt 10\nh = find[[1, 2], big]\nlog[h.ok]\n");
    assert!(rust.contains("find_first"), "find should use parallel find_first\n{}", rust);
}

#[test]
fn test_fmt_emits_kernel_call() {
    let rust = transpile("x = fmt[1234.5, decimals = 2, commas = true]\nlog[x]\n");
    assert!(rust.contains("crow_fmt_f64"), "fmt should call the display kernel\n{}", rust);
    let rust = transpile("x = fmt['hi', width = 4]\nlog[x]\n");
    assert!(rust.contains("crow_fmt_str"), "string fmt should call the str kernel\n{}", rust);
}

#[test]
fn test_fallible_condition_guarded_and_return_widened() {
    // A fn with a fallible condition returns CrowResult and guards the branch
    let rust = transpile("fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['9']\nlog[r]\n");
    assert!(rust.contains("if !_c.ok"), "condition should be guarded\n{}", rust);
    assert!(rust.contains("-> CrowResult<String>"), "return type should widen to Result\n{}", rust);
}

/// int/float rule across the program: a fact a handler assigns a float
/// (a table's float key column) is a float from its declaration
#[test]
fn a_fact_a_handler_assigns_a_float_is_a_float_from_its_declaration() {
    let rust = transpile("use web, ui, frame\ndeleted = 0\nfn delete_row[id]\n    deleted = id\nview records[]\n    rows = frame.new[]\n    rows = with[rows, 'id', [3, 4]]\n    return table[rows, click = delete_row, key = 'id']\npage['/', records]\n");
    assert!(rust.contains("state.get_f64(\"deleted\", (0_i64) as f64)"), "the key column is float, so the fact is\n{}", rust);
}

/// A plain function's local of a fact's name is a new binding: it says
/// nothing about the fact's type
#[test]
fn a_plain_functions_local_does_not_widen_a_fact() {
    let rust = transpile("use web, ui\nn = 0\nfn scale[x]\n    n = 2.5\n    return n * x\nview render[]\n    return p['`n` `scale[2]`']\npage['/', render]\n");
    assert!(rust.contains("state.get_i64(\"n\", 0_i64)"), "a local n is a new binding\n{}", rust);
}

/// A handler hands no error back: a fallible condition stops it, the
/// facts it wrote so far land, and the reason is logged
#[test]
fn a_handlers_fallible_condition_stops_the_handler() {
    let rust = transpile("use web, ui\nn = 0\nentry = ''\nfn apply[]\n    v = int[entry]\n    if v gt 5\n        n = n + 1\nview render[]\n    return stack[field['Text', entry], button['Go', click = apply], p['`n`']]\npage['/', render]\n");
    assert!(rust.contains("stopped: {}\", _c.reason)"), "{}", rust);
    assert!(!rust.contains("return CrowResult { ok: false, value: ()"), "{}", rust);
}

/// A result consumed into a fact inside a handler that also branches on
/// a result: both stop the handler; neither returns an error from it
#[test]
fn a_handlers_consumed_result_stops_the_handler_too() {
    let rust = transpile("use web, ui\nn = 0\nentry = ''\nfn apply[]\n    v = int[entry]\n    if v gt 5\n        n = n + 1\n    n = int[entry]\nview render[]\n    return stack[field['Text', entry], button['Go', click = apply], p['`n`']]\npage['/', render]\n");
    assert!(!rust.contains("value: Default::default()"), "{}", rust);
    assert!(rust.matches("stopped: {}\"").count() >= 2, "{}", rust);
}

/// `len[b] lt 16` on a result-typed frame: the cast is parenthesised,
/// or Rust reads `as i64 <` as the start of generic arguments
#[test]
fn len_of_a_result_frame_compares_cleanly() {
    let rust = transpile("use frame\nfn get[s]\n    n = int[s]\n    if n gt 0\n        return frame.new[]\n    return frame.new[]\nb = get['1']\nif len[b] lt 16\n    log['small']\n");
    assert!(!rust.contains("as i64 <"), "{}", rust);
}

/// A helper a handler calls sees what the handler wrote so far: the
/// handler's own facts are written through before the call
#[test]
fn a_helper_sees_what_its_caller_wrote_so_far() {
    let rust = transpile("use web, ui\nskipped = ['']\nsym = 'A'\nstatus = ''\nfn hop[]\n    if contains[skipped, sym]\n        status = 'skipped'\n    else\n        status = 'kept'\nfn skip[]\n    skipped = concat[skipped, [sym]]\n    hop[]\nview render[]\n    return stack[p[status], button['Skip', click = skip], button['Hop', click = hop]]\npage['/', render]\n");
    assert!(rust.contains("state.set(\"skipped\", serde_json::json!(skipped)); let _r = hop(state"), "the caller's write reaches the helper\n{}", rust);
}

/// `if not b.ok return b` in a function whose value is another type:
/// the error travels and the value is the function's default
#[test]
fn an_error_of_another_type_returned_from_a_function_is_converted() {
    let rust = transpile("fn half[s]\n    n = int[s]\n    if not n.ok\n        return n\n    return n / 2.0\nlog[half['8']]\n");
    assert!(rust.contains("ok: _r.ok, value: 0.0_f64"), "{}", rust);
}

/// A frame assigned beside an independent statement keeps its columns'
/// types for the reads that follow (this once exercised the wave
/// emitter; statements run in order now, and the schema must survive
/// that path too)
#[test]
fn a_frame_assigned_beside_an_independent_statement_keeps_its_column_types() {
    let rust = transpile("use frame\nfn go[]\n    rows = with[frame.new[], 'v', [3.5, 4.5]]\n    k = 2\n    return rows.v[0] * k\nlog[go[]]\n");
    assert!(!rust.contains("rayon::join"), "statements run in order\n{}", rust);
    assert!(rust.contains("-> f64"), "the cell is a float, so the product is\n{}", rust);
}

#[test]
fn test_float_var_declared_unified() {
    // int-then-float reassignment declares the variable as f64 and coerces
    let rust = transpile("v = 10\nv = 2.5\nv = 1\nlog[v]\n");
    assert!(rust.contains("let mut v: f64"), "v should be declared f64\n{}", rust);
    assert!(rust.contains("as f64"), "int assignments should coerce\n{}", rust);
}

#[test]
fn test_frame_column_args_are_plain_strings() {
    // Regression: bare-name force conversion removed — quoted strings pass through
    let rust = transpile("use frame\ndf = load['d.csv']\ns = sort[df, 'age', desc = true]\n");
    assert!(rust.contains("\"age\""), "column name string should appear in codegen\n{}", rust);
}
