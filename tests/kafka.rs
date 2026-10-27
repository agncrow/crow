//! The Kafka door in both modes (spec, section 13: kafka): a Crow program
//! serving `serve[root, port]` is driven by the pure-Python client — a
//! producer sends three records, a consumer group reads them from the
//! start and commits, a second consumer of the group takes up after the
//! commit — interpreted and compiled. The client is looked for as
//! `CROW_KAFKA_PYTHON` (a Python with kafka-python installed) or a
//! `python3` on the PATH that imports it; without one the test is skipped,
//! as the Postgres tests skip without `initdb`.
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn python_with_kafka() -> Option<String> {
    let candidates: Vec<String> = std::env::var("CROW_KAFKA_PYTHON").ok().into_iter().chain(["python3".to_string()]).collect();
    candidates.into_iter().find(|p| Command::new(p).args(["-c", "import kafka"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false))
}

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

fn drive(interpreted: bool, script: &str) {
    let Some(python) = python_with_kafka() else {
        eprintln!("skipped: no Python with kafka-python (set CROW_KAFKA_PYTHON)");
        return;
    };
    // a directory per scenario and mode: two tests of one process had
    // shared one, and the second's clearing of it ended the first's door
    let dir = std::env::temp_dir().join(format!("crow-kafka-{}-{}-{}", if interpreted { "i" } else { "c" }, script.trim_end_matches(".py"), std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let port = free_port();
    std::fs::write(dir.join("door.co"), format!("use kafka\n\nserved = serve['magpie', {port}]\nlog[served]\n")).unwrap();
    let crow = env!("CARGO_BIN_EXE_crow");
    let mut cmd = Command::new(crow);
    cmd.arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    // the door in a process group of its own: `crow run` builds and runs
    // the program as a child, and ending the group ends both at once — a
    // door left running had written half a coverage profile at the test's
    // end, which no profile could be merged with
    let mut door = cmd.arg("door.co").current_dir(&dir).process_group(0).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("the door starts");
    assert!(wait_for(port, if interpreted { 60 } else { 600 }), "the door did not listen on {port}");
    let driver = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/kafka").join(script);
    let out = Command::new(&python).arg(&driver).arg(format!("127.0.0.1:{port}")).output().expect("the driver runs");
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", door.id())]).status();
    let _ = door.kill();
    let _ = door.wait();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the client against the {} door:\n{stdout}\n{stderr}", if interpreted { "interpreted" } else { "compiled" });
    assert!(stdout.contains("OK"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_python_client_produces_and_consumes_through_the_interpreted_door() {
    drive(true, "driver.py");
}

#[test]
fn a_python_client_produces_and_consumes_through_the_compiled_door() {
    drive(false, "driver.py");
}

/// A gzip producer, and two consumers of one group that read every record
/// exactly once between them (the second joins while the first runs)
#[test]
fn two_consumers_of_a_group_and_a_gzip_producer_through_the_interpreted_door() {
    drive(true, "driver2.py");
}

#[test]
fn two_consumers_of_a_group_and_a_gzip_producer_through_the_compiled_door() {
    drive(false, "driver2.py");
}
