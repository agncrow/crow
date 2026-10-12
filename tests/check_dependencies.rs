//! `crow check` probes the app's own functions with edge cases, not a
//! dependency's (crow-check.md): a package under crow_modules is held by
//! its own tests, and the app's report names the app's functions.

use std::process::{Command, Stdio};

/// `crow` run in the app's directory: what it said, both streams.
fn crow(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn a_dependencys_functions_are_not_probed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("crow_check_dependency");
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("host");
    std::fs::create_dir_all(app.join("dep_src")).unwrap();
    // a package of one function that stops on a short list, and an app with
    // the same function of its own
    std::fs::write(app.join("dep_src").join("crowfile"), "name = 'dep'\n").unwrap();
    std::fs::write(app.join("dep_src").join("f.co"), "fn third[xs]\n    return xs[2]\n").unwrap();
    std::fs::write(app.join("crowfile"), "name = 'host'\ndeps = map[dep = './dep_src']\n").unwrap();
    std::fs::write(app.join("main.co"), "use dep.f\n\nfn own_third[xs]\n    return xs[2]\n\nlog[own_third[[1, 2, 3]]]\nlog[f.third[[1, 2, 3]]]\n").unwrap();
    let installed = crow(&app, &["install"]);
    assert!(installed.contains("dep: linked to"), "{}", installed);
    let said = crow(&app, &["check", "--verbose", "main.co"]);
    assert!(said.contains("own_third crashes on"), "the app's own function is probed:\n{}", said);
    assert!(!said.contains("dep_f_third crashes on"), "the dependency's is not:\n{}", said);
    let _ = std::fs::remove_dir_all(&root);
}
