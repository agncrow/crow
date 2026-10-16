//! Where Crow lives on a machine (docs/distribution-design.md): its home
//! (`$CROW_HOME`, else `~/.crow`); the runtime crates a compiled program
//! links — a developer's checkout, the checkout this binary was built
//! from, the release's bundle for this version, or an install's source
//! snapshot; and the Rust toolchain, Crow's own pinned copy when fetched,
//! the machine's otherwise. `crow setup` fetches the toolchain from Rust's
//! own servers and the runtime bundle from the release, each file held to
//! its published hash, so the first compiled build on a fresh machine
//! needs nothing but the binary and a linker. The interpreter needs none
//! of it: `crow run` with no compiler present says so and interprets.
//! `crow upgrade` replaces the binary the install line put in the home with
//! another release, held to its published hash as the install line holds it.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The Rust version every Crow release is held to: the toolchain `crow
/// setup` fetches, and the one the runtime bundle was vendored with.
pub const RUST_VERSION: &str = "1.93.1";

/// Where Rust's own releases are served from; `CROW_RUST_DIST` overrides it
/// (a test's local server).
const RUST_DIST: &str = "https://static.rust-lang.org/dist";

/// Where a Crow release's files are served from; `CROW_DIST_URL` overrides
/// it (a test's local server, or a mirror), and then names no version of
/// its own: whoever sets it names the release (the install line's and
/// `crow upgrade`'s `--version`).
fn release_url(version: &str) -> String {
    std::env::var("CROW_DIST_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://github.com/agncrow/crow/releases/download/v{}", version))
}

/// This version's release.
fn dist_url() -> String {
    release_url(env!("CARGO_PKG_VERSION"))
}

fn rust_dist() -> String {
    std::env::var("CROW_RUST_DIST").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| RUST_DIST.to_string())
}

/// Crow's home: `$CROW_HOME`, else `~/.crow`.
pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("CROW_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".crow")).unwrap_or_else(|| PathBuf::from(".crow"))
}

/// The host, as Rust names its targets.
pub fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "macos" => format!("{}-apple-darwin", arch),
        "linux" => format!("{}-unknown-linux-gnu", arch),
        "windows" => format!("{}-pc-windows-msvc", arch),
        other => format!("{}-{}", arch, other),
    }
}

/// Where the runtime crates a compiled program links come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Runtime {
    /// A developer's checkout, named by `home/dev` (`install.sh --dev`).
    Dev(PathBuf),
    /// The checkout this binary was built from, where it still exists —
    /// the gate's `target/release/crow`, or an install built from its
    /// source snapshot.
    Built(PathBuf),
    /// The release's bundle for this version: the crates' source, their
    /// dependencies vendored, the lockfile (`crow setup`).
    Bundle(PathBuf),
    /// An install's source snapshot (`install.sh` without `--dev`).
    Snapshot(PathBuf),
}

impl Runtime {
    pub fn dir(&self) -> &Path {
        match self {
            Runtime::Dev(p) | Runtime::Built(p) | Runtime::Bundle(p) | Runtime::Snapshot(p) => p,
        }
    }

    /// Whether the dependencies are vendored beside the crates, so a build
    /// is offline and reads no registry.
    pub fn vendored(&self) -> bool {
        matches!(self, Runtime::Bundle(_)) && self.dir().join("vendor").is_dir()
    }

    pub fn what(&self) -> &'static str {
        match self {
            Runtime::Dev(_) => "a checkout (home/dev)",
            Runtime::Built(_) => "the checkout this binary was built from",
            Runtime::Bundle(_) => "the release's bundle",
            Runtime::Snapshot(_) => "the install's source snapshot",
        }
    }
}

fn holds_runtime(p: &Path) -> bool {
    p.join("crow_runtime").join("Cargo.toml").is_file()
}

/// The runtime under `home`, with `built` the checkout this binary was
/// built from: a developer's checkout first, then that checkout where it
/// still exists, then the bundle for this version, then a snapshot.
pub fn runtime_in(home: &Path, built: &Path, version: &str) -> Option<Runtime> {
    if let Ok(dev) = std::fs::read_to_string(home.join("dev")) {
        let p = PathBuf::from(dev.trim());
        if holds_runtime(&p) {
            return Some(Runtime::Dev(p));
        }
    }
    // (`CROW_IGNORE_CHECKOUT` makes a binary built from a checkout behave as
    // a release's: the setup test, and a release-shaped run from a dev build)
    if holds_runtime(built) && std::env::var_os("CROW_IGNORE_CHECKOUT").is_none() {
        return Some(Runtime::Built(built.to_path_buf()));
    }
    let bundle = home.join("runtime").join(version);
    if holds_runtime(&bundle) {
        return Some(Runtime::Bundle(bundle));
    }
    let snapshot = home.join("src");
    if holds_runtime(&snapshot) {
        return Some(Runtime::Snapshot(snapshot));
    }
    None
}

pub fn runtime() -> Option<Runtime> {
    runtime_in(&home(), Path::new(env!("CARGO_MANIFEST_DIR")), env!("CARGO_PKG_VERSION"))
}

/// The directories that may hold the standard modules' source
/// (`src/modules`), for the door `use sys` opens to them alone.
pub fn module_source_dirs() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("modules")];
    if let Some(rt) = runtime() {
        out.push(rt.dir().join("src").join("modules"));
    }
    out
}

/// Crow's own toolchain for this Rust version, when fetched.
pub fn toolchain_dir_in(home: &Path) -> PathBuf {
    home.join("toolchain").join(RUST_VERSION)
}

fn own_toolchain(home: &Path) -> Option<PathBuf> {
    let dir = toolchain_dir_in(home);
    dir.join("bin").join("cargo").is_file().then_some(dir)
}

/// Whether a program of the name is on PATH.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

/// cargo to build with: Crow's pinned copy when fetched — its rustc named
/// beside it, its own cargo home, no rustup in the way — else the machine's.
pub fn cargo() -> Command {
    let home = home();
    match own_toolchain(&home) {
        Some(dir) => {
            let bin = dir.join("bin");
            let mut c = Command::new(bin.join("cargo"));
            c.env("RUSTC", bin.join("rustc"));
            c.env("CARGO_HOME", home.join("cargo"));
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![bin];
            paths.extend(std::env::split_paths(&path));
            if let Ok(joined) = std::env::join_paths(paths) {
                c.env("PATH", joined);
            }
            c.env_remove("RUSTUP_TOOLCHAIN");
            c
        }
        None => Command::new("cargo"),
    }
}

/// Whether a cargo is at hand, Crow's own or the machine's.
pub fn has_cargo() -> bool {
    own_toolchain(&home()).is_some() || on_path("cargo")
}

/// The one thing asked of the machine for a compiled build: a linker.
pub fn linker() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let ok = Command::new("xcode-select").arg("-p").output().map(|o| o.status.success()).unwrap_or(false);
        if ok {
            return Ok(());
        }
        return Err("no linker: run `xcode-select --install` (the Xcode command line tools), then try again".to_string());
    }
    if on_path("cc") || on_path("gcc") || on_path("clang") {
        return Ok(());
    }
    Err("no linker: install build-essential (Debian, Ubuntu) or gcc (Fedora, Arch), then try again".to_string())
}

/// What a compiled build needs, each named when missing.
pub fn missing_for_build() -> Vec<String> {
    let mut out = Vec::new();
    if runtime().is_none() {
        out.push("the runtime".to_string());
    }
    if !has_cargo() {
        out.push("the Rust toolchain".to_string());
    }
    out
}

/// Whether a program can be compiled here as things stand.
pub fn can_compile() -> bool {
    missing_for_build().is_empty() && linker().is_ok()
}

/// What `crow setup --check` says: each piece, present or not.
pub fn status() -> String {
    let home = home();
    let mut lines = vec![format!("crow {} for {}, home {}", env!("CARGO_PKG_VERSION"), target_triple(), home.display())];
    lines.push("  the interpreter: in this binary — crow run -i, crow test, crow check, crow dev need nothing else".to_string());
    match runtime() {
        Some(rt) => lines.push(format!("  the runtime: {} at {}", rt.what(), rt.dir().display())),
        None => lines.push(format!("  the runtime: not here — crow setup fetches crow-runtime-{} from the release", env!("CARGO_PKG_VERSION"))),
    }
    match own_toolchain(&home) {
        Some(dir) => lines.push(format!("  the Rust toolchain: Crow's own {} at {}", RUST_VERSION, dir.display())),
        None if on_path("cargo") => lines.push("  the Rust toolchain: the machine's cargo (crow setup fetches Crow's own, pinned)".to_string()),
        None => lines.push(format!("  the Rust toolchain: not here — crow setup fetches {} from static.rust-lang.org", RUST_VERSION)),
    }
    match linker() {
        Ok(()) => lines.push("  the linker: present".to_string()),
        Err(e) => lines.push(format!("  the linker: {}", e)),
    }
    lines.push(if can_compile() { "  compiled builds: ready".to_string() } else { "  compiled builds: not yet — crow run interprets until then".to_string() });
    lines.join("\n")
}

/// `crow setup`: the toolchain and the runtime fetched where missing,
/// each file verified; `check` only reports.
pub fn setup(check: bool) -> Result<(), String> {
    println!("{}", status());
    if check {
        return Ok(());
    }
    let home = home();
    if own_toolchain(&home).is_none() {
        fetch_toolchain(&home)?;
    }
    if !matches!(runtime(), Some(Runtime::Dev(_) | Runtime::Built(_) | Runtime::Bundle(_))) {
        fetch_runtime(&home)?;
    }
    if let Err(e) = linker() {
        println!("(o.o) {}", e);
    }
    println!("(o.o) done — compiled builds {}", if can_compile() { "are ready" } else { "wait on the linker" });
    Ok(())
}

/// Before a compiled build: what is missing fetched, saying so, or the
/// reason it cannot be.
pub fn ensure_for_build() -> Result<(), String> {
    let missing = missing_for_build();
    if !missing.is_empty() {
        println!("(o.o) a compiled build needs {}: fetching (once)", missing.join(" and "));
        let home = home();
        if !has_cargo() {
            fetch_toolchain(&home)?;
        }
        if runtime().is_none() {
            fetch_runtime(&home)?;
        }
    }
    linker()
}

// ── fetching ───────────────────────────────────────────────────────────

/// A file fetched to `to` with curl (present on every macOS and nearly
/// every Linux; the install line needs it too), through a part file so
/// nothing half-fetched is kept.
fn fetch(url: &str, to: &Path) -> Result<(), String> {
    if !on_path("curl") {
        return Err(format!("cannot fetch {}: curl is not on this machine", url));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
    }
    let part = to.with_extension("part");
    let status = Command::new("curl")
        .args(["-fL", "--retry", "3", "--progress-bar", "-o"])
        .arg(&part)
        .arg(url)
        .status()
        .map_err(|e| format!("cannot run curl: {}", e))?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("cannot fetch {}: curl exited with {}", url, status));
    }
    std::fs::rename(&part, to).map_err(|e| format!("cannot keep {}: {}", to.display(), e))
}

/// A small text fetched whole — a hash, a manifest, GitHub's answer naming
/// the latest release — read from curl's output, so nothing is written.
fn fetch_text(url: &str) -> Result<String, String> {
    if !on_path("curl") {
        return Err(format!("cannot fetch {}: curl is not on this machine", url));
    }
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "3"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run curl: {}", e))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let said = if said.is_empty() { String::new() } else { format!(" ({})", said) };
        return Err(format!("cannot fetch {}: curl exited with {}{}", url, out.status, said));
    }
    String::from_utf8(out.stdout).map_err(|_| format!("cannot read {}: it is not text", url))
}

/// The hash a release publishes beside a file, as `<file>.sha256`: the
/// SHA-256 in hex, then the file's name.
fn published_hash(url: &str) -> Result<String, String> {
    let text = fetch_text(url)?;
    let hash = text.split_whitespace().next().unwrap_or_default().to_lowercase();
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hash)
    } else {
        Err(format!("{} holds no SHA-256 hash", url))
    }
}

/// The SHA-256 of a file, as lowercase hex.
pub fn sha256_of(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let digest = crow_runtime::sys::digest(&crow_runtime::sys::Bytes(data), "sha256")?;
    Ok(crow_runtime::sys::hex(&digest))
}

/// A file fetched and held to its published hash; a mismatch is removed
/// and named.
fn fetch_verified(url: &str, to: &Path, expected: &str) -> Result<(), String> {
    fetch(url, to)?;
    let seen = sha256_of(to)?;
    if seen != expected.trim().to_lowercase() {
        let _ = std::fs::remove_file(to);
        return Err(format!("{} does not match its published hash: expected {}, saw {}", url, expected.trim(), seen));
    }
    Ok(())
}

/// A tarball extracted into `into` (made first).
fn extract(tarball: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| format!("cannot create {}: {}", into.display(), e))?;
    let status = Command::new("tar").arg("-xzf").arg(tarball).arg("-C").arg(into).status().map_err(|e| format!("cannot run tar: {}", e))?;
    if !status.success() {
        return Err(format!("cannot extract {}: tar exited with {}", tarball.display(), status));
    }
    Ok(())
}

/// A component's archive and hash in Rust's channel manifest: the section
/// `[pkg.<component>.target.<triple>]`, its `url` and `hash` (the gzip
/// form, which every tar reads).
pub fn component_in(manifest: &str, component: &str, triple: &str) -> Result<(String, String), String> {
    let header = format!("[pkg.{}.target.{}]", component, triple);
    let mut in_section = false;
    let (mut url, mut hash, mut available) = (None, None, true);
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_section {
                break;
            }
            in_section = line == header;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "url" => url = Some(v.to_string()),
            "hash" => hash = Some(v.to_string()),
            "available" => available = v == "true",
            _ => {}
        }
    }
    if !available {
        return Err(format!("Rust {} has no {} for {}", RUST_VERSION, component, triple));
    }
    match (url, hash) {
        (Some(u), Some(h)) => Ok((u, h)),
        _ => Err(format!("Rust's manifest for {} names no {} for {}", RUST_VERSION, component, triple)),
    }
}

/// Rust's toolchain for this host, pinned: rustc, cargo and the standard
/// library, each archive held to the hash Rust's channel manifest
/// publishes, installed by its own installer into Crow's home.
fn fetch_toolchain(home: &Path) -> Result<(), String> {
    let triple = target_triple();
    let prefix = toolchain_dir_in(home);
    println!("(o.o) fetching the Rust toolchain {} for {} from {} (about 130 MB)", RUST_VERSION, triple, rust_dist());
    let manifest = fetch_text(&format!("{}/channel-rust-{}.toml", rust_dist(), RUST_VERSION))?;
    let tmp = home.join("tmp").join("toolchain");
    let _ = std::fs::remove_dir_all(&tmp);
    for component in ["rustc", "cargo", "rust-std"] {
        let (url, hash) = component_in(&manifest, component, &triple)?;
        let url = match std::env::var("CROW_RUST_DIST") {
            // (a test's local server serves the archives at the manifest's names)
            Ok(local) if !local.is_empty() => format!("{}/{}", local, url.rsplit('/').next().unwrap_or(&url)),
            _ => url,
        };
        let name = url.rsplit('/').next().unwrap_or("component.tar.gz").to_string();
        let archive = tmp.join(&name);
        println!("(o.o)   {}", name);
        fetch_verified(&url, &archive, &hash)?;
        let unpacked = tmp.join(name.trim_end_matches(".tar.gz"));
        extract(&archive, &tmp)?;
        let installer = unpacked.join("install.sh");
        let status = Command::new("sh")
            .arg(&installer)
            .arg(format!("--prefix={}", prefix.display()))
            .arg("--disable-ldconfig")
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("cannot run {}: {}", installer.display(), e))?;
        if !status.success() {
            return Err(format!("{} did not install: its installer exited with {}", name, status));
        }
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&unpacked);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if !prefix.join("bin").join("cargo").is_file() {
        return Err(format!("the toolchain was installed but {} holds no cargo", prefix.display()));
    }
    println!("(o.o) the Rust toolchain {} is at {}", RUST_VERSION, prefix.display());
    Ok(())
}

/// The release's runtime bundle for this version: the crates, their
/// dependencies vendored, the lockfile — held to the hash published
/// beside it.
fn fetch_runtime(home: &Path) -> Result<(), String> {
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("crow-runtime-{}.tar.gz", version);
    let base = dist_url();
    println!("(o.o) fetching the crow {} runtime from {} (about 43 MB)", version, base);
    let hash = published_hash(&format!("{}/{}.sha256", base, name))?;
    let archive = home.join("tmp").join(&name);
    fetch_verified(&format!("{}/{}", base, name), &archive, &hash)?;
    let into = home.join("runtime").join(version);
    let _ = std::fs::remove_dir_all(&into);
    extract(&archive, &into)?;
    let _ = std::fs::remove_file(&archive);
    if !holds_runtime(&into) {
        return Err(format!("{} holds no crow_runtime after extraction", into.display()));
    }
    println!("(o.o) the runtime is at {}", into.display());
    Ok(())
}

/// What a generated project's `.cargo/config.toml` says when the runtime
/// is a bundle: the registry replaced by the vendored sources, offline.
pub fn vendored_config(vendor: &Path) -> String {
    format!(
        "# written by crow: the dependencies the release vendored, no registry\n[source.crates-io]\nreplace-with = \"crow-vendored\"\n\n[source.crow-vendored]\ndirectory = \"{}\"\n\n[net]\noffline = true\n",
        vendor.display()
    )
}

// ── upgrading ──────────────────────────────────────────────────────────

/// The install line (README.md, install.sh): what puts the latest release
/// at `<home>/bin/crow`, and so what upgrades a crow installed elsewhere.
const INSTALL_LINE: &str = "curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash";

/// Where GitHub names the latest release (its `tag_name`), where the
/// install line reads it.
const LATEST_RELEASE: &str = "https://api.github.com/repos/agncrow/crow/releases/latest";

/// A release's version as a person writes it — `0.7.0`, or its tag
/// `v0.7.0` — or why it is not one.
pub fn release_version(text: &str) -> Result<String, String> {
    let v = text.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    if v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
        return Ok(v.to_string());
    }
    Err(format!("'{}' is not a release: --version takes one, e.g. --version {}", text, env!("CARGO_PKG_VERSION")))
}

/// Two releases in order, as semantic versions are ordered: the numbers of
/// `major.minor.patch` one by one, and a pre-release (`1.0.0-rc.1`) before
/// its release, its parts compared numbers by value, words by text.
pub fn version_order(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.split('+').next().unwrap_or(v);
        let (numbers, pre) = match v.split_once('-') {
            Some((n, p)) => (n, Some(p)),
            None => (v, None),
        };
        (numbers.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
    }
    let ((na, pa), (nb, pb)) = (parts(a), parts(b));
    for i in 0..na.len().max(nb.len()) {
        let o = na.get(i).unwrap_or(&0).cmp(nb.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(p), Some(q)) => {
            let (mut p, mut q) = (p.split('.'), q.split('.'));
            loop {
                let o = match (p.next(), q.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                        (Ok(m), Ok(n)) => m.cmp(&n),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => x.cmp(y),
                    },
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// The version GitHub's answer for the latest release names: its tag, the
/// `v` dropped.
pub fn latest_in(answer: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(answer).map_err(|_| "api.github.com did not answer with a release".to_string())?;
    let tag = json.get("tag_name").and_then(|t| t.as_str()).ok_or_else(|| "api.github.com names no latest release".to_string())?;
    release_version(tag).map_err(|_| format!("the latest release's tag, '{}', names no version", tag))
}

/// The latest release's version, read where the install line reads it.
/// With `CROW_DIST_URL` set the files come from elsewhere, and `--version`
/// names the release, as the install line asks.
fn latest_release() -> Result<String, String> {
    if std::env::var("CROW_DIST_URL").is_ok_and(|u| !u.is_empty()) {
        return Err("--version is needed with CROW_DIST_URL, as the install line needs it: the latest release is GitHub's to name".to_string());
    }
    let answer = fetch_text(LATEST_RELEASE).map_err(|e| format!("cannot read the latest release from api.github.com — pass --version X ({})", e))?;
    latest_in(&answer)
}

/// `crow upgrade`'s arguments: whether it only checks, and the release
/// `--version` names (none: the latest).
pub fn upgrade_args(args: &[String]) -> Result<(bool, Option<String>), String> {
    let (mut check, mut version) = (false, None);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--check" => check = true,
            "--version" => {
                let v = rest.next().ok_or_else(|| format!("--version takes a release, e.g. --version {}", env!("CARGO_PKG_VERSION")))?;
                version = Some(release_version(v)?);
            }
            other => match other.strip_prefix("--version=") {
                Some(v) => version = Some(release_version(v)?),
                None => return Err(format!("unknown argument '{}' for crow upgrade (crow upgrade [--check] [--version X])", other)),
            },
        }
    }
    Ok((check, version))
}

/// How the crow that runs was installed, which decides what upgrades it.
#[derive(Debug, Clone, PartialEq)]
pub enum Install {
    /// `<home>/bin/crow`, a file of its own: the install line put it there,
    /// and `crow upgrade` replaces it.
    Line,
    /// A build of a checkout that is still there — its `target/`, or an
    /// `install.sh --dev` install.
    Checkout(PathBuf),
    /// Built from the source snapshot `<home>/src`: `install.sh --source`.
    Snapshot,
    /// Anywhere else: a copy, or a release's binary moved.
    Elsewhere,
}

/// How the crow at `exe` was installed, with `home` Crow's home and
/// `built` the directory the binary was built from.
pub fn install_of(exe: &Path, home: &Path, built: &Path) -> Install {
    let line = home.join("bin").join("crow");
    let same = |a: &Path, b: &Path| matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y);
    // (a link in the line's place points at a binary kept elsewhere: the
    // link is not the install line's, and neither is what it points at)
    if std::fs::symlink_metadata(&line).is_ok_and(|m| m.file_type().is_file()) && same(exe, &line) {
        return Install::Line;
    }
    match built.canonicalize() {
        Ok(b) if same(&b, &home.join("src")) => Install::Snapshot,
        Ok(b) if b.join("Cargo.toml").is_file() => Install::Checkout(b),
        _ => Install::Elsewhere,
    }
}

/// What `crow upgrade` says of a crow it does not replace: where it is,
/// what it is, and how that install upgrades.
fn elsewhere(exe: &Path, home: &Path, install: &Install, refused: bool) -> String {
    let line = home.join("bin").join("crow");
    let (what, how) = match install {
        Install::Checkout(dir) => (
            format!(", a build of the checkout at {}", dir.display()),
            format!("`git pull` in {}, then `./install.sh --dev` or `./install.sh --source` again", dir.display()),
        ),
        Install::Snapshot => (
            format!(", built from the source snapshot {} (`./install.sh --source`)", home.join("src").display()),
            format!("`git pull` in your checkout and run `./install.sh --source` there again — or move to the prebuilt binary with `{}`, then remove {}", INSTALL_LINE, exe.display()),
        ),
        Install::Line | Install::Elsewhere => (
            String::new(),
            format!("`{}` puts the latest release at {}; then remove {}, which your PATH may find first", INSTALL_LINE, line.display(), exe.display()),
        ),
    };
    let changed = if refused { ": nothing was changed" } else { "" };
    format!("this crow is {}{}, and `crow upgrade` replaces only {}, the binary the install line puts there{}. To upgrade this one, {}", exe.display(), what, line.display(), changed, how)
}

/// `crow upgrade`: the binary the install line put at `<home>/bin/crow`
/// replaced by the latest release, or by the one `--version` names (a
/// downgrade too), held to its published hash; `--check` says what there is
/// and changes nothing. A crow installed any other way is left as it is,
/// and told how that install upgrades.
pub fn upgrade(args: &[String]) -> Result<(), String> {
    let (check, asked) = upgrade_args(args)?;
    let running = env!("CARGO_PKG_VERSION");
    let version = match &asked {
        Some(v) => v.clone(),
        None => latest_release()?,
    };
    let order = version_order(&version, running);
    if order == Ordering::Equal {
        println!("(o.o) crow {} is current: it is {}", running, if asked.is_some() { "the release asked for" } else { "the latest release" });
        return Ok(());
    }
    if order == Ordering::Less && asked.is_none() {
        println!("(o.o) crow {} is newer than the latest release, {}: nothing to upgrade — `crow upgrade --version {}` installs that one", running, version, version);
        return Ok(());
    }
    let home = home();
    let exe = std::env::current_exe().map_err(|e| format!("cannot tell where this crow is: {}", e))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let install = install_of(&exe, &home, Path::new(env!("CARGO_MANIFEST_DIR")));
    if check {
        let command = if asked.is_some() { format!("crow upgrade --version {}", version) } else { "crow upgrade".to_string() };
        match (order, asked.is_some()) {
            (Ordering::Greater, false) => println!("(o.o) crow {} is running; the latest release is {} — `{}` installs it", running, version, command),
            (Ordering::Greater, true) => println!("(o.o) crow {} is running; {} is newer — `{}` installs it", running, version, command),
            _ => println!("(o.o) crow {} is running; {} is older — `{}` installs it, a downgrade", running, version, command),
        }
        if install != Install::Line {
            println!("(o.o) {}", elsewhere(&exe, &home, &install, false));
        }
        return Ok(());
    }
    if install != Install::Line {
        return Err(elsewhere(&exe, &home, &install, true));
    }
    let line = home.join("bin").join("crow");
    install_release(&home, &version, &line)?;
    println!("(o.o) {} {} from {} to {}, verified", if order == Ordering::Greater { "upgraded" } else { "downgraded" }, line.display(), running, version);
    // (the install line removes a checkout's mark too: a release's binary
    // links the release's runtime, not a checkout's)
    let dev = home.join("dev");
    if std::fs::remove_file(&dev).is_ok() {
        println!("(o.o) removed {}, as the install line does: compiled builds link the release's runtime, not that checkout's", dev.display());
    }
    println!("(o.o) crow {}'s Rust toolchain and runtime are fetched where missing by its first compiled build, or now by `crow setup`", version);
    Ok(())
}

/// A path removed when dropped, however the work on it ends: a fetch's
/// scratch directory, a binary not yet in its place.
struct Transient(PathBuf);

impl Drop for Transient {
    fn drop(&mut self) {
        let _ = if self.0.is_dir() { std::fs::remove_dir_all(&self.0) } else { std::fs::remove_file(&self.0) };
    }
}

/// The release's binary for this machine, fetched into a directory of its
/// own under the home, held to its published hash, extracted, and put in
/// place of `line`; nothing fetched is kept.
fn install_release(home: &Path, version: &str, line: &Path) -> Result<(), String> {
    let triple = target_triple();
    let name = format!("crow-{}-{}.tar.gz", version, triple);
    let base = release_url(version);
    println!("(o.o) fetching crow {} for {} from {}", version, triple, base);
    let hash = published_hash(&format!("{}/{}.sha256", base, name))?;
    let tmp = Transient(home.join("tmp").join(format!("upgrade-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&tmp.0);
    let archive = tmp.0.join(&name);
    fetch_verified(&format!("{}/{}", base, name), &archive, &hash)?;
    let unpacked = tmp.0.join("unpacked");
    extract(&archive, &unpacked)?;
    let new = unpacked.join("crow");
    if !new.is_file() {
        return Err(format!("{} holds no crow: nothing was replaced", name));
    }
    replace(&new, line, &name)
}

/// `new` put in place of `line`: copied beside it, made executable, run
/// once to show it runs here, then renamed over it — atomic on unix, and a
/// crow running from `line` keeps the file it started from. A binary that
/// does not run replaces nothing.
fn replace(new: &Path, line: &Path, name: &str) -> Result<(), String> {
    let part = Transient(line.with_extension("part"));
    let _ = std::fs::remove_file(&part.0);
    std::fs::copy(new, &part.0).map_err(|e| format!("cannot write {}: {}", part.0.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part.0, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("cannot make {} executable: {}", part.0.display(), e))?;
    }
    let ran = Command::new(&part.0)
        .arg("-v")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the crow in {} does not run here ({}): nothing was replaced", name, e))?;
    if !ran.status.success() {
        return Err(format!("the crow in {} does not run here (`crow -v` exited with {}): nothing was replaced", name, ran.status));
    }
    std::fs::rename(&part.0, line).map_err(|e| format!("cannot put the new crow at {}: {}", line.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
manifest-version = "2"
[pkg.rustc]
version = "1.93.1 (01f6ddf75 2026-02-11)"
[pkg.rustc.target.aarch64-apple-darwin]
available = true
url = "https://static.rust-lang.org/dist/2026-02-11/rustc-1.93.1-aarch64-apple-darwin.tar.gz"
hash = "0123abcd"
xz_url = "https://static.rust-lang.org/dist/2026-02-11/rustc-1.93.1-aarch64-apple-darwin.tar.xz"
xz_hash = "ffff"
[pkg.rustc.target.riscv32imac-unknown-none-elf]
available = false
[pkg.cargo.target.aarch64-apple-darwin]
available = true
url = "https://static.rust-lang.org/dist/2026-02-11/cargo-1.93.1-aarch64-apple-darwin.tar.gz"
hash = "4567"
"#;

    #[test]
    fn a_component_is_read_off_the_channel_manifest() {
        let (url, hash) = component_in(MANIFEST, "rustc", "aarch64-apple-darwin").unwrap();
        assert!(url.ends_with("rustc-1.93.1-aarch64-apple-darwin.tar.gz"));
        assert_eq!(hash, "0123abcd");
        let (url, _) = component_in(MANIFEST, "cargo", "aarch64-apple-darwin").unwrap();
        assert!(url.contains("cargo-1.93.1"));
        assert!(component_in(MANIFEST, "rustc", "riscv32imac-unknown-none-elf").unwrap_err().contains("has no rustc"));
        assert!(component_in(MANIFEST, "rust-std", "aarch64-apple-darwin").unwrap_err().contains("names no rust-std"));
    }

    /// The runtime is found in order: a checkout named by `home/dev`, the
    /// checkout the binary was built from, the bundle for this version,
    /// a source snapshot — and none of them holds a missing one.
    #[test]
    fn the_runtime_is_found_in_order() {
        let root = std::env::temp_dir().join(format!("crow_home_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let built = root.join("built");
        let make = |p: &Path| std::fs::create_dir_all(p.join("crow_runtime")).and_then(|_| std::fs::write(p.join("crow_runtime").join("Cargo.toml"), "[package]\n")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(runtime_in(&home, &built, "9.9.9"), None);
        make(&home.join("src"));
        assert_eq!(runtime_in(&home, &built, "9.9.9"), Some(Runtime::Snapshot(home.join("src"))));
        make(&home.join("runtime").join("9.9.9"));
        let bundle = runtime_in(&home, &built, "9.9.9").unwrap();
        assert_eq!(bundle, Runtime::Bundle(home.join("runtime").join("9.9.9")));
        assert!(!bundle.vendored(), "no vendor directory yet");
        std::fs::create_dir_all(home.join("runtime").join("9.9.9").join("vendor")).unwrap();
        assert!(runtime_in(&home, &built, "9.9.9").unwrap().vendored());
        make(&built);
        assert_eq!(runtime_in(&home, &built, "9.9.9"), Some(Runtime::Built(built.clone())));
        let dev = root.join("dev_checkout");
        make(&dev);
        std::fs::write(home.join("dev"), format!("{}\n", dev.display())).unwrap();
        assert_eq!(runtime_in(&home, &built, "9.9.9"), Some(Runtime::Dev(dev.clone())));
        // a dev file naming nothing is passed over
        std::fs::write(home.join("dev"), "/no/such/checkout\n").unwrap();
        assert_eq!(runtime_in(&home, &built, "9.9.9"), Some(Runtime::Built(built)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_vendored_config_replaces_the_registry_and_stays_offline() {
        let text = vendored_config(Path::new("/x/runtime/0.6.0/vendor"));
        assert!(text.contains("replace-with = \"crow-vendored\"") && text.contains("directory = \"/x/runtime/0.6.0/vendor\"") && text.contains("offline = true"));
    }

    #[test]
    fn the_host_is_named_as_rust_names_it() {
        let t = target_triple();
        assert!(t.contains("apple-darwin") || t.contains("linux-gnu") || t.contains("windows"), "{}", t);
        assert!(t.starts_with("aarch64") || t.starts_with("x86_64"), "{}", t);
    }

    #[test]
    fn releases_are_ordered_as_semantic_versions_are() {
        use std::cmp::Ordering::*;
        assert_eq!(version_order("0.7.0", "0.6.1"), Greater);
        assert_eq!(version_order("0.10.0", "0.9.9"), Greater, "by number, not by text");
        assert_eq!(version_order("0.0.1", "0.6.1"), Less);
        assert_eq!(version_order("0.6.1", "0.6.1"), Equal);
        assert_eq!(version_order("1.0", "1.0.0"), Equal);
        assert_eq!(version_order("1.0.0-rc.1", "1.0.0"), Less, "a pre-release comes before its release");
        assert_eq!(version_order("1.0.0-rc.2", "1.0.0-rc.10"), Less);
        assert_eq!(version_order("1.0.0-alpha", "1.0.0-alpha.1"), Less);
        assert_eq!(version_order("1.0.0-1", "1.0.0-alpha"), Less);
        assert_eq!(version_order("1.0.0+build.5", "1.0.0"), Equal);
    }

    #[test]
    fn a_release_is_named_by_its_number_or_its_tag() {
        assert_eq!(release_version("0.7.0").unwrap(), "0.7.0");
        assert_eq!(release_version("v0.7.0").unwrap(), "0.7.0");
        assert_eq!(release_version("1.0.0-rc.1").unwrap(), "1.0.0-rc.1");
        for not in ["", "v", "latest", "../0.7.0", "0.7.0/x", "0.7 .0"] {
            assert!(release_version(not).unwrap_err().contains("is not a release"), "{:?}", not);
        }
    }

    #[test]
    fn the_latest_release_is_read_off_githubs_answer() {
        assert_eq!(latest_in(r#"{"url": "https://api.github.com/x", "tag_name": "v0.7.0", "name": "v0.7.0"}"#).unwrap(), "0.7.0");
        assert!(latest_in(r#"{"message": "Not Found"}"#).unwrap_err().contains("names no latest release"));
        assert!(latest_in(r#"{"tag_name": "nightly"}"#).unwrap_err().contains("names no version"));
        assert!(latest_in("<html>rate limited</html>").unwrap_err().contains("did not answer with a release"));
    }

    #[test]
    fn upgrades_arguments_are_read() {
        let read = |xs: &[&str]| upgrade_args(&xs.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(read(&[]).unwrap(), (false, None));
        assert_eq!(read(&["--check"]).unwrap(), (true, None));
        assert_eq!(read(&["--version", "0.6.0"]).unwrap(), (false, Some("0.6.0".to_string())));
        assert_eq!(read(&["--check", "--version=v0.7.0"]).unwrap(), (true, Some("0.7.0".to_string())));
        assert!(read(&["--version"]).unwrap_err().contains("--version takes a release"));
        assert!(read(&["--version", "--check"]).unwrap_err().contains("is not a release"));
        assert!(read(&["--force"]).unwrap_err().contains("unknown argument '--force' for crow upgrade"));
    }

    /// The install line's binary is the one `crow upgrade` replaces; every
    /// other is told by what it was built from, and told how it upgrades.
    #[test]
    fn an_install_is_told_by_where_its_binary_is_and_what_it_was_built_from() {
        let root = std::env::temp_dir().join(format!("crow_upgrade_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let line = home.join("bin").join("crow");
        let checkout = root.join("checkout");
        let built = checkout.join("target").join("release").join("crow");
        for p in [&line, &built] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "a binary").unwrap();
        }
        std::fs::write(checkout.join("Cargo.toml"), "[package]\nname = \"crow\"\n").unwrap();
        std::fs::create_dir_all(home.join("src")).unwrap();
        let gone = root.join("the_release_runner"); // where a release was built: not this machine
        assert_eq!(install_of(&line, &home, &gone), Install::Line);
        assert_eq!(install_of(&line, &home, &checkout), Install::Line, "wherever it was built");
        let canon = checkout.canonicalize().unwrap();
        assert_eq!(install_of(&built, &home, &checkout), Install::Checkout(canon.clone()));
        let cargo_bin = root.join("cargo").join("bin").join("crow");
        assert_eq!(install_of(&cargo_bin, &home, &home.join("src")), Install::Snapshot);
        assert_eq!(install_of(&root.join("usr_local_bin_crow"), &home, &gone), Install::Elsewhere);
        // a link in the line's place is not the line's: what runs is where it points
        #[cfg(unix)]
        {
            let home2 = root.join("home2");
            std::fs::create_dir_all(home2.join("bin")).unwrap();
            std::os::unix::fs::symlink(&built, home2.join("bin").join("crow")).unwrap();
            assert_eq!(install_of(&home2.join("bin").join("crow"), &home2, &checkout), Install::Checkout(canon.clone()));
        }
        let said = elsewhere(&built, &home, &Install::Checkout(canon.clone()), true);
        assert!(said.contains(&built.display().to_string()) && said.contains("nothing was changed") && said.contains("git pull") && said.contains("./install.sh --dev"), "{}", said);
        let said = elsewhere(&cargo_bin, &home, &Install::Snapshot, false);
        assert!(!said.contains("nothing was changed") && said.contains("./install.sh --source") && said.contains(INSTALL_LINE), "{}", said);
        let said = elsewhere(&cargo_bin, &home, &Install::Elsewhere, true);
        assert!(said.contains(INSTALL_LINE) && said.contains(&line.display().to_string()) && said.contains(&format!("then remove {}", cargo_bin.display())), "{}", said);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The new binary runs once before it takes the old one's name, as a
    /// new file: a crow running from the old file keeps it.
    #[cfg(unix)]
    #[test]
    fn a_binary_is_renamed_over_the_old_one_only_when_it_runs() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // (under target/, not the system's temp: a temp mounted noexec would refuse the scripts)
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("crow_replace_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let line = root.join("bin").join("crow");
        std::fs::write(&line, "#!/bin/sh\necho old\n").unwrap();
        let before = std::fs::metadata(&line).unwrap().ino();
        let script = |name: &str, body: &str| {
            let p = root.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let e = replace(&script("broken", "#!/bin/sh\nexit 3\n"), &line, "crow-9.9.9-x.tar.gz").unwrap_err();
        assert!(e.contains("does not run here") && e.contains("nothing was replaced"), "{}", e);
        assert_eq!(std::fs::read_to_string(&line).unwrap(), "#!/bin/sh\necho old\n");
        assert!(!root.join("bin").join("crow.part").exists(), "nothing half-placed is kept");
        replace(&script("good", "#!/bin/sh\necho new\n"), &line, "crow-9.9.9-x.tar.gz").unwrap();
        assert_eq!(std::fs::read_to_string(&line).unwrap(), "#!/bin/sh\necho new\n");
        let after = std::fs::metadata(&line).unwrap();
        assert_ne!(after.ino(), before, "renamed over the old file, not written into it");
        assert_eq!(after.permissions().mode() & 0o777, 0o755);
        assert!(!root.join("bin").join("crow.part").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_published_hash_is_read_off_the_file_beside_the_release() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("crow_hash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let url = |name: &str| format!("file://{}", root.join(name).display());
        let hash = "AB".repeat(32);
        std::fs::write(root.join("ok.sha256"), format!("{}  crow-9.9.9-x.tar.gz\n", hash)).unwrap();
        std::fs::write(root.join("bad.sha256"), "<html>Not Found</html>\n").unwrap();
        assert_eq!(published_hash(&url("ok.sha256")).unwrap(), hash.to_lowercase());
        assert!(published_hash(&url("bad.sha256")).unwrap_err().contains("holds no SHA-256 hash"));
        assert!(published_hash(&url("missing.sha256")).unwrap_err().contains("cannot fetch"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
