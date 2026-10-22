//! `crow new` from the outside: every flavour it scaffolds — plain, pages,
//! a training job, a database, and all of them at once — writes a project
//! whose every file is a program the language takes, whose entry runs (a
//! served one is lowered, not served), whose job runs, and whose files'
//! tests run; it lists what it wrote and nothing else. (`--db` had written
//! a `table users` block that does not parse and listed a
//! `jobs/migrate.co` it never wrote; `--ml`'s job called `train` bare,
//! which is undefined.)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn crow(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// The project's files, relative to it.
fn files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path: PathBuf = entry.path();
        if path.is_dir() {
            if !matches!(entry.file_name().to_str(), Some(".crow" | ".crow_build")) {
                files(root, &path, out);
            }
        } else {
            out.push(path.strip_prefix(root).unwrap().to_string_lossy().to_string());
        }
    }
}

#[test]
fn every_flavour_scaffolds_a_project_that_works() {
    let base = std::env::temp_dir().join(format!("crow_new_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // (a file two flavours write alike is checked once: a program that
    // uses db takes seconds to lower in a debug build)
    let mut checked: BTreeSet<String> = BTreeSet::new();
    for (name, flags) in [("plain", vec![]), ("pages", vec!["--web"]), ("trains", vec!["--ml"]), ("stores", vec!["--db"]), ("every", vec!["--web", "--ml", "--db"])] {
        let mut args = vec!["new", name];
        args.extend(flags.iter().copied());
        let (ok, said) = crow(&base, &args);
        assert!(ok, "{}: {}", name, said);
        let root = base.join(name);
        // it lists what it wrote, and wrote what it lists
        let mut written = Vec::new();
        files(&root, &root, &mut written);
        written.sort();
        let mut listed: Vec<String> = said.lines().filter(|l| l.starts_with("  ")).map(|l| l.trim().to_string()).collect();
        listed.sort();
        assert_eq!(written, listed, "{}: {}", name, said);
        let (web, ml, db) = (flags.contains(&"--web"), flags.contains(&"--ml"), flags.contains(&"--db"));
        // the entry runs (a page is served, so it is checked below, not run)
        if !web {
            let (ok, said) = crow(&root, &["run", "-i"]);
            assert!(ok && said.contains(&format!("hello, {}", name)), "{}: {}", name, said);
            checked.insert(std::fs::read_to_string(root.join("main.co")).unwrap());
        }
        if ml {
            let (ok, said) = crow(&root, &["run", "-i", "train"]);
            assert!(ok && said.contains("training complete"), "{}: {}", name, said);
            checked.insert(std::fs::read_to_string(root.join("jobs/train.co")).unwrap());
        }
        // every other file is a program the language takes, and its tests run
        for f in written.iter().filter(|f| f.ends_with(".co")) {
            if !checked.insert(std::fs::read_to_string(root.join(f)).unwrap()) {
                continue;
            }
            let (ok, said) = crow(&root, &["test", f]);
            assert!(ok, "{}: crow test {}: {}", name, f, said);
        }
        if db {
            assert!(std::fs::read_to_string(root.join(".gitignore")).unwrap().contains(&format!("{}.db", name)));
        }
        if db && web {
            // the tables main.co declares, drafted for a database that lacks them
            let (ok, said) = crow(&root, &["db", "diff"]);
            assert!(ok && said.contains("CREATE TABLE IF NOT EXISTS users (name TEXT NOT NULL, email TEXT);"), "{}: {}", name, said);
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A program's own `db/schema.co` is imported by its dot path, as
/// `views/home.co` is: every `use db.…` had been refused as a driver's own
/// module, while a driver's (`db.postgres`) still is.
#[test]
fn a_programs_own_db_folder_is_imported_by_its_dot_path() {
    let base = std::env::temp_dir().join(format!("crow_db_folder_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("db")).unwrap();
    std::fs::write(base.join("db/schema.co"), "fn greeting[]\n    return 'tables declared'\n").unwrap();
    std::fs::write(base.join("main.co"), "use db.schema\n\nlog[schema.greeting[]]\n").unwrap();
    std::fs::write(base.join("driver.co"), "use db.postgres\n\nlog['no']\n").unwrap();
    let (ok, said) = crow(&base, &["run", "-i", "main.co"]);
    assert!(ok && said.contains("tables declared"), "{}", said);
    let (ok, said) = crow(&base, &["run", "-i", "driver.co"]);
    assert!(!ok && said.contains("the driver's own"), "{}", said);
    let _ = std::fs::remove_dir_all(&base);
}
