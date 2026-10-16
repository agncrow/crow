//! The console (`crow` with no file) from the outside, its lines piped in:
//! a `view` block is read whole as a `fn` block is, an `else` goes on with
//! its `if`, and a program that serves pages is told where it runs — the
//! console serves nothing (a pasted app had looked like nothing happening).

use std::io::Write;
use std::process::{Command, Stdio};

fn console(lines: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_crow")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(lines.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn an_else_goes_on_with_its_if_and_a_view_is_one_block() {
    let said = console("x = 3\nif x gt 5\n    log['big']\nelse\n    log['small']\n\nuse ui\nview card[]\n    return p['hi']\n\nlog['after']\n");
    assert!(said.contains("small") && said.contains("after"), "{}", said);
    assert!(!said.contains("error"), "no line of a block was read alone: {}", said);
}

#[test]
fn a_program_that_serves_is_told_where_it_runs() {
    let said = console("use web, ui\nn = 0\nview home[]\n    return p['hello']\n\nserve[3459]\npage['/', home]\n");
    assert!(said.contains("the console serves no pages") && said.contains("crow run -i"), "{}", said);
}
