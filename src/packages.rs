//! `crow install`: a package is a git repository, a version is a tag (or a
//! branch, or a commit), and `crow.lock` pins the commit each dependency
//! checked out, so every install of a project is the same code (spec,
//! Package Manager). No registry server stands between a program and its
//! dependencies: a dependency names its repository.
//!
//! ```text
//! deps = map[models = 'github.com/agncrow/crow-models@v0.1.0', mine = '../mine']
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

/// `crow.lock`: each dependency's name, its source as the crowfile wrote
/// it, and the commit it checked out.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Lock {
    pub entries: BTreeMap<String, (String, String)>,
}

impl Lock {
    pub fn read(path: &Path) -> Lock {
        let mut entries = BTreeMap::new();
        for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // name = 'spec' commit
            let Some((name, rest)) = line.split_once('=') else { continue };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('\'') else { continue };
            let Some((spec, commit)) = rest.split_once('\'') else { continue };
            entries.insert(name.trim().to_string(), (spec.to_string(), commit.trim().to_string()));
        }
        Lock { entries }
    }

    pub fn text(&self) -> String {
        let mut out = String::from("# crow.lock — the commit each dependency checked out; commit this file\n");
        for (name, (spec, commit)) in &self.entries {
            out.push_str(&format!("{} = '{}' {}\n", name, spec, commit));
        }
        out
    }
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

/// The file inside an installed package that says what it is: its source
/// and its commit, so an install that has them already fetches nothing.
const MARKER: &str = ".crow-package";

/// Install `deps` (name → source, as the crowfile writes them) into
/// `root/crow_modules/`, pinned by `root/crow.lock`: a dependency the lock
/// holds checks out the locked commit, and one it does not — or one named
/// in `update`, or every one with `update_all` — takes what its reference
/// names now, and the lock records it. What each did, one line each.
pub fn install(root: &Path, deps: &[(String, String)], update: &[String], update_all: bool) -> Result<Vec<String>, String> {
    let modules = root.join("crow_modules");
    std::fs::create_dir_all(&modules).map_err(|e| format!("crow_modules/: {}", e))?;
    let lock_path = root.join("crow.lock");
    let old = Lock::read(&lock_path);
    let mut lock = Lock::default();
    let mut said = Vec::new();
    for (name, spec) in deps {
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
                copy_tree(&abs, &dest)?;
                said.push(format!("{}: linked to {}", name, abs.display()));
            }
            Source::Git { url, subdir, reference } => {
                let refresh = update_all || update.contains(name);
                let locked = old.entries.get(name).filter(|(s, _)| s == spec && !refresh).map(|(_, c)| c.clone());
                let have = std::fs::read_to_string(dest.join(MARKER)).unwrap_or_default();
                if let Some(commit) = &locked
                    && have.trim() == format!("{}\n{}", spec, commit)
                {
                    lock.entries.insert(name.clone(), (spec.clone(), commit.clone()));
                    said.push(format!("{}: {} at {} (installed)", name, spec, &commit[..commit.len().min(12)]));
                    continue;
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
                copy_tree(&package, &dest)?;
                std::fs::write(dest.join(MARKER), format!("{}\n{}\n", spec, commit)).map_err(|e| format!("{}: {}", name, e))?;
                let _ = std::fs::remove_dir_all(&fetch);
                said.push(format!("{}: {} at {}", name, spec, &commit[..commit.len().min(12)]));
                lock.entries.insert(name.clone(), (spec.clone(), commit));
            }
        }
    }
    std::fs::write(&lock_path, lock.text()).map_err(|e| format!("crow.lock: {}", e))?;
    Ok(said)
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
        lock.entries.insert("models".into(), ("github.com/a/b@v1".into(), "abc123".into()));
        let dir = std::env::temp_dir().join(format!("crow_lock_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("crow.lock"), lock.text()).unwrap();
        assert_eq!(Lock::read(&dir.join("crow.lock")), lock);
        let _ = std::fs::remove_dir_all(&dir);
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
        let said = install(&project, &deps, &[], false).unwrap();
        assert!(said[0].starts_with("models: "), "{:?}", said);
        assert!(project.join("crow_modules/models/line.co").exists());
        let locked = Lock::read(&project.join("crow.lock"));
        let first = locked.entries["models"].1.clone();
        // installed already: nothing fetched
        let said = install(&project, &deps, &[], false).unwrap();
        assert!(said[0].ends_with("(installed)"), "{:?}", said);
        // the tag moves to a new commit: the lock keeps the old one
        std::fs::write(work.join("models/line.co"), "fn slope[a, b]\n    return (b - a) * 1.0\n").unwrap();
        g(&["commit", "--quiet", "-am", "two"], &work);
        g(&["tag", "-f", "v0.1.0"], &work);
        g(&["push", "--quiet", "--force", "--tags", &bare.to_string_lossy()], &work);
        let _ = std::fs::remove_dir_all(project.join("crow_modules/models"));
        let said = install(&project, &deps, &[], false).unwrap();
        assert!(said.iter().any(|l| l.contains("now names")), "{:?}", said);
        assert_eq!(Lock::read(&project.join("crow.lock")).entries["models"].1, first);
        assert!(!std::fs::read_to_string(project.join("crow_modules/models/line.co")).unwrap().contains("1.0"));
        // --update takes what the tag names now
        install(&project, &deps, &["models".to_string()], false).unwrap();
        assert_ne!(Lock::read(&project.join("crow.lock")).entries["models"].1, first);
        assert!(std::fs::read_to_string(project.join("crow_modules/models/line.co")).unwrap().contains("1.0"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
