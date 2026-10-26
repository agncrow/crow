//! A log shared and followed across two processes (spec, section 13:
//! lake: `share`, `follow`): a leader program writes two batches and
//! serves its log; a follower program in another directory fetches the
//! files, reads the table the log holds and its head, and a second pass
//! fetches nothing. The leader runs interpreted and compiled; the
//! follower in both modes against each, with one output expected.
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_for(port: u16, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

fn run_in(dir: &std::path::Path, interpreted: bool, file: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crow"));
    cmd.arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    cmd.arg(file).current_dir(dir);
    cmd
}

fn drive(leader_interpreted: bool) {
    let tag = if leader_interpreted { "i" } else { "c" };
    let base = std::env::temp_dir().join(format!("crow-share-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let leader = base.join("leader");
    std::fs::create_dir_all(&leader).unwrap();
    let port = free_port();
    std::fs::write(
        leader.join("share.co"),
        format!("use lake\nuse frame\n\nfirst = batch['log', frame.new[id = [1, 2], qty = [10, 20]], by = ['id']]\nsecond = batch['log', frame.new[id = [2, 3], qty = [25, 30]], by = ['id']]\nserved = share['log', {port}]\nlog[served]\n"),
    )
    .unwrap();
    // the leader in a process group of its own, ended with its build child
    let mut door = run_in(&leader, leader_interpreted, "share.co").process_group(0).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("the leader starts");
    assert!(wait_for(port, if leader_interpreted { 60 } else { 600 }), "the leader did not listen on {port}");
    for follower_interpreted in [true, false] {
        let follower = base.join(format!("follower-{}", if follower_interpreted { "i" } else { "c" }));
        std::fs::create_dir_all(&follower).unwrap();
        std::fs::write(
            follower.join("follow.co"),
            format!("use lake\nuse frame\n\ngot = follow['copy', '127.0.0.1', {port}]\nlog[got]\nlog[latest['copy', ['id']].qty]\nlog[head['copy']]\nagain = follow['copy', '127.0.0.1', {port}]\nlog[again]\n"),
        )
        .unwrap();
        let out = run_in(&follower, follower_interpreted, "follow.co").output().expect("the follower runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stdout, "2\n[10, 25, 30]\n3\n0\n", "follower (interpreted: {follower_interpreted}) against leader (interpreted: {leader_interpreted}): {stderr}");
    }
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", door.id())]).status();
    let _ = door.kill();
    let _ = door.wait();
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_log_shared_by_the_interpreted_leader_is_followed_in_both_modes() {
    drive(true);
}

#[test]
fn a_log_shared_by_the_compiled_leader_is_followed_in_both_modes() {
    drive(false);
}
