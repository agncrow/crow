//! `crow check --db <url>` and `crow fix` from the outside, over SQLite:
//! the check prints the exact command that applies a constraint fix — with
//! its `--db`, without which `crow fix` had found no hazard to fix — and
//! the exact file it writes; run as printed, the fix writes that file and
//! applies it as `crow db apply` does, recorded, so the next `crow db apply`
//! has nothing to run (it had run the index again: "index … already
//! exists").

use std::path::Path;
use std::process::Command;

fn crow(dir: &Path, args: &[String]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(|w| w.trim_matches('\'').to_string()).collect()
}

#[test]
fn the_printed_fix_writes_the_named_file_and_applies_it_recorded() {
    let dir = std::env::temp_dir().join(format!("crow_fix_migration_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/db_check/write_skew.co")).unwrap();
    std::fs::write(dir.join("trading.co"), program).unwrap();
    let schema = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/db_check/write_skew.schema.sql")).unwrap();
    let url = format!("sqlite://{}", dir.join("dev.db").display());
    crow_db::connect(&url, Some(1)).unwrap().exec(schema.trim().trim_end_matches(';'), &[]).unwrap();

    let (_, report) = crow(&dir, &words(&format!("check --db {} --strict trading.co", url)));
    let named = report.lines().find_map(|l| l.split("— writes ").nth(1)).map(|f| f.trim_end_matches(':').to_string()).unwrap_or_else(|| panic!("no migration named:\n{}", report));
    assert_eq!(named, "migrations/0001_one_active_per_symbol.sql", "{}", report);
    let apply = report.lines().find_map(|l| l.trim().strip_prefix("apply: crow ")).unwrap_or_else(|| panic!("no apply line:\n{}", report));
    assert!(apply.ends_with(&format!("--db '{}'", url)), "{}", apply);

    let (ok, said) = crow(&dir, &words(apply));
    assert!(ok, "{}", said);
    assert!(said.contains(&format!("wrote {}", named)) && said.contains("applied 1 migration from migrations") && said.contains("hazard no longer reported"), "{}", said);
    assert!(dir.join(&named).exists());
    let (ok, said) = crow(&dir, &words(&format!("db apply migrations {}", url)));
    assert!(ok && said.contains("applied 0 migrations"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}
