//! `crow upgrade` (docs/distribution-design.md), against a server of the
//! test's own standing in for the release: `crow-<version>-<triple>.tar.gz`,
//! whose `crow` is a shell script saying which stand-in it is, and its
//! `.sha256`. The test's binary is copied to `<home>/bin/crow`, where the
//! install line puts it, and that copy is what upgrades: replaced by the
//! release asked for, or left as it was when the release does not match
//! its published hash, does not run here, is not there, or is the version
//! running. Run from anywhere else, the binary replaces nothing and says
//! where it is.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The host, as Rust names it (what `crow` asks the release for).
fn triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "macos" => format!("{}-apple-darwin", arch),
        "linux" => format!("{}-unknown-linux-gnu", arch),
        other => format!("{}-{}", arch, other),
    }
}

fn sha256(path: &Path) -> String {
    let data = std::fs::read(path).unwrap();
    crow_runtime::sys::hex(&crow_runtime::sys::digest(&crow_runtime::sys::Bytes(data), "sha256").unwrap())
}

/// A release of the stand-in, packed as the release workflow packs one: a
/// tarball holding `crow` alone — here a script that prints `says` and
/// exits with `code` — and beside it its hash, or `hash` in its place.
fn release(dist: &Path, version: &str, says: &str, code: i32, hash: Option<&str>) {
    let stage = dist.join("stage").join(version);
    std::fs::create_dir_all(&stage).unwrap();
    let bin = stage.join("crow");
    std::fs::write(&bin, format!("#!/bin/sh\necho '{}'\nexit {}\n", says, code)).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let name = format!("crow-{}-{}.tar.gz", version, triple());
    let tarball = dist.join(&name);
    let status = Command::new("tar").arg("-czf").arg(&tarball).arg("-C").arg(&stage).arg("crow").status().unwrap();
    assert!(status.success(), "tar");
    let hash = hash.map(str::to_string).unwrap_or_else(|| sha256(&tarball));
    std::fs::write(dist.join(format!("{}.sha256", name)), format!("{}  {}\n", hash, name)).unwrap();
}

/// A stand-in for the release's server: files under `dist`, by path.
fn serve(dist: PathBuf) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut head = [0u8; 4096];
            let n = s.read(&mut head).unwrap_or(0);
            let request = String::from_utf8_lossy(&head[..n]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
            match std::fs::read(dist.join(&path)) {
                Ok(body) => {
                    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes());
                    let _ = s.write_all(&body);
                }
                Err(_) => {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                }
            }
        }
    });
    port
}

/// `bin` run with the test's home and the stand-in release: what it said
/// (both streams) and whether it succeeded.
fn crow(bin: &Path, home: &Path, port: u16, args: &[&str]) -> (bool, String) {
    let out = Command::new(bin)
        .args(args)
        .current_dir(home.parent().unwrap())
        .env("CROW_HOME", home)
        .env("CROW_DIST_URL", format!("http://127.0.0.1:{}", port))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// What a binary prints, run with nothing.
fn ran(bin: &Path) -> String {
    String::from_utf8_lossy(&Command::new(bin).stdin(Stdio::null()).output().unwrap().stdout).to_string()
}

/// The test's binary put where the install line puts crow, a file of its own.
fn install(line: &Path) {
    std::fs::create_dir_all(line.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(line);
    std::fs::copy(env!("CARGO_BIN_EXE_crow"), line).unwrap();
}

/// Which file a path is, and as what it was last written: a binary left as
/// it was keeps all four (an upgrade never writes into the old file; it
/// renames a new one over it).
fn fingerprint(path: &Path) -> (u64, u64, u64, i64, i64) {
    let m = std::fs::metadata(path).unwrap();
    (m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec())
}

/// Nothing an upgrade fetched or half-placed is left in the home.
fn nothing_left(home: &Path) {
    let tmp: Vec<_> = std::fs::read_dir(home.join("tmp")).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
    assert!(tmp.is_empty(), "nothing fetched is kept: {:?}", tmp);
    assert!(!home.join("bin").join("crow.part").exists(), "nothing half-placed is kept");
}

#[test]
fn upgrade_replaces_the_installed_binary_with_a_release_held_to_its_hash() {
    let running = env!("CARGO_PKG_VERSION");
    let t = triple();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("crow_upgrade_test");
    let _ = std::fs::remove_dir_all(&root);
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    release(&dist, "9.9.9", "the stand-in crow 9.9.9", 0, None);
    release(&dist, "0.0.1", "the stand-in crow 0.0.1", 0, None);
    release(&dist, "9.9.8", "the stand-in crow 9.9.8", 0, Some(&"0".repeat(64)));
    release(&dist, "9.9.7", "the stand-in crow 9.9.7, which cannot run here", 1, None);
    let port = serve(dist.clone());
    let base = format!("http://127.0.0.1:{}", port);
    let home = root.join("home");
    let line = home.join("bin").join("crow");
    install(&line);
    let installed = fingerprint(&line);

    // the version running is current: nothing is fetched, nothing changes
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--check", "--version", running]);
    assert!(ok && said.contains(&format!("crow {} is current", running)), "{}", said);
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", running]);
    assert!(ok && said.contains(&format!("crow {} is current", running)), "{}", said);
    assert_eq!(fingerprint(&line), installed, "the version running replaces nothing");

    // the latest release is GitHub's to name: with the files elsewhere,
    // --version names it, as the install line asks
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--check"]);
    assert!(!ok && said.contains("--version is needed with CROW_DIST_URL"), "{}", said);

    // --check says what is newer or older and changes nothing
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--check", "--version", "9.9.9"]);
    assert!(ok && said.contains("9.9.9 is newer") && said.contains("`crow upgrade --version 9.9.9` installs it"), "{}", said);
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--check", "--version", "0.0.1"]);
    assert!(ok && said.contains("0.0.1 is older") && said.contains("a downgrade"), "{}", said);
    assert_eq!(fingerprint(&line), installed, "--check changes nothing");

    // run from its own place — a checkout's target/ — the binary replaces
    // nothing, says where it is, and how a checkout's build upgrades
    let own = Path::new(env!("CARGO_BIN_EXE_crow"));
    let own_before = fingerprint(own);
    let (ok, said) = crow(own, &home, port, &["upgrade", "--version", "9.9.9"]);
    let at = own.canonicalize().unwrap().display().to_string();
    assert!(!ok && said.contains(&format!("this crow is {}", at)) && said.contains("nothing was changed") && said.contains("./install.sh --dev"), "{}", said);
    assert_eq!(fingerprint(own), own_before, "the binary run from elsewhere is left as it is");
    assert_eq!(fingerprint(&line), installed, "and so is the install line's");
    // its check names the release, and says the same of where it is
    let (ok, said) = crow(own, &home, port, &["upgrade", "--check", "--version", "9.9.9"]);
    assert!(ok && said.contains("9.9.9 is newer") && said.contains(&format!("this crow is {}", at)), "{}", said);

    // a release that does not match its published hash is named, with its
    // address and both hashes, and nothing of it is kept
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", "9.9.8"]);
    let expected = format!("{}/crow-9.9.8-{}.tar.gz does not match its published hash: expected {}, saw ", base, t, "0".repeat(64));
    assert!(!ok && said.contains(&expected), "{}", said);
    assert_eq!(fingerprint(&line), installed, "a release that does not match replaces nothing");
    nothing_left(&home);

    // a release whose crow does not run here replaces nothing
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", "9.9.7"]);
    assert!(!ok && said.contains("does not run here") && said.contains("nothing was replaced"), "{}", said);
    assert_eq!(fingerprint(&line), installed, "a binary that does not run replaces nothing");
    nothing_left(&home);

    // a release that is not there is named by its address
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", "9.9.6"]);
    assert!(!ok && said.contains(&format!("cannot fetch {}/crow-9.9.6-{}.tar.gz.sha256", base, t)), "{}", said);
    assert_eq!(fingerprint(&line), installed);
    nothing_left(&home);

    // a downgrade, said as one: the release asked for is the binary now
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", "0.0.1"]);
    assert!(ok && said.contains(&format!("downgraded {} from {} to 0.0.1, verified", line.display(), running)), "{}", said);
    assert!(ran(&line).contains("the stand-in crow 0.0.1"));
    nothing_left(&home);

    // the upgrade: a fresh copy of the test's binary replaced by the
    // release, verified; the checkout's mark the install line removes goes
    // with it, and where the new version's compiler comes from is said
    install(&line);
    std::fs::write(home.join("dev"), "/a/checkout\n").unwrap();
    let (ok, said) = crow(&line, &home, port, &["upgrade", "--version", "9.9.9"]);
    assert!(ok && said.contains(&format!("upgraded {} from {} to 9.9.9, verified", line.display(), running)), "{}", said);
    assert!(said.contains("crow 9.9.9's Rust toolchain and runtime are fetched") && said.contains("`crow setup`"), "{}", said);
    assert!(!home.join("dev").exists(), "the checkout's mark is gone, as the install line removes it");
    assert!(ran(&line).contains("the stand-in crow 9.9.9"), "the binary is the release's now");
    nothing_left(&home);
    let _ = std::fs::remove_dir_all(&root);
}
