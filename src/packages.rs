//! `crow install`: a package is a git repository, a version is a tag (or a
//! branch, or a commit), and `crow.lock` pins the commit each dependency
//! checked out and the hash of what was installed, so every install of a
//! project is the same code, and a package changed under the lock is
//! refused before a program reads it (spec, Package Manager). No registry
//! server stands between a program and its dependencies: a dependency
//! names its repository. A project holds one version of a package: two
//! requirements at tags of one major take the higher. A requirer may
//! attest the hash it verified after the version, and the install holds
//! the bytes to it.
//!
//! ```text
//! deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', mine = '../mine']
//! deps = map[models = 'github.com/agncrow/crow-models@v0.1.2#sha256:3f9a…']
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A git repository: its url, the directory inside it that is the
    /// package (empty for its root), and the tag, branch or commit.
    Git { url: String, subdir: String, reference: String },
    /// A directory on this machine, linked as it is (for working on a
    /// package beside the program).
    Local(PathBuf),
}

/// The hosts whose repositories are the first two segments of a path
/// (`github.com/org/repo/sub/dir`): the rest is a directory inside it.
const FORGES: &[&str] = &["github.com", "gitlab.com", "codeberg.org", "bitbucket.org"];

/// A dependency's source, as the crowfile writes it: a local path (`../x`,
/// `/x`, `~/x`), or a repository and a reference after its last `@` —
/// `github.com/org/repo@v1.0.0`, `github.com/org/repo/sub/dir@v1.0.0`,
/// `https://host/repo.git@main`, `file:///path/repo.git@v2`.
pub fn parse(spec: &str) -> Result<Source, String> {
    let (spec, _) = split_attested(spec);
    let spec = spec.as_str();
    if spec.starts_with('.') || spec.starts_with('/') || spec.starts_with('~') {
        let path = match spec.strip_prefix("~/") {
            Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
            None => PathBuf::from(spec),
        };
        return Ok(Source::Local(path));
    }
    let Some((place, reference)) = spec.rsplit_once('@').filter(|(p, r)| !p.is_empty() && !r.is_empty()) else {
        return Err(format!("'{}' names no version: a git dependency ends with @ and a tag, a branch or a commit — 'github.com/org/repo@v1.0.0'", spec));
    };
    if let Some((scheme, rest)) = place.split_once("://") {
        // a url: a directory inside the repository follows a `//`
        let (repo, subdir) = rest.split_once("//").unwrap_or((rest, ""));
        return Ok(Source::Git { url: format!("{}://{}", scheme, repo), subdir: subdir.trim_matches('/').to_string(), reference: reference.to_string() });
    }
    let segments: Vec<&str> = place.split('/').filter(|s| !s.is_empty()).collect();
    let Some(host) = segments.first() else { return Err(format!("'{}' names no repository", spec)) };
    let take = if FORGES.contains(host) { 3 } else { segments.len() };
    if segments.len() < take || segments.len() < 2 {
        return Err(format!("'{}' names no repository: 'github.com/org/repo@v1.0.0'", spec));
    }
    let repo = segments[..take].join("/");
    let repo = repo.strip_suffix(".git").unwrap_or(&repo).to_string();
    Ok(Source::Git { url: format!("https://{}.git", repo), subdir: segments[take..].join("/"), reference: reference.to_string() })
}

/// A dependency's source after its version, as the crowfile wrote it, and
/// the hash a requirer attested after a `#` (`…@v0.1.2#sha256:3f9a…`).
pub fn split_attested(spec: &str) -> (String, Option<String>) {
    match spec.split_once('#') {
        Some((bare, hash)) if hash.starts_with("sha256:") => (bare.to_string(), Some(hash.to_string())),
        _ => (spec.to_string(), None),
    }
}

/// One line of `crow.lock`: a dependency's source as the crowfile wrote
/// it, the commit it checked out, and the hash of what was installed
/// (`tree_hash`; empty in a lock written before hashes were).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Entry {
    pub spec: String,
    pub commit: String,
    pub hash: String,
}

/// `crow.lock`: each dependency's name and its entry.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Lock {
    pub entries: BTreeMap<String, Entry>,
}

impl Lock {
    pub fn read(path: &Path) -> Lock {
        let mut entries = BTreeMap::new();
        for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // name = 'spec' commit sha256:…
            let Some((name, rest)) = line.split_once('=') else { continue };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('\'') else { continue };
            let Some((spec, rest)) = rest.split_once('\'') else { continue };
            let mut words = rest.split_whitespace();
            let commit = words.next().unwrap_or("").to_string();
            let hash = words.next().unwrap_or("").to_string();
            entries.insert(name.trim().to_string(), Entry { spec: spec.to_string(), commit, hash });
        }
        Lock { entries }
    }

    pub fn text(&self) -> String {
        let mut out = String::from("# crow.lock — the commit each dependency checked out, and the hash of what was installed; commit this file\n");
        for (name, e) in &self.entries {
            out.push_str(&format!("{} = '{}' {} {}\n", name, e.spec, e.commit, e.hash));
        }
        out
    }
}

/// The hash of a package as installed: every file under the directory but
/// git's own and the install's marker, by its path and its bytes, in path
/// order (`sha256:` and the hex).
pub fn tree_hash(dir: &Path) -> Result<String, String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name == ".git" || name == MARKER {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out)?;
            } else if p.is_file() {
                out.push((p.strip_prefix(base).unwrap_or(&p).to_string_lossy().to_string(), p.clone()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let mut manifest = String::new();
    for (rel, path) in files {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let digest = crow_runtime::sys::hash(&crow_runtime::sys::Bytes(bytes), "sha256")?;
        manifest.push_str(&format!("{} {}\n", rel, digest));
    }
    Ok(format!("sha256:{}", crow_runtime::hash::hash(&manifest, "sha256")?))
}

/// The installed packages held to the lock: one whose files no longer
/// hash as the lock recorded is refused, before a program reads any of
/// it. A linked package (a local path) is not held; a lock written
/// before hashes were holds nothing.
pub fn verify(root: &Path) -> Result<(), String> {
    let lock = Lock::read(&root.join("crow.lock"));
    for (name, e) in &lock.entries {
        if e.hash.is_empty() {
            continue;
        }
        let dest = root.join("crow_modules").join(name);
        if dest.is_symlink() || !dest.is_dir() {
            continue;
        }
        let now = tree_hash(&dest)?;
        if now != e.hash {
            return Err(format!(
                "crow_modules/{} differs from what crow.lock installed ({} at {}): run `crow install` to restore it, or change the package where it lives",
                name,
                e.spec,
                &e.commit[..e.commit.len().min(12)]
            ));
        }
    }
    Ok(())
}

thread_local! {
    /// The project roots this process has verified: once each.
    static VERIFIED: std::cell::RefCell<std::collections::HashSet<PathBuf>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

/// `verify`, once per project root in a process (every command that reads
/// a program calls it where a `use` is expanded).
pub fn verified(root: &Path) -> Result<(), String> {
    let seen = VERIFIED.with(|v| v.borrow().contains(root));
    if seen {
        return Ok(());
    }
    verify(root)?;
    VERIFIED.with(|v| v.borrow_mut().insert(root.to_path_buf()));
    Ok(())
}

/// A tag read as a version: `v1.2.3` or `1.2.3`, its three numbers; none
/// for a branch, a commit or any other tag.
fn version(reference: &str) -> Option<(u64, u64, u64)> {
    let digits = reference.strip_prefix('v').unwrap_or(reference);
    let mut parts = digits.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((a.parse().ok()?, b.parse().ok()?, c.parse().ok()?))
}

/// Which of two sources of one package the project takes: the higher of
/// two tags of one repository and one major (`Some(true)` when the second
/// is), neither where they are of different majors, of different
/// repositories, or branches or commits — then the project cannot hold
/// both, and the install is refused naming who wanted each.
fn higher(have: &str, want: &str) -> Option<bool> {
    let (Ok(Source::Git { url: u1, subdir: d1, reference: r1 }), Ok(Source::Git { url: u2, subdir: d2, reference: r2 })) = (parse(have), parse(want)) else { return None };
    if u1 != u2 || d1 != d2 {
        return None;
    }
    let (v1, v2) = (version(&r1)?, version(&r2)?);
    if v1.0 != v2.0 {
        return None;
    }
    Some(v2 > v1)
}

/// A source's version, as a message names it: the reference after its `@`.
fn version_of(spec: &str) -> String {
    spec.rsplit_once('@').map_or(spec.to_string(), |(_, r)| r.to_string())
}

fn git(args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.output().map_err(|e| format!("git: {}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// A package's files, copied from a checkout (its `.git` left behind).
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {}", to.display(), e))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {}", from.display(), e))?.flatten() {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {}", src.display(), e))?;
        }
    }
    Ok(())
}

/// A package's files copied into its place under crow_modules: all but its
/// own `crow_modules/` — the links its repository keeps to the packages it
/// is written over, while it is worked on — since `crow install` puts its
/// dependencies beside it, flat. Copied with them, kg's link to magpie had
/// become a second magpie inside kg, whose files took the prefixes of the
/// magpie beside it: "declared twice", for every name.
fn copy_package(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {}", to.display(), e))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {}", from.display(), e))?.flatten() {
        let name = entry.file_name();
        if name == ".git" || name == "crow_modules" {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {}", src.display(), e))?;
        }
    }
    Ok(())
}

/// The file inside an installed package that says what it is: its source
/// and its commit, so an install that has them already fetches nothing.
const MARKER: &str = ".crow-package";

/// Install `deps` (name → source, as the crowfile writes them) into
/// `root/crow_modules/`, pinned by `root/crow.lock`: a dependency the lock
/// holds checks out the locked commit, and one it does not — or one named
/// in `update`, or every one with `update_all` — takes what its reference
/// names now, and the lock records it. Each package's own dependencies —
/// its crowfile's, which `deps_of` reads — are installed beside it, flat,
/// and locked the same way; one package wanted at two sources is refused,
/// naming who wanted each: a project holds one version of a package. What
/// each did, one line each.
pub fn install(
    root: &Path,
    deps: &[(String, String)],
    update: &[String],
    update_all: bool,
    deps_of: &dyn Fn(&Path) -> Result<Vec<(String, String)>, String>,
) -> Result<Vec<String>, String> {
    let modules = root.join("crow_modules");
    std::fs::create_dir_all(&modules).map_err(|e| format!("crow_modules/: {}", e))?;
    let lock_path = root.join("crow.lock");
    let old = Lock::read(&lock_path);
    let mut lock = Lock::default();
    let mut said = Vec::new();
    // the project's dependencies, then each package's own as they are met:
    // name, source, and the package that asked (none: the project)
    let mut queue: std::collections::VecDeque<(String, String, Option<String>)> = deps.iter().map(|(n, s)| (n.clone(), s.clone(), None)).collect();
    let mut wanted: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    let who = |by: &Option<String>| by.as_ref().map(|p| format!("the package {}", p)).unwrap_or_else(|| "the project".to_string());
    // what a requirer attested a version hashes to, held to the bytes once
    // the package is installed (and to every other attestation of it)
    let attested = |name: &str, hash: &Option<String>, by: &Option<String>, lock: &Lock, modules: &Path| -> Result<(), String> {
        let (Some(expected), Some(e)) = (hash, lock.entries.get(name)) else { return Ok(()) };
        if *expected != e.hash {
            let _ = std::fs::remove_dir_all(modules.join(name));
            return Err(format!(
                "{} at {} does not hash as {} attested — {} attested, {} installed: the version's content is not what was verified, so check the repository before changing the attestation",
                name,
                version_of(&e.spec),
                who(by),
                expected,
                e.hash
            ));
        }
        Ok(())
    };
    while let Some((name, spec, by)) = queue.pop_front() {
        let (spec, hash) = split_attested(&spec);
        if let Some((have, first)) = wanted.get(&name).cloned() {
            if have == spec {
                attested(&name, &hash, &by, &lock, &modules)?;
                continue;
            }
            // two versions of one package: the higher of one major is taken
            // for both, and the install says so
            match higher(&have, &spec) {
                Some(true) => said.push(format!("{}: {} wanted by {}, {} by {} — {} taken for both", name, version_of(&have), who(&first), version_of(&spec), who(&by), version_of(&spec))),
                Some(false) => {
                    said.push(format!("{}: {} wanted by {}, {} by {} — {} taken for both", name, version_of(&have), who(&first), version_of(&spec), who(&by), version_of(&have)));
                    attested(&name, &hash, &by, &lock, &modules)?;
                    continue;
                }
                None => {
                    return Err(format!(
                        "{} is wanted at two sources — '{}' by {} and '{}' by {}: a project holds one version of a package, and only tags of one major can be joined (the higher is taken), so name the one they can share",
                        name,
                        have,
                        who(&first),
                        spec,
                        who(&by)
                    ))
                }
            }
        }
        wanted.insert(name.clone(), (spec.clone(), by.clone()));
        let refresh = update_all || update.contains(&name);
        let (dest, line) = install_one(&root.to_path_buf(), &modules, &name, &spec, &old, refresh, &mut lock, &mut said)?;
        attested(&name, &hash, &by, &lock, &modules)?;
        if let Some(line) = line {
            said.push(match &by {
                Some(p) => format!("{} (for {})", line, p),
                None => line,
            });
        }
        // its own dependencies, a path it names read from where it is
        let real = std::fs::canonicalize(&dest).unwrap_or(dest.clone());
        for (n, s) in deps_of(&real)? {
            let s = match parse(&s)? {
                Source::Local(p) if p.is_relative() => real.join(p).display().to_string(),
                _ => s,
            };
            queue.push_back((n, s, Some(name.clone())));
        }
    }
    std::fs::write(&lock_path, lock.text()).map_err(|e| format!("crow.lock: {}", e))?;
    Ok(said)
}

/// One dependency installed (or found installed): where it is, and the
/// line that says what was done (a warning about a moved tag goes straight
/// to `said`).
#[allow(clippy::too_many_arguments)]
fn install_one(
    root: &PathBuf,
    modules: &Path,
    name: &str,
    spec: &str,
    old: &Lock,
    refresh: bool,
    lock: &mut Lock,
    said: &mut Vec<String>,
) -> Result<(PathBuf, Option<String>), String> {
    let dest = modules.join(name);
    match parse(spec)? {
        Source::Local(path) => {
            let abs = std::fs::canonicalize(root.join(&path)).map_err(|_| format!("{}: no directory at '{}'", name, spec))?;
            if dest.exists() || dest.is_symlink() {
                let _ = std::fs::remove_file(&dest).or_else(|_| std::fs::remove_dir_all(&dest));
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&abs, &dest).map_err(|e| format!("{}: {}", name, e))?;
            #[cfg(not(unix))]
            copy_package(&abs, &dest)?;
            Ok((dest, Some(format!("{}: linked to {}", name, abs.display()))))
        }
        Source::Git { url, subdir, reference } => {
            let locked = old.entries.get(name).filter(|e| e.spec == spec && !refresh).map(|e| e.commit.clone());
            let have = std::fs::read_to_string(dest.join(MARKER)).unwrap_or_default();
            // installed as the lock says, and hashing as it recorded (a
            // package changed under the lock is fetched again, restored)
            if let Some(commit) = &locked
                && have.lines().take(2).collect::<Vec<_>>() == vec![spec, commit.as_str()]
            {
                let recorded = old.entries.get(name).map(|e| e.hash.clone()).unwrap_or_default();
                let now = tree_hash(&dest)?;
                if recorded.is_empty() || recorded == now {
                    lock.entries.insert(name.to_string(), Entry { spec: spec.to_string(), commit: commit.clone(), hash: now });
                    return Ok((dest, Some(format!("{}: {} at {} (installed)", name, spec, &commit[..commit.len().min(12)]))));
                }
                said.push(format!("{}: changed since it was installed — restored", name));
            }
            let fetch = modules.join(format!(".fetch-{}", name));
            let _ = std::fs::remove_dir_all(&fetch);
            let fetch_s = fetch.to_string_lossy().to_string();
            let commit = match &locked {
                Some(commit) => {
                    git(&["clone", "--quiet", "--no-checkout", &url, &fetch_s], None).map_err(|e| format!("{}: cannot fetch {}: {}", name, url, e))?;
                    git(&["checkout", "--quiet", commit], Some(&fetch)).map_err(|e| format!("{}: the locked commit {} is not in {}: {}", name, commit, url, e))?;
                    commit.clone()
                }
                None => {
                    git(&["clone", "--quiet", &url, &fetch_s], None).map_err(|e| format!("{}: cannot fetch {}: {}", name, url, e))?;
                    git(&["checkout", "--quiet", &reference], Some(&fetch)).map_err(|_| format!("{}: {} has no tag, branch or commit '{}'", name, url, reference))?;
                    git(&["rev-parse", "HEAD"], Some(&fetch))?
                }
            };
            // a tag moved since the lock was written: the lock wins, and
            // the install says so
            if locked.is_some()
                && let Ok(now) = git(&["rev-parse", &format!("{}^{{commit}}", reference)], Some(&fetch))
                && now != commit
            {
                said.push(format!("{}: '{}' now names {}, the lock keeps {} — `crow install --update {}` takes it", name, reference, &now[..now.len().min(12)], &commit[..commit.len().min(12)], name));
            }
            let package = if subdir.is_empty() { fetch.clone() } else { fetch.join(&subdir) };
            if !package.is_dir() {
                let _ = std::fs::remove_dir_all(&fetch);
                return Err(format!("{}: {} has no directory '{}' at {}", name, url, subdir, reference));
            }
            if dest.exists() || dest.is_symlink() {
                let _ = std::fs::remove_file(&dest).or_else(|_| std::fs::remove_dir_all(&dest));
            }
            copy_package(&package, &dest)?;
            let hash = tree_hash(&dest)?;
            std::fs::write(dest.join(MARKER), format!("{}\n{}\n{}\n", spec, commit, hash)).map_err(|e| format!("{}: {}", name, e))?;
            let _ = std::fs::remove_dir_all(&fetch);
            // the lock's hash for this commit, when it has one, is what the
            // bytes must be: a commit's content cannot change, so a
            // difference is the fetch, not the package
            if let Some(e) = old.entries.get(name).filter(|e| e.spec == spec && e.commit == commit && !e.hash.is_empty() && e.hash != hash) {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(format!("{}: {} at {} does not hash as crow.lock recorded — {} recorded, {} fetched: the content of a commit cannot change, so the fetch is not the package the lock pinned", name, spec, &commit[..commit.len().min(12)], e.hash, hash));
            }
            let line = format!("{}: {} at {}", name, spec, &commit[..commit.len().min(12)]);
            lock.entries.insert(name.to_string(), Entry { spec: spec.to_string(), commit, hash });
            Ok((dest, Some(line)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dependency_names_its_repository_and_version() {
        assert_eq!(
            parse("github.com/agncrow/crow-models@v0.1.0").unwrap(),
            Source::Git { url: "https://github.com/agncrow/crow-models.git".into(), subdir: String::new(), reference: "v0.1.0".into() }
        );
        assert_eq!(
            parse("github.com/agncrow/crow/packages/models@v0.6.0").unwrap(),
            Source::Git { url: "https://github.com/agncrow/crow.git".into(), subdir: "packages/models".into(), reference: "v0.6.0".into() }
        );
        assert_eq!(
            parse("file:///tmp/repo.git//pkg@main").unwrap(),
            Source::Git { url: "file:///tmp/repo.git".into(), subdir: "pkg".into(), reference: "main".into() }
        );
        assert_eq!(parse("../mine").unwrap(), Source::Local(PathBuf::from("../mine")));
        assert!(parse("github.com/agncrow/crow-models").unwrap_err().contains("names no version"));
    }

    #[test]
    fn a_lock_reads_back_what_it_wrote() {
        let mut lock = Lock::default();
        lock.entries.insert("models".into(), Entry { spec: "github.com/a/b@v1".into(), commit: "abc123".into(), hash: "sha256:00".into() });
        let dir = std::env::temp_dir().join(format!("crow_lock_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("crow.lock"), lock.text()).unwrap();
        assert_eq!(Lock::read(&dir.join("crow.lock")), lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No package's crowfile: no dependencies of its own.
    fn none(_: &Path) -> Result<Vec<(String, String)>, String> {
        Ok(Vec::new())
    }

    /// A package's own dependencies install beside it and are locked, and a
    /// package of the project reaches them; one package wanted at two
    /// sources is refused, naming who wanted each
    #[test]
    fn a_packages_own_dependencies_install_beside_it() {
        let base = std::env::temp_dir().join(format!("crow_pkg_deps_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (models, market, project) = (base.join("models"), base.join("market"), base.join("app"));
        for d in [&models, &market, &project] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(market.join("bars.co"), "fn close[x]\n    return x\n").unwrap();
        std::fs::write(models.join("crowfile"), "deps = map[market = '../market']\n").unwrap();
        std::fs::write(models.join("fit.co"), "use market.bars\nfn fit[x]\n    return bars.close[x] * 2\n").unwrap();
        let deps = vec![("models".to_string(), models.display().to_string())];
        let said = install(&project, &deps, &[], false, &crate::crowfile_deps).unwrap();
        assert!(said.iter().any(|l| l.starts_with("market: linked to") && l.ends_with("(for models)")), "{:?}", said);
        assert!(project.join("crow_modules/market/bars.co").exists());
        // the program uses models, whose fit uses market beside it
        std::fs::write(project.join("main.co"), "use models.fit\nlog[fit.fit[21]]\n").unwrap();
        let stmts = crate::build(&std::fs::read_to_string(project.join("main.co")).unwrap()).unwrap();
        let expanded = crate::expand_module_imports(&stmts, &project, &mut std::collections::HashSet::new());
        let it = crate::ir_interp::run_script(&expanded, "main").unwrap();
        let _ = it;
        // two sources for one package: refused, naming both
        let other = base.join("market2");
        std::fs::create_dir_all(&other).unwrap();
        let deps = vec![("market".to_string(), other.display().to_string()), ("models".to_string(), models.display().to_string())];
        let err = install(&project, &deps, &[], false, &crate::crowfile_deps).unwrap_err();
        assert!(err.contains("market is wanted at two sources") && err.contains("by the project") && err.contains("by the package models"), "{}", err);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A repository with two tags of one major and one of another, bare,
    /// for the tests of versions and attestations.
    fn tagged_repo(base: &Path) -> PathBuf {
        let (work, bare) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(work.join("models")).unwrap();
        let g = |args: &[&str], dir: &Path| git(&[&["-c", "user.name=t", "-c", "user.email=t@t"], args].concat(), Some(dir)).unwrap();
        g(&["init", "--quiet"], &work);
        for (tag, body) in [("v0.1.0", "fn slope[a, b]\n    return b - a\n"), ("v0.1.1", "fn slope[a, b]\n    return (b - a) * 1.0\n"), ("v1.0.0", "fn slope[a, b]\n    return (b - a) * 2.0\n")] {
            std::fs::write(work.join("models/line.co"), body).unwrap();
            g(&["add", "."], &work);
            g(&["commit", "--quiet", "-m", tag], &work);
            g(&["tag", tag], &work);
        }
        git(&["clone", "--quiet", "--bare", &work.to_string_lossy(), &bare.to_string_lossy()], None).unwrap();
        bare
    }

    /// Two requirements of one package at tags of one major take the
    /// higher, saying so; tags of two majors are refused naming both.
    #[test]
    fn the_higher_of_two_tags_of_one_major_is_taken() {
        let base = std::env::temp_dir().join(format!("crow_pkg_versions_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bare = tagged_repo(&base);
        let (project, vectors) = (base.join("app"), base.join("vectors"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&vectors).unwrap();
        let at = |tag: &str| format!("file://{}//models@{}", bare.display(), tag);
        // the project wants v0.1.0, a package beside it wants v0.1.1
        std::fs::write(vectors.join("crowfile"), format!("deps = map[models = '{}']\n", at("v0.1.1"))).unwrap();
        std::fs::write(vectors.join("flat.co"), "use models.line\nfn twice[a, b]\n    return line.slope[a, b] * 2\n").unwrap();
        let deps = vec![("models".to_string(), at("v0.1.0")), ("vectors".to_string(), vectors.display().to_string())];
        let said = install(&project, &deps, &[], false, &crate::crowfile_deps).unwrap();
        assert!(said.iter().any(|l| l.contains("models: v0.1.0 wanted by the project, v0.1.1 by the package vectors — v0.1.1 taken for both")), "{:?}", said);
        assert_eq!(Lock::read(&project.join("crow.lock")).entries["models"].spec, at("v0.1.1"));
        assert!(std::fs::read_to_string(project.join("crow_modules/models/line.co")).unwrap().contains("1.0"));
        // the other way round: the project's higher version wins, said once
        let deps = vec![("models".to_string(), at("v0.1.1")), ("vectors".to_string(), vectors.display().to_string())];
        std::fs::write(vectors.join("crowfile"), format!("deps = map[models = '{}']\n", at("v0.1.0"))).unwrap();
        let said = install(&project, &deps, &[], false, &crate::crowfile_deps).unwrap();
        assert!(said.iter().any(|l| l.contains("v0.1.1 taken for both")), "{:?}", said);
        // two majors: refused, naming both
        std::fs::write(vectors.join("crowfile"), format!("deps = map[models = '{}']\n", at("v1.0.0"))).unwrap();
        let err = install(&project, &deps, &[], false, &crate::crowfile_deps).unwrap_err();
        assert!(err.contains("models is wanted at two sources") && err.contains("v1.0.0") && err.contains("by the package vectors"), "{}", err);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A package installed from a repository comes without its own
    /// crow_modules — the links its repository keeps to the packages beside
    /// it there, which `crow install` puts beside it here: kg's link to
    /// magpie had been copied as a second magpie inside kg, its names
    /// "declared twice" beside the magpie installed for it.
    #[cfg(unix)]
    #[test]
    fn a_packages_own_links_to_its_dependencies_are_not_installed() {
        let base = std::env::temp_dir().join(format!("crow_pkg_links_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (work, bare) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(work.join("dep")).unwrap();
        std::fs::create_dir_all(work.join("pk/crow_modules")).unwrap();
        let g = |args: &[&str], dir: &Path| git(&[&["-c", "user.name=t", "-c", "user.email=t@t"], args].concat(), Some(dir)).unwrap();
        g(&["init", "--quiet"], &work);
        let at = |dir: &str| format!("file://{}//{}@v0.1.0", bare.display(), dir);
        std::fs::write(work.join("dep/main.co"), "fn one[]\n    return 1\n").unwrap();
        std::fs::write(work.join("pk/crowfile"), format!("deps = map[dep = '{}']\n", at("dep"))).unwrap();
        std::fs::write(work.join("pk/main.co"), "use dep\nfn two[]\n    return dep.one[] + 1\n").unwrap();
        std::os::unix::fs::symlink("../../dep", work.join("pk/crow_modules/dep")).unwrap();
        g(&["add", "."], &work);
        g(&["commit", "--quiet", "-m", "v0.1.0"], &work);
        g(&["tag", "v0.1.0"], &work);
        git(&["clone", "--quiet", "--bare", &work.to_string_lossy(), &bare.to_string_lossy()], None).unwrap();
        let project = base.join("app");
        std::fs::create_dir_all(&project).unwrap();
        install(&project, &[("pk".to_string(), at("pk"))], &[], false, &crate::crowfile_deps).unwrap();
        assert!(project.join("crow_modules/pk/main.co").is_file());
        assert!(project.join("crow_modules/dep/main.co").is_file(), "the dependency, beside it");
        assert!(!project.join("crow_modules/pk/crow_modules").exists(), "no copy of it inside");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A hash attested after the version is held to the bytes: the right
    /// one installs, a wrong one is refused naming both hashes.
    #[test]
    fn an_attested_hash_is_held_to_the_bytes() {
        let base = std::env::temp_dir().join(format!("crow_pkg_attest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bare = tagged_repo(&base);
        let project = base.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let spec = format!("file://{}//models@v0.1.0", bare.display());
        install(&project, &[("models".to_string(), spec.clone())], &[], false, &none).unwrap();
        let hash = Lock::read(&project.join("crow.lock")).entries["models"].hash.clone();
        assert!(hash.starts_with("sha256:"));
        let right = vec![("models".to_string(), format!("{}#{}", spec, hash))];
        install(&project, &right, &[], false, &none).unwrap();
        assert_eq!(Lock::read(&project.join("crow.lock")).entries["models"].spec, spec, "the lock holds the source without the attestation");
        let wrong = vec![("models".to_string(), format!("{}#sha256:{}", spec, "0".repeat(64)))];
        let err = install(&project, &wrong, &[], false, &none).unwrap_err();
        assert!(err.contains("models at v0.1.0 does not hash as the project attested"), "{}", err);
        assert!(!project.join("crow_modules/models").exists(), "a package that does not hash as attested is not left installed");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A package from a repository: installed at its tag and locked; again,
    /// from what is installed, fetching nothing; a tag moved keeps the
    /// locked commit and says so; `--update` takes the new one
    #[test]
    fn a_package_is_pinned_by_its_lock() {
        let base = std::env::temp_dir().join(format!("crow_pkg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (work, bare, project) = (base.join("work"), base.join("remote.git"), base.join("app"));
        for d in [&work, &project] {
            std::fs::create_dir_all(d).unwrap();
        }
        let g = |args: &[&str], dir: &Path| git(&[&["-c", "user.name=t", "-c", "user.email=t@t"], args].concat(), Some(dir)).unwrap();
        g(&["init", "--quiet"], &work);
        std::fs::create_dir_all(work.join("models")).unwrap();
        std::fs::write(work.join("models/line.co"), "fn slope[a, b]\n    return b - a\n").unwrap();
        g(&["add", "."], &work);
        g(&["commit", "--quiet", "-m", "one"], &work);
        g(&["tag", "v0.1.0"], &work);
        git(&["clone", "--quiet", "--bare", &work.to_string_lossy(), &bare.to_string_lossy()], None).unwrap();
        let spec = format!("file://{}//models@v0.1.0", bare.display());
        let deps = vec![("models".to_string(), spec.clone())];
        let said = install(&project, &deps, &[], false, &none).unwrap();
        assert!(said[0].starts_with("models: "), "{:?}", said);
        assert!(project.join("crow_modules/models/line.co").exists());
        let locked = Lock::read(&project.join("crow.lock"));
        let first = locked.entries["models"].commit.clone();
        assert!(locked.entries["models"].hash.starts_with("sha256:"), "{:?}", locked);
        assert!(verify(&project).is_ok());
        // installed already: nothing fetched
        let said = install(&project, &deps, &[], false, &none).unwrap();
        assert!(said[0].ends_with("(installed)"), "{:?}", said);
        // the tag moves to a new commit: the lock keeps the old one
        std::fs::write(work.join("models/line.co"), "fn slope[a, b]\n    return (b - a) * 1.0\n").unwrap();
        g(&["commit", "--quiet", "-am", "two"], &work);
        g(&["tag", "-f", "v0.1.0"], &work);
        g(&["push", "--quiet", "--force", "--tags", &bare.to_string_lossy()], &work);
        let _ = std::fs::remove_dir_all(project.join("crow_modules/models"));
        let said = install(&project, &deps, &[], false, &none).unwrap();
        assert!(said.iter().any(|l| l.contains("now names")), "{:?}", said);
        assert_eq!(Lock::read(&project.join("crow.lock")).entries["models"].commit, first);
        assert!(!std::fs::read_to_string(project.join("crow_modules/models/line.co")).unwrap().contains("1.0"));
        // a package changed under the lock is refused, and restored by an install
        std::fs::write(project.join("crow_modules/models/line.co"), "fn slope[a, b]\n    return 0\n").unwrap();
        let err = verify(&project).unwrap_err();
        assert!(err.contains("crow_modules/models differs from what crow.lock installed"), "{}", err);
        let said = install(&project, &deps, &[], false, &none).unwrap();
        assert!(said.iter().any(|l| l.contains("restored")), "{:?}", said);
        assert!(verify(&project).is_ok());
        // --update takes what the tag names now
        install(&project, &deps, &["models".to_string()], false, &none).unwrap();
        assert_ne!(Lock::read(&project.join("crow.lock")).entries["models"].commit, first);
        assert!(std::fs::read_to_string(project.join("crow_modules/models/line.co")).unwrap().contains("1.0"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
