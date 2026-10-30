//! Where a stop inside a function of another file says it happened, in
//! both modes: inside a package installed under crow_modules, the
//! program's line whose call reached it, as inside a standard module's —
//! the package's own lines are no line the program wrote (Magpie's stop on
//! a parsed value handed to `merge` had said `line 1582`, a line of the
//! package's main.co) — and inside a part of the program's own, the part's
//! line, as before.

use std::process::Command;

fn crow() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_crow"))
}

/// What a run prints on its error output, its last line.
fn stop(dir: &std::path::Path, interpreted: bool, file: &str) -> String {
    let mut cmd = Command::new(crow());
    cmd.arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    let out = cmd.arg(file).current_dir(dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
    assert!(!out.status.success(), "{} stops", file);
    String::from_utf8_lossy(&out.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string()
}

#[test]
fn a_stop_in_a_package_names_the_programs_line_and_one_in_a_part_its_own() {
    let dir = std::env::temp_dir().join(format!("crow_package_stops_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'stops'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    // the package stops on its own line 2, two calls down
    std::fs::write(dir.join("crow_modules/pk/main.co"), "fn second[xs]\n    return xs[1]\n\nfn hello[xs]\n    n = second[xs]\n    return n + 1\n").unwrap();
    std::fs::write(dir.join("main.co"), "use pk\n\nlog[pk.hello[[1, 2]]]\nlog[pk.hello[[1]]]\n").unwrap();
    // the part stops on its own line 3, called from the program's line 4
    std::fs::write(dir.join("lib/own.co"), "fn first[xs]\n    a = 1\n    return xs[5]\n").unwrap();
    std::fs::write(dir.join("own.co"), "use lib.own\n\nlog['a']\nlog[own.first[[1, 2]]]\n").unwrap();
    for interpreted in [true, false] {
        assert_eq!(stop(&dir, interpreted, "main.co"), "runtime error: line 4: index 1 is out of range for an array of 1 item", "interpreted: {}", interpreted);
        assert_eq!(stop(&dir, interpreted, "own.co"), "runtime error: line 3: index 5 is out of range for an array of 2 items", "interpreted: {}", interpreted);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
