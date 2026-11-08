//! `args[]` answers the words after the source file whatever crow's binary
//! is named. The runtime had taken the process for crow when the binary's
//! name was `crow`, so a crow under another name answered its own
//! arguments too, interpreted — `['run', '-i', 'main.co', '1', 'two']` —
//! where the compiled program answered `['1', 'two']` (found when raven's
//! tests ran a crow named `crow-nollm`, 2026-11-07).

use std::process::Command;

#[test]
fn a_programs_arguments_are_the_words_after_its_file_whatever_crow_is_named() {
    let dir = std::env::temp_dir().join(format!("crow_program_args_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), "log[args[]]\n").unwrap();
    let renamed = dir.join("crow-renamed");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_crow"), &renamed).unwrap();
    for mode in [&["run", "-i", "main.co", "1", "two"][..], &["run", "main.co", "1", "two"][..]] {
        let out = Command::new(&renamed).args(mode).current_dir(&dir).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{:?} failed:\n{}{}", mode, stdout, String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout.trim(), "['1', 'two']", "{:?}", mode);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
