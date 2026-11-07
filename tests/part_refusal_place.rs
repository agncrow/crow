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

/// A package's parameter named like a builtin is refused at its function's
/// line. A dependency's part lowers no top-level statement, so its line
/// markers set no line, and the refusal had been placed at the line
/// lowered last — another function's body (Lory's `measure[c, text,
/// size]` shown at a line forty below it) — or at line 0, where the
/// printer panicked subtracting one from it.
#[test]
fn a_packages_parameter_is_refused_at_its_functions_line() {
    let dir = std::env::temp_dir().join(format!("crow_part_param_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/main.co"), "--- a package\nfn one[x]\n    x + 1\n\nfn two[x]\n    x + 2\n\nfn shown[c, input]\n    c + input\n").unwrap();
    for (main, case) in [("use pk\n\nlog[pk.shown[1, 2]]\n", "alone"), ("use pk\n\nlog[pk.two[1]]\nlog[pk.shown[1, 2]]\n", "after another")] {
        std::fs::write(dir.join("main.co"), main).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["run", "-i", "main.co"]).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "{}: {}", case, said);
        assert!(!said.contains("panicked"), "{}: {}", case, said);
        assert!(said.contains("crow_modules/pk/main.co:8:"), "{}: {}", case, said);
        assert!(said.contains("'input' is a core builtin"), "{}: {}", case, said);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package's local named like a function of its own file is refused by
/// the name the file writes: the part's names are prefixed where they are
/// inlined, and the refusal had said "'pk_row' is a function of this
/// program" of a local `row` (Lory's gui, whose drawn keyboard had a local
/// named like its `row` control).
#[test]
fn a_packages_local_named_like_its_function_is_refused_by_its_own_name() {
    let dir = std::env::temp_dir().join(format!("crow_part_local_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/main.co"), "fn row[k]\n    [k]\n\nfn f[]\n    rows = [[1, 2]]\n    out = 0\n    for r in range[1]\n        row = rows[r]\n        out = row[0]\n    out\n").unwrap();
    std::fs::write(dir.join("main.co"), "use pk\n\nlog[pk.f[]]\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["run", "-i", "main.co"]).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{}", said);
    assert!(said.contains("crow_modules/pk/main.co:8:9: 'row' is a function of this program"), "{}", said);
    assert!(!said.contains("pk_row"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A package's call of its own helper with the wrong count names the
/// helper as the package writes it: the refusal had said "'pk__modes'
/// expects 0 arguments" of `_modes` (Lory's glTF reader).
#[test]
fn a_packages_miscounted_call_names_its_function_as_written() {
    let dir = std::env::temp_dir().join(format!("crow_part_count_name_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/main.co"), "fn _modes[]\n    [1, 2]\n\nfn f[x]\n    m = _modes[x]\n    m[0]\n").unwrap();
    std::fs::write(dir.join("main.co"), "use pk\n\nlog[pk.f[1]]\n").unwrap();
    for mode in [&["run", "-i", "main.co"][..], &["check", "main.co"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).current_dir(&dir).stdin(std::process::Stdio::null()).output().unwrap();
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "{:?}: {}", mode, said);
        assert!(said.contains("crow_modules/pk/main.co:5:9: '_modes' expects 0 arguments, got 1"), "{:?}: {}", mode, said);
        assert!(!said.contains("pk__modes"), "{:?}: {}", mode, said);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
