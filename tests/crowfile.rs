//! A project's crowfile from the outside: the keys the spec documents are
//! read — `cpu` and `gpu` are builtins' names, which had stopped every
//! command as variables taking a builtin's name — and a job the crowfile
//! names runs interpreted as it runs compiled (`crow run -i <job>` had read
//! the job's name as a file's).

use std::process::Command;

fn crow(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_crowfile_with_cpu_and_gpu_runs_its_entry_and_its_jobs() {
    let dir = std::env::temp_dir().join(format!("crow_crowfile_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("jobs")).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'res'\nentry = 'main.co'\ncpu = 4\ngpu = 1\njobs = map[hello = 'jobs/hello.co']\n").unwrap();
    std::fs::write(dir.join("main.co"), "log['the entry ran']\n").unwrap();
    std::fs::write(dir.join("jobs/hello.co"), "log['the job ran']\n").unwrap();
    let (ok, said) = crow(&dir, &["run", "-i"]);
    assert!(ok && said.contains("the entry ran"), "{}", said);
    let (ok, said) = crow(&dir, &["run", "-i", "hello"]);
    assert!(ok && said.contains("the job ran"), "{}", said);
    let (ok, said) = crow(&dir, &["run", "-i", "nosuch"]);
    assert!(!ok && said.contains("unknown job 'nosuch'") && said.contains("hello"), "{}", said);
    // a count is a whole number of 1 or more, as cpu[n] takes it
    std::fs::write(dir.join("crowfile"), "name = 'res'\ncpu = 0\n").unwrap();
    let (ok, said) = crow(&dir, &["run", "-i"]);
    assert!(!ok && said.contains("'cpu' is a count"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `crow build --ir` with no file prints the crowfile's entry with the
/// crowfile's counts, as the build compiles it: the printing flags had
/// taken the entry and left its `cpu` and `gpu` behind.
#[test]
fn the_printed_ir_carries_the_crowfiles_counts() {
    let dir = std::env::temp_dir().join(format!("crow_crowfile_ir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'res'\ncpu = 3\n").unwrap();
    std::fs::write(dir.join("main.co"), "log['hi']\n").unwrap();
    let (ok, said) = crow(&dir, &["build", "--ir"]);
    assert!(ok && said.lines().any(|l| l.trim() == "cpu 3"), "{}", said);
    let (ok, said) = crow(&dir, &["build", "--ir", "main.co"]);
    assert!(ok && !said.lines().any(|l| l.trim() == "cpu 3"), "a file named is built as itself: {}", said);
    let _ = std::fs::remove_dir_all(&dir);
}
