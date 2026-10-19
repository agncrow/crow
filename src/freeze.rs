//! The compiler holds the language and names no module and no builtin
//! (design rule 24; docs/extending-crow.md). This test counts, in every
//! source file of the compiler, the string literals that are a module's
//! name or a builtin's name or row id, and the references to a module
//! crate (`crow_web::`, `crow_db::`), and holds each file to the count in
//! tests/freeze/names.txt. A count above its baseline fails: the file
//! learned a name. A count below it fails too, naming the file, so that
//! the baseline is lowered (`CROW_FREEZE_BLESS=1 cargo test freeze`) and
//! committed: the baseline only shrinks. The tools of the binary (the
//! checker, the baseline, the database checks, the self-check, the
//! harnesses) are not held: they may know a module, reading its facts
//! from the registry.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// The sources that are not the compiler: tools of the binary, and
    /// this test. Every other file under src/ is held.
    const TOOLS: &[&str] = &[
        "appbook.rs",
        "baseline.rs",
        "checker.rs",
        "compiled_harness.rs",
        "db_check",
        "dev.rs",
        "diagnostic.rs",
        "driver.rs",
        "errors_corpus.rs",
        "freeze.rs",
        "fuzz.rs",
        "harness.rs",
        "home.rs",
        "interp_tests.rs",
        "lsp",
        "packages.rs",
        "rehearsal.rs",
        "selfcheck",
        "served_parity.rs",
        "states.rs",
        "wireframe.rs",
    ];

    const BASELINE: &str = "tests/freeze/names.txt";

    /// Every name the compiler must not know: the modules (each dotted
    /// path and each of its parts), the builtins by name and by row id,
    /// and the forms that are not rows.
    fn names() -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for row in crate::table::ROWS.iter().chain(crate::table::FORMS) {
            names.insert(row.name.to_string());
            names.insert(row.id.to_string());
            if !row.written.is_empty() {
                names.insert(row.written.to_string());
            }
            for path in [row.module, row.id.rsplit_once('.').map_or("", |(m, _)| m)] {
                if path.is_empty() {
                    continue;
                }
                names.insert(path.to_string());
                for part in path.split('.') {
                    names.insert(part.to_string());
                }
            }
        }
        for list in [crate::lower::UI_ELEMENTS, crate::lower::UI_CONTROLS] {
            for n in list {
                names.insert(n.to_string());
            }
        }
        // (ui has no rows and no manifest entry yet)
        names.insert("ui".to_string());
        names
    }

    /// The string literals of a Rust source, line comments and char
    /// literals skipped; a raw or byte string's prefix is a letter before
    /// the quote, so it reads as the plain string it holds.
    fn literals(source: &str) -> Vec<String> {
        let chars: Vec<char> = source.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if c == '\'' {
                // a char literal: 'x', '\n', '\'' — a lifetime has no closing quote
                let close = if chars.get(i + 1) == Some(&'\\') { i + 3 } else { i + 2 };
                if chars.get(close) == Some(&'\'') {
                    i = close + 1;
                    continue;
                }
            }
            if c == '"' {
                let mut j = i + 1;
                let mut s = String::new();
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' && j + 1 < chars.len() {
                        s.push(chars[j]);
                        s.push(chars[j + 1]);
                        j += 2;
                        continue;
                    }
                    s.push(chars[j]);
                    j += 1;
                }
                out.push(s);
                i = j + 1;
                continue;
            }
            i += 1;
        }
        out
    }

    /// The rows table is the registry: its `ROWS` and `FORMS` arrays name
    /// every builtin by right, so table.rs is read without them.
    fn without_rows(source: &str) -> String {
        let mut out = String::new();
        let mut inside = false;
        for line in source.lines() {
            if line.starts_with("pub static ROWS") || line.starts_with("pub static FORMS") {
                inside = true;
            }
            if !inside {
                out.push_str(line);
                out.push('\n');
            }
            if inside && line == "];" {
                inside = false;
            }
        }
        out
    }

    fn held(path: &Path, src: &Path) -> bool {
        let rel = path.strip_prefix(src).unwrap_or(path);
        let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
        path.extension().is_some_and(|e| e == "rs") && !TOOLS.contains(&first.as_str())
    }

    fn sources(dir: &Path, src: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).expect("src/ is readable").flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                sources(&p, src, out);
            } else if held(&p, src) {
                out.push(p);
            }
        }
    }

    /// (names, crate references) of one compiler file.
    fn counts(path: &Path, names: &BTreeSet<String>) -> (usize, usize) {
        let mut source = std::fs::read_to_string(path).expect("a source file is readable");
        if path.file_name().is_some_and(|f| f == "table.rs") {
            source = without_rows(&source);
        }
        let named = literals(&source).iter().filter(|s| names.contains(s.as_str())).count();
        let crates = source.matches("crow_web::").count() + source.matches("crow_db::").count();
        (named, crates)
    }

    fn read_baseline(path: &Path) -> BTreeMap<String, (usize, usize)> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut parts = l.split('\t');
                let file = parts.next().unwrap_or_default().to_string();
                let n = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let c = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                (file, (n, c))
            })
            .collect()
    }

    #[test]
    fn the_compiler_learns_no_module_or_builtin_name() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = root.join("src");
        let names = names();
        let mut files = Vec::new();
        sources(&src, &src, &mut files);
        let now: BTreeMap<String, (usize, usize)> = files
            .iter()
            .map(|p| (p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/"), counts(p, &names)))
            .collect();
        let baseline_path = root.join(BASELINE);
        if std::env::var("CROW_FREEZE_BLESS").is_ok() {
            let mut text = String::from(
                "# design rule 24: module and builtin names (tab 1) and module-crate references (tab 2)\n# the compiler's sources hold; src/freeze.rs holds each file to its line, which only shrinks\n",
            );
            for (file, (n, c)) in &now {
                text.push_str(&format!("{file}\t{n}\t{c}\n"));
            }
            std::fs::create_dir_all(baseline_path.parent().expect("a parent")).expect("tests/freeze exists");
            std::fs::write(&baseline_path, text).expect("the baseline is written");
            return;
        }
        let baseline = read_baseline(&baseline_path);
        let mut grew = Vec::new();
        let mut shrank = Vec::new();
        for (file, (n, c)) in &now {
            match baseline.get(file) {
                None => grew.push(format!("{file}: a compiler file not in the baseline ({n} names, {c} crate references)")),
                Some((bn, bc)) => {
                    if n > bn || c > bc {
                        grew.push(format!("{file}: {n} names, {c} crate references (baseline {bn}, {bc})"));
                    } else if n < bn || c < bc {
                        shrank.push(format!("{file}: {n} names, {c} crate references (baseline {bn}, {bc})"));
                    }
                }
            }
        }
        for file in baseline.keys() {
            if !now.contains_key(file) {
                shrank.push(format!("{file}: in the baseline, no longer a compiler file"));
            }
        }
        assert!(
            grew.is_empty(),
            "the compiler learned a module's or a builtin's name — a module is built with the language, not inside it (design rule 24; ask before changing the compiler):\n  {}",
            grew.join("\n  ")
        );
        assert!(
            shrank.is_empty(),
            "the compiler forgot names — lower the baseline so it stays forgotten: CROW_FREEZE_BLESS=1 cargo test freeze\n  {}",
            shrank.join("\n  ")
        );
    }

    #[test]
    fn literals_are_read_past_comments_chars_and_escapes() {
        let src = r#"let a = "push"; // "len" in a comment
            let c = '"'; let d = "a \"quoted\" word"; let e = r"raw";"#;
        assert_eq!(literals(src), vec!["push", "a \\\"quoted\\\" word", "raw"]);
    }

    #[test]
    fn the_rows_array_is_left_out_of_the_table() {
        let src = "fn a() {}\npub static ROWS: &[Row] = &[\n    row!(\"core.len\"),\n];\nfn b() { \"len\" }\n";
        assert_eq!(literals(&without_rows(src)), vec!["len"]);
    }
}
