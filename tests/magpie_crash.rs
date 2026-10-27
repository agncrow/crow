//! The crash harness (docs/magpie-design.md, phase 3): a merge ended at
//! each of its cuts — after the segments are placed and before the
//! manifest is switched (`placed`), and after the switch, before the
//! inputs are removed (`listed`) — leaves a log the next read and the
//! next merge answer whole. The crashing merge runs in both modes; the
//! kernel's cut (`CROW_MAGPIE_CRASH`) is the same in both.

use std::path::Path;
use std::process::Command;

fn crow() -> String {
    let release = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/release/crow");
    if release.exists() {
        return release.to_string_lossy().to_string();
    }
    env!("CARGO_BIN_EXE_crow").to_string()
}

fn run(dir: &Path, interpreted: bool, file: &str, crash: Option<&str>) -> (i32, String, String) {
    let mut cmd = Command::new(crow());
    cmd.current_dir(dir).arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    cmd.arg(file);
    if let Some(step) = crash {
        cmd.env("CROW_MAGPIE_CRASH", step);
    }
    let out = cmd.output().expect("crow runs");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

fn drive(interpreted: bool, step: &str) {
    let tag = format!("{}-{}", if interpreted { "i" } else { "c" }, step);
    let base = std::env::temp_dir().join(format!("crow-magpie-crash-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // three batches, then a merge that the cut ends
    std::fs::write(
        base.join("crash.co"),
        "use magpie\nuse frame\n\na = batch['log', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]\nb = batch['log', frame.new[id = [2, 3], qty = [25, 30]]]\nc = batch['log', frame.new[id = [1], qty = [15]]]\nlog[head['log']]\nm = merge['log']\nlog['merged: `m`']\n",
    )
    .unwrap();
    let (code, stdout, stderr) = run(&base, interpreted, "crash.co", Some(step));
    assert_eq!(code, 3, "the cut ends the process (interpreted: {interpreted}, step {step}): {stdout} {stderr}");
    assert_eq!(stdout, "4\n", "nothing past the cut ran: {stdout}");
    let segments: Vec<String> = std::fs::read_dir(base.join("log")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".segment")).collect();
    assert_eq!(segments.len(), 1, "the segment was placed before either cut: {segments:?}");
    // the next read answers the table whole, and the next merge leaves one live segment
    std::fs::write(
        base.join("after.co"),
        "use magpie\nuse frame\n\nb = batches['log']\nlog[b.form]\nlog[latest['log'].qty]\nlog[head['log']]\nm = merge['log']\nlog[len[m] gt 0]\nafter = batches['log']\nlog[after.form]\nlog[latest['log'].qty]\nlog[lookup['log', ['id'], ['2']].qty]\nlog[batch['log', frame.new[id = [4], qty = [40]]]]\n",
    )
    .unwrap();
    let (code, stdout, stderr) = run(&base, true, "after.co", None);
    assert_eq!(code, 0, "the log reads after the cut: {stderr}");
    let forms_before = if step == "placed" { "['batch', 'batch', 'batch']" } else { "['segment', 'stale', 'stale', 'stale']" };
    let merged = if step == "placed" { "true" } else { "false" };
    assert_eq!(stdout, format!("{forms_before}\n[25, 30, 15]\n4\n{merged}\n['segment']\n[25, 30, 15]\n[25]\n5\n"), "after the cut at {step} (interpreted: {interpreted}): {stdout}");
    let leftovers: Vec<String> = std::fs::read_dir(base.join("log")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".segment") || n.ends_with(".batch")).collect();
    assert_eq!(leftovers.len(), 2, "one segment and the batch written after: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&base);
}

/// A put is visible to its process at once and durable at the flush: a
/// process ended after a put and before the flush loses that put's
/// rows, and keeps every row a `batch` or a `sync` had flushed.
fn drive_pending(interpreted: bool) {
    let base = std::env::temp_dir().join(format!("crow-magpie-pending-{}-{}", if interpreted { "i" } else { "c" }, std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // a batch (a put and a flush: the cut's first reach), then a put the
    // cut ends the process after, before its flush
    std::fs::write(
        base.join("cut.co"),
        "use magpie\nuse frame\n\na = batch['log', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]\nlog[a]\nb = put['log', frame.new[id = [3], qty = [30]]]\nlog[since['log', 0].qty]\nlog['past the cut']\n",
    )
    .unwrap();
    let (code, stdout, stderr) = run(&base, interpreted, "cut.co", Some("pending:2"));
    assert_eq!(code, 3, "the cut ends the process (interpreted: {interpreted}): {stdout} {stderr}");
    assert_eq!(stdout, "0\n", "the batch answered, the put did not: {stdout}");
    std::fs::write(base.join("after.co"), "use magpie\nuse frame\n\nlog[batches['log'].form]\nlog[since['log', 0].qty]\nlog[head['log']]\nlog[put['log', frame.new[id = [4], qty = [40]]]]\nlog[sync['log']]\nlog[latest['log'].qty]\n").unwrap();
    let (code, stdout, stderr) = run(&base, true, "after.co", None);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "['batch']\n[10, 20]\n1\n2\n3\n[10, 20, 40]\n", "the flushed rows stay, the pending one is lost, the next put takes its position: {stdout}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_put_ended_before_its_flush_is_lost_and_the_flushed_rows_stay() {
    drive_pending(true);
    drive_pending(false);
}

#[test]
fn a_merge_ended_after_placing_its_segment_is_cleaned_by_the_next() {
    drive(true, "placed");
}

#[test]
fn a_merge_ended_after_switching_the_manifest_leaves_stale_batches_the_next_removes() {
    drive(true, "listed");
}

#[test]
fn the_cuts_are_the_same_compiled() {
    drive(false, "placed");
    drive(false, "listed");
}
