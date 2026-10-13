//! `crow setup` and the interpreted fallback (docs/distribution-design.md),
//! against a server of the test's own standing in for the release and for
//! Rust's own servers: a channel manifest naming three archives whose
//! installers write stub tools, and a runtime bundle, each with its hash.
//! With nothing fetched, `crow run` says there is no compiler and
//! interprets; `crow setup` lays the toolchain and the runtime out under a
//! home of the test's own, every file held to its published hash; a file
//! that does not match is named and not kept.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The host, as Rust names it (what `crow` asks the manifest for).
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

/// A gzipped tarball of `dir`'s contents, written by tar.
fn tarball(dir: &Path, out: &Path) {
    let status = Command::new("tar").arg("-czf").arg(out).arg("-C").arg(dir).arg(".").status().unwrap();
    assert!(status.success(), "tar");
}

/// A Rust component archive as the real ones are laid out: a directory of
/// the archive's name holding `install.sh`, which here writes stub tools
/// under `--prefix`.
fn component(dist: &Path, name: &str, tools: &[&str]) -> (String, String) {
    let archive_name = format!("{}.tar.gz", name);
    let stage = dist.join("stage").join(name);
    std::fs::create_dir_all(stage.join(name)).unwrap();
    let mut script = String::from("#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in --prefix=*) prefix=\"${a#--prefix=}\";; esac; done\nmkdir -p \"$prefix/bin\" \"$prefix/lib/rustlib\"\n");
    for tool in tools {
        script.push_str(&format!("printf '#!/bin/sh\\necho stub {} 1.93.1\\n' > \"$prefix/bin/{}\"\nchmod +x \"$prefix/bin/{}\"\n", tool, tool, tool));
    }
    script.push_str("touch \"$prefix/lib/rustlib/installed\"\n");
    std::fs::write(stage.join(name).join("install.sh"), script).unwrap();
    let out = dist.join(&archive_name);
    tarball(&stage, &out);
    (archive_name, sha256(&out))
}

/// A stand-in for both servers: files under `dist`, by path.
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

/// `crow` with a home of the test's own and the stand-in servers: what it
/// said (both streams) and whether it succeeded.
fn crow(home: &Path, port: u16, dir: &Path, args: &[&str]) -> (bool, String) {
    let base = format!("http://127.0.0.1:{}", port);
    let out = Command::new(env!("CARGO_BIN_EXE_crow"))
        .args(args)
        .current_dir(dir)
        .env("CROW_HOME", home)
        .env("CROW_DIST_URL", &base)
        .env("CROW_RUST_DIST", &base)
        .env("CROW_IGNORE_CHECKOUT", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn setup_fetches_and_verifies_the_toolchain_and_the_runtime() {
    let version = env!("CARGO_PKG_VERSION");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("crow_setup_test");
    let _ = std::fs::remove_dir_all(&root);
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    let t = triple();
    // Rust's side: the channel manifest and the three archives it names
    let mut manifest = String::from("manifest-version = \"2\"\n");
    for (comp, tools) in [("rustc", &["rustc"][..]), ("cargo", &["cargo"][..]), ("rust-std", &[][..])] {
        let (name, hash) = component(&dist, &format!("{}-1.93.1-{}", comp, t), tools);
        manifest.push_str(&format!("[pkg.{}.target.{}]\navailable = true\nurl = \"http://x.test/dist/{}\"\nhash = \"{}\"\n", comp, t, name, hash));
    }
    std::fs::write(dist.join("channel-rust-1.93.1.toml"), manifest).unwrap();
    // the release's side: the runtime bundle and its hash
    let stage = dist.join("stage").join("runtime");
    for c in ["crow_runtime", "crow_db", "crow_web"] {
        std::fs::create_dir_all(stage.join(c)).unwrap();
        std::fs::write(stage.join(c).join("Cargo.toml"), format!("[package]\nname = \"{}\"\nversion = \"0.0.0\"\n", c)).unwrap();
    }
    std::fs::create_dir_all(stage.join("vendor")).unwrap();
    std::fs::write(stage.join("Cargo.lock"), "# lock\n").unwrap();
    let bundle = dist.join(format!("crow-runtime-{}.tar.gz", version));
    tarball(&stage, &bundle);
    std::fs::write(dist.join(format!("crow-runtime-{}.tar.gz.sha256", version)), format!("{}  crow-runtime-{}.tar.gz\n", sha256(&bundle), version)).unwrap();
    let port = serve(dist.clone());
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("hello.co"), "log['hello from the alpha']\n").unwrap();

    // nothing fetched yet: the check says so, and `crow run` interprets
    let (ok, said) = crow(&home, port, &project, &["setup", "--check"]);
    assert!(ok && said.contains("the runtime: not here") && said.contains("compiled builds: not yet"), "{}", said);
    let (ok, said) = crow(&home, port, &project, &["run", "hello.co"]);
    assert!(ok && said.contains("no compiler here yet: running interpreted") && said.contains("hello from the alpha"), "{}", said);

    // setup: the toolchain through its installers, the runtime extracted
    let (ok, said) = crow(&home, port, &project, &["setup"]);
    assert!(ok, "{}", said);
    assert!(said.contains("the Rust toolchain 1.93.1 is at") && said.contains("the runtime is at"), "{}", said);
    let toolchain = home.join("toolchain").join("1.93.1");
    assert!(toolchain.join("bin").join("cargo").is_file() && toolchain.join("bin").join("rustc").is_file(), "the stub tools are installed");
    let runtime = home.join("runtime").join(version);
    assert!(runtime.join("crow_runtime").join("Cargo.toml").is_file() && runtime.join("vendor").is_dir() && runtime.join("Cargo.lock").is_file());
    assert!(!home.join("tmp").join("toolchain").exists(), "the archives are not kept");
    let (ok, said) = crow(&home, port, &project, &["setup", "--check"]);
    assert!(ok && said.contains("the Rust toolchain: Crow's own 1.93.1") && said.contains("the runtime: the release's bundle"), "{}", said);

    // a bundle that does not match its published hash is named and not kept
    std::fs::write(dist.join(format!("crow-runtime-{}.tar.gz.sha256", version)), format!("{}  crow-runtime-{}.tar.gz\n", "0".repeat(64), version)).unwrap();
    let home2 = root.join("home2");
    let (ok, said) = crow(&home2, port, &project, &["setup"]);
    assert!(!ok && said.contains("does not match its published hash"), "{}", said);
    assert!(!home2.join("runtime").join(version).exists() && !home2.join("tmp").join(format!("crow-runtime-{}.tar.gz", version)).exists(), "nothing of it is kept");
    let _ = std::fs::remove_dir_all(&root);
}
