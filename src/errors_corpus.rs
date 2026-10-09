//! `crow test --errors`: the corpus of wrong programs (`tests/errors/`),
//! each run in both modes and through the language server, its refusal held
//! to what was reviewed (`<name>.expect`) and to the rules every message
//! keeps (spec, Error Messages). `--bless` writes what the programs answer
//! now, for review.
//!
//! A case's first comment lines say what it is:
//!
//! ```text
//! --- frame.new without `use frame`
//! --- mistakes: 2
//! --- known: names, count — frame.new is reported as the `with` calls it becomes
//! ```
//!
//! `mistakes` (one when unsaid) is how many entries the refusal must have;
//! `runs` marks a program that runs on, its failure a value it shows, not a
//! refusal (`log[int['abc'] + 1]`).
//! `known` lists the rules the case breaks today; a rule listed that holds
//! fails the case until it is struck, so the list only shrinks.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A refusal's entry as a harness reads it: its line, its message, and the
/// character ranges its line has underlined.
type Entry = (Option<usize>, String, Vec<(usize, usize)>);

/// The rules a case's answer is held to, by the name `known` gives them.
const RULES: &[(&str, &str)] = &[
    ("refused", "a wrong program is refused or stops"),
    ("names", "a message names only what the program wrote"),
    ("count", "one mistake, one message"),
    ("place", "every message has its line (and a refusal its column)"),
    ("internal", "a program is never refused as a bug in Crow"),
    ("editor", "the language server says what the terminal says, at the same text"),
];

struct Case {
    name: String,
    path: PathBuf,
    source: String,
    title: String,
    mistakes: usize,
    runs: bool,
    known: BTreeSet<String>,
}

fn case_of(path: &Path) -> Case {
    let source = std::fs::read_to_string(path).unwrap_or_default();
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (mut title, mut mistakes, mut runs, mut known) = (String::new(), 1, false, BTreeSet::new());
    for line in source.lines().take_while(|l| l.starts_with("---")) {
        let body = line.trim_start_matches('-').trim();
        if body == "runs" || body.starts_with("runs:") {
            runs = true;
        } else if let Some(n) = body.strip_prefix("mistakes:") {
            mistakes = n.trim().parse().unwrap_or(1);
        } else if let Some(k) = body.strip_prefix("known:") {
            let rules = k.split(" — ").next().unwrap_or(k);
            known.extend(rules.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()));
        } else if title.is_empty() {
            title = body.to_string();
        }
    }
    Case { name, path: path.to_path_buf(), source, title, mistakes, runs, known }
}

/// What a run printed: its standard output, then its error output, with a
/// compiled build's progress lines left out.
fn run(exe: &Path, args: &[&str], dir: &Path) -> (bool, String) {
    let out = std::process::Command::new(exe)
        .args(args)
        .current_dir(dir)
        .env("CROW_BUILD_PROFILE", "dev")
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) => {
            let noise = ["Compiling", "Finished", "Updating", "Locking", "Adding", "Downloaded", "Downloading", "Blocking", "Building"];
            let err: Vec<String> = String::from_utf8_lossy(&o.stderr)
                .lines()
                .filter(|l| !noise.iter().any(|n| l.trim_start().starts_with(n)))
                .map(String::from)
                .collect();
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            if !err.is_empty() {
                text.push_str(&err.join("\n"));
                text.push('\n');
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("failed to start crow: {}\n", e)),
    }
}

/// A refusal as printed, read back: each entry's line, its message, and the
/// ranges its caret line marks (0-based characters).
fn printed(file: &str, out: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let head = format!("{}:", file);
    for l in out.lines() {
        if let Some(rest) = l.strip_prefix(&head) {
            let mut parts = rest.splitn(3, ':');
            let (a, b, c) = (parts.next(), parts.next(), parts.next());
            match (a.and_then(|a| a.parse::<usize>().ok()), b.and_then(|b| b.parse::<usize>().ok()), c) {
                (Some(line), Some(_), Some(msg)) => entries.push((Some(line), msg.trim().to_string(), Vec::new())),
                (Some(line), None, _) => entries.push((Some(line), rest.split_once(": ").map_or("", |(_, m)| m).to_string(), Vec::new())),
                _ => entries.push((None, rest.trim().to_string(), Vec::new())),
            }
        } else if let Some((gutter, marks)) = l.split_once(" │ ")
            && gutter.trim().is_empty()
            && marks.contains('^')
        {
            {
                if let Some(last) = entries.last_mut() {
                    let chars: Vec<char> = marks.chars().collect();
                    let mut i = 0;
                    while i < chars.len() {
                        if chars[i] == '^' {
                            let s = i;
                            while i < chars.len() && chars[i] == '^' {
                                i += 1;
                            }
                            last.2.push((s, i));
                        } else {
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    entries
}

/// The names a message speaks of before its advice — quoted, or called
/// with `[]` — that the program's code does not hold; and any name of the
/// compiler's own (`'<expression>'`, a token's name).
fn foreign_names(source: &str, message: &str) -> Vec<String> {
    let code: String = source.lines().map(|l| l.split("---").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let head = message.split(" — ").next().unwrap_or(message).split(": add `use").next().unwrap_or(message).split("did you mean").next().unwrap_or(message);
    let mut names: Vec<String> = crate::diagnostic::quoted(head).into_iter().map(String::from).collect();
    names.extend(crate::diagnostic::subjects(head));
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut foreign = Vec::new();
    for n in names {
        let internal = n.starts_with('<') || ["INDENT", "DEDENT", "NEWLINE", "EOF"].contains(&n.as_str()) || n.starts_with("__");
        let named = !n.is_empty() && n.chars().all(|c| word(c) || c == '.');
        let held = || {
            let last = n.rsplit('.').next().unwrap_or(&n);
            code.match_indices(last).any(|(i, _)| {
                let before = code[..i].chars().last();
                let after = code[i + last.len()..].chars().next();
                before.is_none_or(|c| !word(c)) && after.is_none_or(|c| !word(c))
            })
        };
        if (internal || (named && !held())) && !foreign.contains(&n) {
            foreign.push(n);
        }
    }
    foreign
}

struct Outcome {
    failures: Vec<String>,
    answer: String,
}

fn check(case: &Case, exe: &Path, dir: &Path, bless: bool) -> Outcome {
    let _ = std::fs::create_dir_all(dir);
    let file = format!("{}.co", case.name);
    let _ = std::fs::write(dir.join(&file), &case.source);
    let (i_ok, i_out) = run(exe, &["run", "-i", &file], dir);
    let (t_ok, t_out) = run(exe, &["run", &file], dir);
    let mut failures = Vec::new();
    let mut broken: BTreeSet<&str> = BTreeSet::new();

    // both modes, one answer
    if i_out != t_out || i_ok != t_ok {
        failures.push(format!("the modes differ:\n          interpreted: {}\n          compiled:    {}", i_out.trim().replace('\n', "\n                       "), t_out.trim().replace('\n', "\n                       ")));
    }
    let stops = case.runs || i_out.contains("runtime error") || (!i_ok && !i_out.contains(&format!("{}:", file)));
    let entries = printed(&file, &i_out);
    // a stop's or a shown failure's lines
    let failed: Vec<&str> = i_out.lines().filter(|l| l.starts_with("runtime error") || l.starts_with("error:")).collect();

    if i_ok && !case.runs {
        broken.insert("refused");
    }
    if i_out.contains("a bug in Crow") || i_out.contains("internal error") {
        broken.insert("internal");
    }
    if entries.iter().any(|(_, m, _)| !foreign_names(&case.source, m).is_empty()) {
        broken.insert("names");
    }
    let counted = if stops { failed.len() } else { entries.len() };
    let answered = !i_ok || case.runs;
    if answered && counted != case.mistakes {
        broken.insert("count");
    }
    let placed = if stops {
        failed.iter().all(|l| l.contains("line "))
    } else {
        entries.iter().all(|(line, _, marks)| line.is_some() && !marks.is_empty())
    };
    if answered && !placed {
        broken.insert("place");
    }
    // the editor: the same messages on the same lines, the same text underlined
    if !i_ok && !stops {
        let analysis = crate::lsp::analysis::analyze(&case.source, Some(dir), None, &file);
        let mut editor: Vec<Entry> = Vec::new();
        for d in &analysis.diagnostics {
            let line = Some(d.range.start.line as usize + 1);
            let range = (d.range.start.character as usize, d.range.end.character as usize);
            match editor.iter_mut().find(|e| e.0 == line && e.1 == d.message) {
                Some(e) => e.2.push(range),
                None => editor.push((line, d.message.clone(), vec![range])),
            }
        }
        let mut terminal: Vec<_> = entries.iter().map(|(l, m, r)| (*l, m.clone(), r.clone())).collect();
        // (the editor lists them as the checks find them, the terminal by place)
        editor.sort();
        terminal.sort();
        if editor != terminal {
            broken.insert("editor");
            if !case.known.contains("editor") {
                failures.push(format!("the editor says {:?}\n          the terminal {:?}", editor, terminal));
            }
        }
    }
    for rule in &broken {
        if !case.known.contains(*rule) {
            let what = RULES.iter().find(|(r, _)| r == rule).map(|(_, w)| *w).unwrap_or("");
            failures.push(format!("breaks `{}`: {}", rule, what));
        }
    }
    for rule in &case.known {
        if !RULES.iter().any(|(r, _)| r == rule) {
            failures.push(format!("knows a rule there is none of: `{}`", rule));
        } else if !broken.contains(rule.as_str()) {
            failures.push(format!("keeps `{}` now: strike it from `known`", rule));
        }
    }

    // the answer reviewed
    let expect = case.path.with_extension("expect");
    if bless {
        let _ = std::fs::write(&expect, &i_out);
    } else {
        match std::fs::read_to_string(&expect) {
            Ok(want) if want == i_out => {}
            Ok(want) => failures.push(format!("answers otherwise than reviewed:\n{}", diff(&want, &i_out))),
            Err(_) => failures.push(format!("has no {} (crow test --errors --bless writes it)", expect.display())),
        }
    }
    Outcome { failures, answer: i_out }
}

/// The lines two answers differ on, marked.
fn diff(want: &str, got: &str) -> String {
    let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
    let mut out = String::new();
    for i in 0..w.len().max(g.len()) {
        match (w.get(i), g.get(i)) {
            (Some(a), Some(b)) if a == b => {}
            (a, b) => {
                if let Some(a) = a {
                    out.push_str(&format!("          - {}\n", a));
                }
                if let Some(b) = b {
                    out.push_str(&format!("          + {}\n", b));
                }
            }
        }
    }
    out.trim_end().to_string()
}

pub fn cmd_test_errors(args: &[String]) -> i32 {
    let bless = args.iter().any(|a| a == "--bless");
    let only: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let dir = Path::new("tests/errors");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "co")).collect())
        .unwrap_or_default();
    paths.retain(|p| only.is_empty() || only.iter().any(|w| p.to_string_lossy().contains(w.as_str())));
    paths.sort();
    if paths.is_empty() {
        eprintln!("error: no programs in tests/errors/ match (run from the repository's root)");
        return 1;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("crow"));
    let cases: Vec<Case> = paths.iter().map(|p| case_of(p)).collect();
    let root = Path::new("target/crow_errors");
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6);
    let start = std::time::Instant::now();
    println!("errors: {} wrong programs, both modes and the language server\n", cases.len());
    let results: Vec<std::sync::Mutex<Option<Outcome>>> = cases.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for w in 0..workers {
            let (cases, results, next, exe) = (&cases, &results, &next, &exe);
            let dir = root.join(format!("w{}", w));
            scope.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= cases.len() {
                    break;
                }
                let r = check(&cases[i], exe, &dir, bless);
                if let Ok(mut slot) = results[i].lock() {
                    *slot = Some(r);
                }
            });
        }
    });
    let mut failed = 0;
    let mut known = 0;
    for (case, r) in cases.iter().zip(results) {
        let Some(r) = r.into_inner().ok().flatten() else { continue };
        known += case.known.len();
        if r.failures.is_empty() {
            let note = if case.known.is_empty() { String::new() } else { format!(" (known: {})", case.known.iter().cloned().collect::<Vec<_>>().join(", ")) };
            println!("  ok    {}{}", case.name, note);
        } else {
            failed += 1;
            println!("  FAIL  {} — {}", case.name, case.title);
            for f in &r.failures {
                println!("        {}", f);
            }
            if bless {
                println!("        answers:\n          {}", r.answer.trim().replace('\n', "\n          "));
            }
        }
    }
    println!(
        "\n{} programs, {} failed, {} rules known broken ({:.1}s)",
        cases.len(),
        failed,
        known,
        start.elapsed().as_secs_f64()
    );
    if failed > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_the_program_never_wrote_is_foreign() {
        let src = "ours = frame.new[node = keys[m]]\n";
        assert_eq!(foreign_names(src, "frame has no 'with'"), vec!["with"]);
        assert!(foreign_names(src, "frame.new[] is frame's — add `use frame`").is_empty());
        assert!(foreign_names(src, "undefined variable 'm'").is_empty());
        assert_eq!(foreign_names(src, "expected 'INDENT', got 'ours'"), vec!["INDENT"]);
        assert_eq!(foreign_names(src, "'<expression>' takes no named arguments"), vec!["<expression>"]);
        // advice names what to write
        assert!(foreign_names("lenn[x]\n", "undefined function 'lenn' — did you mean 'len'?").is_empty());
    }

    #[test]
    fn a_printed_refusal_reads_back() {
        let out = "a.co:1:30: undefined variable 'm'\n  1 │ ours = frame.new[node = keys[m], value = values[m]]\n    │                              ^                  ^\na.co: no place\n";
        assert_eq!(printed("a.co", out), vec![(Some(1), "undefined variable 'm'".to_string(), vec![(29, 30), (48, 49)]), (None, "no place".to_string(), vec![])]);
    }

    #[test]
    fn a_case_says_what_it_is() {
        let dir = std::env::temp_dir().join(format!("crow_errors_case_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("x.co");
        std::fs::write(&p, "--- a slip\n--- mistakes: 2\n--- known: names, count — why\nlog[1]\n").unwrap();
        let c = case_of(&p);
        assert_eq!((c.title.as_str(), c.mistakes, c.runs), ("a slip", 2, false));
        assert_eq!(c.known.iter().cloned().collect::<Vec<_>>(), vec!["count", "names"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
