//! A list literal's shape is read off any item whose shape is known: in a
//! test file, a function no test calls — its parameters typed by nothing —
//! loops over `[2.0 / w, 0.0]` as over a vector. Read off the first item
//! alone, the rank had been unknown and the loop refused ("a loop over `m`
//! walks rows or numbers by its rank"), failing every test in the file
//! (Lory's `_pixels`, reached by a frame's batch no chart test called).

use std::process::Command;

#[test]
fn a_loop_over_a_literal_of_an_untyped_item_is_a_loop_over_a_vector() {
    let dir = std::env::temp_dir().join(format!("crow_uncalled_rank_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.co"),
        "fn pixels[w, h]\n    [2.0 / w, 0.0, 0.0 - 2.0 / h, 1.0]\n\nfn text_of[m]\n    parts = []\n    for x in m\n        parts = push[parts, str[x]]\n    join[parts, ' ']\n\nfn made[c]\n    out = []\n    for v in c.views\n        m = pixels[c.width, c.height]\n        out = push[out, text_of[m]]\n    out\n\nfn other[n]\n    n + 1\n\ntest plain[]\n    other[1] eq 2\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["test", "main.co"]).current_dir(&dir).output().unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", said);
    assert!(said.contains("1 block pass"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A list a function answers is that function's to type: in a test that
/// reaches a frame's function by no call, a package's function reading a
/// record's parsed frame took `m = _floats[v.camera]`, its items of no kind
/// yet, and `m[2]` was refused as an item of a list "never given an item"
/// (Lory's `walk`, in every test of a scene that did not draw a frame).
#[test]
fn a_list_a_function_answers_is_not_refused_as_never_given_an_item() {
    let dir = std::env::temp_dir().join(format!("crow_answered_list_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'app'\n").unwrap();
    std::fs::write(dir.join("crow_modules/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(
        dir.join("crow_modules/pk/main.co"),
        "struct Canvas [frame, n]\n\nfn begin[frame]\n    Canvas[frame, 0]\n\nfn _floats[j]\n    out = []\n    for x in j\n        out = push[out, 0.0 + x]\n    out\n\nfn walked[c]\n    fx = 0.0\n    for v in c.frame.views\n        m = _floats[v.camera]\n        if len[m] eq 16\n            fx = 0.0 - m[2]\n    fx\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.co"), "use sys\nuse pk\n\nfn draw[frame]\n    log[pk.walked[pk.begin[frame]]]\n\nsys.frames[draw]\n\ntest plain[]\n    m = [-1.0]\n    m[0] eq -1.0\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(["test", "main.co"]).current_dir(&dir).output().unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", said);
    assert!(said.contains("1 block pass"), "{}", said);
    let _ = std::fs::remove_dir_all(&dir);
}
