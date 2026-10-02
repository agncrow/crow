//! Chains computed in one pass answer what the operators answer
//! (second-half-design.md §3): an element-wise chain of tensors
//! (crow_runtime::fused) and a filter's predicate (crow_runtime::mask).
//! Each program here runs fused and with `CROW_NO_FUSE=1`, in both modes,
//! and prints the same — every number, every row kept, and a stop's words.
//! (A benchmark's own timing lines are left out.)

use std::process::Command;

const PROGRAMS: &[&str] = &[
    "tests/parity/148_an_elementwise_chain_in_one_pass.co",
    "tests/bench/tensors.co",
    "tests/bench/train.co",
    "tests/parity/149_a_mask_compares_each_cell_as_two_values_do.co",
    "tests/parity/65_missing_cells.co",
];

/// What a run printed, its timings left out.
fn run(mode: &[&str], file: &str, fused: bool) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("target").join(format!("crow_fusion_{}", if fused { "fused" } else { "unfused" }));
    std::fs::create_dir_all(&dir).unwrap();
    let name = std::path::Path::new(file).file_name().unwrap();
    std::fs::copy(root.join(file), dir.join(name)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crow"));
    cmd.args(mode).arg(name).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").env("BENCH_QUICK", "1");
    if fused {
        cmd.env_remove("CROW_NO_FUSE");
    } else {
        cmd.env("CROW_NO_FUSE", "1");
    }
    let out = cmd.output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    text.lines().filter(|l| !l.starts_with("bench:")).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_fused_chain_answers_what_the_operators_answer_in_both_modes() {
    for file in PROGRAMS {
        for mode in [&["run", "-i"][..], &["run"][..]] {
            let (fused, unfused) = (run(mode, file, true), run(mode, file, false));
            assert_eq!(fused, unfused, "{} ({:?}): fused and unfused differ", file, mode);
            assert!(!fused.is_empty(), "{} ({:?}) printed nothing", file, mode);
        }
    }
}
