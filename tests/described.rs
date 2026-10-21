//! A part's function is named as the program wrote it wherever `describe`
//! names it — the name a model is told, calls back and the steps frame
//! records, and the name a refusal uses — whichever file is the entry.
//! The loop's check of a gate names the gate as `describe` names it, before
//! any model is asked, so no model is needed: run from the part, the name
//! had been `wrong`; run from an entry importing the part, the
//! part-qualified internal name `part_tools_wrong` (found building
//! examples/agent, whose pages told the model `agent_support_order_status`
//! where its evals said `order_status`, and recorded fixtures of their own).

use std::process::Command;

const PART: &str = "--- a part holding a tool, a gate that is none, and the loop over them:
--- the loop checks each gate before any model is asked, and its refusal
--- names the gate as `describe` names it — the name a model would be
--- told for a tool
use llm

--- the weather in a city
fn weather[city]
    return 'sunny in `upper[city]`'

--- two parameters and text answered: no gate
fn wrong[a, b]
    return '`upper[a]` `upper[b]`'

fn named[]
    m = reach['openai://127.0.0.1:9/m']
    done = act[m, ask['hi'], tools = [weather], allow = [wrong]]
    return done.reason

--- from the part itself, the function is named as written
test the_function_is_named_as_written[]
    assert[named[] eq 'a gate takes the call and answers a bool: wrong takes 2 and answers text']
";

const MAIN: &str = "--- the entry importing the part: the loop names the function as the
--- part wrote it, not by its part-qualified internal name
use part.tools

log[tools.named[]]
";

#[test]
fn a_parts_function_is_named_as_written_from_any_entry() {
    let dir = std::env::temp_dir().join(format!("crow_described_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("part")).unwrap();
    std::fs::write(dir.join("part").join("tools.co"), PART).unwrap();
    std::fs::write(dir.join("main.co"), MAIN).unwrap();
    let expected = "a gate takes the call and answers a bool: wrong takes 2 and answers text";
    // the part's own test, from the part
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["test", "part/tools.co"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "the part's test failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    // the entry, in both modes
    for mode in [&["run", "-i", "main.co"][..], &["run", "main.co"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).current_dir(&dir).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{:?} failed:\n{}{}", mode, stdout, String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout.trim(), expected, "{:?}", mode);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
