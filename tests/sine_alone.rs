//! A sine and a cosine of one value are each their own libm call in every
//! build: LLVM fuses the pair into one call where it sees both
//! (`__sincos_stret` on Apple's platforms), which answers the last digit
//! differently from `sin` now and then, and a release build — `crow
//! build`, where the program's two calls were inlined into one function —
//! had printed other sines than `crow run -i` (18 of 20,000 here; pie's
//! Box–Muller draws found it). `crow_runtime::math::sine` and `cosine` are
//! never inlined, so no build sees the pair.

use std::process::Command;

fn crow(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_release_build_prints_the_interpreters_sines() {
    let dir = std::env::temp_dir().join(format!("crow_sine_alone_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = "use math\n\nlines = []\nfor k in range[20000]\n    t = float[k] * 0.000731 + 0.25\n    s = sin[t]\n    c = cos[t]\n    lines = push[lines, '`s` `c`']\nlog[join[lines, '\\n']]\n";
    std::fs::write(dir.join("pair.co"), program).unwrap();
    let (ok, interpreted) = crow(&dir, &["run", "-i", "pair.co"]);
    assert!(ok, "{}", interpreted);
    let (ok, said) = crow(&dir, &["build", "pair.co"]);
    assert!(ok, "{}", said);
    let out = Command::new(dir.join("pair")).current_dir(&dir).output().unwrap();
    let built = String::from_utf8_lossy(&out.stdout).to_string();
    let differ: Vec<(&str, &str)> = interpreted.lines().zip(built.lines()).filter(|(a, b)| a != b).take(3).collect();
    assert!(differ.is_empty() && interpreted.lines().count() == built.lines().count(), "the release build's sines differ from the interpreter's: {:?}", differ);
    let _ = std::fs::remove_dir_all(&dir);
}
