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

use std::path::{Path, PathBuf};
use std::process::Command;

/// The Rust version every Crow release is held to: the toolchain `crow
/// setup` fetches, and the one the runtime bundle was vendored with.
pub const RUST_VERSION: &str = "1.93.1";

/// Where Rust's own releases are served from; `CROW_RUST_DIST` overrides it
/// (a test's local server).
const RUST_DIST: &str = "https://static.rust-lang.org/dist";

/// Where a Crow release's files are served from; `CROW_DIST_URL` overrides
/// it (a test's local server, or a mirror).
fn dist_url() -> String {
    std::env::var("CROW_DIST_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://github.com/agncrow/crow/releases/download/v{}", env!("CARGO_PKG_VERSION")))
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

fn fetch_text(url: &str, home: &Path) -> Result<String, String> {
    let to = home.join("tmp").join("fetched.txt");
    fetch(url, &to)?;
    let text = std::fs::read_to_string(&to).map_err(|e| format!("cannot read {}: {}", to.display(), e))?;
    let _ = std::fs::remove_file(&to);
    Ok(text)
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
    let manifest = fetch_text(&format!("{}/channel-rust-{}.toml", rust_dist(), RUST_VERSION), home)?;
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
    println!("(o.o) fetching the crow {} runtime from {} (about 84 MB)", version, base);
    let published = fetch_text(&format!("{}/{}.sha256", base, name), home)?;
    let hash = published.split_whitespace().next().unwrap_or_default().to_string();
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
}
