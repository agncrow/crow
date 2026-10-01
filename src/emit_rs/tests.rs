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
    // (an item of text is the loop's own, as the body reads it)
    assert!(body.contains("for x in xs.iter().cloned() {"), "a loop, not a closure:\n{}", body);
    assert!(body.contains("return CrowResult { ok: false"), "the error goes back:\n{}", body);
    assert!(!body.contains(".map(|"), "{}", body);
}

/// What a list or a map keeps is its own: a variable read is cloned into
/// push[] (set[]'s key is read, and kept by the runtime), so the variable
/// stays usable and a loop's borrowed variable becomes a value; the
/// collection the statement assigns is moved into the call, not copied
#[test]
fn a_collection_keeps_a_clone_of_a_variable() {
    let rust = transpile("out = []\nfor w in ['a', 'b']\n    out = push[out, w]\nm = map[]\nfor k in ['p', 'q']\n    m = set[m, k, 1]\nlog[out]\nlog[m]\n");
    assert!(rust.contains("crow_runtime::core::push((out), ((w).clone()))"), "{}", rust);
    assert!(rust.contains("crow_runtime::core::set((m), &*(k), (1_i64))"), "{}", rust);
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
    assert!(rust.contains("= crow_runtime::core::push((_yielded_"), "{}", rust);
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

/// A failed value assigned to a variable whose first value could not fail
/// is refused there: a function whose only failure it is can fail, and the
/// top level halts naming the variable (compiled returned from main
/// silently)
#[test]
fn a_plain_variable_refuses_a_failed_value() {
    let rust = transpile("fn parse[s]\n    x = 1.0\n    x = float[s]\n    return 1\nlog[parse['2']]\ny = 1.0\ny = float['q']\nlog[y]\n");
    let start = rust.find("fn parse(").expect("parse");
    let rest = &rust[start..];
    let body = &rest[..rest.find("\n}\n").unwrap()];
    assert!(body.contains("-> CrowResult<i64>"), "{}", body);
    assert!(rust.contains("runtime error: line 7: cannot assign an error value to 'y' — {}"), "{}", rust);
}

/// A function that writes a fact but that nothing calls or wires (the
/// part it lives in lost the button that wired it) can never run, and is
/// not compiled: it was emitted without the session's state and did not
/// build
#[test]
fn an_unreached_fact_writer_is_left_out() {
    let rust = transpile("use web, ui\ncount = 0\nfn reset[]\n    count = 0\nview v[]\n    return p['`count`']\npage['/', v]\n");
    assert!(!rust.contains("fn reset("), "{}", rust);
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
    crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false, None).expect("emitting failed").source
}

/// The runtime features a program's build asks for (crow_runtime::sources).
fn runtime_features(source: &str) -> Vec<String> {
    let stmts = crate::lower::lower_program(&crate::build(source).expect("parse failed")).expect("lowering failed");
    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowering to the IR failed");
    let emitted = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false, None).expect("emitting failed");
    emitted.runtime_features.into_iter().collect()
}

/// A build asks for network sources and Parquet only where the program's
/// paths may name them: a local CSV asks for neither, a local Parquet file
/// for Parquet, a URL naming JSON for the network alone, an extensionless
/// URL for both (its server may answer Parquet), and a path the program
/// computes for both
#[test]
fn a_build_asks_for_what_its_paths_may_name() {
    assert!(runtime_features("use frame\ndf = load['data.csv']\nlog[len[df]]\n").is_empty());
    assert!(runtime_features("t = read['notes/a.txt']\nlog[t]\n").is_empty());
    assert_eq!(runtime_features("use frame\ndf = load['bars.parquet']\nlog[len[df]]\n"), vec!["parquet"]);
    assert_eq!(runtime_features("use frame\ns = 'AAPL'\ndf = load['https://api.test/`s`.json']\nlog[len[df]]\n"), vec!["net"]);
    assert_eq!(runtime_features("use frame\ndf = load['https://api.test/v2/aggs']\nlog[len[df]]\n"), vec!["net", "parquet"]);
    assert_eq!(runtime_features("use frame\nd = 'x'\ndf = load['data/`d`.csv']\nlog[len[df]]\n").len(), 0);
    assert_eq!(runtime_features("use frame\np = args[][0]\ndf = load[p]\nlog[len[df]]\n"), vec!["net", "parquet"]);
    assert_eq!(runtime_features("p = args[][0]\nt = read[p]\nlog[t]\n"), vec!["net"]);
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
    // the message is the runtime's, which both modes call
    let src = "x = int['bad']\n";
    let rust = transpile(src);
    assert!(rust.contains("crow_runtime::cell::int_of("), "should call the runtime's conversion\n{}", rust);
    assert!(rust.contains("kind: \"parse\""), "{}", rust);
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
    assert!(rust.contains("crow_runtime::json::parse(&*("), "json[] is its row, the runtime's parse\n{}", rust);
    assert!(rust.contains("CrowResult"), "json[] should return CrowResult\n{}", rust);
}

#[test]
fn test_json_null_returns_error() {
    let src = "n = json['null']\nlog[n.ok]\n";
    let rust = transpile(src);
    // the runtime's parse answers a top-level null as its reason, which the
    // row makes an error value of kind 'parse'
    assert!(rust.contains("crow_runtime::json::parse(&*("), "json[] is its row\n{}", rust);
    assert!(rust.contains("kind: \"parse\".to_string()"), "a failed parse is kind 'parse'\n{}", rust);
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
    assert!(rust.contains("crow_runtime::stops::mul((crow_runtime::stops::add(1_i64, 2_i64).unwrap_or_else(|e| crow_stop(1, e))), 3_i64)"), "should preserve grouping\n{}", rust);
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
    // a numeric literal is a tensor: its row, the one the interpreter calls
    assert!(rust.contains("crow_runtime::core::len_tensor(&(x))"), "should call the tensor's len\n{}", rust);
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
    // the runtime resolves a negative index, and stops on one outside
    assert!(rust.contains("crow_runtime::stops::item(_i, _t.len()).unwrap_or_else(|e| crow_stop(2, e))"), "should resolve the index through the runtime\n{}", rust);
}

#[test]
fn test_negative_index_assign_transpile() {
    let rust = transpile("nums = [10, 20, 30]\nnums[-1] = 99\n");
    assert!(rust.contains("crow_runtime::stops::item(_i, _tmp.len()).unwrap_or_else(|e| crow_stop(2, e))"), "should resolve the assigned index through the runtime\n{}", rust);
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
    assert!(rust.contains("crow_runtime::stops::char_at(_i, _s.len())"), "should handle negative string index\n{}", rust);
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
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\"+\""), "NumArray + NumArray checks its shapes\n{}", rust);
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
    assert!(rust.contains("crow_runtime::stops::item("), "should handle negative index\n{}", rust);
}

#[test]
fn test_numarray_type_builtin_transpile() {
    let rust = transpile("x = type[[1, 2]]\n");
    assert!(rust.contains("\"array\""), "type[[1,2]] should produce 'array'\n{}", rust);
}

#[test]
fn test_numarray_len_field_transpile() {
    // `a.len` is `len[a]`: the one row, a matrix's rows
    let rust = transpile("a = [1, 2, 3]\nn = a.len\n");
    assert!(rust.contains("crow_runtime::core::len_tensor(&(a))"), "a tensor's .len is len[]'s row\n{}", rust);
}

#[test]
fn test_numarray_sort_transpile() {
    let rust = transpile("a = [3, 1, 2]\nb = sort[a]\n");
    assert!(rust.contains("sort_by") || rust.contains("CrowTensor::from_vec"), "NumArray sort should produce CrowTensor\n{}", rust);
}

#[test]
fn test_numarray_contains_transpile() {
    let rust = transpile("a = [1, 2, 3]\nb = contains[a, 2]\n");
    assert!(rust.contains("crow_runtime::core::contains_number(&(a), (2_i64) as f64)"), "a tensor's contains is its row\n{}", rust);
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
    assert!(rust.contains("crow_runtime::math::sign_t("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_floor_tensor_transpile() {
    let rust = transpile("use math\nx = floor[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::floor_t("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_clip_transpile() {
    let rust = transpile("use math\nx = clip[ones[3], 0, 1]\n");
    assert!(rust.contains("crow_runtime::math::clip("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_mod_transpile() {
    let rust = transpile("use math\nx = mod[ones[3], 2]\n");
    assert!(rust.contains("crow_runtime::math::modulo_by("), "should call the runtime's modulo_by (the builtin table)\n{}", rust);
}

#[test]
fn test_math_tan_transpile() {
    let rust = transpile("use math\nx = tan[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::tan_t("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_tanh_transpile() {
    let rust = transpile("use math\nx = tanh[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::tanh_t("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_atan2_transpile() {
    let rust = transpile("use math\nx = atan2[ones[3], ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::atan2("), "should call the runtime's function (the builtin table)\n{}", rust);
}

// ===== Operator tests: ^, .*, ./, .^ =====

#[test]
fn test_pow_int_transpile() {
    let rust = transpile("x = 2 ^ 10\n");
    // exact, or the program stops: never a wrapped int
    assert!(rust.contains("crow_runtime::stops::pow(2_i64, 10_i64).unwrap_or_else(|e| crow_stop(1, e))"), "should check the power\n{}", rust);
}

#[test]
fn test_pow_float_transpile() {
    let rust = transpile("x = 2.0 ^ 3.0\n");
    assert!(rust.contains(".powf("), "should use .powf()\n{}", rust);
}

#[test]
fn test_elem_mul_transpile() {
    let rust = transpile("use math\nx = ones[3] .* ones[3]\n");
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\".*\""), "an element-wise product checks its shapes\n{}", rust);
}

#[test]
fn test_elem_div_transpile() {
    let rust = transpile("use math\nx = ones[3] ./ ones[3]\n");
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\"./\""), "an element-wise quotient checks its shapes\n{}", rust);
}

#[test]
fn test_elem_pow_transpile() {
    let rust = transpile("use math\nx = ones[3] .^ ones[3]\n");
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\".^\""), "an element-wise power checks its shapes\n{}", rust);
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
    assert!(rust.contains("crow_runtime::math::prod("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_argmax_transpile() {
    let rust = transpile("use math\nx = argmax[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::argmax("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_argmin_transpile() {
    let rust = transpile("use math\nx = argmin[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::argmin("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_sum_axis_transpile() {
    let rust = transpile("use math\nx = sum[ones[2, 3], 0]\n");
    assert!(rust.contains("crow_runtime::math::sum_axis("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_max_axis_transpile() {
    let rust = transpile("use math\nx = max[ones[2, 3], 1]\n");
    assert!(rust.contains("crow_runtime::math::max_axis("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_prod_axis_transpile() {
    let rust = transpile("use math\nx = prod[ones[2, 3], 0]\n");
    assert!(rust.contains("crow_runtime::math::prod_axis("), "should call the runtime's function (the builtin table)\n{}", rust);
}

// ===== Sorting & selection tests =====

#[test]
fn test_math_argsort_transpile() {
    let rust = transpile("use math\nx = argsort[ones[5]]\n");
    assert!(rust.contains("crow_runtime::math::argsort("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_topk_transpile() {
    let rust = transpile("use math\nx = topk[ones[5], 3]\n");
    assert!(rust.contains("crow_runtime::math::topk("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_unique_transpile() {
    let rust = transpile("use math\nx = unique[ones[5]]\n");
    assert!(rust.contains("crow_runtime::math::unique("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_where_transpile() {
    let rust = transpile("use math\nx = where[ones[3], ones[3], zeros[3]]\n");
    assert!(rust.contains("crow_runtime::math::where_("), "should call the runtime's function (the builtin table)\n{}", rust);
}

// ===== Cumulative ops tests =====

#[test]
fn test_math_cumsum_transpile() {
    let rust = transpile("use math\nx = cumsum[ones[5]]\n");
    assert!(rust.contains("crow_runtime::math::cumsum("), "should call the runtime's function (the builtin table)\n{}", rust);
}

#[test]
fn test_math_cumprod_transpile() {
    let rust = transpile("use math\nx = cumprod[ones[5]]\n");
    assert!(rust.contains("crow_runtime::math::cumprod("), "should call the runtime's function (the builtin table)\n{}", rust);
}

// ===== Combining ops tests =====

#[test]
fn test_math_concat_transpile() {
    let rust = transpile("use math\nx = concat[ones[3], ones[2]]\n");
    // shapes that do not join stop with their reason (the kernel asserted)
    assert!(rust.contains("crow_runtime::core::concat_tensors("), "should call the tensors' concat\n{}", rust);
    assert!(rust.contains(".unwrap_or_else(|e| crow_stop(2, e));"), "{}", rust);
}

#[test]
fn test_math_stack_transpile() {
    let rust = transpile("use math\nx = stack[[ones[3], ones[3]]]\n");
    // a literal list of tensors is a tensor: its rows restacked
    assert!(rust.contains("crow_runtime::math::stack_rows(&("), "should call the runtime's stack_rows\n{}", rust);
}

// ===== Broadcasting tests =====

#[test]
fn test_broadcast_add_transpile() {
    let rust = transpile("use math\na = reshape[ones[3], 3, 1]\nb = reshape[ones[4], 1, 4]\nx = a + b\n");
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\"+\""), "a broadcast sum checks its shapes\n{}", rust);
}

#[test]
fn test_broadcast_elem_mul_transpile() {
    let rust = transpile("use math\na = reshape[ones[6], 3, 1]\nb = reshape[ones[2], 1, 2]\nx = a .* b\n");
    assert!(rust.contains(".try_elementwise(&") && rust.contains("\".*\""), "a broadcast product checks its shapes\n{}", rust);
}

// ===== Advanced indexing (gather) tests =====

#[test]
fn test_gather_transpile() {
    let rust = transpile("use math\na = linspace[10, 50, 5]\nidx = linspace[0, 2, 3]\nx = a[idx]\n");
    // an index outside stops with the words of xs[i] (the kernel asserted)
    assert!(rust.contains(".try_gather("), "should emit .try_gather() for tensor indexing by tensor\n{}", rust);
}

// ===== Norm tests =====

#[test]
fn test_norm_l2_transpile() {
    let rust = transpile("use math\nx = norm[ones[3]]\n");
    // one argument: the table's norm row
    assert!(rust.contains("crow_runtime::math::norm(&("), "should call the runtime's norm\n{}", rust);
}

#[test]
fn test_norm_lp_transpile() {
    let rust = transpile("use math\nx = norm[ones[3], 1]\n");
    // two arguments: the row the count picks
    assert!(rust.contains("crow_runtime::math::norm_p(&("), "should call the runtime's norm_p\n{}", rust);
}

// ===== Layer 3: Random & Distributions =====

#[test]
fn test_math_seed_transpile() {
    let rust = transpile("use math\nseed[42]\n");
    assert!(rust.contains("crow_runtime::math::seed("), "should call the runtime's seed (the builtin table)\n{}", rust);
}

#[test]
fn test_math_normal_transpile() {
    let rust = transpile("use math\nx = normal[0, 1, 3, 4]\n");
    // the builtin table's row: the runtime function, its sizes as a slice
    assert!(rust.contains("crow_runtime::math::normal((0_i64) as f64, (1_i64) as f64, &[(3_i64) as i64, (4_i64) as i64]).unwrap_or_else(|e| crow_stop(2, e))"), "should call the runtime's normal\n{}", rust);
}

#[test]
fn test_math_uniform_transpile() {
    let rust = transpile("use math\nx = uniform[2, 5, 6]\n");
    assert!(rust.contains("crow_runtime::math::uniform("), "should call the runtime's uniform\n{}", rust);
}

#[test]
fn test_math_poisson_transpile() {
    let rust = transpile("use math\nx = poisson[3.5, 4]\n");
    assert!(rust.contains("crow_runtime::math::poisson((3.5_f64) as f64, &[(4_i64) as i64])"), "should call the runtime's poisson\n{}", rust);
}

#[test]
fn test_math_bernoulli_transpile() {
    let rust = transpile("use math\nx = bernoulli[0.5, 8]\n");
    assert!(rust.contains("crow_runtime::math::bernoulli("), "should call the runtime's bernoulli\n{}", rust);
}

#[test]
fn test_math_shuffle_transpile() {
    let rust = transpile("use math\nx = shuffle[ones[5]]\n");
    assert!(rust.contains("crow_runtime::math::shuffle("), "should call the runtime's shuffle (the builtin table)\n{}", rust);
}

#[test]
fn test_math_choice_transpile() {
    let rust = transpile("use math\nx = choice[ones[5], 3]\n");
    assert!(rust.contains("crow_runtime::math::choice("), "should call the runtime's choice (the builtin table)\n{}", rust);
}

#[test]
fn test_math_permutation_transpile() {
    let rust = transpile("use math\nx = permutation[5]\n");
    assert!(rust.contains("crow_runtime::math::permutation("), "should call the runtime's permutation (the builtin table)\n{}", rust);
}

// ===== Layer 4: Signal Processing =====

#[test]
fn test_math_fft_transpile() {
    let rust = transpile("use math\nx = fft[ones[4]]\n");
    assert!(rust.contains("crow_runtime::math::fft("), "should call the runtime's fft (the builtin table)\n{}", rust);
}

#[test]
fn test_math_ifft_transpile() {
    let rust = transpile("use math\nx = ifft[ones[4], zeros[4]]\n");
    assert!(rust.contains("crow_runtime::math::ifft("), "should call the runtime's ifft (the builtin table)\n{}", rust);
}

#[test]
fn test_math_fft2_transpile() {
    let rust = transpile("use math\nx = fft2[ones[4]]\n");
    assert!(rust.contains("crow_runtime::math::fft2("), "should call the runtime's fft2 (the builtin table)\n{}", rust);
}

#[test]
fn test_math_conv_transpile() {
    let rust = transpile("use math\nx = conv[ones[3], ones[2]]\n");
    assert!(rust.contains("crow_runtime::math::conv("), "should call the runtime's conv (the builtin table)\n{}", rust);
}

#[test]
fn test_math_conv2d_transpile() {
    let rust = transpile("use math\nx = conv2d[ones[4], ones[4]]\n");
    assert!(rust.contains("crow_runtime::math::conv2d("), "should call the runtime's conv2d (the builtin table)\n{}", rust);
}

#[test]
fn test_math_hamming_transpile() {
    let rust = transpile("use math\nx = hamming[5]\n");
    assert!(rust.contains("crow_runtime::math::hamming("), "should call the runtime's hamming (the builtin table)\n{}", rust);
}

#[test]
fn test_math_hanning_transpile() {
    let rust = transpile("use math\nx = hanning[5]\n");
    assert!(rust.contains("crow_runtime::math::hanning("), "should call the runtime's hanning (the builtin table)\n{}", rust);
}

// ===== Layer 5: Special Functions & Complex Numbers =====

#[test]
fn test_math_gamma_transpile() {
    let rust = transpile("use math\nx = gamma[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::gamma("), "should call the runtime's gamma (the builtin table)\n{}", rust);
}

#[test]
fn test_math_erf_transpile() {
    let rust = transpile("use math\nx = erf[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::erf("), "should call the runtime's erf (the builtin table)\n{}", rust);
}

#[test]
fn test_math_bessel_transpile() {
    let rust = transpile("use math\nx = bessel[ones[3], 0]\n");
    assert!(rust.contains("crow_runtime::math::bessel("), "should call the runtime's bessel (the builtin table)\n{}", rust);
}

#[test]
fn test_math_factorial_transpile() {
    let rust = transpile("use math\nx = factorial[ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::factorial("), "should call the runtime's factorial (the builtin table)\n{}", rust);
}

#[test]
fn test_math_choose_transpile() {
    let rust = transpile("use math\nx = choose[ones[3], ones[3]]\n");
    assert!(rust.contains("crow_runtime::math::choose("), "should call the runtime's choose (the builtin table)\n{}", rust);
}

#[test]
fn test_math_complex_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\n");
    assert!(rust.contains("crow_runtime::math::complex("), "should call the runtime's complex (the builtin table)\n{}", rust);
}

#[test]
fn test_math_magnitude_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\nm = magnitude[c]\n");
    assert!(rust.contains("crow_runtime::math::magnitude("), "should call the runtime's magnitude (the builtin table)\n{}", rust);
}

#[test]
fn test_math_conjugate_transpile() {
    let rust = transpile("use math\nc = complex[3, 4]\ncc = conjugate[c]\n");
    assert!(rust.contains("crow_runtime::math::conjugate("), "should call the runtime's conjugate (the builtin table)\n{}", rust);
}

// ===== Sparse Arrays =====

#[test]
fn test_math_sparse_from_dense_transpile() {
    let rust = transpile("use math\ns = sparse[eye[3]]\n");
    assert!(rust.contains("crow_runtime::math::sparse(&("), "should call the runtime's sparse\n{}", rust);
}

#[test]
fn test_math_sparse_from_coo_transpile() {
    let rust = transpile("use math\ns = sparse[[0, 1], [0, 1], [5, 6], [2, 2]]\n");
    // four arguments: the row from entries, which checks each one
    assert!(rust.contains("crow_runtime::math::sparse_coo("), "should call the runtime's sparse_coo\n{}", rust);
}

#[test]
fn test_math_dense_transpile() {
    let rust = transpile("use math\ns = sparse[eye[2]]\nd = dense[s]\n");
    // a sparse argument takes dense's sparse row
    assert!(rust.contains("crow_runtime::math::dense(&("), "should call the runtime's dense\n{}", rust);
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
    // a sparse matrix takes solve's sparse row
    assert!(rust.contains("crow_runtime::math::solve_sparse(&("), "should call the runtime's solve_sparse\n{}", rust);
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
    assert!(rust.contains("crow_runtime::io::exists(&*"), "exists goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_mkdir_transpile() {
    let rust = transpile("mkdir['dir']\n");
    assert!(rust.contains("crow_runtime::io::mkdir(&*"), "mkdir goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_ls_transpile() {
    let rust = transpile("f = ls['.']\n");
    assert!(rust.contains("crow_runtime::io::ls(&*"), "ls goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_rm_transpile() {
    let rust = transpile("rm['file.txt']\n");
    assert!(rust.contains("crow_runtime::io::rm(&*"), "rm is its row, which borrows its path\n{}", rust);
}

#[test]
fn test_fs_mv_transpile() {
    let rust = transpile("mv['a.txt', 'b.txt']\n");
    assert!(rust.contains("crow_runtime::io::mv(&*"), "mv goes through the runtime\n{}", rust);
}

#[test]
fn test_fs_cp_transpile() {
    let rust = transpile("cp['a.txt', 'b.txt']\n");
    assert!(rust.contains("crow_runtime::io::cp(&*"), "cp goes through the runtime\n{}", rust);
}

// ===== env builtin =====

#[test]
fn test_env_one_arg_transpile() {
    let rust = transpile("x = env['PORT']\n");
    assert!(rust.contains("crow_runtime::core::env(&*("), "env is its row\n{}", rust);
    assert!(rust.contains("\"env\".to_string()"), "1-arg env is an error value (kind env) when unset\n{}", rust);
    assert!(rust.contains("captured: crow_runtime::moments::now()"), "env is a reader: its result carries a capture moment\n{}", rust);
}

#[test]
fn test_env_two_arg_transpile() {
    let rust = transpile("x = env['PORT', 3000]\n");
    // the default is text, as the variable is (a number shows as it displays)
    assert!(rust.contains("crow_runtime::core::env_or(&*("), "the 2-arg env is its row\n{}", rust);
    assert!(rust.contains("crow_runtime::cell::Cell::from("), "the default is read as a cell\n{}", rust);
}

// ===== Web tests =====

#[test]
fn test_web_imports() {
    let rust = transpile("use web\nfn h[req]\n    return text['ok']\nroute['/', h]\n");
    assert!(rust.contains("use crow_web::server::CrowApp;"), "should import CrowApp\n{}", rust);
    assert!(rust.contains("use crow_web::core::{CrowRequest, CrowResponse};"), "should import CrowRequest/CrowResponse\n{}", rust);
    assert!(rust.contains("use std::sync::Arc;"), "should import Arc\n{}", rust);
}

#[test]
fn test_web_async_main() {
    let rust = transpile("use web\nfn h[req]\n    return text['ok']\nroute['/', h]\n");
    assert!(rust.contains("async fn __crow_main()"), "should emit an async main\n{}", rust);
    // the runtime's threads get stacks sized for a page's work
    assert!(rust.contains(".thread_stack_size(64 * 1024 * 1024)"), "should size the runtime's stacks\n{}", rust);
}

#[test]
fn test_web_route_transpile() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nroute['/', handler]\n");
    assert!(rust.contains(".route("), "should emit .route()\n{}", rust);
    // the handler goes by its name: a stop answers 500 naming it
    assert!(rust.contains("crow_web::server::CrowHandler::new(\"handler\", handler)"), "should name the handler\n{}", rust);
}

#[test]
fn test_web_serve_transpile() {
    let rust = transpile("use web\nuse ui\nview render[s]\n    return div[h1['hi']]\n\nfn handle[s, e, v]\n    log[e]\n\nserve[3000]\npage['/', render, handle]\n");
    assert!(rust.contains(".files_from(&\"\".to_string()).serve(3000_i64 as u16)"), "should serve with the server's options\n{}", rust);
    assert!(rust.contains(".await"), "should emit .await\n{}", rust);
}

#[test]
fn test_web_text_response() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nroute['/', handler]\n");
    assert!(rust.contains("CrowResponse::text("), "should emit CrowResponse::text()\n{}", rust);
}

#[test]
fn test_web_html_response() {
    let rust = transpile("use web\nfn handler[req]\n    return html['<h1>hi</h1>']\n\nroute['/', handler]\n");
    assert!(rust.contains("CrowResponse::html("), "should emit CrowResponse::html()\n{}", rust);
}

#[test]
fn test_web_redirect_response() {
    let rust = transpile("use web\nfn handler[req]\n    return redirect['/home']\n\nroute['/', handler]\n");
    assert!(rust.contains("CrowResponse::redirect("), "should emit CrowResponse::redirect()\n{}", rust);
}

#[test]
fn test_web_respond_response() {
    let rust = transpile("use web\nfn handler[req]\n    return respond[404, 'not found']\n\nroute['/', handler]\n");
    assert!(rust.contains("CrowResponse::respond("), "should emit CrowResponse::respond()\n{}", rust);
}

#[test]
fn test_web_handler_types() {
    let rust = transpile("use web\nfn handler[req]\n    return text['hello']\n\nroute['/', handler]\n");
    // Handler should have CrowRequest param and CrowResponse return
    assert!(rust.contains("crow_web::core::CrowRequest"), "handler param should be CrowRequest\n{}", rust);
    assert!(rust.contains("crow_web::core::CrowResponse"), "handler return should be CrowResponse\n{}", rust);
}

#[test]
fn test_web_no_async_without_web() {
    let rust = transpile("x = 1 + 2\n");
    assert!(!rust.contains("async fn"), "should NOT emit async main without web\n{}", rust);
    assert!(!rust.contains("tokio::runtime"), "should NOT start a runtime without web\n{}", rust);
}

// --- P4: Web module codegen tests ---

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

/// A function nothing the program runs calls, whose list parameter holds
/// items of no known type, is left out: its list would be text in Rust,
/// and a kernel that takes numbers refused it (a shared part's scanner in
/// a part's test app). Called with numbers, it is compiled.
#[test]
fn an_unreached_function_of_unknown_items_is_left_out() {
    let src = "use chart\nfn inner[xs]\n    r = rsi[xs, 14]\n    return r[0]\nfn outer[b]\n    xs = []\n    xs = push[xs, b]\n    return inner[xs]\n";
    let rust = transpile(&format!("{}log[1]\n", src));
    assert!(!rust.contains("fn inner("), "{}", rust);
    let rust = transpile(&format!("{}log[inner[[1.0, 2.0]]]\n", src));
    assert!(rust.contains("fn inner("), "{}", rust);
}

/// A handler's mail waits for the session's commit, one whose answer is
/// read is sent when asked, and the mailer is a process-wide static like a
/// connection, never a session's fact.
#[test]
fn mail_in_a_handler_goes_through_the_session() {
    let rust = transpile("use web, ui\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\nn = 0\nfn welcome[]\n    mail[mailer, 'ada@example.com', 'Hello', 'Welcome']\nfn asked[]\n    r = mail[mailer, 'cy@example.com', 'Asked', 'now']\n    if r.ok\n        n = 1\nview v[]\n    return stack[button['A', click = welcome], button['B', click = asked]]\npage['/', v]\n");
    assert!(rust.contains("state.mail(_mail, _send)"), "held to the commit\n{}", rust);
    assert!(rust.contains("state.mail_now(_mail, _send)"), "sent when asked\n{}", rust);
    assert!(rust.contains("static __crow_handle_mailer: std::sync::OnceLock<crow_web::mail::CrowMailer>"), "a static mailer\n{}", rust);
    assert!(!rust.contains("\"mailer\", serde_json"), "not a session fact\n{}", rust);
    // a script sends at once
    let rust = transpile("use web\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']\nr = mail[mailer, 'ada@example.com', 'Hello', 'Welcome']\nlog[r.ok]\n");
    assert!(rust.contains(".send_blocking(&_to, &_subject, &_body)"), "{}", rust);
}

#[test]
fn test_web_socket() {
    // a socket, and what acts on it, are crow_web's
    let rust = transpile("use web\nuse web.ws\nurl = 'ws://localhost:3000'\nsock = socket[url]\nsend[sock, 'hi']\nmsg = recv[sock, 5]\nclose[sock]\n");
    assert!(rust.contains("crow_web::ws::open("), "should open a socket\n{}", rust);
    assert!(rust.contains(".send(&"), "should emit .send()\n{}", rust);
    assert!(rust.contains(".recv(Some((5_i64) as f64))"), "should emit .recv() with its wait\n{}", rust);
    assert!(rust.contains(".close();"), "should emit .close()\n{}", rust);
}

// --- DB module codegen tests ---

#[test]
fn test_db_connect() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\n");
    assert!(rust.contains("crow_db::schema::connect("), "should emit crow_db::schema::connect()\n{}", rust);
    assert!(rust.contains("None"), "should pass None for default pool\n{}", rust);
    // a connection that cannot open stops as the interpreter stops
    assert!(rust.contains(".unwrap_or_else(|e| crow_runtime::stops::halt(2, &e))"), "should stop with the reason\n{}", rust);
}

#[test]
fn test_db_connect_pool_size() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db', 5]\n");
    assert!(rust.contains("crow_db::schema::connect("), "should emit crow_db::schema::connect()\n{}", rust);
    assert!(rust.contains("Some(5"), "should pass Some(5) for pool size\n{}", rust);
}

/// A connection is the database's in a program that also uses web, and
/// a URL the source does not spell builds every backend it could name: a
/// route's handler connecting with `env['DATABASE_URL']` had compiled to a
/// call to nothing, and a URL put together from parts had found no backend
#[test]
fn a_web_programs_connection_from_the_environment_compiles() {
    let src = "use web, db\nfn count[req]\n    conn = connect[env['DATABASE_URL']]\n    q = query[conn, 'SELECT 1 AS one']\n    return text['ok']\nroute['/count', count]\nserve[3000]\n";
    let rust = transpile(src);
    assert!(rust.contains("crow_db::schema::connect("), "{}", rust);
    assert!(!rust.contains("= connect("), "{}", rust);
    let stmts = crate::lower::lower_program(&crate::build(src).expect("parse failed")).expect("lowering failed");
    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowering to the IR failed");
    let emitted = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false, None).expect("emitting failed");
    let features: Vec<&str> = emitted.db_features.iter().map(String::as_str).collect();
    assert_eq!(features, ["clickhouse", "postgres", "sqlite"]);
    // a URL the source spells builds its own backend alone
    let stmts = crate::lower::lower_program(&crate::build("use db\nconn = connect['sqlite://a.db']\n").expect("parse failed")).expect("lowering failed");
    let mut ctx = TypeContext::new();
    ctx.infer_program(&stmts);
    let program = crate::lower_ir::lower(&stmts, &mut ctx, "t").expect("lowering to the IR failed");
    let emitted = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false, None).expect("emitting failed");
    assert_eq!(emitted.db_features.iter().map(String::as_str).collect::<Vec<_>>(), ["sqlite"]);
}

#[test]
fn test_db_connect_prefixed() {
    let rust = transpile("use db\nconn = db.connect['sqlite://test.db']\n");
    assert!(rust.contains("crow_db::schema::connect("), "should emit crow_db::schema::connect() for db.connect\n{}", rust);
}

/// A connection the program declares tables on opens with the statements
/// declaring them (spec, db, The schema a program declares).
#[test]
fn test_db_connect_declared() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\nexec[conn, 'CREATE TABLE IF NOT EXISTS t (id INTEGER, name TEXT)']\n");
    assert!(
        rust.contains("crow_db::schema::connect(&\"sqlite://test.db\".to_string(), Some(0_i64 as usize), &[\"CREATE TABLE IF NOT EXISTS t (id INTEGER, name TEXT)\"])"),
        "should open with the declared statements\n{}",
        rust
    );
}

#[test]
fn test_db_exec() {
    let rust = transpile("use db\nconn = connect['sqlite://test.db']\nexec[conn, 'CREATE TABLE t [id INT]']\n");
    assert!(rust.contains(".exec("), "should emit .exec()\n{}", rust);
    // a statement that fails answers its reason (the `.unwrap()` this had
    // looked for was the connection's, which now stops with its reason)
    assert!(rust.contains("Err(_e) => CrowResult { ok: false, value: 0_i64, reason: _e"), "should answer the reason\n{}", rust);
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
    let rust = transpile("use web\nfn handler[req]\n    m = map[]\n    return msgpack[m]\n\nroute['/', handler]\n");
    // packed by crow_web, which the interpreter calls too
    assert!(rust.contains("CrowResponse::msgpack_of(&serde_json::to_value("), "should emit CrowResponse::msgpack_of()\n{}", rust);
}

#[test]
fn test_web_arrow_response() {
    let rust = transpile("use web\nuse frame\nfn handler[req]\n    f = load['data.csv']\n    return arrow[f]\n\nroute['/', handler]\n");
    // written by crow_web, which the interpreter calls too
    assert!(rust.contains("CrowResponse::arrow_of("), "should emit CrowResponse::arrow_of()\n{}", rust);
}

/// A route's handler whose value can fail answers the failure as the
/// interpreter's route does — 500, its name and the reason — where it had
/// been declared to answer a plain response and had not built
#[test]
fn test_web_fallible_handler_answers_its_failure() {
    let rust = transpile("use web, frame\nfn table[req]\n    return arrow[frame.load['data.csv']]\nroute['/table', table]\nserve[3999]\n");
    assert!(rust.contains("fn table(req: crow_web::core::CrowRequest) -> CrowResult<crow_web::core::CrowResponse>"), "the handler keeps its Result\n{}", rust);
    assert!(
        rust.contains("crow_web::server::CrowHandler::new(\"table\", move |__req: crow_web::core::CrowRequest| { let _r = table(__req); if _r.ok { _r.value } else { crow_web::server::route_stopped(\"table\", &_r.reason) } })"),
        "the route answers the failure\n{}",
        rust
    );
}

#[test]
fn test_web_bytes_response() {
    let rust = transpile("use web\nfn handler[req]\n    return bytes['binary data']\n\nroute['/', handler]\n");
    assert!(rust.contains("CrowResponse::binary("), "should emit CrowResponse::binary()\n{}", rust);
    assert!(rust.contains("application/octet-stream"), "should set octet-stream content-type\n{}", rust);
}

#[test]
fn test_web_stream_response() {
    let rust = transpile("use web\nfn sender[emit, n]\n    emit['chunk `n`']\n\nfn handler[req]\n    return stream[sender, 3]\n\nroute['/', handler]\n");
    // the feed runs when the response is sent, handed put and the
    // arguments taken by value
    assert!(rust.contains("CrowResponse::stream(\"sender\", move |put| { let _ = sender(put, __s0); Ok(()) })"), "should hand the feed put and its arguments\n{}", rust);
    assert!(rust.contains("fn sender(emit: fn(String) -> bool, n: i64)"), "the feed takes put, a function of text\n{}", rust);
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
    let rust = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, true, None).expect("emitting failed").source;
    assert!(rust.contains("fn __crow_view(") && rust.contains("\"v\" => Some(v(state)),"), "{}", rust);
    assert!(rust.contains("fn __crow_derived(") && rust.contains("\"twice\" =>"), "{}", rust);
    assert!(rust.contains("crow_web::live::test_control::router(") && rust.contains("__crow_app.serve(0)"), "{}", rust);
    let plain = crate::emit_rs::emit_with(&program, ctx.registry.clone(), "t.co", Default::default(), None, false, None).map(|e| e.source).unwrap_or_default();
    assert!(!plain.contains("test_control"), "{}", plain);
}

#[test]
fn test_ui_imports() {
    let rust = transpile("use web\nuse ui\nview v[]\n    return p['x']\npage['/', v]\n");
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
    assert!(rust.contains(".serve(3000_i64 as u16)"), "{}", rust);
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
    // one runtime function for a number and a text, as the interpreter calls
    let rust = transpile("x = fmt[1234.5, decimals = 2, commas = true]\nlog[x]\n");
    assert!(rust.contains("crow_runtime::display::fmt_value(&crow_runtime::cell::Cell::from("), "fmt should call the display kernel\n{}", rust);
    let rust = transpile("x = fmt['hi', width = 4]\nlog[x]\n");
    assert!(rust.contains("crow_runtime::display::fmt_value("), "string fmt should call the same kernel\n{}", rust);
    // a moment's time style is its own row, able to fail (kind 'time')
    let rust = transpile("t = now[]\nx = fmt[t, time = 'iso']\nlog[x]\n");
    assert!(rust.contains("crow_runtime::moments::fmt_moment("), "{}", rust);
    assert!(rust.contains("kind: \"time\""), "{}", rust);
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
    assert!(rust.contains("state.get_f64(\"deleted\", ((0_i64) as f64))"), "the key column is float, so the fact is\n{}", rust);
}

/// A function that assigns a fact's name, called from a view, would change
/// state mid-render: the interpreter wrote the fact, and the compiled
/// program assigned a variable it never declared and did not build (the IR
/// verifier found the fact stored a float it did not hold). It is refused,
/// in both modes (docs/bug-classes.md, decision 2).
#[test]
fn a_view_calling_what_assigns_a_fact_is_refused() {
    let stmts = crate::build("use web, ui\nn = 0\nfn scale[x]\n    n = 2.5\n    return n * x\nview render[]\n    return p['`n` `scale[2]`']\npage['/', render]\n").expect("parses");
    let err = crate::ir_interp::prepare(&stmts, "t").map(|_| ()).expect_err("a view may not call what writes state");
    assert!(err.contains("a view calls 'scale', which assigns reactive state"), "{}", err);
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
/// that path too). `with` can fail — values of another length than the
/// frame's rows — and a column of a frame that failed is refused before it
/// is read, so the function hands that failure back: its value is a result
/// of the float the cell is
#[test]
fn a_frame_assigned_beside_an_independent_statement_keeps_its_column_types() {
    let rust = transpile("use frame\nfn go[]\n    rows = with[frame.new[], 'v', [3.5, 4.5]]\n    k = 2\n    return rows.v[0] * k\nlog[go[]]\n");
    assert!(!rust.contains("rayon::join"), "statements run in order\n{}", rust);
    assert!(rust.contains("-> CrowResult<f64>"), "the cell is a float, so the product is\n{}", rust);
    assert!(rust.contains("if !_c.ok { return CrowResult { ok: false, value: 0.0_f64"), "a frame that failed is handed back\n{}", rust);
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
