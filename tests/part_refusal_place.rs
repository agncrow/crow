//! A call inside an imported part, refused by the pass that fills in named
//! arguments, is placed in the part's file at its own line. It had been
//! placed in the entry at the entry's line of that number: a bare `move`
//! in Magpie's manifest.co:220, taken by a Myna test file's own `move`,
//! was shown as that test file's line 220, a line it does not have.

use std::process::Command;

#[test]
fn a_parts_call_is_refused_in_the_parts_file() {
    let dir = std::env::temp_dir().join(format!("crow_part_refusal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/helper.co"), "use sys\n\n--- a helper\nfn shift[p, q]\n    placed = move[p, q]\n    return placed\n").unwrap();
    std::fs::write(dir.join("main.co"), "use lib.helper\n\nfn move[a, b, c]\n    return a\n\nlog[helper.shift['x', 'y']]\n").unwrap();
    for mode in [&["run", "-i", "main.co"][..], &["check", "main.co"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "{:?}: {}", mode, said);
        assert!(said.contains("lib/helper.co:5:14: missing argument 'c' in call to 'move'"), "{:?}: {}", mode, said);
        assert!(said.contains("placed = move[p, q]"), "{:?}: {}", mode, said);
        assert!(!said.contains("main.co:5"), "{:?}: {}", mode, said);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package's call of a row it imports bare, taken by a program's own
/// function of that name (the names a program and its packages share), is
/// refused for its count in the package's file. A dependency's function is
/// lowered muted where the checks would have counted, and Magpie's
/// `ints_of[col, from, to]` against a Myna part's `ints_of[col]` had
/// reached the verifier: "internal error: the lowered program disagrees
/// with itself".
#[test]
fn a_packages_call_taken_by_a_program_function_is_counted() {
    let dir = std::env::temp_dir().join(format!("crow_part_count_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pkg")).unwrap();
    std::fs::write(dir.join("crow_modules/pkg/main.co"), "use sys\n\n--- the first item, by a row the program also names\nfn first_of[xs]\n    return ints_of[xs, 0, 1]\n").unwrap();
    std::fs::write(dir.join("main.co"), "use pkg\n\nfn ints_of[col]\n    return col\n\nlog[pkg.first_of[[1, 2]]]\n").unwrap();
    for mode in [&["run", "-i", "main.co"][..], &["check", "main.co"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "{:?}: {}", mode, said);
        assert!(said.contains("crow_modules/pkg/main.co:5:12: 'ints_of' expects 1 argument, got 3"), "{:?}: {}", mode, said);
        assert!(!said.contains("internal error"), "{:?}: {}", mode, said);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
