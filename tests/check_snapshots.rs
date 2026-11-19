//! `crow check` keeps each probed function's answers under the file that
//! declares it (`.crow/snapshots/<file>/<function>.snap`), so two files of
//! one folder, each with a function of one name, are checked in turn
//! without either meeting the other's answers (crow-check.md).

use std::process::{Command, Stdio};

/// `crow` run in the folder: what it said, both streams.
fn crow(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// Lory's main.co, gui.co and plot.co each hold a `_fresh`: kept under the
/// function's name alone, each check found the one checked before it and
/// reported "_fresh behavior changed". A store an older crow kept
/// (`.crow/snapshots/_fresh.snap`) is not read, and its file is removed
/// once `_fresh` is kept under its file.
#[test]
fn two_files_functions_of_one_name_never_meet() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("crow_check_snapshots");
    let _ = std::fs::remove_dir_all(&root);
    let store = root.join(".crow").join("snapshots");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(&store).unwrap();
    let fresh = |answer: u32| format!("fn _fresh[n]\n    if n gt 0\n        return {}\n    return 0\n\nlog[_fresh[3]]\n", answer);
    std::fs::write(root.join("main.co"), fresh(1)).unwrap();
    std::fs::write(root.join("lib").join("gui.co"), fresh(2)).unwrap();
    std::fs::write(store.join("_fresh.snap"), "_fresh[1] = 9").unwrap();
    for file in ["main.co", "lib/gui.co", "main.co", "lib/gui.co"] {
        let said = crow(&root, &["check", file]);
        assert!(!said.contains("behavior changed"), "{} met another file's `_fresh`:\n{}", file, said);
    }
    assert!(store.join("main").join("_fresh.snap").exists());
    assert!(store.join("lib-gui").join("_fresh.snap").exists());
    assert!(!store.join("_fresh.snap").exists(), "the old file is removed");
    // a changed answer of one file's own function is still found
    std::fs::write(root.join("main.co"), fresh(5)).unwrap();
    let said = crow(&root, &["check", "main.co"]);
    assert!(said.contains("_fresh behavior changed: _fresh[1] was 1, now 5"), "{}", said);
    let _ = std::fs::remove_dir_all(&root);
}
