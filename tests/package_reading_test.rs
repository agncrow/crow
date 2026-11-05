//! A test file whose one test writes through a package and whose other
//! only reads: the package's function the writing test reaches hands a
//! helper of its own to `revise`, and in the reading test, where nothing
//! types that function's parameters, the helper was refused as "handed as
//! a value, and nothing types its parameters" (Myna's query tests, over
//! Magpie's sequencer: `put_rows` hands `_appended`). A dependency's
//! function its program does not call is a standard module's in this.

use std::process::Command;

#[test]
fn a_test_that_only_reads_a_package_beside_one_that_writes_passes() {
    let dir = std::env::temp_dir().join(format!("crow_package_reading_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'reads'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(
        dir.join("crow_modules/pk/main.co"),
        "use memory\n\nstruct Tally [n]\n\nfn _added[t, x]\n    return Tally[t.n + x]\n\nfn add[name, x]\n    t = revise[name, Tally[0], _added, x]\n    return t.n\n\nfn peek[name]\n    return len[name]\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.co"), "use pk\n\ntest writes[]\n    assert[pk.add['k', 3] eq 3]\n\ntest reads[]\n    assert[pk.peek['k'] eq 1]\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["test", "main.co"]).current_dir(&dir).output().unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", said);
    assert!(said.contains("2 blocks pass"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}
