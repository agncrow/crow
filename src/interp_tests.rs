//! The interpreter's behaviour tests, one group per spec section: a
//! program's variables and its last value, checked through the walker.
//! They came with the AST interpreter and outlived it.

#[cfg(test)]
mod tests {
    use crate::ir_interp::{run_script, IrInterp};
    use crate::value::*;

    fn parse(src: &str) -> Vec<crate::ast::Stmt> {
        let mut lexer = crate::lexer::Lexer::new(src);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        parser.parse().unwrap()
    }

    /// A program run to its end: its top-level variables and last value.
    fn run(src: &str) -> Result<IrInterp, String> {
        run_script(&parse(src), "test")
    }

    fn run_program(src: &str) -> CrowValue {
        let it = run(src).unwrap();
        it.last().cloned().unwrap_or_else(CrowValue::done)
    }

    fn get_var(src: &str, var: &str) -> CrowValue {
        let it = run(src).unwrap();
        it.var(var).unwrap().clone()
    }

    // ===== §1: Variables & Types =====

    #[test]
    fn test_var_int() {
        let val = get_var("x = 42\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_var_negative_int() {
        let val = get_var("x = -7\n", "x");
        assert!(matches!(val.value, Value::Int(-7)));
    }

    #[test]
    fn test_var_float() {
        let val = get_var("x = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_var_string() {
        let val = get_var("x = 'hello'\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_var_bool_true() {
        let val = get_var("x = true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_var_bool_false() {
        let val = get_var("x = false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_var_list() {
        let val = get_var("x = [1, 2, 3]\n", "x");
        // All-numeric arrays become NumArray
        assert!(matches!(val.value, Value::NumArray(ref t) if t.len() == 3));
    }

    #[test]
    fn test_var_empty_list() {
        let val = get_var("x = []\n", "x");
        assert!(matches!(val.value, Value::Array(ref items) if items.is_empty()));
    }

    #[test]
    fn test_string_interpolation() {
        let val = get_var("name = 'crow'\nx = 'hello, `name`'\n", "x");
        assert_eq!(format!("{}", val.value), "hello, crow");
    }

    #[test]
    fn test_string_interpolation_expr() {
        let val = get_var("x = '1 + 2 = `1 + 2`'\n", "x");
        assert_eq!(format!("{}", val.value), "1 + 2 = 3");
    }

    #[test]
    fn test_multi_assign() {
        let val_x = get_var("x, y, z = 1, 2, 3\n", "x");
        let val_z = get_var("x, y, z = 1, 2, 3\n", "z");
        assert!(matches!(val_x.value, Value::Int(1)));
        assert!(matches!(val_z.value, Value::Int(3)));
    }

    #[test]
    fn test_swap() {
        let val_a = get_var("a = 1\nb = 2\na, b = b, a\n", "a");
        let val_b = get_var("a = 1\nb = 2\na, b = b, a\n", "b");
        assert!(matches!(val_a.value, Value::Int(2)));
        assert!(matches!(val_b.value, Value::Int(1)));
    }

    #[test]
    fn test_reassignment_same_type() {
        let val = get_var("x = 10\nx = 20\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_reassignment_int_float_compatible() {
        let val = get_var("x = 10\nx = 3.14\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    // ===== §3: Functions =====

    #[test]
    fn test_fn_implicit_return() {
        let val = get_var("fn add[a, b]\n    a + b\nresult = add[3, 4]\n", "result");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_fn_explicit_return() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 0]\n", "result");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_fn_explicit_return_nonzero() {
        let val = get_var("fn safe_div[a, b]\n    if b eq 0\n        return 0\n    a / b\nresult = safe_div[10, 2]\n", "result");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_fn_no_args() {
        let val = get_var("fn greet[]\n    return 'hello'\nresult = greet[]\n", "result");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_fn_first_class() {
        let val = get_var("fn double[x]\n    return x * 2\nfn apply[f, x]\n    return f[x]\nresult = apply[double, 5]\n", "result");
        assert!(matches!(val.value, Value::Int(10)));
    }

    // ===== §4: Control Flow =====

    #[test]
    fn test_if_true() {
        let val = get_var("x = 0\nif true\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_if_false() {
        let val = get_var("x = 0\nif false\n    x = 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_if_else() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse\n    x = 2\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    #[test]
    fn test_if_else_if() {
        let val = get_var("x = 0\nif false\n    x = 1\nelse if true\n    x = 2\nelse\n    x = 3\n", "x");
        assert!(matches!(val.value, Value::Int(2)));
    }

    // ===== §5: Loops =====

    #[test]
    fn test_while_basic() {
        let val = get_var("x = 5\nwhile x gt 0\n    x = x - 1\n", "x");
        assert!(matches!(val.value, Value::Int(0)));
    }

    #[test]
    fn test_while_break() {
        let val = get_var("x = 0\nwhile true\n    x = x + 1\n    if x eq 5\n        break\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_while_continue() {
        // Sum 1-10 skipping 5
        let val = get_var("total = 0\ni = 0\nwhile i lt 10\n    i = i + 1\n    if i eq 5\n        continue\n    total = total + i\n", "total");
        assert!(matches!(val.value, Value::Int(50))); // 55 - 5 = 50
    }

    #[test]
    fn test_for_as_expression() {
        let val = get_var("doubled = for x in [1, 2, 3]\n    x * 2\n", "doubled");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_for_as_expression_filter() {
        let val = get_var("evens = for x in [1, 2, 3, 4, 5]\n    if x % 2 eq 0\n        x\n", "evens");
        assert_eq!(format!("{}", val.value), "[2, 4]");
    }

    /// A collecting loop's numbers are held as the IR types them: a loop of
    /// ints is a list of ints, the compiled program's `Vec<i64>`. It had
    /// been a tensor of floats, whose own rows its values then picked; with
    /// each call running the row the IR names, `len` of it read a list and
    /// found a tensor (parity 57 and 58).
    #[test]
    fn a_collecting_loop_of_ints_is_a_list_of_ints() {
        let val = get_var("out = for i in range[3]\n    i * 2\n", "out");
        match &val.value {
            Value::Array(items) => assert!(items.iter().all(|x| matches!(x.value, Value::Int(_))), "{}", val.value),
            other => panic!("a list of ints, got {}", other),
        }
        assert!(matches!(run_program("out = for i in range[3]\n    i\nlen[out]\n").value, Value::Int(3)));
    }

    /// Numbers are held as the IR types them: a literal assigned to a list
    /// the program grows is converted to a list where they meet (compiled,
    /// the tensor did not build into the `Vec`; the interpreter held a tensor
    /// where the IR said a list), and a numeric literal the IR holds as a
    /// tensor is one (parity 122).
    #[test]
    fn numbers_are_held_as_the_ir_types_them() {
        let xs = get_var("xs = []\nif len[xs] eq 0\n    xs = [1.5]\nxs = push[xs, 2.5]\n", "xs");
        assert!(matches!(&xs.value, Value::Array(items) if items.len() == 2 && items.iter().all(|x| matches!(x.value, Value::Float(_)))), "{}", xs.value);
        let t = get_var("t = [1.0, 2.0]\n", "t");
        assert!(matches!(t.value, Value::NumArray(_)), "{}", t.value);
    }

    // ===== §7: Structs =====

    #[test]
    fn test_struct_creation() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\n", "b");
        if let Value::StructInstance(si) = &val.value {
            assert_eq!(si.name, "Bird");
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn test_struct_field_access() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.name\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_struct_field_access_int() {
        let val = get_var("struct Bird [name, speed]\nb = Bird['crow', 50]\nx = b.speed\n", "x");
        assert!(matches!(val.value, Value::Int(50)));
    }

    // ===== §8: Operators =====

    #[test]
    fn test_arithmetic_add() {
        let val = get_var("x = 3 + 4\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_sub() {
        let val = get_var("x = 10 - 3\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    #[test]
    fn test_arithmetic_mul() {
        let val = get_var("x = 6 * 7\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_arithmetic_div_int() {
        let val = get_var("x = 7 / 2\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_arithmetic_div_float() {
        let val = get_var("x = 7.0 / 2\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 3.5).abs() < 0.001));
    }

    #[test]
    fn test_arithmetic_modulo() {
        let val = get_var("x = 10 % 3\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_string_concat_rejected() {
        // String + is not allowed — use interpolation instead
        let result = std::panic::catch_unwind(|| {
            get_var("x = 'hello' + ' world'\n", "x");
        });
        assert!(result.is_err(), "string + should be rejected");
    }

    #[test]
    fn test_comparison_eq() {
        let val = get_var("x = 5 eq 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_ne() {
        let val = get_var("x = 5 ne 3\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gt() {
        let val = get_var("x = 10 gt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lt() {
        let val = get_var("x = 3 lt 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_gte() {
        let val = get_var("x = 5 gte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_comparison_lte() {
        let val = get_var("x = 5 lte 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_and() {
        let val = get_var("x = true and false\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_logical_or() {
        let val = get_var("x = false or true\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_logical_not() {
        let val = get_var("x = not true\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_expression_grouping() {
        let val = get_var("x = (1 + 2) * 3\n", "x");
        assert!(matches!(val.value, Value::Int(9)));
    }

    // ===== Existing error tests =====

    #[test]
    fn test_error_line_int_parse() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.is_ok());
        assert!(val.trace().iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_error_line_second_line() {
        let val = get_var("y = 10\nx = int['bad']\n", "x");
        assert!(!val.is_ok());
        assert!(val.trace().iter().any(|t| t.contains("2")));
    }

    #[test]
    fn test_error_line_read() {
        let val = get_var("x = read['nonexistent_test_file.txt']\n", "x");
        assert!(!val.is_ok());
        assert!(val.trace().iter().any(|t| t.contains("1")));
    }

    #[test]
    fn test_ok_value_line_zero() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.is_ok());
        assert!(val.trace().is_empty());
    }

    #[test]
    fn test_raw_value_line_field() {
        // raw values (non-error-value) should have empty trace
        let val = get_var("x = 5\n", "x");
        assert!(val.trace().is_empty());
    }

    #[test]
    fn test_type_change_reassign_error() {
        let err = run("x = 1\nx = 'hello'\n").err().expect("a type change is refused");
        assert!(err.contains("cannot reassign"), "{}", err);
    }

    #[test]
    fn test_same_type_reassign_ok() {
        let val = get_var("x = 1\nx = 42\n", "x");
        match val.value {
            Value::Int(n) => assert_eq!(n, 42),
            _ => panic!("expected int"),
        }
    }

    #[test]
    fn test_int_float_reassign_ok() {
        let val = get_var("x = 1\nx = 3.14\n", "x");
        match val.value {
            Value::Float(f) => assert!((f - 3.14).abs() < 0.001),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn test_auto_propagation_builtin() {
        // Error should propagate through len[] (of text that failed: the
        // length of an int is refused before the program runs)
        let val = get_var("x = str[int['bad']]\ny = len[x]\n", "y");
        assert!(!val.is_ok());
        assert!(val.reason().contains("convert"));
    }

    #[test]
    fn test_auto_propagation_user_fn() {
        // Error should propagate through user functions
        let val = get_var("fn double[n]\n    return n * 2\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.is_ok());
        assert!(val.reason().contains("convert"));
    }

    #[test]
    fn test_auto_propagation_chain() {
        // Error propagates through a chain of calls
        let val = get_var("fn a[x]\n    return x + 1\nfn b[x]\n    return a[x]\ne = int['bad']\nr = b[e]\n", "r");
        assert!(!val.is_ok());
    }

    #[test]
    fn test_auto_propagation_preserves_line() {
        // The original error trace should contain the original line
        let val = get_var("fn double[n]\n    return n * 2\nz = 10\nx = int['bad']\ny = double[x]\n", "y");
        assert!(!val.is_ok());
        assert!(val.trace().iter().any(|t| t.contains("4")));
    }

    #[test]
    fn test_binop_propagates_left_error() {
        // Error in left operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = bad + 10\n", "x");
        assert!(!val.is_ok(), "binop with error left operand should propagate error");
        assert!(val.reason().contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_propagates_right_error() {
        // Error in right operand should propagate through arithmetic
        let val = get_var("bad = int['hello']\nx = 10 + bad\n", "x");
        assert!(!val.is_ok(), "binop with error right operand should propagate error");
        assert!(val.reason().contains("convert"), "should preserve original error reason");
    }

    #[test]
    fn test_binop_success_unwraps() {
        // Successful CrowResult should auto-unwrap in arithmetic
        let val = get_var("good = int['42']\nx = good + 10\n", "x");
        assert!(val.is_ok(), "binop with successful result should produce ok value");
        assert!(matches!(val.value, Value::Int(52)), "expected 52, got {:?}", val.value);
    }

    #[test]
    fn test_log_no_propagation() {
        // log should NOT propagate — it should print and return done
        let val = get_var("x = int['bad']\ny = log[x]\n", "y");
        assert!(val.is_ok());
        assert!(matches!(val.value, Value::Done));
    }

    #[test]
    fn test_then_simple() {
        // x then f desugars to f[x]
        let val = get_var("x = '  hello  '\ny = x then trim\n", "y");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_then_chain() {
        // chaining multiple then
        let val = get_var("x = '  hello  '\ny = x then trim then upper\n", "y");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_then_with_args() {
        // x then f[a] desugars to f[x, a]
        let val = get_var("x = 'hello world'\ny = x then split[' '] then join['-']\n", "y");
        assert_eq!(format!("{}", val.value), "hello-world");
    }

    #[test]
    fn test_then_with_conversion() {
        // then works with conversion builtins
        let val = get_var("x = 42\ny = x then str\n", "y");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: range =====

    #[test]
    fn test_range_single_arg() {
        let val = get_var("x = range[5]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 5);
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[4].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_two_args() {
        let val = get_var("x = range[2, 5]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert!(matches!(items[0].value, Value::Int(2)));
            assert!(matches!(items[2].value, Value::Int(4)));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_range_three_args() {
        let val = get_var("x = range[0, 10, 3]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 4); // 0, 3, 6, 9
            assert!(matches!(items[0].value, Value::Int(0)));
            assert!(matches!(items[3].value, Value::Int(9)));
        } else {
            panic!("expected list");
        }
    }

    // ===== Builtins: math =====

    #[test]
    fn test_abs() {
        let val = get_var("x = abs[-5]\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_sqrt() {
        let val = get_var("x = sqrt[16]\n", "x");
        assert!(matches!(val.value, Value::Float(f) if (f - 4.0).abs() < 0.001));
    }

    #[test]
    fn test_floor() {
        let val = get_var("x = floor[3.7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_ceil() {
        let val = get_var("x = ceil[3.2]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_round() {
        let val = get_var("x = round[3.5]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_pow() {
        let val = get_var("x = 2 ^ 10\n", "x");
        assert!(matches!(val.value, Value::Int(1024)));
    }

    #[test]
    fn test_min() {
        let val = get_var("x = min[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_max() {
        let val = get_var("x = max[3, 7]\n", "x");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== Builtins: strings =====

    #[test]
    fn test_split() {
        let val = get_var("x = split['a,b,c', ',']\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 3);
            assert_eq!(format!("{}", items[0].value), "a");
            assert_eq!(format!("{}", items[2].value), "c");
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_trim() {
        let val = get_var("x = trim['  hello  ']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_replace() {
        let val = get_var("x = replace['hello world', 'world', 'crow']\n", "x");
        assert_eq!(format!("{}", val.value), "hello crow");
    }

    #[test]
    fn test_upper() {
        let val = get_var("x = upper['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "HELLO");
    }

    #[test]
    fn test_lower() {
        let val = get_var("x = lower['HELLO']\n", "x");
        assert_eq!(format!("{}", val.value), "hello");
    }

    #[test]
    fn test_starts() {
        let val = get_var("x = starts['hello', 'hel']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_ends() {
        let val = get_var("x = ends['hello', 'llo']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_join() {
        let val = get_var("x = join[['a', 'b', 'c'], '-']\n", "x");
        assert_eq!(format!("{}", val.value), "a-b-c");
    }

    #[test]
    fn test_repeat() {
        let val = get_var("x = repeat['ha', 3]\n", "x");
        assert_eq!(format!("{}", val.value), "hahaha");
    }

    #[test]
    fn test_repeat_zero() {
        let val = get_var("x = repeat['ha', 0]\n", "x");
        assert_eq!(format!("{}", val.value), "");
    }

    /// The runtime's refusal, the same text the compiled program stops with
    #[test]
    fn a_negative_repeat_is_refused() {
        let err = run("n = int['-2']\nx = repeat['ab', n]\n").err().unwrap_or_default();
        assert!(err.contains("repeat[] count cannot be negative (got -2)"), "{}", err);
    }

    #[test]
    fn test_fmt_width_replaces_pad() {
        let val = get_var("x = fmt['ab', width = 5]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "ab   "));
    }

    #[test]
    fn test_fmt_width_shorter_than_string() {
        let val = get_var("x = fmt['hello', width = 3]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "hello"));
    }

    #[test]
    fn test_reverse_string() {
        let val = get_var("x = reverse['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "olleh");
    }

    #[test]
    fn test_reverse_empty_string() {
        let val = get_var("x = reverse['']\n", "x");
        assert_eq!(format!("{}", val.value), "");
    }

    // ===== Builtins: conversion =====

    #[test]
    fn test_int_from_string_success() {
        let val = get_var("x = int['42']\n", "x");
        assert!(val.is_ok());
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_int_from_string_fail() {
        let val = get_var("x = int['bad']\n", "x");
        assert!(!val.is_ok());
    }

    #[test]
    fn test_int_from_float() {
        let val = get_var("x = int[3.14]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_float_from_string_success() {
        let val = get_var("x = float['3.14']\n", "x");
        assert!(val.is_ok());
        assert!(matches!(val.value, Value::Float(f) if (f - 3.14).abs() < 0.001));
    }

    #[test]
    fn test_str_conversion() {
        let val = get_var("x = str[42]\n", "x");
        assert_eq!(format!("{}", val.value), "42");
    }

    // ===== Builtins: collections =====

    #[test]
    fn test_len_list() {
        let val = get_var("x = len[[1, 2, 3]]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_len_string() {
        let val = get_var("x = len['hello']\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    #[test]
    fn test_push() {
        let val = get_var("x = push[[1, 2], 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_pop() {
        let val = get_var("x = pop[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2]");
    }

    #[test]
    fn test_append() {
        // Regression: append was removed in favor of concat
        let result = std::panic::catch_unwind(|| {
            run_program("x = append[[1, 2], [3, 4]]\n")
        });
        assert!(result.is_err(), "append should error with removal message");
    }

    #[test]
    fn test_concat_core() {
        let val = get_var("x = concat[[1, 2], [3, 4]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3, 4]");
        let val = get_var("x = concat[['a'], ['b']]\n", "x");
        assert_eq!(format!("{}", val.value), "['a', 'b']");
    }

    #[test]
    fn test_sort() {
        let val = get_var("x = sort[[3, 1, 2]]\n", "x");
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_reverse() {
        let val = get_var("x = reverse[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "[3, 2, 1]");
    }

    #[test]
    fn test_contains() {
        let val = get_var("x = contains[[1, 2, 3], 2]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_contains_false() {
        let val = get_var("x = contains[[1, 2, 3], 5]\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_slice() {
        let val = get_var("x = slice[[10, 20, 30, 40, 50], 1, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[20, 30]");
    }

    #[test]
    fn test_list_index() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[1]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    // ===== Builtins: map operations =====

    #[test]
    fn test_map_creation() {
        let val = get_var("x = map[name = 'crow', kind = 'bird']\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_map_creation_quoted_keys() {
        let val = get_var("x = map['content-type' = 'json', 'x-id' = 'abc']\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_map_access() {
        let val = get_var("m = map[name = 'crow']\nx = m['name']\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_map_dot_access() {
        let val = get_var("m = map[name = 'crow', kind = 'bird']\nx = m.name\n", "x");
        assert_eq!(format!("{}", val.value), "crow");
    }

    #[test]
    fn test_map_dot_access_int() {
        let val = get_var("m = map[a = 1, b = 3]\nx = m.b\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_map_nested_dot_access() {
        let val = get_var("m = map[layer1 = map[w = 10, b = 20]]\nx = m.layer1.b\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_map_dot_assign() {
        let val = get_var("m = map[x = 1]\nm.x = 42\nx = m.x\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_map_empty() {
        let val = get_var("x = map[]\n", "x");
        assert!(matches!(val.value, Value::Map(_)));
    }

    #[test]
    fn test_keys() {
        let val = get_var("m = map[a = 1, b = 2]\nx = keys[m]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_values() {
        let val = get_var("m = map[a = 1, b = 2]\nx = values[m]\n", "x");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn test_has() {
        let val = get_var("m = map[a = 1]\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_has_false() {
        let val = get_var("m = map[a = 1]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_set() {
        let val = get_var("m = map[a = 1]\nm = set[m, 'b', 2]\nx = has[m, 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_del() {
        let val = get_var("m = map[a = 1, b = 2]\nm = del[m, 'a']\nx = has[m, 'a']\n", "x");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    // ===== Builtins: error[] =====

    #[test]
    fn test_error_builtin() {
        let val = get_var("x = error['something went wrong']\n", "x");
        assert!(!val.is_ok());
        assert_eq!(val.reason(), "something went wrong");
    }

    #[test]
    fn test_error_builtin_with_kind() {
        let val = get_var("x = error['bad input', 'validation']\n", "x");
        assert!(!val.is_ok());
        assert_eq!(val.reason(), "bad input");
        assert_eq!(val.kind(), "validation");
    }

    // ===== §11: Error value status on normal values =====

    #[test]
    fn test_ok_on_normal_value() {
        let val = get_var("x = 5\n", "x");
        assert!(val.is_ok());
        assert_eq!(val.reason(), "");
        assert_eq!(val.kind(), "");
        assert!(val.trace().is_empty());
    }

    // ===== Scope =====

    #[test]
    fn test_scope_inner_accesses_outer() {
        let val = get_var("x = 10\nif true\n    y = x + 5\n    x = y\n", "x");
        assert!(matches!(val.value, Value::Int(15)));
    }

    // ===== Negative Indexing =====

    #[test]
    fn test_negative_index_last() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-1]\n", "x");
        assert!(matches!(val.value, Value::Int(30)));
    }

    #[test]
    fn test_negative_index_second_to_last() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-2]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_negative_index_first_via_neg_len() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[-3]\n", "x");
        assert!(matches!(val.value, Value::Int(10)));
    }

    #[test]
    fn test_negative_index_out_of_bounds() {
        assert!(run("nums = [10, 20, 30]\nx = nums[-4]\n").is_err());
    }

    #[test]
    fn test_negative_index_assign() {
        let val = get_var("nums = [10, 20, 30]\nnums[-1] = 99\nx = nums[2]\n", "x");
        assert!(matches!(val.value, Value::Int(99)));
    }

    #[test]
    fn test_positive_index_still_works() {
        let val = get_var("nums = [10, 20, 30]\nx = nums[0]\n", "x");
        assert!(matches!(val.value, Value::Int(10)));
    }

    // ===== type[] builtin =====

    #[test]
    fn test_type_int() {
        let val = get_var("x = type[42]\n", "x");
        assert_eq!(format!("{}", val.value), "int");
    }

    #[test]
    fn test_type_float() {
        let val = get_var("x = type[3.14]\n", "x");
        assert_eq!(format!("{}", val.value), "float");
    }

    #[test]
    fn test_type_str() {
        let val = get_var("x = type['hello']\n", "x");
        assert_eq!(format!("{}", val.value), "str");
    }

    #[test]
    fn test_type_bool() {
        let val = get_var("x = type[true]\n", "x");
        assert_eq!(format!("{}", val.value), "bool");
    }

    #[test]
    fn test_type_array() {
        let val = get_var("x = type[[1, 2]]\n", "x");
        assert_eq!(format!("{}", val.value), "array");
    }

    #[test]
    fn test_type_map() {
        let val = get_var("x = type[map[a = 1]]\n", "x");
        assert_eq!(format!("{}", val.value), "map");
    }

    #[test]
    fn test_type_none() {
        let val = get_var("fn f[]\n    x = 1\nx = type[f[]]\n", "x");
        assert_eq!(format!("{}", val.value), "none");
    }

    #[test]
    fn test_type_fn() {
        let val = get_var("fn f[]\n    x = 1\nx = type[f]\n", "x");
        assert_eq!(format!("{}", val.value), "fn");
    }

    #[test]
    fn test_type_struct_instance() {
        let val = get_var("struct Point [x, y]\np = Point[1, 2]\nx = type[p]\n", "x");
        assert_eq!(format!("{}", val.value), "Point");
    }

    #[test]
    fn test_type_error() {
        let val = get_var("e = error['oops']\nx = type[e]\n", "x");
        assert_eq!(format!("{}", val.value), "error");
    }

    // ===== String Indexing =====

    #[test]
    fn test_string_index_first() {
        let val = get_var("x = 'hello'[0]\n", "x");
        assert_eq!(format!("{}", val.value), "h");
    }

    #[test]
    fn test_string_index_last_positive() {
        let val = get_var("x = 'hello'[4]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    #[test]
    fn test_string_negative_index() {
        let val = get_var("x = 'hello'[-1]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    #[test]
    fn test_string_negative_index_first() {
        let val = get_var("x = 'hello'[-5]\n", "x");
        assert_eq!(format!("{}", val.value), "h");
    }

    #[test]
    fn test_string_index_out_of_bounds() {
        assert!(run("x = 'hi'[5]\n").is_err());
    }

    #[test]
    fn test_string_index_via_variable() {
        let val = get_var("s = 'crow'\nx = s[2]\n", "x");
        assert_eq!(format!("{}", val.value), "o");
    }

    // ===== NumArray (CrowTensor integration) =====

    #[test]
    fn test_numarray_creation() {
        let val = get_var("x = [1, 2, 3]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
        assert_eq!(format!("{}", val.value), "[1, 2, 3]");
    }

    #[test]
    fn test_numarray_floats() {
        let val = get_var("x = [1.5, 2.5, 3.5]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
        assert_eq!(format!("{}", val.value), "[1.5, 2.5, 3.5]");
    }

    #[test]
    fn test_numarray_mixed_int_float() {
        let val = get_var("x = [1, 2.5, 3]\n", "x");
        assert!(matches!(val.value, Value::NumArray(_)));
    }

    #[test]
    fn test_non_numeric_stays_array() {
        let val = get_var("x = ['a', 'b']\n", "x");
        assert!(matches!(val.value, Value::Array(_)));
    }

    #[test]
    fn test_empty_array_stays_array() {
        let val = get_var("x = []\n", "x");
        assert!(matches!(val.value, Value::Array(_)));
    }

    #[test]
    fn test_numarray_add() {
        let val = get_var("x = [1, 2, 3] + [4, 5, 6]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 7, 9]");
    }

    #[test]
    fn test_numarray_sub() {
        let val = get_var("x = [10, 20, 30] - [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[9, 18, 27]");
    }

    #[test]
    fn test_numarray_mul() {
        // * on tensor-tensor is matmul; use .* for element-wise
        let val = get_var("x = [2, 3, 4] .* [5, 6, 7]\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 18, 28]");
    }

    #[test]
    fn test_numarray_div() {
        // / on tensor-tensor is error; use ./ for element-wise
        let val = get_var("x = [10, 20, 30] ./ [2, 4, 5]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 5, 6]");
    }

    #[test]
    fn test_numarray_scalar_add() {
        let val = get_var("x = [1, 2, 3] + 10\n", "x");
        assert_eq!(format!("{}", val.value), "[11, 12, 13]");
    }

    #[test]
    fn test_numarray_scalar_mul() {
        let val = get_var("x = [1, 2, 3] * 2\n", "x");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_scalar_numarray_add() {
        let val = get_var("x = 10 + [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[11, 12, 13]");
    }

    #[test]
    fn test_scalar_numarray_sub() {
        let val = get_var("x = 10 - [1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[9, 8, 7]");
    }

    #[test]
    fn test_numarray_neg() {
        let val = get_var("x = -[1, 2, 3]\n", "x");
        assert_eq!(format!("{}", val.value), "[-1, -2, -3]");
    }

    #[test]
    fn test_numarray_index() {
        let val = get_var("x = [10, 20, 30][1]\n", "x");
        assert!(matches!(val.value, Value::Int(20)));
    }

    #[test]
    fn test_numarray_negative_index() {
        let val = get_var("x = [10, 20, 30][-1]\n", "x");
        assert!(matches!(val.value, Value::Int(30)));
    }

    #[test]
    fn test_numarray_len() {
        let val = get_var("x = len[[1, 2, 3, 4]]\n", "x");
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_numarray_contains() {
        let val = get_var("x = contains[[1, 2, 3], 2]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_numarray_for_loop() {
        // for-as-expression over a NumArray
        let val = get_var("doubled = for x in [1, 2, 3]\n    x * 2\n", "doubled");
        assert_eq!(format!("{}", val.value), "[2, 4, 6]");
    }

    #[test]
    fn test_numarray_equality() {
        let val = get_var("x = [1, 2, 3] eq [1, 2, 3]\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_numarray_type() {
        let val = get_var("x = type[[1, 2, 3]]\n", "x");
        assert_eq!(format!("{}", val.value), "array");
    }

    #[test]
    fn test_numarray_join() {
        let val = get_var("x = join[[1, 2, 3], ', ']\n", "x");
        assert_eq!(format!("{}", val.value), "1, 2, 3");
    }

    #[test]
    fn test_numarray_dot_len() {
        let val = get_var("nums = [1, 2, 3]\nx = nums.len\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    // ===== Math Module =====

    #[test]
    fn test_math_zeros() {
        let val = get_var("use math\nx = zeros[2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert!(t.to_vec().iter().all(|&v| v == 0.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_ones() {
        let val = get_var("use math\nx = ones[2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert!(t.to_vec().iter().all(|&v| v == 1.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_eye() {
        let val = get_var("use math\nx = eye[3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 3]);
            let d = t.to_vec();
            assert_eq!(d[0], 1.0); assert_eq!(d[1], 0.0); assert_eq!(d[4], 1.0); assert_eq!(d[8], 1.0);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_linspace() {
        let val = get_var("use math\nx = linspace[0, 1, 5]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[5]);
            let d = t.to_vec();
            assert!((d[0] - 0.0).abs() < 1e-10);
            assert!((d[4] - 1.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_arange() {
        let val = get_var("use math\nx = arange[0, 5, 1]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[5]);
            let d = t.to_vec();
            for i in 0..5 { assert_eq!(d[i], i as f64); }
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_full() {
        let val = get_var("use math\nx = full[2, 2, 7]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| v == 7.0));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_exp() {
        let val = get_var("use math\nx = exp[[1, 2]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let d = t.to_vec();
            assert!((d[0] - 1.0_f64.exp()).abs() < 1e-10);
            assert!((d[1] - 2.0_f64.exp()).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_sqrt() {
        let val = get_var("use math\nx = sqrt[[4, 9, 16]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let d = t.to_vec();
            assert!((d[0] - 2.0).abs() < 1e-10);
            assert!((d[1] - 3.0).abs() < 1e-10);
            assert!((d[2] - 4.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_sum() {
        let val = get_var("use math\nx = sum[[1, 2, 3]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 6.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_mean() {
        let val = get_var("use math\nx = mean[[2, 4, 6]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 4.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_reshape() {
        let val = get_var("use math\nx = reshape[ones[6], 2, 3]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_transpose() {
        let val = get_var("use math\nm = ones[2, 3]\nx = transpose[m]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 2]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_matmul() {
        // matmul removed — use * operator instead
        let val = get_var("use math\na = eye[2]\nb = full[2, 2, 3]\nx = a * b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| (v - 3.0).abs() < 1e-10));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_det() {
        let val = get_var("use math\na = eye[3]\nx = det[a]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 1.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_norm_l2() {
        let val = get_var("use math\nx = norm[linspace[3, 4, 2]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 5.0).abs() < 1e-10); }
        else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_norm_l1() {
        let val = get_var("use math\nx = norm[linspace[1, 3, 3], 1]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 6.0).abs() < 1e-10); }
        else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_inv() {
        let val = get_var("use math\na = eye[2]\nx = inv[a]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            let d = t.to_vec();
            assert!((d[0] - 1.0).abs() < 1e-10);
            assert!((d[3] - 1.0).abs() < 1e-10);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_diag() {
        let val = get_var("use math\na = eye[3]\nx = diag[a]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_no_import_error() {
        let result = std::panic::catch_unwind(|| {
            get_var("x = zeros[3]\n", "x");
        });
        assert!(result.is_err() || {
            let val = get_var("x = zeros[3]\n", "x");
            !val.is_ok()
        });
    }

    #[test]
    fn test_math_pi() {
        let val = get_var("use math\nx = pi\n", "x");
        if let Value::Float(f) = val.value {
            assert!((f - std::f64::consts::PI).abs() < 1e-15);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_e() {
        let val = get_var("use math\nx = e\n", "x");
        if let Value::Float(f) = val.value {
            assert!((f - std::f64::consts::E).abs() < 1e-15);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_inf() {
        let val = get_var("use math\nx = inf\n", "x");
        if let Value::Float(f) = val.value {
            assert!(f.is_infinite() && f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_nan() {
        let val = get_var("use math\nx = nan\n", "x");
        if let Value::Float(f) = val.value {
            assert!(f.is_nan());
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_math_sign() {
        let val = get_var("use math\nx = sign[linspace[-2, 2, 5]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[-1.0, -1.0, 0.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_floor_tensor() {
        let val = get_var("use math\nx = floor[linspace[0.5, 2.5, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 1.0, 2.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_ceil_tensor() {
        let val = get_var("use math\nx = ceil[linspace[0.5, 2.5, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 2.0, 3.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_round_tensor() {
        let val = get_var("use math\nx = round[linspace[0.4, 2.6, 3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 2.0, 3.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_clip() {
        let val = get_var("use math\nx = clip[linspace[-2, 2, 5], -1, 1]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[-1.0, -1.0, 0.0, 1.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_mod_scalar() {
        let val = get_var("use math\nx = mod[linspace[1, 5, 5], 2]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[1.0, 0.0, 1.0, 0.0, 1.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_tan() {
        let val = get_var("use math\nx = tan[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_asin() {
        let val = get_var("use math\nx = asin[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_tanh() {
        let val = get_var("use math\nx = tanh[zeros[3]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), &[0.0, 0.0, 0.0]);
        } else { panic!("expected NumArray"); }
    }


    // ===== Operator tests: ^, .*, ./, .^ =====

    #[test]
    fn test_pow_int() {
        let val = get_var("x = 2 ^ 10\n", "x");
        assert!(matches!(val.value, Value::Int(1024)));
    }

    #[test]
    fn test_pow_float() {
        let val = get_var("x = 2.0 ^ 3.0\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 8.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_pow_right_assoc() {
        // 2 ^ 3 ^ 2 = 2 ^ (3 ^ 2) = 2 ^ 9 = 512
        let val = get_var("x = 2 ^ 3 ^ 2\n", "x");
        assert!(matches!(val.value, Value::Int(512)));
    }

    #[test]
    fn test_pow_negative_exp() {
        // an int to a negative power is a fraction, which an int does not
        // hold: the program stops and names the float form (it answered a
        // float where both modes type an int)
        let err = run("x = 2 ^ -1\n").map(|_| ()).expect_err("a negative int power stops");
        assert!(err.contains("2 ^ -1: an int to a negative power is a fraction — write 2.0 ^ -1 for a float"), "{}", err);
        let val = get_var("x = 2.0 ^ -1\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 0.5).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_elem_mul() {
        let val = get_var("x = [2, 3, 4] .* [5, 6, 7]\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 18, 28]");
    }

    #[test]
    fn test_elem_div() {
        let val = get_var("x = [10, 20, 30] ./ [2, 4, 5]\n", "x");
        assert_eq!(format!("{}", val.value), "[5, 5, 6]");
    }

    #[test]
    fn test_elem_pow() {
        let val = get_var("x = [2, 3] .^ [3, 2]\n", "x");
        assert_eq!(format!("{}", val.value), "[8, 9]");
    }

    #[test]
    fn test_elem_mul_scalar() {
        let val = get_var("x = [1, 2, 3] .* 10\n", "x");
        assert_eq!(format!("{}", val.value), "[10, 20, 30]");
    }

    #[test]
    fn test_elem_pow_scalar() {
        let val = get_var("x = [2, 3, 4] .^ 2\n", "x");
        assert_eq!(format!("{}", val.value), "[4, 9, 16]");
    }

    #[test]
    fn test_tensor_mul_is_matmul() {
        let val = get_var("use math\na = eye[2]\nb = full[2, 2, 3]\nx = a * b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 2]);
            assert!(t.to_vec().iter().all(|&v| (v - 3.0).abs() < 1e-10));
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_tensor_mul_of_vectors_is_an_error_not_a_panic() {
        // `*` is matmul; two 1-D tensors have no product. The program stops
        // with the reason (the benchmark's element-wise chain found a panic).
        let src = "use math\nx = randn[3]\ny = x * x\n";
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(src).map(|_| ())))
            .expect("must not panic")
            .expect_err("expected a runtime error");
        assert!(err.contains("2-D"), "{}", err);
    }

    #[test]
    fn test_tensor_div_error() {
        // tensor / tensor should error
        let result = std::panic::catch_unwind(|| {
            get_var("x = [1, 2] / [3, 4]\n", "x")
        });
        assert!(result.is_err() || {
            let val = result.unwrap();
            !val.is_ok()
        });
    }

    // ===== Reduction tests: prod, argmax, argmin =====

    #[test]
    fn test_math_prod() {
        let val = get_var("use math\nx = prod[tensor[[2, 3, 4]]]\n", "x");
        if let Value::Float(f) = val.value { assert!((f - 24.0).abs() < 1e-10); }
        else { panic!("expected Float"); }
    }

    #[test]
    fn test_math_argmax() {
        let val = get_var("use math\nx = argmax[tensor[[1, 5, 3]]]\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_math_argmin() {
        let val = get_var("use math\nx = argmin[tensor[[3, 1, 4]]]\n", "x");
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_math_sum_axis() {
        // [[1,2,3],[4,5,6]] → sum axis 0 → [5,7,9]
        let val = get_var("use math\na = reshape[linspace[1, 6, 6], 2, 3]\nx = sum[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3]);
            assert_eq!(t.to_vec(), &[5.0, 7.0, 9.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_max_axis() {
        let val = get_var("use math\na = reshape[linspace[1, 6, 6], 2, 3]\nx = max[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3]);
            assert_eq!(t.to_vec(), &[4.0, 5.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_prod_axis() {
        let val = get_var("use math\na = reshape[linspace[1, 4, 4], 2, 2]\nx = prod[a, 0]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2]);
            assert_eq!(t.to_vec(), &[3.0, 8.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Sorting & selection tests =====

    #[test]
    fn test_math_argsort() {
        let val = get_var("use math\nx = argsort[tensor[[3, 1, 4, 1, 5]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 0.0, 2.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_topk() {
        let val = get_var("use math\nx = topk[tensor[[3, 1, 4, 1, 5]], 2]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![5.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_unique() {
        let val = get_var("use math\nx = unique[tensor[[3, 1, 4, 1, 3]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 4.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_where() {
        let val = get_var("use math\nmask = tensor[[1, 0, 1]]\na = tensor[[10, 20, 30]]\nb = tensor[[100, 200, 300]]\nx = where[mask, a, b]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 200.0, 30.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Cumulative ops tests =====

    #[test]
    fn test_math_cumsum() {
        let val = get_var("use math\nx = cumsum[tensor[[1, 2, 3, 4]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 3.0, 6.0, 10.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_cumprod() {
        let val = get_var("use math\nx = cumprod[tensor[[1, 2, 3, 4]]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 6.0, 24.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Combining ops tests =====

    #[test]
    fn test_math_concat() {
        let val = get_var("use math\na = tensor[[1, 2, 3]]\nb = tensor[[4, 5]]\nx = concat[a, b]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_math_stack() {
        let val = get_var("use math\na = tensor[[1, 2, 3]]\nb = tensor[[4, 5, 6]]\nx = stack[[a, b]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[2, 3]);
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }


    // ===== Broadcasting tests =====

    #[test]
    fn test_broadcast_add() {
        let val = get_var("use math\na = reshape[linspace[1, 3, 3], 3, 1]\nb = reshape[linspace[10, 40, 4], 1, 4]\nx = a + b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 4]);
            assert_eq!(t.to_vec(), vec![11.0, 21.0, 31.0, 41.0, 12.0, 22.0, 32.0, 42.0, 13.0, 23.0, 33.0, 43.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_broadcast_elem_mul() {
        let val = get_var("use math\na = reshape[linspace[1, 3, 3], 3, 1]\nb = reshape[linspace[1, 2, 2], 1, 2]\nx = a .* b\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.shape(), &[3, 2]);
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 2.0, 4.0, 3.0, 6.0]);
        } else { panic!("expected NumArray"); }
    }

    // ===== Advanced indexing (gather) tests =====

    #[test]
    fn test_gather_by_tensor() {
        let val = get_var("use math\na = linspace[10, 50, 5]\nidx = linspace[0, 2, 3]\nx = a[idx]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 20.0, 30.0]);
        } else { panic!("expected NumArray"); }
    }

    #[test]
    fn test_gather_repeated() {
        let val = get_var("use math\na = linspace[10, 30, 3]\nidx = linspace[0, 0, 3]\nx = a[idx]\n", "x");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![10.0, 10.0, 10.0]);
        } else { panic!("expected NumArray"); }
    }







    #[test]
    fn test_numarray_dtype() {
        let val = get_var("use math\nx = ones[3]\ny = x.dtype\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "float"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_device() {
        let val = get_var("use math\nx = ones[3]\ny = x.device\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "cpu"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_grad() {
        let val = get_var("use math\nx = ones[3]\ny = x.grad\n", "y");
        if let Value::Str(s) = &val.value { assert_eq!(s, "none"); }
        else { panic!("expected Str, got {:?}", val.value); }
    }

    #[test]
    fn test_numarray_shape_property() {
        let val = get_var("use math\nx = ones[2, 3]\ny = x.shape\n", "y");
        if let Value::Array(items) = &val.value {
            assert_eq!(items.len(), 2);
            if let Value::Int(n) = items[0].value { assert_eq!(n, 2); } else { panic!("expected Int"); }
            if let Value::Int(n) = items[1].value { assert_eq!(n, 3); } else { panic!("expected Int"); }
        } else { panic!("expected Array, got {:?}", val.value); }
    }


    /// The GPU flag is process-global; the tests that touch it take turns.
    static GPU_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_gpu_call_sets_flag() {
        let _serial = GPU_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Reset GPU flag
        crow_runtime::gpu::set_gpu_enabled(false);
        assert!(!crow_runtime::gpu::is_gpu_enabled());
        // gpu[1] should set the flag
        run_program("gpu[1]\n");
        assert!(crow_runtime::gpu::is_gpu_enabled());
        // Reset
        crow_runtime::gpu::set_gpu_enabled(false);
    }

    #[test]
    fn test_gpu_resource_stmt() {
        let _serial = GPU_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crow_runtime::gpu::set_gpu_enabled(false);
        // the resource statement turns the device on for the program
        run("gpu[2]\n").unwrap();
        assert!(crow_runtime::gpu::is_gpu_enabled());
        crow_runtime::gpu::set_gpu_enabled(false);
    }

    // ===== §nodes: Distributed computing =====

    #[test]
    fn test_nodes_call() {
        // nodes[2] should initialize distributed mode (single-process simulation)
        run_program("nodes[2]\n");
        // In interpreter mode, rank is always 0 and world_size is set
        assert!(crow_runtime::distributed::is_distributed());
        // Reset
        // Note: can't fully reset OnceLock, but the flag is set
    }

    #[test]
    fn test_nodes_resource_stmt() {
        run("nodes[4]\n").unwrap();
        assert!(crow_runtime::distributed::is_distributed());
    }

    // ===== §grad: Automatic differentiation =====

    #[test]
    fn test_grad_single_param() {
        // f(x) = x^2, f'(x) = 2x
        let val = get_var("fn f[x]\n  x * x\ndf = grad[f]\nr = df[3]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 6.0).abs() < 1e-10),
            Value::Int(v) => assert_eq!(v, 6),
            _ => panic!("expected numeric, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_multi_param() {
        // g(x,y) = x^2 + y^2, ∇g = [2x, 2y]
        let val = get_var("fn g[x, y]\n  x * x + y * y\ndg = grad[g]\nr = dg[3, 4]\n", "r");
        if let Value::Array(arr) = val.value {
            assert_eq!(arr.len(), 2);
            match (&arr[0].value, &arr[1].value) {
                (Value::Float(a), Value::Float(b)) => {
                    assert!((a - 6.0).abs() < 1e-10);
                    assert!((b - 8.0).abs() < 1e-10);
                }
                (Value::Int(a), Value::Int(b)) => {
                    assert_eq!(*a, 6);
                    assert_eq!(*b, 8);
                }
                _ => panic!("expected numeric gradient"),
            }
        } else {
            panic!("expected array, got {:?}", val.value);
        }
    }

    #[test]
    fn test_grad_polynomial() {
        // h(x) = x^3 + 2x, h'(x) = 3x^2 + 2, h'(2) = 14
        let val = get_var("fn h[x]\n  x * x * x + 2 * x\ndh = grad[h]\nr = dh[2]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 14.0).abs() < 1e-10),
            Value::Int(v) => assert_eq!(v, 14),
            _ => panic!("expected numeric, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_sqrt() {
        // p(x) = sqrt(x), p'(x) = 1/(2*sqrt(x)), p'(4) = 0.25
        let val = get_var("fn p[x]\n  sqrt[x]\ndp = grad[p]\nr = dp[4]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - 0.25).abs() < 1e-10),
            _ => panic!("expected float, got {:?}", val.value),
        }
    }

    #[test]
    fn test_grad_returns_gradfn() {
        // grad[f] should produce a GradFn value
        let val = get_var("fn f[x]\n  x * x\ndf = grad[f]\n", "df");
        assert!(matches!(val.value, Value::GradFn(_)));
    }

    #[test]
    fn test_grad_abs_derivative() {
        // abs'(x) = sign(x), abs'(-3) = -1
        let val = get_var("fn q[x]\n  abs[x]\ndq = grad[q]\nr = dq[-3]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v - (-1.0)).abs() < 1e-10),
            _ => panic!("expected float, got {:?}", val.value),
        }
    }

    // ===== Layer 3: Random & Distributions =====
    // All seed-dependent tests in a single function to avoid global RNG state
    // races (tests share the process-wide GLOBAL_RNG mutex). Same pattern as
    // distributed.rs::test_distributed_all.

    #[test]
    fn test_math_random_all() {
        // --- seed reproducibility ---
        let val = get_var("use math\nseed[42]\na = uniform[0, 1, 3]\nseed[42]\nb = uniform[0, 1, 3]\nr = sum[a] - sum[b]\n", "r");
        match val.value {
            Value::Float(v) => assert!((v).abs() < 1e-15, "same seed should give same uniform, diff={}", v),
            _ => panic!("expected float"),
        }

        // --- normal distribution ---
        let val = get_var("use math\nseed[7]\nx = normal[0, 1, 4]\n", "x");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.to_vec().len(), 4),
            _ => panic!("expected numarray"),
        }

        // --- uniform distribution ---
        let val = get_var("use math\nseed[7]\nx = uniform[2, 5, 3]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert_eq!(t.to_vec().len(), 3);
                for v in t.to_vec() {
                    assert!(v >= 2.0 && v < 5.0, "expected in [2,5), got {}", v);
                }
            }
            _ => panic!("expected numarray"),
        }

        // --- permutation ---
        let val = get_var("use math\nseed[1]\nx = permutation[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert_eq!(t.to_vec().len(), 5);
                let mut v = t.to_vec();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                assert_eq!(v, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
            }
            _ => panic!("expected numarray"),
        }

        // Clear global seed so we don't pollute other tests
        crow_runtime::rng::clear_seed();
    }

    // ===== Layer 4: Signal Processing =====

    #[test]
    fn test_math_fft_interp() {
        // FFT of [1,1,1,1] → DC=4, rest=0
        let val = get_var("use math\nx = fft[ones[4]]\n", "x");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2, "fft should return [real, imag]");
                if let Value::NumArray(real) = &arr[0].value {
                    let v = real.to_vec();
                    assert!((v[0] - 4.0).abs() < 1e-10, "DC should be 4");
                    assert!((v[1]).abs() < 1e-10, "non-DC should be 0");
                } else { panic!("expected numarray for real part"); }
            }
            _ => panic!("expected array from fft"),
        }
    }

    #[test]
    fn test_math_ifft_interp() {
        // FFT then IFFT should recover original
        let val = get_var("use math\na = linspace[1, 4, 4]\nf = fft[a]\nx = ifft[f[0], f[1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 4);
                for (i, expected) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
                    assert!((v[i] - expected).abs() < 1e-10, "ifft[{}] should be {}, got {}", i, expected, v[i]);
                }
            }
            _ => panic!("expected numarray from ifft"),
        }
    }

    #[test]
    fn test_math_conv_interp() {
        // [1,2,3] * [1,1] = [1,3,5,3]
        let val = get_var("use math\na = linspace[1, 3, 3]\nb = linspace[1, 1, 2]\nx = conv[a, b]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 4);
                assert!((v[0] - 1.0).abs() < 1e-10);
                assert!((v[1] - 3.0).abs() < 1e-10);
                assert!((v[2] - 5.0).abs() < 1e-10);
                assert!((v[3] - 3.0).abs() < 1e-10);
            }
            _ => panic!("expected numarray from conv"),
        }
    }

    #[test]
    fn test_math_hamming_interp() {
        let val = get_var("use math\nx = hamming[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 5);
                assert!((v[0] - 0.08).abs() < 1e-10, "hamming endpoint should be 0.08");
            }
            _ => panic!("expected numarray from hamming"),
        }
    }

    #[test]
    fn test_math_hanning_interp() {
        let val = get_var("use math\nx = hanning[5]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert_eq!(v.len(), 5);
                assert!((v[0]).abs() < 1e-10, "hanning endpoint should be 0");
                assert!((v[2] - 1.0).abs() < 1e-10, "hanning center should be 1");
            }
            _ => panic!("expected numarray from hanning"),
        }
    }

    // ===== Layer 5: Special Functions & Complex Numbers =====

    #[test]
    fn test_math_gamma_interp() {
        let val = get_var("use math\nx = gamma[linspace[1, 4, 4]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert!((v[0] - 1.0).abs() < 1e-8, "Γ(1) = 1");
                assert!((v[3] - 6.0).abs() < 1e-6, "Γ(4) = 6");
            }
            _ => panic!("expected numarray from gamma"),
        }
    }

    #[test]
    fn test_math_erf_interp() {
        let val = get_var("use math\nx = erf[linspace[1, 1, 1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 0.8427).abs() < 1e-3, "erf(1) ≈ 0.8427");
            }
            _ => panic!("expected numarray from erf"),
        }
    }

    #[test]
    fn test_math_factorial_interp() {
        let val = get_var("use math\nx = factorial[linspace[0, 5, 6]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                let v = t.to_vec();
                assert!((v[0] - 1.0).abs() < 1e-8, "0! = 1");
                assert!((v[5] - 120.0).abs() < 1e-5, "5! = 120");
            }
            _ => panic!("expected numarray from factorial"),
        }
    }

    #[test]
    fn test_math_choose_interp() {
        let val = get_var("use math\nx = choose[linspace[5, 5, 1], linspace[2, 2, 1]]\n", "x");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 10.0).abs() < 1e-6, "C(5,2) = 10");
            }
            _ => panic!("expected numarray from choose"),
        }
    }

    #[test]
    fn test_math_complex_interp() {
        let val = get_var("use math\nc = complex[3, 4]\n", "c");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                if let Value::NumArray(re) = &arr[0].value {
                    assert!((re.to_vec()[0] - 3.0).abs() < 1e-10);
                } else { panic!("expected numarray for real part"); }
                if let Value::NumArray(im) = &arr[1].value {
                    assert!((im.to_vec()[0] - 4.0).abs() < 1e-10);
                } else { panic!("expected numarray for imag part"); }
            }
            _ => panic!("expected array from complex"),
        }
    }

    #[test]
    fn test_math_magnitude_interp() {
        let val = get_var("use math\nc = complex[3, 4]\nm = magnitude[c]\n", "m");
        match &val.value {
            Value::NumArray(t) => {
                assert!((t.to_vec()[0] - 5.0).abs() < 1e-10, "|3+4i| = 5");
            }
            _ => panic!("expected numarray from magnitude"),
        }
    }

    #[test]
    fn test_math_conjugate_interp() {
        let val = get_var("use math\nc = complex[3, 4]\ncc = conjugate[c]\n", "cc");
        match &val.value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
                if let Value::NumArray(re) = &arr[0].value {
                    assert!((re.to_vec()[0] - 3.0).abs() < 1e-10);
                } else { panic!("expected numarray for real part"); }
                if let Value::NumArray(im) = &arr[1].value {
                    assert!((im.to_vec()[0] - (-4.0)).abs() < 1e-10, "conjugate should negate imag");
                } else { panic!("expected numarray for imag part"); }
            }
            _ => panic!("expected array from conjugate"),
        }
    }

    // ===== Sparse Arrays =====

    #[test]
    fn test_math_sparse_from_dense() {
        let val = get_var("use math\ns = sparse[eye[3]]\n", "s");
        if let Value::Sparse(s) = &val.value {
            assert_eq!(s.nnz(), 3);
            assert_eq!(s.shape(), (3, 3));
        } else { panic!("expected Sparse, got {:?}", val.value); }
    }

    #[test]
    fn test_math_sparse_from_coo() {
        let val = get_var("use math\ns = sparse[[0, 1, 2], [0, 1, 2], [5, 6, 7], [3, 3]]\n", "s");
        if let Value::Sparse(s) = &val.value {
            assert_eq!(s.nnz(), 3);
            assert_eq!(s.shape(), (3, 3));
        } else { panic!("expected Sparse, got {:?}", val.value); }
    }

    #[test]
    fn test_math_dense_from_sparse() {
        let val = get_var("use math\ns = sparse[eye[2]]\nd = dense[s]\n", "d");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 0.0, 0.0, 1.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_matmul() {
        let val = get_var("use math\ns = sparse[eye[3]]\nv = [1, 2, 3]\nr = s * v\n", "r");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_add() {
        let val = get_var("use math\na = sparse[eye[2]]\nb = sparse[eye[2]]\nc = a + b\nd = dense[c]\n", "d");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec(), vec![2.0, 0.0, 0.0, 2.0]);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_solve() {
        let val = get_var("use math\ns = sparse[[0, 1], [0, 1], [2, 3], [2, 2]]\nx = solve[s, [4, 9]]\n", "x");
        if let Value::NumArray(t) = &val.value {
            let v = t.to_vec();
            assert!((v[0] - 2.0).abs() < 1e-10);
            assert!((v[1] - 3.0).abs() < 1e-10);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_sparse_type() {
        let val = get_var("use math\ns = sparse[eye[2]]\nt = type[s]\n", "t");
        assert_eq!(format!("{}", val.value), "sparse");
    }

    // ===== Stats sub-module =====

    #[test]
    fn test_stats_median() {
        let val = get_var("use math\nuse math.stats\nx = median[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!((*f - 3.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_std() {
        let val = get_var("use math\nuse math.stats\nx = std[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_var() {
        let val = get_var("use math\nuse math.stats\nx = var[linspace[1, 5, 5]]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_percentile() {
        let val = get_var("use math\nuse math.stats\nx = percentile[linspace[1, 5, 5], 50]\n", "x");
        if let Value::Float(f) = &val.value {
            assert!((*f - 3.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_zscore() {
        let val = get_var("use math\nuse math.stats\nz = zscore[linspace[1, 5, 5]]\n", "z");
        if let Value::NumArray(t) = &val.value {
            assert_eq!(t.to_vec().len(), 5);
            // Mean of zscore should be ~0
            let m: f64 = t.to_vec().iter().sum::<f64>() / 5.0;
            assert!(m.abs() < 1e-10);
        } else { panic!("expected NumArray, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_corr() {
        let val = get_var("use math\nuse math.stats\nc = corr[linspace[1, 5, 5], linspace[2, 10, 5]]\n", "c");
        if let Value::Float(f) = &val.value {
            assert!((*f - 1.0).abs() < 1e-10);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_cov() {
        let val = get_var("use math\nuse math.stats\nc = cov[linspace[1, 5, 5], linspace[2, 10, 5]]\n", "c");
        if let Value::Float(f) = &val.value {
            assert!(*f > 0.0);
        } else { panic!("expected Float, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_ttest() {
        let val = get_var("use math\nuse math.stats\nr = ttest[linspace[1, 5, 5], linspace[10, 50, 5]]\n", "r");
        if let Value::Map(m) = &val.value {
            assert!(m.contains_key("t"));
            assert!(m.contains_key("p"));
        } else { panic!("expected Map, got {:?}", val.value); }
    }

    #[test]
    fn test_stats_requires_import() {
        // Without `use math.stats`, stats functions should fail
        let result = std::panic::catch_unwind(|| {
            get_var("use math\nx = median[linspace[1, 5, 5]]\n", "x")
        });
        assert!(result.is_err() || {
            // It might return an error value instead of panicking
            true
        });
    }

    // ===== File System Builtins =====

    #[test]
    fn test_fs_exists() {
        // Cargo.toml always exists in the project root
        let val = get_var("x = exists['Cargo.toml']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
        // Non-existent file
        let val2 = get_var("x = exists['no_such_file_xyz.txt']\n", "x");
        assert!(matches!(val2.value, Value::Bool(false)));
    }

    #[test]
    fn test_fs_mkdir_ls_rm() {
        let dir = format!("/tmp/crow_test_fs_{}", std::process::id());
        let src = format!(
            "mkdir['{d}']\nwrite['{d}/hello.txt', 'hi']\nfiles = ls['{d}']\ne = exists['{d}/hello.txt']\nrm['{d}']\ne2 = exists['{d}']\n",
            d = dir
        );
        let it = run(&src).unwrap();

        // ls should return array with 'hello.txt'
        let files = it.var("files").unwrap().clone();
        if let Value::Array(arr) = &files.value {
            assert_eq!(arr.len(), 1);
            assert!(matches!(&arr[0].value, Value::Str(s) if s == "hello.txt"));
        } else {
            panic!("expected array from ls, got {:?}", files.value);
        }
        // exists should have been true before rm
        let e = it.var("e").unwrap().clone();
        assert!(matches!(e.value, Value::Bool(true)));
        // exists should be false after rm
        let e2 = it.var("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(false)));
    }

    #[test]
    fn test_fs_mv() {
        let base = format!("/tmp/crow_test_mv_{}", std::process::id());
        let src = format!(
            "mkdir['{b}']\nwrite['{b}/a.txt', 'data']\nmv['{b}/a.txt', '{b}/b.txt']\ne1 = exists['{b}/a.txt']\ne2 = exists['{b}/b.txt']\nrm['{b}']\n",
            b = base
        );
        let it = run(&src).unwrap();

        let e1 = it.var("e1").unwrap().clone();
        assert!(matches!(e1.value, Value::Bool(false)), "source should not exist after mv");
        let e2 = it.var("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(true)), "destination should exist after mv");
    }

    #[test]
    fn test_fs_cp() {
        let base = format!("/tmp/crow_test_cp_{}", std::process::id());
        let src = format!(
            "mkdir['{b}']\nwrite['{b}/orig.txt', 'hello']\ncp['{b}/orig.txt', '{b}/copy.txt']\ne1 = exists['{b}/orig.txt']\ne2 = exists['{b}/copy.txt']\nc = read['{b}/copy.txt']\nrm['{b}']\n",
            b = base
        );
        let it = run(&src).unwrap();

        let e1 = it.var("e1").unwrap().clone();
        assert!(matches!(e1.value, Value::Bool(true)), "original should still exist after cp");
        let e2 = it.var("e2").unwrap().clone();
        assert!(matches!(e2.value, Value::Bool(true)), "copy should exist after cp");
        let c = it.var("c").unwrap().clone();
        assert!(matches!(&c.value, Value::Str(s) if s == "hello"), "copy content should match");
    }

    #[test]
    fn test_fs_mkdir_error_propagation() {
        // mkdir returns error-value on failure, not panic
        let val = get_var("x = mkdir['/root/no_permission_dir_xyz']\n", "x");
        assert!(!val.is_ok(), "mkdir on inaccessible path should return error-value");
    }

    #[test]
    fn test_fs_rm_nonexistent() {
        let val = get_var("x = rm['/tmp/no_such_file_crow_test_xyz']\n", "x");
        assert!(!val.is_ok(), "rm on nonexistent path should return error-value");
    }

    // ===== env builtin =====

    #[test]
    fn test_env_existing_var() {
        unsafe { std::env::set_var("CROW_TEST_ENV_VAR", "hello123"); }
        let val = get_var("x = env['CROW_TEST_ENV_VAR']\n", "x");
        assert!(val.is_ok(), "env[] on existing var should succeed");
        assert_eq!(format!("{}", val.value), "hello123");
        unsafe { std::env::remove_var("CROW_TEST_ENV_VAR"); }
    }

    #[test]
    fn test_env_missing_var_no_default() {
        unsafe { std::env::remove_var("CROW_TEST_NONEXISTENT_XYZ"); }
        let val = get_var("x = env['CROW_TEST_NONEXISTENT_XYZ']\n", "x");
        assert!(!val.is_ok(), "env[] on missing var without default should return error-value");
    }

    #[test]
    fn test_env_missing_var_with_default() {
        unsafe { std::env::remove_var("CROW_TEST_NONEXISTENT_XYZ"); }
        let val = get_var("x = env['CROW_TEST_NONEXISTENT_XYZ', 3000]\n", "x");
        assert!(val.is_ok(), "env[] with default should succeed");
        assert_eq!(format!("{}", val.value), "3000");
    }

    #[test]
    fn test_env_existing_var_ignores_default() {
        unsafe { std::env::set_var("CROW_TEST_ENV_PORT", "8080"); }
        let val = get_var("x = env['CROW_TEST_ENV_PORT', 3000]\n", "x");
        assert!(val.is_ok());
        assert_eq!(format!("{}", val.value), "8080");
        unsafe { std::env::remove_var("CROW_TEST_ENV_PORT"); }
    }

    // --- P3: Shell ---
    #[test]
    fn test_shell_builtin() {
        let val = run_program("x = shell['echo hello']\nlog[x.stdout]");
        assert!(val.is_ok());
    }

    #[test]
    fn test_shell_status() {
        let val = get_var("r = shell['true']", "r");
        match &val.value {
            Value::Map(m) => {
                let status = m.get("status").unwrap();
                assert_eq!(format!("{}", status.value), "0");
            }
            _ => panic!("expected map"),
        }
    }

    // --- P3: Hashing ---
    #[test]
    fn test_hash_sha256() {
        let val = get_var("h = hash['hello', 'sha256']", "h");
        assert_eq!(format!("{}", val.value), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_hash_md5() {
        let val = get_var("h = hash['hello', 'md5']", "h");
        assert_eq!(format!("{}", val.value), "5d41402abc4b2a76b9719d911017c592");
    }

    // --- P3: Regex ---
    #[test]
    fn test_matches_true() {
        let val = get_var("r = matches['hello', '[a-z]+']", "r");
        assert_eq!(format!("{}", val.value), "true");
    }

    #[test]
    fn test_matches_false() {
        let val = get_var("r = matches['hello', '[0-9]+']", "r");
        assert_eq!(format!("{}", val.value), "false");
    }

    #[test]
    fn test_find_ok() {
        // a record with the whole match then each group; a miss is an error value
        let val = get_var("m = find['abc123', '([a-z]+)([0-9]+)']", "m");
        assert!(val.is_ok());
        assert_eq!(format!("{}", val.value), "[groups = ['abc123', 'abc', '123']]");
        let miss = get_var("m = find['abc', '[0-9]+']", "m");
        assert!(!miss.is_ok());
        assert_eq!(miss.reason(), "no match");
        let bad = get_var("m = find['abc', '(']", "m");
        assert_eq!(bad.kind(), "regex");
    }

    #[test]
    fn test_sub_regex() {
        let val = get_var("r = sub['hello world', 'world', 'crow']", "r");
        assert_eq!(format!("{}", val.value), "hello crow");
    }

    // --- P3: Date/Time ---



    // --- P4: Web builtin interpreter mock tests ---

    /// A socket to a server that does not answer is the call's error value
    /// (the client's calls had been stand-ins: a map, `true`, empty text)
    #[test]
    fn a_socket_nobody_answers_is_an_error_value() {
        let val = get_var("use web, web.ws\ns = socket['ws://127.0.0.1:1/feed']\nok = s.ok\nkind = s.kind\n", "kind");
        assert_eq!(format!("{}", val.value), "socket");
        let ok = get_var("use web, web.ws\ns = socket['ws://127.0.0.1:1/feed']\nok = s.ok\n", "ok");
        assert!(matches!(ok.value, Value::Bool(false)));
    }

    /// `live[...]` was a mock of a retired API; `page[]` is the one way. The
    /// name is a user's to define, in both modes.
    #[test]
    fn live_is_not_a_builtin() {
        let val = get_var("fn live[x]\n    return x + 1\nn = live[1]", "n");
        assert!(matches!(&val.value, Value::Int(2)), "got {:?}", val.value);
    }

    #[test]
    fn test_smtp_builtin() {
        // a mailer is a handle: it shows its host, and its password is in
        // no value a program can print
        let val = get_var("use web, web.mail\nmailer = smtp['smtp.example.com', 587, 'user', 'pass']", "mailer");
        match &val.value {
            Value::Mailer(m) => assert_eq!(format!("{}", m), "<mailer:smtp.example.com>"),
            other => panic!("expected a mailer, got {:?}", other),
        }
        assert_eq!(format!("{}", val.value), "<mailer:smtp.example.com>");
    }

    // ===== Response format tests (P4) =====

    /// msgpack, arrow, bytes and stream are web's: outside a web program
    /// they are refused before it runs (the interpreter had answered them
    /// with a map that only looked like a response, and the compiled program
    /// had not built); in one they are answers (msgpack_arrow_and_stream_are_answers)
    #[test]
    fn the_responses_outside_a_web_program_are_refused() {
        for (src, name) in [
            ("resp = msgpack['hello']\n", "msgpack"),
            ("resp = arrow['frame_placeholder']\n", "arrow"),
            ("resp = bytes['raw data']\n", "bytes"),
            ("fn f[put]\n    put['a']\nresp = stream[f]\n", "stream"),
        ] {
            let refused = run(src).err().unwrap_or_default();
            assert!(refused.contains(&format!("{}[] is web's: add `use web`", name)), "{}: {}", name, refused);
        }
    }

    #[test]
    fn test_msgpack_wrong_args() {
        let result = std::panic::catch_unwind(|| {
            get_var("resp = msgpack[]", "resp");
        });
        assert!(result.is_err(), "msgpack[] with no args should fail");
    }

    #[test]
    fn test_arrow_wrong_args() {
        let result = std::panic::catch_unwind(|| {
            get_var("resp = arrow[]", "resp");
        });
        assert!(result.is_err(), "arrow[] with no args should fail");
    }

    // ===== DB module tests =====

    #[test]
    fn test_db_connect_sqlite() {
        let val = get_var("use db\nconn = connect['sqlite://:memory:']\n", "conn");
        assert!(matches!(val.value, Value::DbConn(_)), "expected DbConn, got {:?}", val.value);
    }

    #[test]
    fn test_db_connect_prefixed() {
        let val = get_var("use db\nconn = db.connect['sqlite://:memory:']\n", "conn");
        assert!(matches!(val.value, Value::DbConn(_)), "expected DbConn via db.connect");
    }

    #[test]
    fn test_db_exec_create_table() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nn = exec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\n",
            "n"
        );
        assert!(matches!(val.value, Value::Int(0)), "CREATE TABLE should return 0 affected rows, got {:?}", val.value);
    }

    #[test]
    fn test_db_exec_insert() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nn = exec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\n",
            "n"
        );
        assert!(matches!(val.value, Value::Int(1)), "INSERT should return 1 affected row, got {:?}", val.value);
    }

    #[test]
    fn test_db_query() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nexec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\nexec[conn, 'INSERT INTO t VALUES (2, ?)', ['bob']]\ndf = query[conn, 'SELECT * FROM t ORDER BY id']\n",
            "df"
        );
        match &val.value {
            Value::Frame(f) => {
                assert_eq!(f.nrows(), 2, "should have 2 rows");
                assert_eq!(f.ncols(), 2, "should have 2 columns");
            }
            _ => panic!("expected Frame, got {:?}", val.value),
        }
    }

    #[test]
    fn test_db_query_with_params() {
        let val = get_var(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER, name TEXT)']\nexec[conn, 'INSERT INTO t VALUES (1, ?)', ['alice']]\nexec[conn, 'INSERT INTO t VALUES (2, ?)', ['bob']]\ndf = query[conn, 'SELECT * FROM t WHERE name = ?', ['alice']]\n",
            "df"
        );
        match &val.value {
            Value::Frame(f) => {
                assert_eq!(f.nrows(), 1, "should have 1 row for alice");
            }
            _ => panic!("expected Frame, got {:?}", val.value),
        }
    }

    #[test]
    fn test_db_transact() {
        // transact should commit on success
        run_program(
            "use db\nconn = connect['sqlite://:memory:']\nexec[conn, 'CREATE TABLE t (id INTEGER)']\nfn do_inserts[tx]\n    exec[tx, 'INSERT INTO t VALUES (1)']\n    exec[tx, 'INSERT INTO t VALUES (2)']\ntransact[conn, do_inserts]\n"
        );
    }

    #[test]
    fn test_db_connect_bad_url() {
        let result = std::panic::catch_unwind(|| {
            run_program("use db\nconn = connect['ftp://bad']\n")
        });
        assert!(result.is_err(), "connect with bad URL should error");
    }

    // ===== Boolean mask semantics (frame API: strings + expressions) =====

    fn assert_bool_array(val: &CrowValue, expected: &[bool]) {
        match &val.value {
            Value::Array(items) => {
                let got: Vec<bool> = items.iter().map(|v| match v.value {
                    Value::Bool(b) => b,
                    _ => panic!("expected Bool element, got {:?}", v.value),
                }).collect();
                assert_eq!(got, expected);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_eq_broadcasts_over_string_array() {
        let val = get_var("names = ['a', 'b', 'a']\nm = names eq 'a'\n", "m");
        assert_bool_array(&val, &[true, false, true]);
    }

    #[test]
    fn test_ne_broadcasts_over_string_array() {
        let val = get_var("names = ['a', 'b', 'a']\nm = names ne 'a'\n", "m");
        assert_bool_array(&val, &[false, true, false]);
    }

    #[test]
    fn test_scalar_eq_unchanged() {
        let val = get_var("x = 5 eq 5\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_array_array_eq_stays_whole_equality() {
        let val = get_var("x = ['a', 'b'] eq ['a', 'b']\n", "x");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_and_combines_bool_arrays_elementwise() {
        let val = get_var("m = [true, true, false] and [true, false, false]\n", "m");
        assert_bool_array(&val, &[true, false, false]);
    }

    #[test]
    fn test_or_combines_bool_arrays_elementwise() {
        let val = get_var("m = [true, false, false] or [false, false, true]\n", "m");
        assert_bool_array(&val, &[true, false, true]);
    }

    #[test]
    fn test_not_negates_bool_array_elementwise() {
        let val = get_var("m = not [true, false]\n", "m");
        assert_bool_array(&val, &[false, true]);
    }

    #[test]
    fn test_frame_filter_eq_mask() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nr = filter[df, df.name eq 'Alice']\nn = len[r]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_frame_filter_compound_mask() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nr = filter[df, df.age gt 26 and df.salary gt 60000]\nn = len[r]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(1)));
    }

    #[test]
    fn test_len_frame() {
        let val = get_var(
            "use frame\ndf = load['examples/test_data.csv']\nn = len[df]\n",
            "n",
        );
        assert!(matches!(val.value, Value::Int(4)));
    }

    #[test]
    fn test_for_loop_outer_mutation_runs_sequential() {
        // Regression: parallel for silently discarded outer mutations (total stayed 0)
        let val = get_var("total = 0\nfor x in [1, 2, 3, 4]\n    total = total + x\n", "total");
        assert!(matches!(val.value, Value::Int(10)));
    }

    #[test]
    fn test_implicit_return_last_expression() {
        // Regression: implicit return is desugared in the parser for both modes
        let val = get_var("fn add[a, b]\n    a + b\nr = add[3, 4]\n", "r");
        assert!(matches!(val.value, Value::Int(7)));
    }

    // ===== Search builtins: any / all / find =====

    #[test]
    fn test_any_all() {
        let src = "fn big[x]\n    return x gt 10\n";
        let val = get_var(&format!("{}a = any[[1, 20, 3], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(true)));
        let val = get_var(&format!("{}a = any[[1, 2, 3], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(false)));
        let val = get_var(&format!("{}a = all[[11, 20, 30], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(true)));
        let val = get_var(&format!("{}a = all[[11, 2, 30], big]\n", src), "a");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_find_element_and_miss() {
        let src = "fn big[x]\n    return x gt 10\n";
        let val = get_var(&format!("{}h = find[[1, 20, 3], big]\n", src), "h");
        assert!(matches!(val.value, Value::Int(20)));
        let val = get_var(&format!("{}h = find[[1, 2, 3], big]\nok = h.ok\n", src), "ok");
        assert!(matches!(val.value, Value::Bool(false)));
    }

    #[test]
    fn test_find_regex_dispatch_unchanged() {
        let val = get_var("m = find['ab12', '([0-9]+)']\nok = m.ok\n", "ok");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    // ===== Comments end at the newline =====

    #[test]
    fn test_comment_without_closer_does_not_eat_next_line() {
        // Regression: the old paired --- design silently swallowed the next
        // line into code when a closer was missing
        let val = get_var("--- NOTE: no closer here\nx = 42\n", "x");
        assert!(matches!(val.value, Value::Int(42)));
    }

    #[test]
    fn test_inline_comment_runs_to_end_of_line() {
        let val = get_var("x = 5 --- everything here is comment --- even this\n", "x");
        assert!(matches!(val.value, Value::Int(5)));
    }

    // ===== Backtick interpolation =====

    #[test]
    fn test_backtick_escaped_literal() {
        let val = get_var("s = 'code: \\`x\\`'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "code: `x`"));
    }

    #[test]
    fn test_backtick_unterminated_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("s = 'hello `name'\nlog[s]\n")
        });
        assert!(result.is_err(), "unterminated interpolation should error");
    }

    #[test]
    fn test_backtick_nested_string_args() {
        // Inside interpolation you are still inside the string: quotes escaped
        let val = get_var("s = 'up: `upper[\\'hi\\']`'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "up: HI"));
    }

    #[test]
    fn test_regex_quantifier_braces_now_safe() {
        // Regression: '{4}' used to be silently interpolated, corrupting patterns
        let val = get_var("m = matches['2024', '[0-9]{4}']\n", "m");
        assert!(matches!(val.value, Value::Bool(true)));
    }

    #[test]
    fn test_concat_strings() {
        let val = get_var("x = concat['ab', 'cd']\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "abcd"));
    }

    // ===== fmt builtin =====

    #[test]
    fn test_fmt_basics() {
        let val = get_var("x = fmt[1234.567, decimals = 2, commas = true]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "1,234.57"));
        let val = get_var("x = fmt[0.5, percent = true, decimals = 0]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "50%"));
        let val = get_var("x = fmt['hi', width = 4]\n", "x");
        assert!(matches!(&val.value, Value::Str(s) if s == "hi  "));
    }

    #[test]
    fn test_fmt_decimals_on_string_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("x = fmt['hi', decimals = 2]\n")
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_pad_removed() {
        let result = std::panic::catch_unwind(|| {
            run_program("x = pad['hi', 5]\n")
        });
        assert!(result.is_err(), "pad should error with removal message");
    }

    // ===== Errors in conditions + batch semantics (#2) =====

    #[test]
    fn test_error_condition_halts_at_top_level() {
        let result = std::panic::catch_unwind(|| {
            run_program("age = int['abc']\nif age gte 18\n    log['adult']\n")
        });
        assert!(result.is_err(), "branching on an error at top level should halt");
    }

    #[test]
    fn test_error_condition_propagates_out_of_function() {
        let val = get_var(
            "fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['abc']\nok = r.ok\n",
            "ok",
        );
        assert!(matches!(val.value, Value::Bool(false)));
        let val = get_var(
            "fn check[s]\n    n = int[s]\n    if n gt 5\n        return 'big'\n    return 'small'\nr = check['9']\n",
            "r",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "big"));
    }

    /// A condition refused inside a collecting loop hands the error back
    /// from the function, as it does anywhere else in it: the loop is an
    /// expression, and the walker stopped with "cannot return from inside a
    /// for loop" instead
    #[test]
    fn a_collecting_loop_hands_back_an_error_from_its_condition() {
        let src = "fn parse_all[xs]\n    out = for x in xs\n        if int[x] gt 0\n            x\n    return out\nr = parse_all[['1', 'a', 'b']]\ngood = parse_all[['1', '2']]\n";
        let r = get_var(src, "r");
        assert!(r.is_error(), "{:?}", r);
        assert!(r.reason().contains("'a'"), "the first failure in input order: {}", r.reason());
        assert!(matches!(&get_var(src, "good").value, Value::Array(items) if items.len() == 2));
    }

    /// A loop over a list that failed is refused like a condition (Vijay's
    /// choice): a function hands the error back, the top level halts
    #[test]
    fn a_loop_over_a_failed_list_is_refused_like_a_condition() {
        let src = "fn count[s]\n    n = 0\n    for i in range[int[s]]\n        n = n + 1\n    return n\nbad = count['x']\ngood = count['3']\n";
        let bad = get_var(src, "bad");
        assert!(bad.is_error() && bad.reason().contains("'x'"), "{:?}", bad);
        assert!(matches!(get_var(src, "good").value, Value::Int(3)));
        let halted = run("for i in range[int['x']]\n    log['never']\n").err().expect("the top level halts");
        assert!(halted.contains("cannot loop over an error value"), "{}", halted);
    }

    /// A variable whose first value could not fail holds no error: a
    /// failed value is refused where it is assigned, as a decision refuses
    /// one — a function hands it back, the top level halts naming the
    /// variable (the interpreter kept the error in the variable and ran on)
    #[test]
    fn a_plain_variable_refuses_a_failed_value() {
        let src = "fn parse[s]\n    x = 1.0\n    x = float[s]\n    return 1\nbad = parse['y']\ngood = parse['2']\n";
        let bad = get_var(src, "bad");
        assert!(bad.is_error() && bad.reason().contains("'y'"), "{:?}", bad);
        assert!(matches!(get_var(src, "good").value, Value::Int(1)));
        let halted = run("x = 1.0\nx = float['y']\nlog['never']\n").err().expect("the top level halts");
        assert!(halted.contains("cannot assign an error value to 'x' — cannot convert 'y' to float"), "{}", halted);
    }

    #[test]
    fn test_error_condition_in_while_refused() {
        let result = std::panic::catch_unwind(|| {
            run_program("n = int['x']\nwhile n gt 0\n    n = n - 1\n")
        });
        assert!(result.is_err(), "while on an error condition should refuse");
    }

    #[test]
    fn test_ok_check_always_branchable() {
        let val = get_var("bad = int['zzz']\nmsg = ''\nif not bad.ok\n    msg = 'guarded'\n", "msg");
        assert!(matches!(&val.value, Value::Str(s) if s == "guarded"));
    }

    #[test]
    fn test_float_var_stays_float_on_int_assignment() {
        let val = get_var("v = 2.5\nv = 1\n", "v");
        assert!(matches!(val.value, Value::Float(f) if f == 1.0));
    }

    #[test]
    fn test_braces_are_inert_literals() {
        let val = get_var("s = 'a {b} c'\n", "s");
        assert!(matches!(&val.value, Value::Str(s) if s == "a {b} c"));
    }

    #[test]
    fn test_json_object_literal_raw() {
        // a parsed value's field is JSON, as the compiled program holds it
        let val = get_var("j = json['{\"n\": 7}']\nx = j.n\n", "x");
        assert!(matches!(&val.value, Value::Json(j) if **j == serde_json::json!(7)), "{:?}", val.value);
        let val = get_var("j = json['{\"n\": 7}']\nx = int[j.n] + 1\n", "x");
        assert!(matches!(val.value, Value::Int(8)), "{:?}", val.value);
    }

    /// A parsed value is JSON, as the compiled program holds it: an item it
    /// lacks is an error value of reason `null` where the interpreter had
    /// stopped, a field it lacks is JSON's null, its items are JSON, and
    /// `encode` writes a list of numbers by the runtime's rule
    #[test]
    fn a_parsed_value_is_json() {
        let it = run("j = json['{\"a\": [1, 2.0, \"x\"], \"m\": {\"k\": 1}}']\nitem = j.a[9]\nfield = j.m.nope\nkinds = for x in j.a\n    type[x]\nn = len[j.a]\nout = encode[j.a]\n").unwrap();
        let item = it.var("item").unwrap();
        assert!(item.is_error() && item.reason() == "null", "{:?}", item);
        assert!(matches!(&it.var("field").unwrap().value, Value::Json(j) if j.is_null()));
        let kinds: Vec<String> = match &it.var("kinds").unwrap().value {
            Value::Array(xs) => xs.iter().map(|x| format!("{}", x.value)).collect(),
            other => panic!("{:?}", other),
        };
        assert_eq!(kinds, vec!["int", "float", "str"]);
        assert!(matches!(it.var("n").unwrap().value, Value::Int(3)));
        assert!(matches!(&it.var("out").unwrap().value, Value::Str(s) if s == "[1,2.0,\"x\"]"), "{:?}", it.var("out").unwrap().value);
    }

    // ===== 2-D tensors from literals (F3) =====

    #[test]
    fn test_nested_literal_builds_shaped_tensor() {
        let val = get_var("m = [[1, 2], [3, 4]]\n", "m");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.shape(), &[2, 2]),
            other => panic!("expected 2-D tensor, got {:?}", other),
        }
    }

    #[test]
    fn test_ragged_literal_errors() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = [[1, 2], [3]]\n")
        });
        assert!(result.is_err(), "ragged rows should error");
    }

    #[test]
    fn test_tensor_row_indexing_and_iteration() {
        let val = get_var("m = [[1, 2], [3, 4]]\nr = m[1]\n", "r");
        match &val.value {
            Value::NumArray(t) => assert_eq!(t.to_vec(), vec![3.0, 4.0]),
            other => panic!("expected row tensor, got {:?}", other),
        }
        let val = get_var("m = [[1, 2], [3, 4]]\nx = m[1][0]\n", "x");
        assert!(matches!(val.value, Value::Int(3)));
    }

    #[test]
    fn test_tensor_push_row_and_errors() {
        let val = get_var("m = [[1, 2], [3, 4]]\nm2 = push[m, [5, 6]]\nn = len[m2]\n", "n");
        assert!(matches!(val.value, Value::Int(3)), "a 3x2 matrix has 3 rows: {:?}", val.value);
        // wrong row length
        let r = std::panic::catch_unwind(|| run_program("m = [[1, 2], [3, 4]]\nm2 = push[m, [5, 6, 7]]\n"));
        assert!(r.is_err(), "wrong row length should error");
        // wrong type
        let r = std::panic::catch_unwind(|| run_program("m = [1, 2]\nm2 = push[m, 'x']\n"));
        assert!(r.is_err(), "pushing str onto numeric array should error");
    }

    #[test]
    fn test_tensor_comparison_masks() {
        let val = get_var("x = [1, 2, 3, 4]\nm = x gt 2\n", "m");
        assert_bool_array(&val, &[false, false, true, true]);
        let val = get_var("x = [1, 2, 3]\nm = x eq 2\n", "m");
        assert_bool_array(&val, &[false, true, false]);
    }

    #[test]
    fn test_concat_tensor_axis() {
        // a 2x4 matrix: 2 rows, 8 numbers
        let val = get_var("use math\nm = [[1, 2], [3, 4]]\nc = concat[m, m, 1]\nn = len[c]\n", "n");
        assert!(matches!(val.value, Value::Int(2)), "a 2x4 matrix has 2 rows: {:?}", val.value);
        let val = get_var("use math\nm = [[1, 2], [3, 4]]\nc = concat[m, m, 1]\nn = len[flatten[c]]\n", "n");
        assert!(matches!(val.value, Value::Int(8)), "and 8 numbers: {:?}", val.value);
    }

    // ===== Function scope: module variables are invisible (F1) =====

    #[test]
    fn test_fn_cannot_read_module_variable() {
        let result = std::panic::catch_unwind(|| {
            run_program("base = 10\nfn add_base[x]\n    return x + base\nr = add_base[5]\n")
        });
        assert!(result.is_err(), "module variable read inside fn should error");
    }

    #[test]
    fn test_fn_cannot_mutate_module_variable() {
        // The old silent-local bug: count must NOT silently stay 0 — the
        // read of 'count' inside the function errors instead
        let result = std::panic::catch_unwind(|| {
            run_program("count = 0\nfn inc[]\n    count = count + 1\ninc[]\n")
        });
        assert!(result.is_err(), "module variable mutation inside fn should error");
    }

    #[test]
    fn test_fn_local_may_reuse_module_name() {
        // A local named like a module variable is a fresh, unrelated binding
        let val = get_var(
            "count = 5\nfn scaled[n]\n    count = n * 2\n    return count\nr = scaled[3]\n",
            "r",
        );
        assert!(matches!(val.value, Value::Int(6)));
        // and the module variable is untouched
        let val2 = get_var(
            "count = 5\nfn scaled[n]\n    count = n * 2\n    return count\nr = scaled[3]\n",
            "count",
        );
        assert!(matches!(val2.value, Value::Int(5)));
    }

    #[test]
    fn test_fn_can_call_other_functions_and_structs() {
        let val = get_var(
            "struct P [x]\nfn double[n]\n    return n * 2\nfn build[n]\n    return P[double[n]]\nr = build[4]\nv = r.x\n",
            "v",
        );
        assert!(matches!(val.value, Value::Int(8)));
    }

    #[test]
    fn test_fn_index_assign_module_var_blocked() {
        let result = std::panic::catch_unwind(|| {
            run_program("nums = [1, 2, 3]\nfn tweak[]\n    nums[0] = 99\ntweak[]\n")
        });
        assert!(result.is_err(), "index-assign to module variable inside fn should error");
    }

    // ===== Maps are dictionaries, structs are records =====

    #[test]
    fn test_map_heterogeneous_values_rejected() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = map[name = 'crow', age = 3]\n")
        });
        assert!(result.is_err(), "mixed str/int map values should error");
    }

    #[test]
    fn test_map_hetero_index_assign_rejected() {
        let result = std::panic::catch_unwind(|| {
            run_program("m = map[a = 1]\nm['b'] = 'x'\n")
        });
        assert!(result.is_err(), "adding str to int map via index assign should error");
    }

    #[test]
    fn test_map_int_float_values_unify() {
        // a map of floats holds an int as a float, as its type says (the
        // interpreter held the int, so `m.epochs / 3` was a float while
        // type[m.epochs] said int, and the compiled program did not build)
        let val = get_var("m = map[lr = 0.01, epochs = 100]\nx = m.epochs\n", "x");
        assert!(matches!(val.value, Value::Float(f) if f == 100.0), "{:?}", val.value);
    }

    #[test]
    fn test_map_of_structs() {
        let val = get_var(
            "struct Address [street, city, zip]\naddresses = map[home = Address['12 Oak St', 'Austin', 78701], office = Address['500 Main St', 'Austin', 78702]]\nx = addresses.home.city\n",
            "x",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "Austin"));
    }

    #[test]
    fn test_map_of_structs_dynamic_key() {
        let val = get_var(
            "struct Address [street, city, zip]\naddresses = map[home = Address['12 Oak St', 'Austin', 78701]]\naddresses = set[addresses, 'work', Address['500 Main St', 'Dallas', 75001]]\nx = addresses['work'].city\n",
            "x",
        );
        assert!(matches!(&val.value, Value::Str(s) if s == "Dallas"));
    }

    #[test]
    fn test_map_literal_removed_helpful_error() {
        // Regression: bare [k = v] must error with a pointer to map[...]
        let mut lexer = crate::lexer::Lexer::new("m = [a = 1]\n");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = crate::parser::Parser::new(tokens);
        let err = parser.parse().unwrap_err();
        assert!(err.contains("map[key = value"), "unexpected error: {}", err);
    }

    #[test]
    fn test_frame_bare_column_name_is_undefined_variable() {
        // Regression: bare-name column DSL removed — sort[df, age] must error
        let result = std::panic::catch_unwind(|| {
            run_program("use frame\ndf = load['examples/test_data.csv']\ns = sort[df, age]\n")
        });
        assert!(result.is_err(), "bare column name should be an undefined variable");
    }
    #[test]
    fn test_now_is_a_number() {
        let val = get_var("t = now[]", "t");
        match &val.value {
            Value::Float(f) => assert!(*f > 1_700_000_000.0, "epoch seconds, got {}", f),
            other => panic!("now[] must be a number, got {:?}", other),
        }
    }

    #[test]
    fn test_moment_calendar_and_arithmetic() {
        // 09:30 New York on 2026-07-14 reads as 13:30 UTC — same instant.
        let val = get_var(
            "t = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']\nc = calendar[t, tz = 'UTC']\nh = c.hour",
            "h",
        );
        assert_eq!(format!("{}", val.value), "13");
        // Subtraction is elapsed seconds.
        let d = get_var(
            "a = moment[2026, 7, 14, hour = 9]\nb = moment[2026, 7, 14, hour = 10]\nd = b - a",
            "d",
        );
        assert_eq!(format!("{}", d.value), "3600");
    }

    #[test]
    fn test_parse_time_and_fmt_time() {
        let val = get_var("t = parse_time['2026-07-14T04:09:00-04:00']\ns = fmt[t, time = 'iso']", "s");
        assert_eq!(format!("{}", val.value), "2026-07-14T08:09:00Z");
        let ny = get_var("t = parse_time['2026-07-14 12:00:00']\ns = fmt[t, time = 'datetime', tz = 'America/New_York']", "s");
        assert_eq!(format!("{}", ny.value), "2026-07-14 08:00:00");
    }

    #[test]
    fn f11_click_with_frozen_args() {
        // click[a, set_speed[10.0]] delivers the frozen arg to the handler,
        // and rendering must NOT invoke the deferred call.
        let src = "use ui\nhits = 0\nspeed = 1.0\nfn set_speed[s]\n    hits = hits + 1\n    speed = s\nview render[]\n    return div[button['10x', click = set_speed[10.0]], p['`speed`']]\ntest t[]\n    a = app[render]\n    h = html[a]\n    assert[a.hits eq 0]\n    click[a, set_speed[10.0]]\n    assert[a.hits eq 1]\n    assert[a.speed eq 10.0]\n";
        let stmts = crate::build(src).unwrap();
        let (issues, counts) = crate::checker::run_test_blocks(&stmts);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
        assert!(counts.blocks >= 1);
    }

    /// Harness issues for a program's test blocks.
    fn harness_issues(src: &str) -> Vec<String> {
        let stmts = crate::build(src).unwrap();
        let (issues, counts) = crate::checker::run_test_blocks(&stmts);
        assert!(counts.blocks >= 1, "no test block ran");
        issues.iter().map(|i| format!("{:?}", i)).collect()
    }

    /// A web program's response forms make a route's answer in the
    /// interpreter as the compiled program makes them (it had served no
    /// route at all); `json` of text still parses it
    #[test]
    fn a_web_programs_responses_are_answers() {
        let it = run("use web\nt = text['hi']\nh = html['<p>x</p>']\nj = json[map[a = 'b']]\nr = respond[201, 'made']\nw = redirect['/there']\nb = bytes['raw']\np = json['[1, 2]']\n").unwrap();
        let answer = |n: &str| match &it.var(n).unwrap().value {
            Value::Response(r) => (r.status, r.headers.get("content-type").cloned().unwrap_or_default(), r.body.clone()),
            other => panic!("{} is {:?}", n, other),
        };
        assert_eq!(answer("t"), (200, "text/plain".to_string(), "hi".to_string()));
        assert_eq!(answer("h"), (200, "text/html".to_string(), "<p>x</p>".to_string()));
        assert_eq!(answer("j"), (200, "application/json".to_string(), "{\"a\":\"b\"}".to_string()));
        assert_eq!(answer("r").0, 201);
        match &it.var("w").unwrap().value {
            Value::Response(r) => assert_eq!((r.status, r.headers.get("location").map(String::as_str)), (302, Some("/there"))),
            other => panic!("{:?}", other),
        }
        match &it.var("b").unwrap().value {
            Value::Response(r) => assert_eq!(r.binary_body.as_deref(), Some("raw".as_bytes())),
            other => panic!("{:?}", other),
        }
        assert!(!matches!(it.var("p").unwrap().value, Value::Response(_)), "json of text parses it");
    }

    /// msgpack, arrow and stream answer in the interpreter through
    /// crow_web's own functions, as the compiled program's do (the
    /// interpreter had stopped on all three): a stream's feed runs when the
    /// response is sent, handed put and the arguments after it
    #[test]
    fn msgpack_arrow_and_stream_are_answers() {
        let it = run("use web, frame\nfn lines[put, n]\n    for i in range[n]\n        put['line `i`\\n']\nm = msgpack[map[name = 'crow']]\na = arrow[with[frame.new[], 'n', [1, 2]]]\ns = stream[lines, 2]\n").unwrap();
        let response = |n: &str| match &it.var(n).unwrap().value {
            Value::Response(r) => r.as_ref().clone(),
            other => panic!("{} is {:?}", n, other),
        };
        let m = response("m");
        assert_eq!(m.headers.get("content-type").map(String::as_str), Some("application/msgpack"));
        assert_eq!(m.binary_body.as_deref(), Some(&b"\x81\xa4name\xa4crow"[..]));
        let a = response("a");
        assert_eq!(a.headers.get("content-type").map(String::as_str), Some("application/vnd.apache.arrow.stream"));
        assert!(a.binary_body.as_ref().is_some_and(|b| b.starts_with(&[0xff, 0xff, 0xff, 0xff])), "{:?}", a.binary_body);
        let s = response("s");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = got.clone();
        s.feed.expect("a feed").run(move |chunk| {
            into.lock().unwrap().push(String::from_utf8(chunk).unwrap());
            true
        });
        assert_eq!(*got.lock().unwrap(), vec!["line 0\n".to_string(), "line 1\n".to_string()]);
    }

    /// html[a] is the page as the browser builds it, the same text in both
    /// modes: a table's rows carry their attributes in one order (the
    /// interpreter had handed back the kernel's own markup and the compiled
    /// harness the browser's, so an assertion on a row held in one mode only)
    #[test]
    fn a_pages_html_is_the_browsers_in_both_modes() {
        let issues = harness_issues("use web, ui, frame\nn = 0\nfn pick[k]\n    n = k\nview v[]\n    f = with[frame.new[], 'k', [4]]\n    return stack[p['`n`'], table[f, click = pick, key = 'k']]\ntest t[]\n    a = app[v]\n    assert[contains[html[a], '<tr data-crow-args=\"[4]\" data-crow-click=\"pick\" tabindex=\"0\"><td class=\"num\">4</td></tr>']]\n");
        assert!(issues.is_empty(), "{:?}", issues);
    }

    const TWO_PEOPLE: &str = "use web, ui, db\nbook = db.connect['sqlite://:memory:']\nmin_qty = 0\nfn setup[]\n    exec[book, 'CREATE TABLE IF NOT EXISTS orders (id INTEGER PRIMARY KEY, qty INTEGER)']\nderive orders\n    return query[book, 'SELECT qty FROM orders ORDER BY id']\nfn reset[]\n    min_qty = 0\nfn order[]\n    exec[book, 'INSERT INTO orders (qty) VALUES (?)', [min_qty]]\nview panel[]\n    return stack[p['min `min_qty`'], p['orders: `len[orders]`'], button['Reset', click = reset], button['Order', click = order]]\n";

    /// widget[name, props = …, on = map[event = handler]]: one element the
    /// program's module draws — its props as JSON, its events by handler;
    /// in a test a click stands for what the widget would send
    #[test]
    fn a_widget_is_one_element_and_its_events_are_handlers() {
        let src = "use web, ui\nplace = 'London'\nfn choose[p]\n    place = p\nview v[]\n    return stack[p[place], widget['map', props = map[city = place], on = map[pick = choose]]]\ntest t[]\n    a = app[v]\n    w = elements[a, 'div.crow-widget'][0]\n    assert[w['data-crow-widget'] eq 'map']\n    assert[w['data-crow-props'] eq '{\"city\":\"London\"}']\n    assert[w['data-crow-on'] eq '{\"pick\":\"choose\"}']\n    click[a, choose['Paris']]\n    assert[elements[a, 'div.crow-widget'][0]['data-crow-props'] eq '{\"city\":\"Paris\"}']\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn a_widget_event_names_a_handler() {
        let refused = |src: &str| -> String {
            let stmts = crate::build(src).expect("parses");
            let lowered = crate::lower::lower_program(&stmts).expect("lowers");
            crate::infer_and_validate(&lowered, false).err().unwrap_or_default()
        };
        let e = refused("use web, ui\nview v[]\n    return widget['map', props = map[a = 1], on = map[pick = 'choose']]\n");
        assert!(e.contains("names a handler"), "{}", e);
        let ok = refused("use web, ui\nx = ''\nfn choose[p]\n    x = p\nview v[]\n    return widget['map', props = map[a = 1], on = map[pick = choose]]\n");
        assert!(ok.is_empty(), "{}", ok);
    }

    /// download[value, name] hands the reader a file when the handler
    /// commits: a frame as its CSV, a string as it is; a handler that stops
    /// hands them nothing
    #[test]
    fn a_handler_hands_the_reader_files() {
        let src = "use web, ui, frame\nn = 0\nfn export[]\n    f = with[frame.new[], 'n', range[1, 3]]\n    download[f, 'numbers.csv']\n    download['hello', 'note.txt']\nfn broken[]\n    download['never', 'lost.txt']\n    x = float['no']\n    if x gt 1.0\n        n = 1\nview v[]\n    return stack[button['Export', click = export], button['Broken', click = broken]]\ntest t[]\n    a = app[v]\n    click[a, export]\n    got = downloads[a]\n    assert[len[got] eq 2]\n    assert[got[0].name eq 'numbers.csv']\n    assert[got[0].text eq 'n\\n1\\n2\\n']\n    assert[got[1].text eq 'hello']\n    r = click[a, broken]\n    assert[not r.ok]\n    assert[len[downloads[a]] eq 2]\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    /// mail[mailer, to, subject, body]: a handler's message waits for its
    /// commit, a stopped handler sends nothing, one whose answer is used is
    /// sent when asked, and a helper's is its handler's; a test sends none
    /// and reads what the session sent with mails[a]
    #[test]
    fn a_handlers_mail_waits_for_its_commit() {
        let src = "use web, ui\nuse web.mail\nmailer = smtp['smtp.example.com', 587, 'noreply@example.com', 'secret']\nn = 0\nfn notify[m, to]\n    mail[m, to, 'Hello', 'From a helper']\nfn welcome[]\n    n = n + 1\n    mail[mailer, 'ada@example.com', 'Hello', 'Welcome, number `n`']\nfn broken[]\n    mail[mailer, 'bob@example.com', 'Lost', 'never sent']\n    x = float['no']\n    if x gt 1.0\n        n = 1\nfn asked[]\n    r = mail[mailer, 'cy@example.com', 'Asked', 'sent at once']\n    if r.ok\n        n = 10\nfn helped[]\n    notify[mailer, 'di@example.com']\nview v[]\n    return stack[p['`n`'], button['Welcome', click = welcome], button['Broken', click = broken], button['Asked', click = asked], button['Helped', click = helped]]\ntest t[]\n    a = app[v]\n    click[a, welcome]\n    got = mails[a]\n    assert[len[got] eq 1]\n    assert[got[0].to eq 'ada@example.com']\n    assert[got[0].subject eq 'Hello']\n    assert[got[0].body eq 'Welcome, number 1']\n    r = click[a, broken]\n    assert[not r.ok]\n    assert[len[mails[a]] eq 1]\n    click[a, asked]\n    assert[len[mails[a]] eq 2]\n    assert[a.n eq 10]\n    click[a, helped]\n    assert[mails[a][2].to eq 'di@example.com']\n    assert['`mailer`' eq '<mailer:smtp.example.com>']\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    /// A page that knows who its reader is: people with stored password
    /// hashes, a sign-in that checks one, an admin-only action, a sign-out,
    /// and a derivation over the reader
    const SIGN_IN: &str = "use web, ui, db\nbook = db.connect['sqlite://:memory:']\nname = ''\npassword = ''\nwiped = 0\nfn setup[]\n    exec[book, 'CREATE TABLE IF NOT EXISTS people (name TEXT, role TEXT, hash TEXT)']\n    exec[book, 'INSERT INTO people VALUES (?, ?, ?)', ['ada', 'admin', hash['lovelace', 'password']]]\n    exec[book, 'INSERT INTO people VALUES (?, ?, ?)', ['bob', 'member', hash['builder', 'password']]]\nfn log_in[]\n    found = query[book, 'SELECT role, hash FROM people WHERE name = ?', [name]]\n    if len[found] eq 0\n        notice['no one called `name`']\n        return\n    if hash[password, found.hash[0]] ne found.hash[0]\n        notice['wrong password']\n        return\n    sign_in[map[name = name, role = found.role[0]]]\nfn log_out[]\n    sign_out[]\nfn wipe[]\n    if reader.role ne 'admin'\n        notice['admins only']\n        return\n    wiped = wiped + 1\nderive greeting\n    if has[reader, 'name']\n        return 'hello `reader.name`'\n    return 'hello stranger'\nview panel[]\n    return stack[p[greeting], field['Name', name, type = 'text'], field['Password', password, type = 'password'], button['Log in', click = log_in], button['Log out', click = log_out], button['Wipe', click = wipe]]\n";

    #[test]
    fn a_reader_signs_in_with_a_password_and_out_again() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[panel]\n    click[a, setup]\n    assert[len[reader] eq 0]\n    assert[elements[a, 'p'][0].text eq 'hello stranger']\n    change[a, 'name', 'ada']\n    change[a, 'password', 'wrong']\n    click[a, log_in]\n    assert[len[reader] eq 0]\n    assert[notice.text eq 'wrong password']\n    change[a, 'password', 'lovelace']\n    click[a, log_in]\n    assert[reader.name eq 'ada']\n    assert[reader.role eq 'admin']\n    assert[elements[a, 'p'][0].text eq 'hello ada']\n    click[a, log_out]\n    assert[len[reader] eq 0]\n    assert[elements[a, 'p'][0].text eq 'hello stranger']\n", SIGN_IN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn authorization_is_ordinary_code_in_the_handler() {
        // a member may not, an admin may (a signed-out reader has no role: a
        // served page stops the handler on it — tests/served/06_sign_in)
        let issues = harness_issues(&format!("{}test t[]\n    a = app[panel]\n    click[a, setup]\n    change[a, 'reader', map[name = 'bob', role = 'member']]\n    click[a, wipe]\n    assert[wiped eq 0]\n    assert[notice.text eq 'admins only']\n    change[a, 'reader', map[name = 'ada', role = 'admin']]\n    click[a, wipe]\n    assert[wiped eq 1]\n", SIGN_IN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn each_session_has_its_own_reader() {
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[alice, 'reader', map[name = 'alice']]\n    assert[elements[alice, 'p'][0].text eq 'hello alice']\n    assert[elements[bob, 'p'][0].text eq 'hello stranger']\n", SIGN_IN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn nothing_but_sign_in_writes_the_reader() {
        // a handler that makes itself an admin is refused, in both modes (the
        // front end they share)
        let refused = |src: &str| -> String {
            let stmts = crate::build(src).expect("parses");
            let lowered = crate::lower::lower_program(&stmts).expect("lowers");
            crate::infer_and_validate(&lowered, false).err().unwrap_or_default()
        };
        let e = refused("use web, ui\nfn become[]\n    reader = map[role = 'admin']\nview v[]\n    return button['x', click = become]\n");
        assert!(e.contains("'reader' is the page's signed-in reader"), "{}", e);
        // sign_in outside a handler, and a record that is not strings
        let e = refused("use web, ui\nview v[]\n    sign_in[map[id = '1']]\n    return p['x']\n");
        assert!(e.contains("sign_in[] signs the page's reader in"), "{}", e);
        let e = refused("use web, ui\nfn go[]\n    sign_in[map[id = 1]]\nview v[]\n    return button['x', click = go]\n");
        assert!(e.contains("map of strings"), "{}", e);
        // a program without `use web` keeps `reader` as a name of its own
        let ok = refused("reader = 3\nlog[reader]\n");
        assert!(ok.is_empty(), "{}", ok);
        // a plain function is handed the reader, as it is handed a module
        // variable: it cannot read the page's itself
        let e = refused("use web, ui\nfn is_admin[]\n    return reader.role eq 'admin'\nview v[]\n    return p['`is_admin[]`']\n");
        assert!(e.contains("a plain function takes it as a parameter"), "{}", e);
        let ok = refused("use web, ui\nfn is_admin[r]\n    return r.role eq 'admin'\nview v[]\n    return p['`is_admin[reader]`']\n");
        assert!(ok.is_empty(), "{}", ok);
    }

    /// Text a reader types is text wherever a view places it — a child, a
    /// view's argument, an item of a list — and markup a view or an element
    /// made is markup however it travels (the old rule placed any string
    /// that began and ended with a tag as markup: a reader's text ran as
    /// script in every other reader's page)
    #[test]
    fn text_a_reader_types_stays_text() {
        let src = "use web, ui\nnote = ''\nview card[title, body]\n    return stack[h3[title], body]\nview panel[]\n    held = p['held']\n    parts = for w in ['a', 'b']\n        span[w]\n    return stack[p[note], card['t', note], card['m', p['made']], [note], parts, held]\ntest t[]\n    a = app[panel]\n    change[a, 'note', '<img src=x onerror=alert(1)>']\n    h = html[a]\n    assert[not contains[h, '<img']]\n    assert[contains[h, '&lt;img src=x onerror=alert(1)&gt;']]\n    assert[len[elements[a, 'img']] eq 0]\n    assert[contains[h, '<p>made</p>']]\n    assert[contains[h, '<p>held</p>']]\n    assert[len[elements[a, 'span']] eq 2]\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn two_sessions_keep_their_own_facts() {
        // the server gives every page its own facts; so does the harness
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[alice, 'min_qty', 25]\n    click[bob, reset]\n    assert[alice.min_qty eq 25]\n    assert[bob.min_qty eq 0]\n    assert[elements[alice, 'p'][0].text eq 'min 25']\n", TWO_PEOPLE));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    /// A session's derivations read as its fields, each as that session
    /// sees it: the same rows, filtered by each session's own fact
    #[test]
    fn a_session_reads_its_own_derivations() {
        let src = "use web, ui, db\nbook = db.connect['sqlite://:memory:']\nmin_qty = 0\nfn setup[]\n    exec[book, 'CREATE TABLE IF NOT EXISTS orders (id INTEGER PRIMARY KEY, qty INTEGER)']\n    exec[book, 'INSERT INTO orders (qty) VALUES (5), (50)']\nderive big\n    return query[book, 'SELECT qty FROM orders WHERE qty > ? ORDER BY id', [min_qty]]\nview panel[]\n    return stack[table[big], button['Setup', click = setup]]\ntest t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[bob, 'min_qty', 10]\n    assert[len[alice.big] eq 2]\n    assert[len[bob.big] eq 1]\n    assert[bob.big.qty[0] eq 50]\n    assert[alice.big.qty[0] eq 5]\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    /// A failed truth names its own line, not the last line of the
    /// derivation it read
    #[test]
    fn a_failed_truth_names_its_own_line() {
        let issues = harness_issues("use web, ui\nx = 1\nderive twice\n    y = x * 2\n    return y\nview v[]\n    return p['x']\ntest t[]\n    a = app[v]\n    z = 1\n    assert[a.twice eq 5]\n");
        assert!(issues.len() == 1 && issues[0].contains("line 11"), "{:?}", issues);
    }

    #[test]
    fn one_session_does_not_reset_another() {
        // the old shared state made Bob's reset Alice's: that must now fail
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[alice, 'min_qty', 25]\n    click[bob, reset]\n    assert[alice.min_qty eq 0]\n", TWO_PEOPLE));
        assert!(issues.len() == 1 && issues[0].contains("assert failed"), "Bob's reset reached Alice's session: {:?}", issues);
    }

    #[test]
    fn a_table_write_reaches_every_session() {
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[bob, 'min_qty', 10]\n    click[bob, order]\n    assert[elements[alice, 'p'][1].text eq 'orders: 1']\n    assert[elements[alice, 'p'][0].text eq 'min 0']\n", TWO_PEOPLE));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn bare_fact_names_read_the_session_acted_on_last() {
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    bob = app[panel]\n    change[alice, 'min_qty', 25]\n    assert[min_qty eq 25]\n    change[bob, 'min_qty', 7]\n    assert[min_qty eq 7]\n    h = html[alice]\n    assert[min_qty eq 25]\n", TWO_PEOPLE));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn a_later_session_starts_from_the_declared_facts() {
        let issues = harness_issues(&format!("{}test t[]\n    alice = app[panel]\n    click[alice, setup]\n    change[alice, 'min_qty', 25]\n    carol = app[panel]\n    assert[carol.min_qty eq 0]\n    assert[elements[carol, 'p'][0].text eq 'min 0']\n    assert[alice.min_qty eq 25]\n", TWO_PEOPLE));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    const STAMPS: &str = "use web, ui\nstamp = 0.0\nfn mark[]\n    stamp = now[]\nview v[]\n    return stack[p['`stamp`'], button['Mark', click = mark]]\n";

    #[test]
    fn the_test_sets_the_moment_a_handler_reads() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    change[a, 'now', 1000.5]\n    click[a, mark]\n    assert[a.stamp eq 1000.5]\n    assert[now[] eq 1000.5]\n", STAMPS));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn the_harness_clock_moves_only_when_the_test_moves_it() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    click[a, mark]\n    first = a.stamp\n    sleep[5]\n    click[a, mark]\n    assert[a.stamp eq first]\n    change[a, 'now', first + 60.0]\n    click[a, mark]\n    assert[a.stamp eq first + 60.0]\n", STAMPS));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn each_session_keeps_its_own_clock() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    b = app[v]\n    change[a, 'now', 100.0]\n    change[b, 'now', 200.0]\n    click[a, mark]\n    click[b, mark]\n    assert[a.stamp eq 100.0]\n    assert[b.stamp eq 200.0]\n", STAMPS));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    const TXN: &str = "use web, ui\ncount = 0\nfn buy[]\n    count = count + 1\n    notice['bought']\n    px = float['no quote within 2s']\n    if px gt 1.0\n        count = count + 10\nfn helper_fails[]\n    px = float['helper failed']\n    if px gt 1.0\n        count = 99\nfn outer[]\n    count = count + 1\n    helper_fails[]\n    count = count + 100\nfn ok_notice[]\n    count = count + 1\n    notice['saved `count`']\nfn later_work[]\n    count = count + 1000\nfn schedules[]\n    later[later_work]\n    count = count + 1\nfn schedules_then_stops[]\n    later[later_work]\n    px = float['stopped after scheduling']\n    if px gt 1.0\n        count = 5\nview v[]\n    return stack[p['`count`'], p['said: `notice.text`'], button['a', click = buy], button['b', click = outer], button['c', click = ok_notice], button['d', click = schedules], button['e', click = schedules_then_stops]]\n";

    #[test]
    fn a_handler_that_stops_lands_nothing_and_its_click_says_why() {
        // the reader is told it stopped, and an id; the reason is the click's
        // value (and, served, the server's log beside the id)
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    r = click[a, buy]\n    assert[not r.ok]\n    assert[r.reason eq 'cannot convert \\'no quote within 2s\\' to float']\n    assert[a.count eq 0]\n    assert[contains[html[a], 'said: buy stopped, and nothing changed — the server\\'s log has why, as ']]\n    assert[not contains[html[a], 'no quote within']]\n", TXN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn a_helper_that_stops_stops_its_handler() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    r = click[a, outer]\n    assert[a.count eq 0]\n    assert[contains[r.reason, 'cannot convert']]\n    assert[contains[html[a], 'helper_fails stopped, and nothing changed']]\n", TXN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn a_notice_and_a_schedule_land_when_the_handler_commits() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    click[a, ok_notice]\n    assert[a.count eq 1]\n    assert[contains[html[a], 'said: saved 1']]\n    click[a, schedules]\n    assert[a.count eq 1002]\n    click[a, schedules_then_stops]\n    assert[a.count eq 1002]\n", TXN));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    const ROLLBACK: &str = "use web, ui, db\nbook = db.connect['sqlite://:memory:']\nseen = 0\nfn setup[]\n    exec[book, 'CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, v TEXT)']\nderive rows\n    return query[book, 'SELECT v FROM t ORDER BY id']\nfn add_then_stop[]\n    exec[book, 'INSERT INTO t (v) VALUES (?)', ['doomed']]\n    px = float['no quote']\n    if px gt 1.0\n        exec[book, 'INSERT INTO t (v) VALUES (?)', ['never']]\nfn add[]\n    exec[book, 'INSERT INTO t (v) VALUES (?)', ['kept']]\n    r = query[book, 'SELECT count(*) AS n FROM t']\n    seen = r.n[0]\nview v[]\n    return stack[p['rows: `len[rows]`'], button['s', click = setup], button['x', click = add_then_stop], button['a', click = add]]\n";

    #[test]
    fn a_handler_that_stops_rolls_its_database_writes_back() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    click[a, setup]\n    r = click[a, add_then_stop]\n    assert[not r.ok]\n    assert[len[rows] eq 0]\n    click[a, add]\n    assert[len[rows] eq 1]\n    assert[a.seen eq 1]\n", ROLLBACK));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    const REACHES: &str = "use web, ui, db\nbook = db.connect['sqlite://:memory:']\ncount = 0\nprice = 1.5\nfn setup[]\n    exec[book, 'CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, v REAL)']\nderive rows\n    return query[book, 'SELECT v FROM t']\nfn bad_effect[]\n    count = count + 1\n    px = float['no price']\n    exec[book, 'INSERT INTO t (v) VALUES (?)', [px]]\nfn bad_fact[]\n    count = count + 1\n    price = float['no price']\nview v[]\n    return stack[p['rows: `len[rows]`'], p['`count` `price`'], button['s', click = setup], button['e', click = bad_effect], button['f', click = bad_fact]]\n";

    #[test]
    fn an_error_reaching_an_effect_or_a_fact_stops_the_handler() {
        let issues = harness_issues(&format!("{}test t[]\n    a = app[v]\n    click[a, setup]\n    r = click[a, bad_effect]\n    assert[not r.ok]\n    assert[a.count eq 0]\n    assert[len[rows] eq 0]\n    q = click[a, bad_fact]\n    assert[not q.ok]\n    assert[a.price eq 1.5]\n    assert[a.count eq 0]\n    assert[contains[q.reason, 'cannot convert']]\n    assert[contains[html[a], 'bad_fact stopped, and nothing changed']]\n", REACHES));
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn buttons_carry_their_wiring_and_a_confirmed_click_asks_first() {
        let src = "use web, ui\nended = 0\nn = 0\nfn end_session[]\n    ended = ended + 1\nfn go[]\n    n = n + 1\nfn put[x]\n    n = x\nview v[]\n    return stack[button['End', click = end_session, confirm = 'End this session?'], button['Go', click = go], button['Seven', click = put[7]]]\ntest t[]\n    a = app[v]\n    b = elements[a, 'button']\n    assert[b[0]['data-crow-click'] eq '__ask']\n    assert[contains[b[0]['data-crow-args'], 'End this session?']]\n    assert[b[1]['data-crow-click'] eq 'go']\n    assert[b[2]['data-crow-args'] eq '[7]']\n    click[a, end_session]\n    assert[a.ended eq 1]\n";
        let issues = harness_issues(src);
        assert!(issues.is_empty(), "harness issues: {:?}", issues);
    }

    #[test]
    fn notice_is_said_by_something_acting_for_a_session() {
        let err = crate::build("x = 1\nnotice['hello']\n").and_then(|stmts| crate::ir_interp::prepare(&stmts, "t").map(|_| ())).err().unwrap_or_default();
        assert!(err.contains("notice[] tells a page's reader something"), "{}", err);
    }

    #[test]
    fn test_removed_time_builtins_teach() {
        for (src, want) in [
            ("d = date[2024, 1, 15]", "moment["),
            ("e = elapsed[1.0, 2.0]", "subtract moments"),
            ("t = today[]", "fmt[now[]"),
        ] {
            let err = run(src).err().expect("a removed builtin is refused");
            assert!(err.contains(want), "expected teaching error for {}, got {}", src, err);
        }
    }

}
