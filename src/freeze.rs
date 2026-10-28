//! The compiler holds the language and names no module and no builtin
//! (design rule 24; docs/extending-crow.md). This test reads, in every
//! source file of the compiler, the string literals shaped like a name
//! (`[a-z_][a-z0-9_]*`, as every module's and builtin's name is) and the
//! references to a module crate (`crow_web::`, `crow_db::`), and holds
//! each file to tests/freeze/names.txt: the set of such literals the file
//! held when the baseline was blessed, and its count of crate references.
//! A literal that is a module's or a builtin's name now and is not in the
//! file's set fails: the file learned a name. A name in the set that the
//! file no longer holds fails too, naming the file, so that the baseline
//! is lowered (`CROW_FREEZE_BLESS=1 cargo test freeze`) and committed: the
//! names a file may hold only shrink. A literal that is no name is held
//! to nothing, and recorded: a module that later exports that word is
//! not the compiler learning it (magpie's `get` had met the emitter's
//! own `"get"`, the ui state's, and the counts could not tell the two
//! apart — a literal that predates an export is the export's problem to
//! avoid, not the compiler's; one added after the last bless is told
//! apart by the bless, which a person does after looking). The tools of
//! the binary (the checker, the baseline, the database checks, the
//! self-check, the harnesses) are not held: they may know a module,
//! reading its facts from the registry.

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
        // (a module written in Crow: the functions its source exports)
        for m in crate::modules::MODULES {
            for e in crate::modules::exports(m.path) {
                names.insert(e.to_string());
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
    /// every builtin by right, and a handle's declaration (`pub const DB:
    /// Handle = …`, one line each) names its module and the runtime type
    /// it is held as, so table.rs is read without them.
    fn without_rows(source: &str) -> String {
        let mut out = String::new();
        let mut inside = false;
        for line in source.lines() {
            if line.starts_with("pub static ROWS") || line.starts_with("pub static FORMS") {
                inside = true;
            }
            let declares_handle = line.starts_with("pub const ") && line.contains(": Handle = Handle {");
            if !inside && !declares_handle {
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

    /// A file without its test module (`#[cfg(test)] mod tests { … }`, closed
    /// at the column it opened): a test names what it holds the compiler to,
    /// which is not the compiler learning it.
    fn without_tests(source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < lines.len() {
            let opens_tests = lines[i].trim() == "#[cfg(test)]" && lines.get(i + 1).is_some_and(|l| l.starts_with("mod ") || l.starts_with("pub mod ") || l.starts_with("pub(crate) mod "));
            if opens_tests {
                i += 2;
                while i < lines.len() && lines[i] != "}" {
                    i += 1;
                }
                i += 1;
                continue;
            }
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
        }
        out
    }

    /// Whether a literal is shaped like a name: what a module's or a
    /// builtin's name is (`[a-z_][a-z0-9_]*`, at most 32 characters).
    fn name_shaped(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 32
            && s.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
            && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    /// (the name-shaped literals, crate references) of one compiler file.
    fn held_in(path: &Path) -> (BTreeSet<String>, usize) {
        let mut source = without_tests(&std::fs::read_to_string(path).expect("a source file is readable"));
        if path.file_name().is_some_and(|f| f == "table.rs") {
            source = without_rows(&source);
        }
        let words: BTreeSet<String> = literals(&source).into_iter().filter(|s| name_shaped(s)).collect();
        let crates = source.matches("crow_web::").count() + source.matches("crow_db::").count();
        (words, crates)
    }

    /// The baseline: per file, its crate-reference count and the
    /// name-shaped literals it held at the bless (`file\tcrates\ta,b,c`).
    fn read_baseline(path: &Path) -> BTreeMap<String, (usize, BTreeSet<String>)> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut parts = l.split('\t');
                let file = parts.next().unwrap_or_default().to_string();
                let c = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let words: BTreeSet<String> = parts.next().unwrap_or_default().split(',').filter(|w| !w.is_empty()).map(str::to_string).collect();
                (file, (c, words))
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
        let now: BTreeMap<String, (BTreeSet<String>, usize)> = files
            .iter()
            .map(|p| (p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/"), held_in(p)))
            .collect();
        let baseline_path = root.join(BASELINE);
        if std::env::var("CROW_FREEZE_BLESS").is_ok() {
            let mut text = String::from(
                "# design rule 24: per compiler file, its module-crate references (tab 1) and the name-shaped string literals it held at the bless (tab 2)\n# src/freeze.rs holds each file to its line: a literal that is a module's or a builtin's name must be in the set, and a name in the set must still be held — the names only shrink\n",
            );
            for (file, (words, c)) in &now {
                let list: Vec<&str> = words.iter().map(String::as_str).collect();
                text.push_str(&format!("{file}\t{c}\t{}\n", list.join(",")));
            }
            std::fs::create_dir_all(baseline_path.parent().expect("a parent")).expect("tests/freeze exists");
            std::fs::write(&baseline_path, text).expect("the baseline is written");
            return;
        }
        let baseline = read_baseline(&baseline_path);
        let mut grew = Vec::new();
        let mut shrank = Vec::new();
        for (file, (words, c)) in &now {
            match baseline.get(file) {
                None => grew.push(format!("{file}: a compiler file not in the baseline ({} name-shaped literals, {c} crate references)", words.len())),
                Some((bc, held)) => {
                    let learned: Vec<&str> = words.iter().filter(|w| names.contains(*w) && !held.contains(*w)).map(String::as_str).collect();
                    if !learned.is_empty() {
                        grew.push(format!("{file}: {} (a literal the file held before the name was a module's or a builtin's is blessed after looking: CROW_FREEZE_BLESS=1 cargo test freeze)", learned.join(", ")));
                    }
                    if c > bc {
                        grew.push(format!("{file}: {c} crate references (baseline {bc})"));
                    }
                    let forgotten: Vec<&str> = held.iter().filter(|w| names.contains(*w) && !words.contains(*w)).map(String::as_str).collect();
                    if !forgotten.is_empty() {
                        shrank.push(format!("{file}: no longer holds {}", forgotten.join(", ")));
                    }
                    if c < bc {
                        shrank.push(format!("{file}: {c} crate references (baseline {bc})"));
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
    fn a_name_shaped_literal_is_a_lowercase_word() {
        assert!(name_shaped("get") && name_shaped("batch_asof") && name_shaped("_pos") && name_shaped("p90"));
        assert!(!name_shaped("") && !name_shaped("Get") && !name_shaped("a word") && !name_shaped("crow_web::") && !name_shaped("1st"));
    }

    #[test]
    fn literals_are_read_past_comments_chars_and_escapes() {
        let src = r#"let a = "push"; // "len" in a comment
            let c = '"'; let d = "a \"quoted\" word"; let e = r"raw";"#;
        assert_eq!(literals(src), vec!["push", "a \\\"quoted\\\" word", "raw"]);
    }

    #[test]
    fn the_rows_array_is_left_out_of_the_table() {
        let src = "fn a() {}\npub static ROWS: &[Row] = &[\n    row!(\"core.len\"),\n];\npub const DB: Handle = Handle { name: \"db\", rust: \"crow_db::DbHandle\" };\nfn b() { \"len\" }\n";
        assert_eq!(literals(&without_rows(src)), vec!["len"]);
    }
}
