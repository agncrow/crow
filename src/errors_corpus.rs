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
pub(crate) fn foreign_names(source: &str, message: &str) -> Vec<String> {
    let code: String = source.lines().map(|l| l.split("---").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    // the part of a message about what was written: before its advice (` —
    // `, `: `, a second sentence, `did you mean`), less the forms it offers
    // in their place (`the word 'eq', not '=='`)
    let head = [" — ", ": ", ". ", "did you mean"].iter().fold(message, |h, cut| h.split(cut).next().unwrap_or(h));
    let offered: Vec<&str> = head.split(", not '").collect::<Vec<_>>().iter().rev().skip(1).filter_map(|before| crate::diagnostic::quoted(before).last().copied()).collect();
    let mut names: Vec<String> = crate::diagnostic::quoted(head).into_iter().filter(|q| !offered.contains(q)).map(String::from).collect();
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

// ── mistakes planted in generated programs ─────────────────────────────

/// One mistake planted in a program that runs: the line it is on
/// (1-based), when it has one, and what the refusal must say to teach its
/// fix.
struct Planted {
    what: String,
    line: Option<usize>,
    teaches: String,
}

/// A seeded generator of small choices (xorshift).
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

/// The builtins a slip is planted in: names of five letters or more, so a
/// swapped pair is one slip from them and from no other name.
const SLIPPABLE: &[&str] = &["upper", "lower", "split", "strip", "round", "floor", "contains", "reverse", "concat", "values", "replace"];

/// The mistakes a program's lines allow, each with the line changed: the
/// program as it runs, one mistake in it.
fn plantable(lines: &[String]) -> Vec<(Vec<String>, Planted)> {
    let mut out = Vec::new();
    let code = |l: &str| l.split("---").next().unwrap_or("").to_string();
    for (i, l) in lines.iter().enumerate() {
        let c = code(l);
        let n = i + 1;
        // a slip in a builtin's name: two letters swapped
        for name in SLIPPABLE {
            let call = format!("{}[", name);
            if let Some(at) = c.find(&call) {
                let before_ok = at == 0 || !c[..at].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.');
                if !before_ok {
                    continue;
                }
                // (two neighbours that differ: `upper`'s two p's swapped
                // would be `upper`)
                let chars: Vec<char> = name.chars().collect();
                let Some(at) = (1..chars.len() - 1).find(|&k| chars[k] != chars[k + 1]) else { continue };
                let mut slipped = chars.clone();
                slipped.swap(at, at + 1);
                let slipped: String = slipped.into_iter().collect();
                let mut mutated = lines.to_vec();
                mutated[i] = l.replacen(&call, &format!("{}[", slipped), 1);
                out.push((mutated, Planted { what: format!("{} for {}", slipped, name), line: Some(n), teaches: format!("did you mean '{}'", name) }));
                break;
            }
        }
        // a call written with parentheses, its brackets on the line
        if let Some(open) = c.find(|ch: char| ch.is_alphabetic()).and_then(|s| c[s..].find('[').map(|o| s + o)) {
            let before = &c[..open];
            if before.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                let mut depth = 0i32;
                let mut close = None;
                for (k, ch) in c[open..].char_indices() {
                    match ch {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(open + k);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(close) = close {
                    let mut chars: Vec<char> = l.chars().collect();
                    let (o, cl) = (c[..open].chars().count(), c[..close].chars().count());
                    chars[o] = '(';
                    chars[cl] = ')';
                    let mut mutated = lines.to_vec();
                    mutated[i] = chars.into_iter().collect();
                    out.push((mutated, Planted { what: "a call with ( )".into(), line: Some(n), teaches: "square brackets".into() }));
                }
            }
        }
        // a word operator written as another language's symbol
        for (word, symbol, teach) in [(" eq ", " == ", "'eq'"), (" and ", " && ", "'and'"), (" gt ", " > ", "'gt'"), (" ne ", " != ", "'ne'")] {
            if c.contains(word) {
                let mut mutated = lines.to_vec();
                mutated[i] = l.replacen(word, symbol, 1);
                out.push((mutated, Planted { what: format!("{} for {}", symbol.trim(), word.trim()), line: Some(n), teaches: teach.into() }));
                break;
            }
        }
        // a bracket never closed: the line's last `]` dropped, a line after it
        if c.trim_end().ends_with(']') && i + 1 < lines.len() && !c.trim_start().starts_with("fn ") && !c.trim_start().starts_with("struct ") {
            let at = l.rfind(']').unwrap_or(0);
            let mut mutated = lines.to_vec();
            mutated[i] = format!("{}{}", &l[..at], &l[at + 1..]);
            out.push((mutated, Planted { what: "a ']' left out".into(), line: Some(n), teaches: "never closed".into() }));
        }
        // two statements on one line: an assignment and the plain line after it
        if let Some(next) = lines.get(i + 1) {
            let (ci, cn) = (code(l), code(next));
            let indent = |s: &str| s.len() - s.trim_start().len();
            let plain = |s: &str| {
                let t = s.trim_start();
                !t.is_empty() && !["fn ", "if ", "else", "for ", "while ", "struct ", "use ", "return", "test ", "view ", "derive "].iter().any(|k| t.starts_with(k)) && !t.contains("= for ")
            };
            let after_plain = lines.get(i + 2).is_none_or(|a| indent(&code(a)) <= indent(&ci));
            if plain(&ci) && plain(&cn) && indent(&ci) == indent(&cn) && ci.contains(" = ") && after_plain && i > 0 && indent(&code(&lines[i - 1])) <= indent(&ci) {
                let mut mutated = lines.to_vec();
                mutated[i] = format!("{} {}", ci.trim_end(), cn.trim_start());
                mutated.remove(i + 1);
                out.push((mutated, Planted { what: "two statements on a line".into(), line: Some(n), teaches: "one statement".into() }));
            }
        }
    }
    // a module's import left out, where the program calls it: its calls
    // say the import, once — frame, which the generated part imports too
    // (each file's imports are its own), and db where the program connects
    let calls_db = lines.iter().any(|l| l.contains("connect[") || l.contains("query["));
    if let Some((i, l)) = lines.iter().enumerate().find(|(_, l)| l.starts_with("use ")) {
        if l.contains(" frame,") {
            let mut mutated = lines.to_vec();
            mutated[i] = l.replacen(" frame,", "", 1);
            out.push((mutated, Planted { what: "use frame left out".into(), line: None, teaches: "add `use frame`".into() }));
        }
        if calls_db && l.contains(" db,") {
            let mut mutated = lines.to_vec();
            mutated[i] = l.replacen(" db,", "", 1);
            out.push((mutated, Planted { what: "use db left out".into(), line: None, teaches: "add `use db`".into() }));
        }
    }
    out
}

/// `crow test --fuzz n --mistakes [--seed s] [--keep]`: programs the
/// generator makes, each with one mistake planted (a slip in a name, a call
/// written with `( )`, Python's operator, a bracket left open, two
/// statements on a line, an import left out), held to the rules every
/// refusal keeps — refused, once, at the line of the mistake, naming only
/// what the program wrote, never as Crow's own bug, the same in both modes
/// and in the language server — and to the fix the mistake asks for.
pub fn cmd_test_mistakes(args: &[String]) -> i32 {
    let n: usize = args.iter().filter(|a| !a.starts_with("--")).find_map(|a| a.parse().ok()).unwrap_or(10);
    let seed: u64 = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1));
    let keep = args.iter().any(|a| a == "--keep");
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("crow"));
    let dir = Path::new("target").join("crow_mistakes");
    let _ = std::fs::create_dir_all(&dir);
    println!("mistakes: {} generated programs from seed {}, one mistake planted in each (crow test --fuzz {} --mistakes --seed {} repeats it)\n", n, seed, n, seed);
    let start = std::time::Instant::now();
    let mut failed = 0;
    let mut refused_as_made = 0;
    for k in 0..n {
        let s = seed.wrapping_add(k as u64);
        let (source, module_name) = crate::fuzz::program(s, 4);
        let _ = std::fs::write(dir.join(format!("{}.co", module_name)), crate::fuzz::module(&module_name));
        // a program refused as generated says nothing of a mistake planted
        // in it: it is passed over, and counted
        let base = format!("base_{}.co", s);
        let _ = std::fs::write(dir.join(&base), &source);
        let (base_ok, base_out) = run(&exe, &["run", "-i", &base], &dir);
        // (kept with --keep: a program refused as generated is the
        // generator's slip or the compiler's, and is read to tell which)
        if !keep {
            let _ = std::fs::remove_file(dir.join(&base));
        }
        if !base_ok {
            refused_as_made += 1;
            println!("  --    seed {}: refused as generated ({})", s, base_out.lines().find(|l| l.contains(": ")).unwrap_or("").trim());
            continue;
        }
        let lines: Vec<String> = source.lines().map(String::from).collect();
        let options = plantable(&lines);
        if options.is_empty() {
            println!("  --    seed {}: no mistake to plant", s);
            continue;
        }
        let mut draw = Draw(s.wrapping_mul(2654435761).wrapping_add(1));
        // a kind of mistake first, then where: each kind as likely as another
        let mut kinds: Vec<&str> = options.iter().map(|(_, p)| p.what.split(" for ").last().unwrap_or(&p.what)).collect();
        kinds.sort();
        kinds.dedup();
        let kind = kinds[draw.below(kinds.len())].to_string();
        let of_kind: Vec<&(Vec<String>, Planted)> = options.iter().filter(|(_, p)| p.what.split(" for ").last().unwrap_or(&p.what) == kind).collect();
        let (mutated, planted) = of_kind[draw.below(of_kind.len())];
        let file = format!("mistake_{}.co", s);
        let text = mutated.join("\n") + "\n";
        let _ = std::fs::write(dir.join(&file), &text);
        let (i_ok, i_out) = run(&exe, &["run", "-i", &file], &dir);
        let mut why: Vec<String> = Vec::new();
        if i_ok {
            why.push("it ran: the mistake was not refused".into());
        } else {
            let (t_ok, t_out) = run(&exe, &["run", &file], &dir);
            if t_ok != i_ok || t_out != i_out {
                why.push(format!("the modes differ:\n          compiled: {}", t_out.trim().replace('\n', "\n                    ")));
            }
            if i_out.contains("a bug in Crow") || i_out.contains("internal error") {
                why.push("refused as a bug in Crow".into());
            }
            let entries = printed(&file, &i_out);
            if entries.len() != 1 {
                why.push(format!("{} messages for one mistake", entries.len()));
            }
            if let (Some(want), Some((Some(got), _, _))) = (planted.line, entries.first())
                && *got != want
            {
                why.push(format!("pointed at line {}, the mistake is on line {}", got, want));
            }
            if entries.iter().any(|(_, m, _)| !foreign_names(&text, m).is_empty()) {
                why.push("names what the program never wrote".into());
            }
            if !i_out.contains(&planted.teaches) {
                why.push(format!("does not say {}", planted.teaches));
            }
            let analysis = crate::lsp::analysis::analyze(&text, Some(&dir), None, &file);
            let mut editor: Vec<(Option<usize>, String)> = analysis.diagnostics.iter().map(|d| (Some(d.range.start.line as usize + 1), d.message.clone())).collect();
            editor.sort();
            editor.dedup();
            let mut terminal: Vec<(Option<usize>, String)> = entries.iter().map(|(l, m, _)| (*l, m.clone())).collect();
            terminal.sort();
            if editor != terminal {
                why.push(format!("the editor says {:?}", editor));
            }
        }
        if why.is_empty() {
            println!("  ok    seed {}: {}", s, planted.what);
            if !keep {
                let _ = std::fs::remove_file(dir.join(&file));
            }
        } else {
            failed += 1;
            println!("  FAIL  seed {}: {} (kept as {})", s, planted.what, dir.join(&file).display());
            for w in &why {
                println!("        {}", w);
            }
            let first = i_out.lines().next().unwrap_or("(nothing)");
            println!("        said: {}", first);
        }
    }
    println!("\n{} programs, {} failed, {} refused as generated ({:.1}s)", n, failed, refused_as_made, start.elapsed().as_secs_f64());
    i32::from(failed > 0)
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

    /// Each kind of mistake is planted where the program allows it, at the
    /// line it is on, and a slip is one swap of two letters that differ
    #[test]
    fn mistakes_are_planted_where_the_lines_allow() {
        let lines: Vec<String> = "use math, db, frame\nc = connect['sqlite://:memory:']\nfn f[x]\n    a = upper[x]\n    b = len[a]\n    if a eq 'X'\n        log[a]\n    return b\nlog[f['x']]".lines().map(String::from).collect();
        let planted = plantable(&lines);
        let says = |w: &str| planted.iter().any(|(_, p)| p.what.contains(w));
        for w in ["upepr for upper", "a call with ( )", "== for eq", "two statements on a line", "use db left out", "a ']' left out"] {
            assert!(says(w), "{}: {:?}", w, planted.iter().map(|(_, p)| &p.what).collect::<Vec<_>>());
        }
        let slip = planted.iter().find(|(_, p)| p.what.starts_with("upepr")).map(|(m, p)| (m[3].clone(), p.line));
        assert_eq!(slip, Some(("    a = upepr[x]".to_string(), Some(4))));
        // the same seed draws the same mistake
        assert_eq!((Draw(7).below(10), Draw(7).below(10)), (Draw(7).below(10), Draw(7).below(10)));
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
