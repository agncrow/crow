mod ast;
mod checker;
mod db_check;
mod lsp;
mod interpreter;
mod lexer;
mod lower;
mod parallel;
mod parser;
mod token;
mod transpiler;
mod type_infer;
mod validator;
mod formatter;
mod modules;
mod value;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use std::process;
use std::collections::HashMap;

/// Load `.env` file from the current directory if it exists.
/// Each line should be `KEY=VALUE`. Lines starting with `#` are comments.
/// Empty lines and lines without `=` are skipped.
fn load_dotenv() {
    let path = std::path::Path::new(".env");
    if !path.exists() {
        return;
    }
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(pos) = trimmed.find('=') {
                let key = trimmed[..pos].trim();
                let val = trimmed[pos + 1..].trim();
                // Strip surrounding quotes if present
                let val = if (val.starts_with('\'') && val.ends_with('\''))
                    || (val.starts_with('"') && val.ends_with('"'))
                {
                    &val[1..val.len() - 1]
                } else {
                    val
                };
                // Only set if not already set (real env vars take priority)
                if std::env::var(key).is_err() {
                    // SAFETY: load_dotenv runs once at startup before any threads are spawned.
                    unsafe { std::env::set_var(key, val); }
                }
            }
        }
    }
}

fn main() {
    load_dotenv();
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        cmd_repl();
        return;
    }

    match args[1].as_str() {
        "-v" => {
            println!("(o.o) crow {}", env!("CARGO_PKG_VERSION"));
        }
        "new" => cmd_new(),
        "install" => cmd_install(),
        "fmt" => cmd_fmt(),
        "build" => cmd_build(),
        "launch" => cmd_launch(),
        "check" => {
            let mut verbose = false;
            let mut strict = false;
            let mut db_url: Option<String> = None;
            let mut file: Option<String> = None;
            let mut rest = args.iter().skip(2);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--verbose" => verbose = true,
                    "--strict" => strict = true,
                    "--db" => {
                        db_url = rest.next().cloned();
                        if db_url.is_none() {
                            eprintln!("--db needs a url, e.g. crow check --db 'sqlite://dev.db'");
                            process::exit(1);
                        }
                    }
                    f if !f.starts_with('-') => file = Some(f.to_string()),
                    other => {
                        eprintln!("unknown flag '{}' for crow check", other);
                        if other == "-v" {
                            eprintln!("  did you mean 'crow -v' or 'crow check --verbose'?");
                        }
                        process::exit(1);
                    }
                }
            }
            let filename = file.unwrap_or_else(|| {
                let config = parse_crowfile();
                config.entry
            });
            cmd_check(&filename, verbose, db_url, strict);
        }
        "lsp" => {
            if let Err(e) = lsp::run_server() {
                eprintln!("crow lsp: {}", e);
                process::exit(1);
            }
        }
        "fix" => {
            let mut hazard_n: Option<usize> = None;
            let mut option_m: usize = 1;
            let mut db_url: Option<String> = None;
            let mut file: Option<String> = None;
            let mut rest = args.iter().skip(2);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--hazard" => hazard_n = rest.next().and_then(|v| v.parse().ok()),
                    "--option" => {
                        option_m = rest.next().and_then(|v| v.parse().ok()).unwrap_or(1)
                    }
                    "--db" => db_url = rest.next().cloned(),
                    f if !f.starts_with('-') => file = Some(f.to_string()),
                    other => {
                        eprintln!("unknown flag '{}' for crow fix", other);
                        process::exit(1);
                    }
                }
            }
            // No --hazard: run the check and print the numbered menu, so
            // the N crow fix asks for is discoverable from crow fix itself.
            if let (Some(filename), None) = (&file, hazard_n) {
                let source = read_file(filename);
                let url = db_url.clone().or_else(crowfile_db_url);
                match db_check::run(filename, &source, url.as_deref()) {
                    Ok(result) => {
                        let ordered = db_check::report::ordered(&result.hazards);
                        if ordered.is_empty() {
                            println!("no fixable hazards — database checks are clean");
                        } else {
                            println!("hazards in {}:", filename);
                            for (i, h) in ordered.iter().enumerate() {
                                println!(
                                    "  {}. line {} — {}",
                                    i + 1,
                                    h.span.0,
                                    db_check::report::headline(&h.kind)
                                );
                            }
                            println!("\napply one with: crow fix {} --hazard N", filename);
                        }
                        process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("error: {}", e);
                        process::exit(1);
                    }
                }
            }
            let (Some(filename), Some(hazard_n)) = (file, hazard_n) else {
                eprintln!("usage: crow fix file.crow --hazard N [--option M] [--db url]");
                eprintln!("  N and the option numbers come from the crow check report");
                process::exit(1);
            };
            let source = read_file(&filename);
            let resolved_db = db_url.or_else(crowfile_db_url);
            match db_check::fix::apply(
                &filename,
                &source,
                resolved_db.as_deref(),
                hazard_n,
                option_m,
            ) {
                Ok(log) => print!("{}", log),
                Err(e) => {
                    eprintln!("crow fix: {}", e);
                    process::exit(1);
                }
            }
        }
        "test" => {
            if args.iter().any(|a| a == "--parity") {
                cmd_test_parity();
            } else if args.iter().any(|a| a == "--book") {
                cmd_test_book();
            } else if args.iter().any(|a| a == "--docs") {
                cmd_test_docs();
            } else {
                // crow test — run the authored test blocks. `crow check` runs
                // these plus everything automatic (contracts, hazards, edges).
                let filename = args.iter().skip(2)
                    .find(|a| !a.starts_with('-'))
                    .cloned()
                    .unwrap_or_else(|| parse_crowfile().entry);
                cmd_test_blocks(&filename);
            }
        }
        "stop" => {
            cmd_stop();
        }
        "book" => {
            cmd_book();
        }
        "run" => {
            // Check for -i flag (interpreter mode)
            if args.len() > 2 && args[2] == "-i" {
                if args.len() > 3 {
                    cmd_run_file_interpreted(&args[3]);
                } else {
                    cmd_run_interpreted();
                }
            } else if args.len() > 2 {
                let target = &args[2];
                if target.ends_with(".crow") || target.ends_with(".crowc") {
                    // crow run file.crow arg1 arg2 ... -> transpile + compile + run
                    let extra_args = args[3..].to_vec();
                    cmd_run_file(target, &extra_args);
                } else {
                    // crow run <job_name> -> look up in crowfile [jobs]
                    cmd_run_job(target);
                }
            } else {
                // crow run (uses crowfile) -> transpile + compile + run
                cmd_run(&[]);
            }
        }
        other => {
            // check if it's a .crow or .crowc file (shortcut)
            if other.ends_with(".crow") || other.ends_with(".crowc") {
                let extra_args = args[2..].to_vec();
                cmd_run_file(other, &extra_args);
            } else {
                eprintln!("unknown command: {}", other);
                print_usage();
                process::exit(1);
            }
        }
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  crow                   start interactive REPL");
    eprintln!("  crow new myapp         create new project");
    eprintln!("  crow new myapp --web   create web project (views, serve)");
    eprintln!("  crow new myapp --ml    create ML project (jobs, lib)");
    eprintln!("  crow new myapp --db    create DB project (schema, migrate)");
    eprintln!("  crow install           install deps from crowfile into crow_modules/");
    eprintln!("  crow build             transpile to rust and compile to native binary");
    eprintln!("  crow build file.crow   transpile a specific file");
    eprintln!("  crow run               transpile + compile + run main.crow");
    eprintln!("  crow run file.crow     transpile + compile + run a specific file");
    eprintln!("  crow run train         run a named job from crowfile");
    eprintln!("  crow run -i file.crow  run with interpreter (slow, for debugging)");
    eprintln!("  crow stop              stop the running app (pidfile, then project binary)");
    eprintln!("  crow book [--port N]   serve the book with every code block runnable in place (default port 3008)");
    eprintln!("  crow check             auto-verify project (uses crowfile)");
    eprintln!("  crow check file.crow   auto-verify a specific file");
    eprintln!("  crow check --db <url>  + schema-aware db checks (or db = '…' in crowfile)");
    eprintln!("  crow fix file.crow --hazard N [--option M]   apply a fix from the check report");
    eprintln!("  crow lsp               language server over stdio (see docs/crow-lsp.md)");
    eprintln!("  crow fmt               format project (uses crowfile)");
    eprintln!("  crow fmt file.crow     format a specific file");
    eprintln!("  crow fmt --check       check formatting without changing files");
    eprintln!("  crow fmt --fix-interpolation  migrate {{expr}} strings to `expr`");
    eprintln!("  crow fmt --fix-views      migrate ui-building fns to view declarations");
    eprintln!("  crow launch 4 file.crow  build + run on 4 distributed processes");
    eprintln!("  crow test --parity     run tests/parity/ in both modes, diff output");
    eprintln!("  crow test --book       execute every code block of the book in both modes");
    eprintln!("  crow test --docs       check the docs cover every compiler name (and nothing more)");
    eprintln!("  crow check --verbose   verbose output with purity and snapshot details");
    eprintln!("  crow -v                show version");
}

/// Run every tests/parity/*.crow file through both execution modes and diff
/// their stdout. The transpiler and interpreter must agree on every program
/// (design rule 11); this makes drift a test failure instead of a discipline.

/// Execute every fenced `crow` code block in docs/book/src/*.md through both
/// execution modes. The book can never rot: a block either runs (and the two
/// modes must agree — and match a directly following ```text block, if any),
/// builds (`crow,norun`), is expected to fail (`crow,error`), or is
/// explicitly skipped (`crow,ignore`).
/// Every .md page under docs/book/src, recursively (symlinks — the bound
/// spec — excluded: its untagged fences are not book blocks).
fn book_pages(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(false) { continue; }
            if p.is_dir() { out.extend(book_pages(&p)); }
            else if p.extension().map_or(false, |x| x == "md") { out.push(p); }
        }
    }
    out
}

fn cmd_test_book() {
    let dir = std::path::Path::new("docs/book/src");
    if !dir.is_dir() {
        eprintln!("error: no docs/book/src/ directory found (run from the project root)");
        process::exit(1);
    }
    let mut files = book_pages(dir);
    files.sort();

    #[derive(PartialEq)]
    enum Mode { Run, NoRun, Error, Ignore }

    struct Block {
        file: String,
        line: usize,
        mode: Mode,
        code: String,
        expected: Option<String>,
    }

    // Extract fenced blocks (```crow[,flag]) and any directly following ```text block
    let mut blocks: Vec<Block> = Vec::new();
    for file in &files {
        let name = file.strip_prefix(dir).unwrap_or(file).to_string_lossy().to_string();
        let src = std::fs::read_to_string(file).unwrap_or_default();
        let lines: Vec<&str> = src.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let l = lines[i].trim_start();
            if l.starts_with("```crow") {
                let info = l.trim_start_matches("```crow");
                let mode = match info.trim_start_matches(',').trim() {
                    "" => Mode::Run,
                    "norun" => Mode::NoRun,
                    "error" => Mode::Error,
                    "ignore" => Mode::Ignore,
                    other => {
                        eprintln!("{}:{}: unknown crow block flag '{}'", name, i + 1, other);
                        process::exit(1);
                    }
                };
                let start = i + 1;
                let mut code = String::new();
                i += 1;
                while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                    code.push_str(lines[i]);
                    code.push('\n');
                    i += 1;
                }
                i += 1; // closing fence
                // A directly following ```text block is the expected output
                let mut j = i;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                let mut expected = None;
                if j < lines.len() && lines[j].trim_start().starts_with("```text") {
                    let mut out = String::new();
                    j += 1;
                    while j < lines.len() && !lines[j].trim_start().starts_with("```") {
                        out.push_str(lines[j]);
                        out.push('\n');
                        j += 1;
                    }
                    i = j + 1;
                    expected = Some(out);
                }
                blocks.push(Block { file: name.clone(), line: start, mode, code, expected });
            } else {
                i += 1;
            }
        }
    }

    if blocks.is_empty() {
        eprintln!("error: no ```crow blocks found under docs/book/src/");
        process::exit(1);
    }

    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    // One scratch root per invocation (two overlapping runs must never share a
    // block.crow), and one directory per worker inside it: blocks are checked
    // in parallel — each compiled block is a cargo build, which is what makes
    // this harness slow — and reported in book order.
    // The root is persistent so each worker's cargo cache survives between
    // runs (a cold cache is a minute of dependency builds per worker); a lock
    // file refuses a second run while one is in progress.
    let root = std::env::temp_dir().join("crow_book_test");
    let _ = std::fs::create_dir_all(&root);
    let lock = root.join("lock");
    if let Ok(pid) = std::fs::read_to_string(&lock) {
        let alive = std::process::Command::new("kill").arg("-0").arg(pid.trim()).output().map(|o| o.status.success()).unwrap_or(false);
        if alive {
            eprintln!("error: another `crow test --book` (pid {}) is still running — wait for it, or delete {}", pid.trim(), lock.display());
            process::exit(1);
        }
    }
    let _ = std::fs::write(&lock, std::process::id().to_string());
    let workers: usize = std::env::var("CROW_BOOK_JOBS").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 6));

    let start = std::time::Instant::now();
    let total = blocks.iter().filter(|b| b.mode != Mode::Ignore).count();
    println!("book: {} blocks across {} pages, {} workers\n", total, files.len(), workers);

    // Check one block in its own directory; returns (passed, report lines).
    let check = |b: &Block, dir: &std::path::Path| -> (bool, String) {
        let run_mode = |mode_args: &[&str], file: &std::path::Path| -> (bool, String, String) {
            let out = std::process::Command::new(&exe).args(mode_args).arg(file).current_dir(dir).env("CROW_BUILD_PROFILE", "dev").output();
            match out {
                Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()),
                Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
            }
        };
        let label = format!("{}:{}", b.file, b.line);
        let path = dir.join("block.crow");
        let _ = std::fs::write(&path, &b.code);
        match b.mode {
            Mode::Ignore => (true, String::new()),
            Mode::Run => {
                let (i_ok, i_out, i_err) = run_mode(&["run", "-i"], &path);
                let (t_ok, t_out, t_err) = run_mode(&["run"], &path);
                if !i_ok || !t_ok {
                    let tail = if !t_ok { t_err } else { i_err };
                    return (false, format!("  FAIL  {} (execution)\n        {}\n", label, tail.lines().last().unwrap_or("(no stderr)")));
                }
                if i_out != t_out {
                    return (false, format!("  FAIL  {} (modes disagree)\n", label));
                }
                if let Some(exp) = &b.expected {
                    if t_out.trim_end() != exp.trim_end() {
                        return (false, format!("  FAIL  {} (output != book text)\n        expected: {:?}\n        got:      {:?}\n", label, exp.trim_end(), t_out.trim_end()));
                    }
                }
                // A block that declares test blocks must also pass `crow test`:
                // `crow run` skips them, so this is the only place they execute.
                let has_tests = b.code.lines().any(|l| l.starts_with("test ") || l.starts_with("test["));
                if has_tests {
                    let (s_ok, s_out, s_err) = run_mode(&["test"], &path);
                    if !s_ok {
                        let detail = s_out.lines().rev().find(|l| !l.trim().is_empty())
                            .or_else(|| s_err.lines().last()).unwrap_or("(no output)");
                        return (false, format!("  FAIL  {} (test blocks)\n        {}\n", label, detail.trim()));
                    }
                    return (true, format!("  ok    {} (+ test blocks)\n", label));
                }
                (true, format!("  ok    {}\n", label))
            }
            Mode::NoRun => {
                let (ok, _, err) = run_mode(&["build"], &path);
                if ok { (true, format!("  ok    {} (build only)\n", label)) }
                else { (false, format!("  FAIL  {} (does not build)\n        {}\n", label, err.lines().last().unwrap_or("(no stderr)"))) }
            }
            Mode::Error => {
                let (ok, _, _) = run_mode(&["build"], &path);
                if !ok { (true, format!("  ok    {} (fails as documented)\n", label)) }
                else { (false, format!("  FAIL  {} (documented as an error, but builds)\n", label)) }
            }
        }
    };

    let results: Vec<std::sync::Mutex<Option<(bool, String)>>> = blocks.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for w in 0..workers {
            let dir = root.join(format!("w{}", w));
            let _ = std::fs::create_dir_all(&dir);
            let (blocks, results, next, check) = (&blocks, &results, &next, &check);
            scope.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= blocks.len() { break; }
                let r = check(&blocks[i], &dir);
                *results[i].lock().unwrap() = Some(r);
            });
        }
    });
    let mut failures = 0;
    for r in &results {
        if let Some((ok, report)) = r.lock().unwrap().as_ref() {
            if !ok { failures += 1; }
            print!("{}", report);
        }
    }
    let _ = std::fs::remove_file(&lock);
    let elapsed = start.elapsed().as_secs_f32();
    println!();
    if failures == 0 {
        println!("\u{2713} {} book blocks verified in both modes ({:.1}s, {} workers)", total, elapsed, workers);
    } else {
        println!("\u{2717} {} of {} book blocks fail ({:.1}s)", failures, total, elapsed);
        process::exit(1);
    }
}

/// `crow test --docs` — the documentation cannot drift from the compiler.
///
/// Reads the compiler's own name tables (core builtins, module exports, ui
/// elements, web/ui names) and checks, in both directions:
///   1. every name the compiler knows is documented in docs/language-spec.md
///   2. every reference-table row in the spec names something the compiler
///      knows (no phantom builtins)
///   3. every module has a `use` in at least one verified book block, and
///      every core builtin is called in at least one
///   4. every examples/*.crow is listed in examples/README.md
fn cmd_test_docs() {
    use std::collections::BTreeSet;
    let spec = std::fs::read_to_string("docs/language-spec.md").unwrap_or_else(|_| {
        eprintln!("error: docs/language-spec.md not found (run from the project root)");
        process::exit(1);
    });
    let mut tables: Vec<(&str, Vec<&str>)> = vec![
        ("core", validator::CORE_RESERVED.to_vec()),
        ("web", validator::WEB_RESERVED.iter().chain(validator::WEB_EXTRA_FUNCTIONS.iter()).copied().collect()),
        ("ui", validator::UI_RESERVED.iter().copied().filter(|n| *n != "today").collect()),
        ("element", lower::UI_ELEMENTS.to_vec()),
        ("math", modules::math::MATH_FUNCTIONS.to_vec()),
        ("math.stats", modules::stats::STATS_FUNCTIONS.to_vec()),
        ("math.nn", modules::nn::NN_FUNCTIONS.to_vec()),
        ("frame", modules::frame::FRAME_FUNCTIONS.to_vec()),
        ("db", modules::db::DB_FUNCTIONS.to_vec()),
        ("io", modules::io::IO_FUNCTIONS.to_vec()),
        ("doc", modules::doc::DOC_FUNCTIONS.to_vec()),
        ("chart", modules::chart::CHART_FUNCTIONS.to_vec()),
    ];
    for (_, names) in tables.iter_mut() { names.sort(); names.dedup(); }
    let known: BTreeSet<&str> = tables.iter().flat_map(|(_, ns)| ns.iter().copied()).collect();
    let mut failures = 0;

    // 1. compiler → spec
    println!("docs: compiler names documented in the spec");
    for (table, names) in &tables {
        let missing: Vec<&str> = names.iter().copied()
            .filter(|n| !(spec.contains(&format!("`{}[", n)) || spec.contains(&format!("`{}`", n))))
            .collect();
        if missing.is_empty() {
            println!("  ok    {:10} {:3} names", table, names.len());
        } else {
            failures += missing.len();
            println!("  FAIL  {:10} {} undocumented: {}", table, missing.len(), missing.join(" "));
        }
    }

    // 2. spec reference rows → compiler (phantom entries)
    println!("docs: spec reference rows name real builtins");
    let keywords = ["map", "struct", "fn", "view", "test", "use"];
    let mut phantom: Vec<String> = vec![];
    let mut in_reference = false; // only the Core Builtins / Standard Library sections
    for (i, line) in spec.lines().enumerate() {
        if line.starts_with("## ") {
            in_reference = line.starts_with("## 11.") || line.starts_with("## 13.");
            continue;
        }
        if !in_reference { continue; }
        let l = line.trim_start();
        if !l.starts_with("| `") { continue; }
        let rest = &l[3..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
        let after = rest[name.len()..].chars().next().unwrap_or(' ');
        if after != '[' || name.is_empty() { continue; }
        if known.contains(name.as_str()) || keywords.contains(&name.as_str()) { continue; }
        phantom.push(format!("line {}: `{}[`", i + 1, name));
    }
    if phantom.is_empty() {
        println!("  ok    every `name[` table row is a compiler name");
    } else {
        failures += phantom.len();
        for p in &phantom { println!("  FAIL  {} is documented but the compiler has no such builtin", p); }
    }

    // 3. book blocks: every module used, every core builtin called
    println!("docs: book blocks cover every module and core builtin");
    let book_dir = std::path::Path::new("docs/book/src");
    let mut pages = book_pages(book_dir);
    pages.sort();
    // (relative page path, page title, its crow blocks joined)
    let mut page_blocks: Vec<(String, String, String)> = vec![];
    let mut blocks = String::new();
    for p in &pages {
        let src = std::fs::read_to_string(p).unwrap_or_default();
        let title = src.lines().find(|l| l.starts_with("# ")).map(|l| l[2..].trim().to_string()).unwrap_or_default();
        let mut mine = String::new();
        let mut in_block = false;
        for line in src.lines() {
            if line.trim_start().starts_with("```crow") { in_block = true; continue; }
            if line.trim_start().starts_with("```") { in_block = false; continue; }
            if in_block { mine.push_str(line); mine.push('\n'); }
        }
        if !mine.is_empty() {
            blocks.push_str(&mine);
            page_blocks.push((p.strip_prefix(book_dir).unwrap_or(p).to_string_lossy().to_string(), title, mine));
        }
    }
    for m in ["math", "math.stats", "math.nn", "frame", "io", "db", "doc", "chart", "web", "ui"] {
        let used = blocks.lines().any(|l| l.starts_with("use ") && l[4..].split(',').any(|part| {
            let part = part.trim();
            let base = part.split(" as ").next().unwrap_or(part).trim();
            base == m
        }));
        if used { println!("  ok    use {}", m); } else { failures += 1; println!("  FAIL  no book block imports `{}`", m); }
    }
    // Every name in every table must appear in a verified block. The
    // allowlist is the honest exception: web submodule calls whose compiled
    // codegen does not build today — documented in the spec, listed here so
    // the gap is visible (they have no example link in the standard library pages).
    const SPEC_ONLY: &[&str] = &[
        "test", "today",                                    // keyword / removed name
        "sign", "verify", "templates", "render", "uploads", // auth/template/upload codegen
        "connect", "send", "recv", "close",                // websocket client
        "sse", "ws", "cors", "static_dir", "sqlite",       // legacy app-parameter APIs
        "stream",                                           // handler contract undocumented
        "app", "click",                                     // test-harness names (used in test blocks, not run blocks)
    ];
    for (table, names) in &tables {
        let uncalled: Vec<&str> = names.iter().copied()
            .filter(|n| !SPEC_ONLY.contains(n) && !blocks.contains(&format!("{}[", n)))
            .collect();
        if uncalled.is_empty() {
            println!("  ok    {:10} every name is used in a verified block", table);
        } else {
            failures += uncalled.len();
            println!("  FAIL  {:10} {} names never used in a book block: {}", table, uncalled.len(), uncalled.join(" "));
        }
    }

    // 4. examples indexed
    println!("docs: examples/ indexed in examples/README.md");
    let readme = std::fs::read_to_string("examples/README.md").unwrap_or_default();
    let prefixes: Vec<String> = readme.split(|c: char| c.is_whitespace() || c == '`')
        .filter(|t| t.ends_with("*.crow"))
        .map(|t| t.trim_end_matches("*.crow").to_string())
        .collect();
    let mut unlisted: Vec<String> = vec![];
    if let Ok(rd) = std::fs::read_dir("examples") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| x != "crow") { continue; }
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            if readme.contains(&name) || prefixes.iter().any(|pre| name.starts_with(pre.as_str())) { continue; }
            unlisted.push(name);
        }
    }
    unlisted.sort();
    if unlisted.is_empty() {
        println!("  ok    every example is listed");
    } else {
        failures += unlisted.len();
        println!("  FAIL  not listed in examples/README.md: {}", unlisted.join(" "));
    }

    let write = std::env::args().any(|a| a == "--write");

    // --- The Standard Library: one page per module, generated from the
    // spec's section for it (the single source of the descriptions) with an
    // example column pointing at the book pages whose verified blocks use
    // each name. `--write` regenerates; a stale page is a failure.
    println!("docs: standard library pages");
    let lines: Vec<&str> = spec.lines().collect();
    let heading_level = |l: &str| l.chars().take_while(|c| *c == '#').count();
    let section = |start_heading: &str| -> Option<(usize, usize)> {
        let start = lines.iter().position(|l| l.trim_end() == start_heading)?;
        let level = heading_level(start_heading);
        let end = (start + 1..lines.len())
            .find(|&i| { let h = heading_level(lines[i]); h > 0 && h <= level && lines[i].starts_with('#') })
            .unwrap_or(lines.len());
        Some((start, end))
    };
    let hits_for = |name: &str| -> Vec<String> {
        let call = format!("{}[", name);
        page_blocks.iter().filter(|(_, _, b)| b.contains(&call)).map(|(path, title, _)| format!("[{}](../{})", title, path)).collect()
    };
    // a page from a slice of spec lines: headings demoted one level, tables
    // given an example column, the page's own title on top
    let render_page = |title: &str, intro: &str, ranges: &[(usize, usize, bool)], exclude: &[(usize, usize)]| -> String {
        let mut out = format!("# {}\n\n{}\n\n", title, intro);
        for &(a, b, keep_heading) in ranges.iter() {
            let mut i = if keep_heading { a } else { a + 1 };
            while i < b {
                if exclude.iter().any(|&(x, y)| i >= x && i < y) { i += 1; continue; }
                let l = lines[i];
                if l.starts_with("#### ") { out.push_str(&format!("### {}\n", &l[5..])); }
                else if l.starts_with("### ") { out.push_str(&format!("## {}\n", &l[4..])); }
                else if l.starts_with("| ") && i + 1 < b && lines[i + 1].trim_start().starts_with("|--") || (l.starts_with("| ") && i + 1 < b && lines[i + 1].trim_start().starts_with("| ---")) {
                    out.push_str(&format!("{} Example |\n", l.trim_end()));
                    out.push_str(&format!("{}---|\n", lines[i + 1].trim_end()));
                    i += 1;
                } else if l.starts_with("| `") {
                    let name: String = l[3..].chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
                    let ex = hits_for(&name);
                    out.push_str(&format!("{} {} |\n", l.trim_end(), if ex.is_empty() { String::new() } else { ex.join(" · ") }));
                } else if l.starts_with("| ") {
                    out.push_str(&format!("{} |\n", l.trim_end()));
                } else {
                    out.push_str(l); out.push('\n');
                }
                i += 1;
            }
        }
        out.push_str("\nGenerated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.\n");
        out
    };
    let stats = section("#### Statistics (sub-module: `math.stats`)");
    let nn = section("#### Neural Networks (sub-module: `math.nn`)");
    let mut pages: Vec<(String, String)> = Vec::new();
    let mut index = String::from("# The Standard Library\n\nEvery module ships with the compiler: `use` it and its names are there in both execution modes. Each page lists the module's functions with the book examples that use them, generated from the language specification so the two cannot drift. Core builtins need no import.\n\n| Module | Import | Names | Page |\n|---|---|---|---|\n");
    let count = |t: &str| tables.iter().find(|(n, _)| *n == t).map(|(_, ns)| ns.len()).unwrap_or(0);
    let mut add = |file: &str, title: &str, import: &str, table: &str, intro: &str, ranges: Vec<(usize, usize, bool)>, exclude: Vec<(usize, usize)>| {
        if ranges.is_empty() { return; }
        index.push_str(&format!("| {} | `{}` | {} | [{}]({}.md) |\n", title, import, count(table), title, file));
        pages.push((format!("{}.md", file), render_page(title, intro, &ranges, &exclude)));
    };
    let one = |r: Option<(usize, usize)>| -> Vec<(usize, usize, bool)> { r.map(|(a, b)| vec![(a, b, false)]).unwrap_or_default() };
    if let Some(core) = section("## 11. Core Builtins") { add("core", "Core builtins", "(none)", "core", "Always available, no import. The tables below are the spec's, with the book pages whose verified code uses each name.", one(Some(core)), vec![]); }
    if let Some(m) = section("### math") { let ex: Vec<(usize, usize)> = [stats, nn].iter().flatten().copied().collect(); add("math", "math", "use math", "math", "Tensors, element-wise math, reductions, linear algebra, random numbers, signal processing, sparse arrays, and numeric loading.", one(Some(m)), ex); }
    if stats.is_some() { add("math-stats", "math.stats", "use math.stats", "math.stats", "Descriptive statistics, correlation, tests, and fitting over tensors and frame columns.", one(stats), vec![]); }
    if nn.is_some() { add("math-nn", "math.nn", "use math.nn", "math.nn", "Activation functions, losses, and a linear layer for small networks with autograd.", one(nn), vec![]); }
    if let Some(f) = section("### frame") { add("frame", "frame", "use frame", "frame", "Column-oriented data frames: load, select, filter, sort, group, join, and time handling.", one(Some(f)), vec![]); }
    if let Some(d) = section("### db") { add("db", "db", "use db", "db", "Databases as connections: query, exec, bulk insert, transactions, migrations, and pooling.", one(Some(d)), vec![]); }
    if let Some(i) = section("### io") { add("io", "io", "use io", "io", "Structured file parsing beyond the built-in read and write.", one(Some(i)), vec![]); }
    if let Some(d) = section("### doc") { add("doc", "doc", "use doc", "doc", "Documentation from Markdown, rendered by the kernel so both execution modes write the same pages.", one(Some(d)), vec![]); }
    if let Some(c) = section("### chart") { add("chart", "chart", "use chart", "chart", "Technical indicators for the chart element, computed by the kernel so both execution modes draw the same numbers.", one(Some(c)), vec![]); }
    // web splits where the spec does: the view side (`use ui`) and the server side (`use web`)
    // (the spec's `### web` runs, through the `### Layout` section, up to `### db`)
    if let (Some((w0, _)), Some((d0, _))) = (section("### web"), section("### db")) {
        let server = (w0..d0).find(|&i| lines[i].trim_end() == "#### Core (Server)").unwrap_or(d0);
        add("ui", "ui", "use ui", "ui", "Views and the element vocabulary, reactive state and inputs, the layout vocabulary, charts, and styling.", vec![(w0, server, false)], vec![]);
        if server < d0 { add("web", "web", "use web", "web", "The server: routes and responses, auth, CORS, sessions, templates, cache, rate limits, uploads, SSE, sockets, pages, static files, mail, queues, API docs, and the HTTP client.", vec![(server, d0, true)], vec![]); }
    }
    pages.push(("index.md".to_string(), index));
    let stdlib_dir = book_dir.join("stdlib");
    let _ = std::fs::create_dir_all(&stdlib_dir);
    for (file, content) in &pages {
        let path = stdlib_dir.join(file);
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if write {
            let _ = std::fs::write(&path, content);
            println!("  ok    wrote {}", path.display());
        } else if current == *content {
            println!("  ok    {} is current", path.display());
        } else {
            failures += 1;
            println!("  FAIL  {} is stale — run `crow test --docs --write`", path.display());
        }
    }

    println!();
    if failures == 0 {
        println!("\u{2713} documentation covers the compiler ({} names)", known.len());
    } else {
        println!("\u{2717} {} documentation gaps", failures);
        process::exit(1);
    }
}

fn cmd_test_parity() {
    let dir = std::path::Path::new("tests/parity");
    if !dir.is_dir() {
        eprintln!("error: no tests/parity/ directory found (run from the project root)");
        process::exit(1);
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |x| x == "crow"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        eprintln!("error: tests/parity/ contains no .crow files");
        process::exit(1);
    }

    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    let run_mode = |mode_args: &[&str], file: &std::path::Path| -> (bool, String, String) {
        let out = std::process::Command::new(&exe)
            .args(mode_args)
            .arg(file)
            .env("CROW_BUILD_PROFILE", "dev")
            .output();
        match out {
            Ok(o) => (
                o.status.success(),
                String::from_utf8_lossy(&o.stdout).to_string(),
                String::from_utf8_lossy(&o.stderr).to_string(),
            ),
            Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
        }
    };

    let start = std::time::Instant::now();
    let mut failures = 0;
    println!("parity: {} files, interpreter vs compiled\n", files.len());
    for file in &files {
        let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
        let (i_ok, i_out, i_err) = run_mode(&["run", "-i"], file);
        let (t_ok, t_out, t_err) = run_mode(&["run"], file);

        if i_ok && t_ok && i_out == t_out {
            println!("  ok    {}", name);
            continue;
        }
        failures += 1;
        println!("  FAIL  {}", name);
        if !i_ok {
            let tail = i_err.lines().last().unwrap_or("(no stderr)");
            println!("        interpreter exited with error: {}", tail);
        }
        if !t_ok {
            let tail = t_err.lines().last().unwrap_or("(no stderr)");
            println!("        compiled exited with error: {}", tail);
        }
        if i_ok && t_ok {
            // First differing line
            let i_lines: Vec<&str> = i_out.lines().collect();
            let t_lines: Vec<&str> = t_out.lines().collect();
            let max = i_lines.len().max(t_lines.len());
            for n in 0..max {
                let a = i_lines.get(n).copied().unwrap_or("<missing>");
                let b = t_lines.get(n).copied().unwrap_or("<missing>");
                if a != b {
                    println!("        line {}: interpreter: {}", n + 1, a);
                    println!("        line {}: compiled:    {}", n + 1, b);
                    break;
                }
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!();
    if failures == 0 {
        println!("✓ {} files match across both modes ({:.1}s)", files.len(), elapsed);
    } else {
        println!("✗ {} of {} files diverge between modes ({:.1}s)", failures, files.len(), elapsed);
        process::exit(1);
    }
}

fn cmd_repl() {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    println!("(o.o) crow {} — type help for commands, ctrl-c to exit", env!("CARGO_PKG_VERSION"));
    let mut interpreter = Interpreter::new();
    let mut accumulated_stmts: Vec<ast::Stmt> = Vec::new();
    let mut buffer = String::new();
    let mut in_block = false;

    let mut rl = DefaultEditor::new().unwrap_or_else(|e| {
        eprintln!("error initializing editor: {}", e);
        process::exit(1);
    });

    // Load history from ~/.crow_history
    let history_path = home_dir().map(|h| h.join(".crow_history"));
    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let prompt = if in_block { "  " } else { "crow> " };

        match rl.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');

                // Add non-empty, non-block-continuation lines to history
                if !in_block && !trimmed.is_empty() {
                    let _ = rl.add_history_entry(&line);
                }

                // REPL commands
                if !in_block {
                    match trimmed {
                        "exit" => {
                            break;
                        }
                        "check" => {
                            repl_check(&accumulated_stmts);
                            continue;
                        }
                        "help" => {
                            println!("  clear   — reset the session");
                            println!("  check   — run verification on all defined functions");
                            println!("  exit    — exit (or ctrl-c)");
                            continue;
                        }
                        "clear" => {
                            interpreter = Interpreter::new();
                            accumulated_stmts.clear();
                            println!("  session cleared");
                            continue;
                        }
                        _ => {}
                    }
                }

                // Multi-line block handling
                if in_block {
                    if trimmed.is_empty() || (!trimmed.starts_with(' ') && !trimmed.starts_with('\t')) {
                        in_block = false;
                        let source = std::mem::take(&mut buffer);
                        repl_eval(&mut interpreter, &source, &mut accumulated_stmts);
                        if trimmed.is_empty() {
                            continue;
                        }
                    } else {
                        buffer.push_str(trimmed);
                        buffer.push('\n');
                        continue;
                    }
                }

                // Check if this line expects a block to follow
                let needs_block = {
                    let stripped = trimmed.replace(|c: char| c == '\'', "\x00")
                        .split('\x00')
                        .enumerate()
                        .filter(|(i, _)| i % 2 == 0)
                        .map(|(_, s)| s)
                        .collect::<Vec<_>>()
                        .join(" ");
                    stripped.split_whitespace()
                        .any(|w| matches!(w, "if" | "for" | "while" | "fn" | "match" | "struct"))
                };

                if needs_block {
                    in_block = true;
                    buffer = format!("{}\n", trimmed);
                    continue;
                }

                // Single line - evaluate immediately
                repl_eval(&mut interpreter, trimmed, &mut accumulated_stmts);
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: exit
                println!();
                break;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: also exit
                println!();
                break;
            }
            Err(e) => {
                eprintln!("read error: {}", e);
                break;
            }
        }
    }

    // Save history
    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }
}

/// Get user home directory
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

fn repl_check(stmts: &[ast::Stmt]) {
    if stmts.is_empty() {
        println!("  nothing to check — define some functions first");
        return;
    }

    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(stmts);

    let result = checker::run_check(stmts, &ctx, false);

    if result.issues.is_empty() {
        println!("  \x1b[32m✓ all good\x1b[0m — {} functions checked, {} edge cases generated",
            result.total_functions, result.edge_cases_generated);
    } else {
        println!("  \x1b[33m⚠ {} issue{}\x1b[0m — {} functions checked, {} edge cases generated\n",
            result.issues.len(),
            if result.issues.len() == 1 { "" } else { "s" },
            result.total_functions,
            result.edge_cases_generated);

        for (i, issue) in result.issues.iter().enumerate() {
            let func = issue.func();
            let loc = if func != "<top-level>" {
                format!("in {}", func)
            } else {
                "<top-level>".to_string()
            };
            println!("  {}. \x1b[90m{}\x1b[0m {}", i + 1, loc, issue.summary());
            println!("     \x1b[36mfix: {}\x1b[0m", issue.suggest());
        }
    }
}

fn repl_eval(interpreter: &mut Interpreter, source: &str, accumulated: &mut Vec<ast::Stmt>) {
    let source = source.trim();
    if source.is_empty() {
        return;
    }

    match build(source) {
        Ok(stmts) => {
            // Accumulate statements for :check
            accumulated.extend(stmts.clone());

            match interpreter.run(&stmts) {
                Ok(val) => {
                    // Print result if it's not Done (void)
                    if !matches!(val.value, value::Value::Done) {
                        println!("{}", val);
                    }
                }
                Err(e) => eprintln!("error: {}", e),
            }
        }
        Err(e) => eprintln!("error: {}", e),
    }
}

fn cmd_new() {
    let args: Vec<String> = std::env::args().collect();

    // crow new <name> [--web] [--ml] [--db]
    let name = args.iter().skip(2).find(|a| !a.starts_with('-'));
    let name = match name {
        Some(n) => n.clone(),
        None => {
            eprintln!("usage: crow new <name> [--web] [--ml] [--db]");
            process::exit(1);
        }
    };

    let has_web = args.iter().any(|a| a == "--web");
    let has_ml = args.iter().any(|a| a == "--ml");
    let has_db = args.iter().any(|a| a == "--db");

    let project_dir = std::path::PathBuf::from(&name);
    let short_name = project_dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());
    if project_dir.exists() {
        eprintln!("directory '{}' already exists", name);
        process::exit(1);
    }

    // Helper to write a file, creating parent dirs
    let write_file = |path: &std::path::Path, content: &str| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("error creating directory: {}", e);
                process::exit(1);
            });
        }
        std::fs::write(path, content).unwrap_or_else(|e| {
            eprintln!("error writing {}: {}", path.display(), e);
            process::exit(1);
        });
    };

    // Create project directory
    std::fs::create_dir_all(&project_dir).unwrap_or_else(|e| {
        eprintln!("error creating directory '{}': {}", name, e);
        process::exit(1);
    });

    // Build crowfile
    let mut crowfile = format!("name = '{}'\n", short_name);
    let mut jobs_section = Vec::new();

    // main.crow
    if has_web {
        write_file(&project_dir.join("main.crow"), "use web, ui\nuse views.home\n\nserve[3000]\npage['/', home.render]\n");
    } else {
        write_file(&project_dir.join("main.crow"), &format!("log['hello, {}']\n", short_name));
    }

    // --web: add views/home.crow
    if has_web {
        write_file(&project_dir.join("views/home.crow"),
            "--- home page ---\nuse ui\n\nmessage = 'welcome to crow'\n\nview render[]\n    return div[\n        h1['`message`']\n    ]\n");
    }

    // --ml: add jobs/train.crow and lib/model.crow
    if has_ml {
        write_file(&project_dir.join("jobs/train.crow"),
            "use lib.model\n\nlog['training started']\ntrain[]\nlog['training complete']\n");
        write_file(&project_dir.join("lib/model.crow"),
            "fn train[]\n  log['model training...']\n");
        jobs_section.push(("train".to_string(), "jobs/train.crow".to_string()));
    }

    // --db: add db/schema.crow and jobs/migrate.crow
    if has_db {
        write_file(&project_dir.join("db/schema.crow"),
            "use db\n\ntable users\n  name, str\n  email, str\n");
        write_file(&project_dir.join("jobs/migrate.crow"),
            "use db\n\nlog['running migrations']\n");
        jobs_section.push(("migrate".to_string(), "jobs/migrate.crow".to_string()));
    }

    // Add [jobs] section to crowfile if needed
    if !jobs_section.is_empty() {
        crowfile.push_str("\njobs = map[");
        let entries: Vec<String> = jobs_section.iter()
            .map(|(k, v)| format!("{} = '{}'", k, v))
            .collect();
        crowfile.push_str(&entries.join(", "));
        crowfile.push_str("]\n");
    }

    write_file(&project_dir.join("crowfile"), &crowfile);

    println!("created project '{}'", short_name);
    let mut files: Vec<String> = Vec::new();
    files.push("  crowfile".to_string());
    files.push("  main.crow".to_string());
    if has_web { files.push("  views/home.crow".to_string()); }
    if has_ml {
        files.push("  jobs/train.crow".to_string());
        files.push("  lib/model.crow".to_string());
    }
    if has_db {
        files.push("  db/schema.crow".to_string());
        files.push("  jobs/migrate.crow".to_string());
    }
    for f in &files {
        println!("{}", f);
    }
    println!("\ncd {} && crow run", name);
}

fn cmd_install() {
    let config = parse_crowfile();

    if config.deps.is_empty() {
        println!("no deps in crowfile");
        return;
    }

    let modules_dir = std::path::PathBuf::from("crow_modules");
    std::fs::create_dir_all(&modules_dir).unwrap_or_else(|e| {
        eprintln!("error creating crow_modules/: {}", e);
        process::exit(1);
    });

    for (name, source) in &config.deps {
        let dest = modules_dir.join(&name);

        if source.starts_with("https://") || source.starts_with("git://") || source.starts_with("http://") {
            // Git dependency
            if dest.exists() {
                // Pull latest
                println!("updating {}...", name);
                let status = std::process::Command::new("git")
                    .args(["pull", "--quiet"])
                    .current_dir(&dest)
                    .status();
                match status {
                    Ok(s) if s.success() => println!("  updated {}", name),
                    _ => eprintln!("  warning: failed to update {}", name),
                }
            } else {
                // Clone
                println!("installing {}...", name);
                let status = std::process::Command::new("git")
                    .args(["clone", "--quiet", source, &dest.to_string_lossy()])
                    .status();
                match status {
                    Ok(s) if s.success() => println!("  installed {}", name),
                    _ => {
                        eprintln!("error: failed to clone {}", source);
                        process::exit(1);
                    }
                }
            }
        } else if source.starts_with("./") || source.starts_with("/") || source.starts_with("../") {
            // Local path dependency - symlink
            let src_path = std::path::PathBuf::from(source);
            if !src_path.exists() {
                eprintln!("error: local dep path '{}' not found", source);
                process::exit(1);
            }
            if dest.exists() {
                println!("  {} already linked", name);
            } else {
                #[cfg(unix)]
                {
                    let abs_src = std::fs::canonicalize(&src_path).unwrap_or_else(|e| {
                        eprintln!("error resolving path '{}': {}", source, e);
                        process::exit(1);
                    });
                    std::os::unix::fs::symlink(&abs_src, &dest).unwrap_or_else(|e| {
                        eprintln!("error symlinking {}: {}", name, e);
                        process::exit(1);
                    });
                    println!("  linked {} -> {}", name, abs_src.display());
                }
                #[cfg(not(unix))]
                {
                    // On non-unix, copy instead
                    copy_dir_all(&src_path, &dest).unwrap_or_else(|e| {
                        eprintln!("error copying {}: {}", name, e);
                        process::exit(1);
                    });
                    println!("  copied {}", name);
                }
            }
        } else {
            // Version string - resolve from registry
            if !install_from_registry(name, source, &dest) {
                process::exit(1);
            }
        }
    }

    println!("done");
}

#[cfg(not(unix))]
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

// --- Package Registry ---

const DEFAULT_REGISTRY_REPO: &str = "https://github.com/vsriram/registry.git";

#[derive(serde::Deserialize, Debug)]
struct RegistryIndex {
    packages: HashMap<String, RegistryPackage>,
}

#[derive(serde::Deserialize, Debug)]
struct RegistryPackage {
    repo: String,
    versions: Vec<String>,
}

fn get_registry_repo() -> String {
    std::env::var("CROW_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY_REPO.to_string())
}

fn registry_cache_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".crow").join("registry")
}

fn fetch_registry_index() -> Option<RegistryIndex> {
    let cache_dir = registry_cache_dir();
    let index_path = cache_dir.join("index.json");
    let repo_url = get_registry_repo();

    if cache_dir.join(".git").exists() {
        // Registry already cloned — pull latest
        let status = std::process::Command::new("git")
            .args(["pull", "--quiet", "--ff-only"])
            .current_dir(&cache_dir)
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(s) = status {
            if !s.success() {
                eprintln!("warning: failed to update registry, using cached");
            }
        }
    } else {
        // Fresh clone
        let _ = std::fs::create_dir_all(cache_dir.parent().unwrap_or(std::path::Path::new(".")));
        let status = std::process::Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", &repo_url, &cache_dir.to_string_lossy()])
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("warning: failed to clone registry from {}", repo_url);
            }
        }
    }

    // Read index.json from the cloned/cached repo
    if index_path.exists() {
        if let Ok(json) = std::fs::read_to_string(&index_path) {
            match serde_json::from_str(&json) {
                Ok(index) => return Some(index),
                Err(e) => {
                    eprintln!("warning: failed to parse registry index.json: {}", e);
                }
            }
        }
    }

    eprintln!("error: no registry index found at {}", index_path.display());
    None
}

fn install_from_registry(name: &str, version: &str, dest: &std::path::Path) -> bool {
    let index = match fetch_registry_index() {
        Some(idx) => idx,
        None => {
            eprintln!("error: could not fetch package registry");
            return false;
        }
    };

    let pkg = match index.packages.get(name) {
        Some(p) => p,
        None => {
            eprintln!("error: package '{}' not found in registry", name);
            eprintln!("  available packages: {}", index.packages.keys().cloned().collect::<Vec<_>>().join(", "));
            return false;
        }
    };

    if !pkg.versions.contains(&version.to_string()) {
        eprintln!("error: package '{}' has no version '{}'", name, version);
        eprintln!("  available versions: {}", pkg.versions.join(", "));
        return false;
    }

    // Clone at the specific version tag
    let tag = format!("v{}", version);
    println!("installing {} v{} from {}...", name, version, pkg.repo);

    if dest.exists() {
        // Check out the right version
        println!("  updating {}...", name);
        let status = std::process::Command::new("git")
            .args(["fetch", "--quiet", "--tags"])
            .current_dir(dest)
            .status();
        if let Ok(s) = status {
            if s.success() {
                let _ = std::process::Command::new("git")
                    .args(["checkout", "--quiet", &tag])
                    .current_dir(dest)
                    .status();
                println!("  updated {} to v{}", name, version);
                return true;
            }
        }
        eprintln!("  warning: failed to update {}", name);
        return false;
    }

    // Fresh clone at specific tag
    let status = std::process::Command::new("git")
        .args([
            "clone", "--quiet", "--depth", "1",
            "--branch", &tag,
            &pkg.repo, &dest.to_string_lossy(),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("  installed {} v{}", name, version);
            true
        }
        _ => {
            // Tag might not exist, try cloning without tag
            eprintln!("  warning: tag '{}' not found, cloning latest", tag);
            let status = std::process::Command::new("git")
                .args(["clone", "--quiet", "--depth", "1", &pkg.repo, &dest.to_string_lossy()])
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("  installed {} (latest)", name);
                    true
                }
                _ => {
                    eprintln!("error: failed to clone {}", pkg.repo);
                    false
                }
            }
        }
    }
}

/// Transpile a .crow file to Rust, compile it, and return the path to the binary.
/// If `quiet` is true, suppress cargo build output (used for `crow run`).
/// If `release` is true, compile with --release (optimized). Otherwise, use debug mode (fast compile).
fn transpile_and_compile(entry: &str, quiet: bool, cpu: Option<usize>, release: bool) -> std::path::PathBuf {
    // Verification harnesses (`crow test --book`, `--parity`) set
    // CROW_BUILD_PROFILE=dev: a checked program only has to run, not ship,
    // and skipping optimization + LTO is most of the per-block build time.
    let release = release && std::env::var("CROW_BUILD_PROFILE").map(|v| v != "dev").unwrap_or(true);
    let source = read_file(entry);
    let stmts = match build(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    };

    // Expand module imports before type inference so call sites inform param types
    let source_dir = std::path::Path::new(entry)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);

    // Resolve named arguments to positional form (shared with the interpreter)
    let stmts = lower::lower_program(&stmts).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });

    // Run type inference. The compiler may ask the program's databases what
    // each literal query returns, so frame cells are typed natively.
    let mut ctx = type_infer::TypeContext::new();
    ctx.describe_queries = std::env::var("CROW_NO_DESCRIBE").is_err();
    ctx.infer_program(&stmts);

    // Run validation pass — catch user errors before transpilation
    let mut v = validator::Validator::new(&ctx);
    if let Err(e) = v.validate(&stmts) {
        eprintln!("{}", refine_error_lines(&source, &e));
        process::exit(1);
    }

    // Transpile to Rust
    let mut transpiler = transpiler::Transpiler::new(ctx);
    let entry_file = std::path::Path::new(entry).file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.to_string());
    transpiler.set_source_file(&entry_file);
    if let Some(n) = cpu {
        transpiler.set_cpu_count(n);
    }
    let rust_source = transpiler.transpile(&stmts);

    // Create build directory
    let build_dir = std::path::PathBuf::from(".crow_build");
    let src_dir = build_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap_or_else(|e| {
        eprintln!("error creating build dir: {}", e);
        process::exit(1);
    });

    // Write Cargo.toml
    let project_name = entry.replace(".crow", "").replace('/', "_");
    let mut dep_list: Vec<String> = Vec::new();
    if transpiler.needs_rayon {
        dep_list.push("rayon = \"1.11\"".to_string());
    }
    if transpiler.needs_serde_json {
        dep_list.push("serde_json = \"1\"".to_string());
    }
    if transpiler.needs_crow_runtime {
        let runtime_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_runtime");
        if transpiler.web_features.contains("arrow_ipc") {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\", features = [\"arrow\"] }}", runtime_path));
        } else {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\" }}", runtime_path));
        }
    }
    if transpiler.needs_regex {
        dep_list.push("regex = \"1\"".to_string());
    }
    if transpiler.needs_sha2 {
        dep_list.push("sha2 = \"0.10\"".to_string());
        dep_list.push("md-5 = \"0.10\"".to_string());
    }
    if transpiler.needs_chrono {
        dep_list.push("chrono = \"0.4\"".to_string());
    }
    if transpiler.needs_crow_web {
        let web_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_web");
        let features: Vec<&str> = transpiler.web_features.iter().map(|s| s.as_str()).collect();
        let features_str = features.join("\", \"");
        dep_list.push(format!(
            "crow_web = {{ path = \"{}\", features = [\"{}\"] }}",
            web_path, features_str
        ));
        // crow_web needs tokio for async runtime
        dep_list.push("tokio = { version = \"1\", features = [\"full\"] }".to_string());
        // msgpack[] needs rmp-serde in the generated project
        if transpiler.web_features.contains("msgpack") {
            dep_list.push("rmp-serde = \"1\"".to_string());
        }
        // stream[] needs tokio-stream in the generated project
        if transpiler.web_features.contains("stream") {
            dep_list.push("tokio-stream = \"0.1\"".to_string());
        }
    }
    if transpiler.needs_crow_db {
        let db_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_db");
        let features: Vec<&str> = transpiler.db_features.iter().map(|s| s.as_str()).collect();
        let features_str = features.join("\", \"");
        dep_list.push(format!(
            "crow_db = {{ path = \"{}\", features = [\"{}\"] }}",
            db_path, features_str
        ));
        // serde_json may already be present from needs_serde_json
        if !dep_list.iter().any(|d| d.starts_with("serde_json")) {
            dep_list.push("serde_json = \"1\"".to_string());
        }
    }
    let deps = if dep_list.is_empty() {
        String::new()
    } else {
        format!("\n[dependencies]\n{}\n", dep_list.join("\n"))
    };
    let cargo_toml = format!(
        "[workspace]\n\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{}\n[profile.release]\nopt-level = 3\nlto = true\n",
        project_name, deps
    );
    std::fs::write(build_dir.join("Cargo.toml"), &cargo_toml).unwrap_or_else(|e| {
        eprintln!("error writing Cargo.toml: {}", e);
        process::exit(1);
    });

    // Check if source has changed — skip compilation if binary is up-to-date
    let main_rs_path = src_dir.join("main.rs");
    let binary_path = if release {
        build_dir.join("target/release").join(&project_name)
    } else {
        build_dir.join("target/debug").join(&project_name)
    };

    let existing_source = std::fs::read_to_string(&main_rs_path).unwrap_or_default();
    let source_changed = existing_source != rust_source;

    // Write transpiled Rust source (always, so cargo can detect dep changes)
    if source_changed {
        std::fs::write(&main_rs_path, &rust_source).unwrap_or_else(|e| {
            eprintln!("error writing main.rs: {}", e);
            process::exit(1);
        });
    }

    // Compile with cargo (suppress output for quiet mode, but capture errors)
    let mut cmd = std::process::Command::new("cargo");
    if release {
        cmd.args(["build", "--release"]).current_dir(&build_dir);
    } else {
        cmd.args(["build"]).current_dir(&build_dir);
    }
    if quiet {
        cmd.stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::piped());
        let output = cmd.output().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let translated = translate_rust_compile_error(&stderr);
            eprintln!("{}", translated);
            process::exit(1);
        }
    } else {
        let status = cmd.status().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        if !status.success() {
            eprintln!("compilation failed");
            process::exit(1);
        }
    }

    binary_path
}

/// Map Rust type names to Crow type names.
fn rust_type_to_crow(rust_type: &str) -> String {
    let t = rust_type.trim().trim_matches('`').trim_matches('&');
    match t {
        "i64" | "i64_" => "int".to_string(),
        "f64" | "f64_" => "float".to_string(),
        "bool" => "bool".to_string(),
        "String" | "&str" | "str" => "string".to_string(),
        s if s.starts_with("Vec<") => "array".to_string(),
        s if s.starts_with("HashMap<") => "map".to_string(),
        s if s.contains("CrowResult") => "result".to_string(),
        other => other.to_string(),
    }
}

/// Translate Rust compiler errors into Crow-friendly messages.
/// If a known pattern is found, return a clean Crow error.
/// Otherwise, fall back to showing the raw output.
fn translate_rust_compile_error(stderr: &str) -> String {
    // --- Division by zero (compile-time constant) ---
    if stderr.contains("attempt to divide") && stderr.contains("by zero") {
        return "error: cannot divide by zero".to_string();
    }
    if stderr.contains("attempt to calculate the remainder") && stderr.contains("a divisor of zero") {
        return "error: cannot divide by zero".to_string();
    }

    // --- Arithmetic overflow ---
    if stderr.contains("arithmetic operation overflowed") || stderr.contains("overflow") && stderr.contains("attempt to") {
        return "error: arithmetic overflow".to_string();
    }

    // --- Type mismatch (E0308) ---
    // e.g. "expected `&str`, found `i64`"
    if stderr.contains("mismatched types") {
        if let Some(msg) = extract_type_mismatch(stderr) {
            return format!("error: {}", msg);
        }
    }

    // --- Cannot apply operator (E0277) ---
    // e.g. "cannot add `i64` to `String`"
    if let Some(msg) = extract_cannot_op(stderr) {
        return format!("error: {}", msg);
    }

    // --- Non-primitive cast (E0605) ---
    // e.g. "non-primitive cast: `String` as `usize`"
    if stderr.contains("non-primitive cast") {
        return "error: type mismatch — cannot use string as list index (use int)".to_string();
    }

    // --- Undefined variable (E0425) ---
    // e.g. "cannot find value `x` in this scope"
    if stderr.contains("cannot find value") {
        if let Some(name) = extract_between(stderr, "cannot find value `", "`") {
            return format!("error: undefined variable '{}'", name);
        }
        return "error: undefined variable".to_string();
    }

    // --- Undefined function (E0425) ---
    if stderr.contains("cannot find function") {
        if let Some(name) = extract_between(stderr, "cannot find function `", "`") {
            return format!("error: undefined function '{}'", name);
        }
        return "error: undefined function".to_string();
    }

    // --- Immutable variable (E0384) ---
    if stderr.contains("cannot assign twice to immutable variable") {
        if let Some(name) = extract_between(stderr, "immutable variable `", "`") {
            return format!("error: cannot reassign '{}' — variable is immutable", name);
        }
        return "error: cannot reassign immutable variable".to_string();
    }

    // --- Borrow checker (E0502, E0505, E0382) ---
    if stderr.contains("borrow of moved value") || stderr.contains("value used here after move") {
        return "error: value was already used and cannot be accessed again".to_string();
    }
    if stderr.contains("cannot borrow") && stderr.contains("as mutable") {
        return "error: cannot modify value while it is being read".to_string();
    }

    // --- Missing field on struct ---
    if stderr.contains("no field") && stderr.contains("on type") {
        if let Some(field) = extract_between(stderr, "no field `", "`") {
            return format!("error: no field '{}' on this value", field);
        }
    }

    // --- Wrong number of arguments ---
    if stderr.contains("this function takes") && stderr.contains("but") && stderr.contains("supplied") {
        if let Some(msg) = extract_between(stderr, "this function takes ", " but") {
            if let Some(got) = extract_between(stderr, "but ", " was supplied")
                .or_else(|| extract_between(stderr, "but ", " were supplied")) {
                return format!("error: wrong number of arguments — expected {} but got {}", msg, got);
            }
        }
        return "error: wrong number of arguments".to_string();
    }

    // --- Index out of bounds (compile-time) ---
    if stderr.contains("index out of bounds") {
        return "error: index out of range".to_string();
    }

    // --- Fallback: extract just the error lines and clean them up ---
    let error_lines: Vec<&str> = stderr.lines()
        .filter(|l| l.starts_with("error"))
        .filter(|l| !l.contains("could not compile") && !l.contains("aborting due to"))
        .collect();
    if !error_lines.is_empty() {
        let cleaned: Vec<String> = error_lines.iter()
            .map(|l| {
                let mut s = l.to_string();
                // Strip error codes like [E0308]
                if let Some(_start) = s.find('[') {
                    if let Some(end) = s.find("]: ") {
                        s = format!("error: {}", &s[end + 3..]);
                    }
                }
                // Replace Rust types with Crow types
                s = s.replace("i64", "int").replace("f64", "float")
                     .replace("String", "string").replace("&str", "string")
                     .replace("bool", "bool").replace("usize", "int");
                s
            })
            .collect();
        return cleaned.join("\n");
    }

    // Absolute fallback
    format!("compilation failed:\n{}", stderr)
}

/// Extract "expected X, found Y" from a type mismatch error.
fn extract_type_mismatch(stderr: &str) -> Option<String> {
    let expected = extract_between(stderr, "expected `", "`")?;
    let found = extract_between(stderr, "found `", "`")?;
    Some(format!(
        "type mismatch — expected {} but got {}",
        rust_type_to_crow(&expected),
        rust_type_to_crow(&found)
    ))
}

/// Extract "cannot add/multiply/etc" operator errors.
fn extract_cannot_op(stderr: &str) -> Option<String> {
    for op in &["add", "subtract", "multiply", "divide"] {
        let pattern = format!("cannot {} `", op);
        if stderr.contains(&pattern) {
            let rhs = extract_between(stderr, &pattern, "`")?;
            let lhs = extract_between(stderr, &format!("` to `"), "`")?;
            let crow_op = match *op {
                "add" => "+",
                "subtract" => "-",
                "multiply" => "*",
                "divide" => "/",
                _ => op,
            };
            return Some(format!(
                "cannot use '{}' with {} and {}",
                crow_op,
                rust_type_to_crow(&lhs),
                rust_type_to_crow(&rhs)
            ));
        }
    }
    None
}

/// Extract text between two markers in a string.
fn extract_between<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> Option<String> {
    let start = text.find(start_marker)? + start_marker.len();
    let rest = &text[start..];
    let end = rest.find(end_marker)?;
    Some(rest[..end].to_string())
}


fn cmd_fmt() {
    let args: Vec<String> = std::env::args().collect();
    let check_mode = args.iter().any(|a| a == "--check");
    let fix_interpolation = args.iter().any(|a| a == "--fix-interpolation");
    let fix_views = args.iter().any(|a| a == "--fix-views");

    // Collect files to format
    let files: Vec<String> = args.iter().skip(2)
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();

    let files = if files.is_empty() {
        // Use crowfile entry
        if std::path::Path::new("crowfile").exists() {
            let config = parse_crowfile();
            vec![config.entry]
        } else {
            eprintln!("usage: crow fmt <file.crow> or run from a project with crowfile");
            process::exit(1);
        }
    } else {
        files
    };

    let mut any_changed = false;

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error reading {}: {}", file, e);
                process::exit(1);
            }
        };

        let original = source.clone();
        let source = if fix_interpolation {
            formatter::migrate_interpolation(&source)
        } else {
            source
        };
        let source = if fix_views {
            formatter::migrate_views(&source)
        } else {
            source
        };
        let formatted = match formatter::format_source(&source) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("error in {}: {}", file, e);
                process::exit(1);
            }
        };

        if formatted == original {
            if !check_mode {
                println!("  {} (no changes)", file);
            }
            continue;
        }

        any_changed = true;

        if check_mode {
            println!("  {} needs formatting", file);
        } else {
            std::fs::write(file, &formatted).unwrap_or_else(|e| {
                eprintln!("error writing {}: {}", file, e);
                process::exit(1);
            });
            println!("  {} (formatted)", file);
        }
    }

    if check_mode {
        if any_changed {
            process::exit(1);
        } else {
            println!("all files formatted");
        }
    }
}

fn cmd_build() {
    let args: Vec<String> = std::env::args().collect();
    let (entry, cpu) = if args.len() > 2 {
        (args[2].clone(), None)
    } else if std::path::Path::new("crowfile").exists() {
        let config = parse_crowfile();
        (config.entry, config.cpu)
    } else {
        eprintln!("usage: crow build <file.crow> or run from a project with crowfile");
        process::exit(1);
    };

    let binary = transpile_and_compile(&entry, false, cpu, true);

    // Copy binary to current directory
    let binary_name = binary.file_name().unwrap().to_string_lossy().to_string();
    let dest = std::path::PathBuf::from(&binary_name);
    if binary.exists() {
        std::fs::copy(&binary, &dest).unwrap_or_else(|e| {
            eprintln!("error copying binary: {}", e);
            process::exit(1);
        });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
        }
        println!("build ok -> ./{}", binary_name);
    } else {
        println!("build ok -> {}", binary.display());
    }
}

fn cmd_launch() {
    let args: Vec<String> = std::env::args().collect();
    // crow launch <n> <file.crow>
    // crow launch <n>  (uses crowfile)
    if args.len() < 3 {
        eprintln!("usage: crow launch <num_processes> [file.crow]");
        process::exit(1);
    }

    let n: usize = args[2].parse().unwrap_or_else(|_| {
        eprintln!("error: '{}' is not a valid number of processes", args[2]);
        process::exit(1);
    });

    if n < 1 {
        eprintln!("error: need at least 1 process");
        process::exit(1);
    }

    let entry = if args.len() > 3 {
        args[3].clone()
    } else if std::path::Path::new("crowfile").exists() {
        let config = parse_crowfile();
        config.entry
    } else {
        eprintln!("usage: crow launch <num_processes> <file.crow>");
        process::exit(1);
    };

    // Build the binary
    let binary = transpile_and_compile(&entry, true, None, true);

    let master_addr = "127.0.0.1";
    let master_port = 29500u16;

    // Spawn n processes with distributed env vars
    let mut children = Vec::new();
    for rank in 0..n {
        let child = std::process::Command::new(&binary)
            .env("CROW_RANK", rank.to_string())
            .env("CROW_WORLD_SIZE", n.to_string())
            .env("CROW_MASTER_ADDR", master_addr)
            .env("CROW_MASTER_PORT", master_port.to_string())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| {
                eprintln!("error spawning rank {}: {}", rank, e);
                process::exit(1);
            });
        children.push(child);
    }

    // Wait for all processes
    let mut any_failed = false;
    for (rank, mut child) in children.into_iter().enumerate() {
        let status = child.wait().unwrap_or_else(|e| {
            eprintln!("error waiting for rank {}: {}", rank, e);
            process::exit(1);
        });
        if !status.success() {
            eprintln!("rank {} exited with status {}", rank, status.code().unwrap_or(-1));
            any_failed = true;
        }
    }

    if any_failed {
        process::exit(1);
    }
}

struct CrowfileConfig {
    entry: String,
    cpu: Option<usize>,
    gpu: Option<usize>,
    deps: Vec<(String, String)>,  // (name, version)
    jobs: Vec<(String, String)>,  // (job_name, file_path)
}

/// Run a binary, forwarding its exit code and passing extra args.
fn run_binary(binary: &std::path::Path, extra_args: &[String]) {
    let mut child = std::process::Command::new(binary)
        .args(extra_args)
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("error running binary: {}", e);
            process::exit(1);
        });
    // Record the app's pid so `crow stop` can find it from any terminal.
    // Ownership rule: never clobber a LIVE instance's pidfile (a run that
    // dies on port collision must not erase the survivor's record), and
    // only remove the file on exit if it still names our child.
    let _ = std::fs::create_dir_all(".crow");
    let existing_alive = std::fs::read_to_string(".crow/app.pid")
        .ok()
        .and_then(|p| p.trim().parse::<i32>().ok())
        .map(|pid| unsafe { libc_kill(pid, 0) } == 0)
        .unwrap_or(false);
    if !existing_alive {
        let _ = std::fs::write(".crow/app.pid", child.id().to_string());
    }
    let status = child.wait().unwrap_or_else(|e| {
        eprintln!("error running binary: {}", e);
        process::exit(1);
    });
    if std::fs::read_to_string(".crow/app.pid")
        .map(|p| p.trim() == child.id().to_string())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(".crow/app.pid");
    }
    process::exit(status.code().unwrap_or(1));
}

/// `crow stop` — stop the project's running app. Tries the pidfile first,
/// then any process running this project's built binary.
/// `crow book [--port N]` — serve the built book with every code block
/// runnable in place. A tiny std-only HTTP server: static files from
/// docs/book/book, `POST /run` executes a block with this same binary
/// (interpreter, compiler, or `crow test`) in a scratch directory, and
/// `POST /stop` kills it. No new dependencies: the compiler already ships
/// the interpreter, the transpiler, and the runner; this just puts a
/// button in front of them.
fn cmd_book() {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.iter().position(|a| a == "--port")
        .and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok()).unwrap_or(3008);
    // The site is Crow's own: docs/book/site.crow renders every chapter
    // (use doc) with the runner built into each code block. Render it fresh
    // with the interpreter — well under a second — then serve the files.
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("crow"));
    let book_dir = std::path::PathBuf::from("docs/book/crow-site");
    let site = std::path::Path::new("docs/book/site.crow");
    if !site.exists() {
        eprintln!("error: docs/book/site.crow not found — run `crow book` from the crow repository");
        process::exit(1);
    }
    let rendered = std::process::Command::new(&exe).args(["run", "-i", "docs/book/site.crow"]).status().map(|s| s.success()).unwrap_or(false);
    if !rendered || !book_dir.join("index.html").exists() {
        eprintln!("error: rendering the book with docs/book/site.crow failed");
        process::exit(1);
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("error: cannot listen on port {}: {}", port, e);
        process::exit(1);
    });
    println!("crow book: http://localhost:{}/  (every code block has ▶ Run — Ctrl-C to stop)", port);
    let children: Arc<Mutex<std::collections::HashMap<String, u32>>> = Arc::new(Mutex::new(Default::default()));

    fn content_type(path: &std::path::Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "html" => "text/html; charset=utf-8", "js" => "application/javascript", "css" => "text/css",
            "json" => "application/json", "svg" => "image/svg+xml", "png" => "image/png", "woff2" => "font/woff2",
            "woff" => "font/woff", "ttf" => "font/ttf", "txt" => "text/plain; charset=utf-8", _ => "application/octet-stream",
        }
    }
    fn respond(stream: &mut std::net::TcpStream, status: &str, ctype: &str, body: &[u8]) {
        let _ = write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", status, ctype, body.len());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    for stream in listener.incoming() {
        let mut stream = match stream { Ok(s) => s, Err(_) => continue };
        let book_dir = book_dir.clone();
        let exe = exe.clone();
        let children = children.clone();
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            // Read the request head, then the body per Content-Length.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let head_end;
            loop {
                let n = match stream.read(&mut tmp) { Ok(0) | Err(_) => return, Ok(n) => n };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { head_end = i + 4; break; }
                if buf.len() > 1 << 20 { return; }
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.lines();
            let req_line = lines.next().unwrap_or("");
            let mut parts = req_line.split_whitespace();
            let method = parts.next().unwrap_or("GET").to_string();
            let target = parts.next().unwrap_or("/").to_string();
            let content_length: usize = lines
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse().ok()).unwrap_or(0);
            let mut body = buf[head_end..].to_vec();
            while body.len() < content_length {
                let n = match stream.read(&mut tmp) { Ok(0) | Err(_) => break, Ok(n) => n };
                body.extend_from_slice(&tmp[..n]);
            }
            let path = target.split('?').next().unwrap_or("/").to_string();

            if method == "POST" && path == "/run" {
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                let id = req["id"].as_str().unwrap_or("anon").chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
                let code = req["code"].as_str().unwrap_or("").to_string();
                let mode = req["mode"].as_str().unwrap_or("interp").to_string();
                let dir = std::env::temp_dir().join("crow_book_play").join(&id);
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("block.crow");
                let _ = std::fs::write(&file, &code);
                let (cmd_args, limit): (Vec<&str>, u64) = match mode.as_str() {
                    "compile" => (vec!["run", "block.crow"], 600),
                    "test" => (vec!["test", "block.crow"], 120),
                    _ => (vec!["run", "-i", "block.crow"], 60),
                };
                // Own process group: a served app is a grandchild (crow run →
                // native binary), and Stop must take the whole tree down.
                let mut command = std::process::Command::new(&exe);
                command.args(&cmd_args).current_dir(&dir)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
                #[cfg(unix)]
                { use std::os::unix::process::CommandExt; command.process_group(0); }
                let child = command.spawn();
                let mut child = match child {
                    Ok(c) => c,
                    Err(e) => { respond(&mut stream, "200 OK", "application/json", serde_json::json!({"ok": false, "output": format!("cannot start crow: {}", e)}).to_string().as_bytes()); return; }
                };
                children.lock().unwrap().insert(id.clone(), child.id());
                // Drain stdout/stderr on threads so a chatty server block can't block.
                let mut out_pipe = child.stdout.take();
                let mut err_pipe = child.stderr.take();
                let out_t = std::thread::spawn(move || { let mut s = String::new(); if let Some(p) = out_pipe.as_mut() { let _ = p.read_to_string(&mut s); } s });
                let err_t = std::thread::spawn(move || { let mut s = String::new(); if let Some(p) = err_pipe.as_mut() { let _ = p.read_to_string(&mut s); } s });
                let started = std::time::Instant::now();
                let mut timed_out = false;
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(st)) => break Some(st),
                        Ok(None) => {
                            if started.elapsed().as_secs() > limit {
                                let _ = std::process::Command::new("kill").arg("-TERM").arg("--").arg(format!("-{}", child.id())).status();
                                let _ = child.kill(); timed_out = true; break child.wait().ok();
                            }
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        Err(_) => break None,
                    }
                };
                children.lock().unwrap().remove(&id);
                let stdout = out_t.join().unwrap_or_default();
                let stderr = err_t.join().unwrap_or_default();
                // Strip the compiler's cargo chatter; keep the program's own lines.
                let stderr: String = stderr.lines()
                    .filter(|l| { let t = l.trim_start(); !(t.starts_with("Compiling") || t.starts_with("Finished") || t.starts_with("Running") || t.starts_with("Adding") || t.starts_with("Updating") || t.starts_with("Locking") || t.starts_with("Downloaded") || t.starts_with("Downloading") || t.starts_with("Blocking") || t.starts_with("warning:") || t.starts_with("Building") || t.starts_with("build ok")) })
                    .collect::<Vec<_>>().join("\n");
                let mut output = stdout;
                if !stderr.trim().is_empty() { if !output.is_empty() { output.push('\n'); } output.push_str(stderr.trim_end()); }
                // `crow test` colors its report; a <pre> wants plain text
                let output = {
                    let mut s = String::with_capacity(output.len());
                    let mut chars = output.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '\u{1b}' && chars.peek() == Some(&'[') {
                            for d in chars.by_ref() { if d.is_ascii_alphabetic() { break; } }
                        } else { s.push(c); }
                    }
                    s
                };
                let ok = status.map(|s| s.success()).unwrap_or(false) && !timed_out;
                let reply = serde_json::json!({
                    "ok": ok, "output": output,
                    "timeout": if timed_out { format!("after {}s", limit) } else { String::new() },
                });
                respond(&mut stream, "200 OK", "application/json", reply.to_string().as_bytes());
                return;
            }
            if method == "POST" && path == "/stop" {
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                let id = req["id"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
                let pid = children.lock().unwrap().remove(&id);
                if let Some(pid) = pid {
                    // The block runs in its own process group (pgid == pid):
                    // signal the group so the native binary behind `crow run`
                    // goes down with it, then the parent for good measure.
                    let _ = std::process::Command::new("kill").arg("-TERM").arg("--").arg(format!("-{}", pid)).status();
                    let _ = std::process::Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
                }
                respond(&mut stream, "200 OK", "application/json", b"{\"ok\":true}");
                return;
            }
            // Static book files.
            let rel = path.trim_start_matches('/');
            let rel = if rel.is_empty() { "index.html" } else { rel };
            let file = book_dir.join(rel);
            let safe = file.canonicalize().ok().map(|f| f.starts_with(book_dir.canonicalize().unwrap_or(book_dir.clone()))).unwrap_or(false);
            match if safe { std::fs::read(&file).ok() } else { None } {
                Some(bytes) => respond(&mut stream, "200 OK", content_type(&file), &bytes),
                None => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"not found"),
            }
        });
    }
}

fn cmd_stop() {
    if let Ok(pid_s) = std::fs::read_to_string(".crow/app.pid") {
        if let Ok(pid) = pid_s.trim().parse::<i32>() {
            let alive = unsafe { libc_kill(pid, 0) } == 0;
            if alive {
                unsafe { libc_kill(pid, 15) };
                let _ = std::fs::remove_file(".crow/app.pid");
                println!("stopped (pid {})", pid);
                return;
            }
        }
        // Stale pidfile — the app died some other way.
        let _ = std::fs::remove_file(".crow/app.pid");
    }
    // Fallback: any process running a .crow_build binary WHOSE WORKING
    // DIRECTORY is this project — matches absolute, ./-relative, and bare
    // relative launches, and never touches another project's app.
    let here = std::env::current_dir().unwrap_or_default();
    let here_s = here.to_string_lossy().to_string();
    // Candidates: the cargo-built binary AND the copied ./main that
    // `crow build` announces. Over-matching is safe — every candidate is
    // cwd-verified against this project before any signal is sent.
    let mut pids = String::new();
    for pattern in [".crow_build/target", "^\\./main( |$)", "/main$"] {
        if let Ok(o) = std::process::Command::new("pgrep").args(["-f", pattern]).output() {
            pids.push_str(&String::from_utf8_lossy(&o.stdout));
            pids.push('\n');
        }
    }
    let mut stopped = 0;
    let mut seen: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    for pid_s in pids.split_whitespace() {
        let Ok(pid) = pid_s.parse::<i32>() else { continue };
        if !seen.insert(pid) {
            continue;
        }
        let cwd = std::process::Command::new("lsof")
            .args(["-a", "-p", pid_s, "-d", "cwd", "-Fn"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if cwd.lines().any(|l| l.strip_prefix('n') == Some(here_s.as_str())) {
            unsafe { libc_kill(pid, 15) };
            stopped += 1;
        }
    }
    if stopped > 0 {
        println!("stopped ({} process{})", stopped, if stopped == 1 { "" } else { "es" });
    } else {
        println!("nothing running — start with: crow run");
    }
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// Record a deliberate render-time remote read: insert the acknowledgment
/// comment above the flagged view so the finding stays visible in the code
/// instead of in every check run.
fn apply_remote_accept(filename: &str, view_line_1based: usize) -> Result<(), String> {
    let source = read_file(filename);
    let mut lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();
    if view_line_1based == 0 || view_line_1based > lines.len() {
        return Err("could not locate the view declaration".into());
    }
    lines.insert(view_line_1based - 1, "--- crow: remote-render".to_string());
    std::fs::write(filename, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

/// Tier-2 applier for blocking-in-view: inside `view`'s body, rewrite the
/// bare call `helper[]` to `later[helper]` — the work moves off the live
/// loop and its state writes re-render every client. Only fires on the
/// exact zero-arg statement shape; anything else stays a manual edit.
fn apply_later_fix(filename: &str, view: &str, helper: &str) -> Result<(), String> {
    let source = read_file(filename);
    let mut lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();
    let decl = format!("fn {}[", view);
    let start = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&decl))
        .ok_or_else(|| format!("could not find fn {}", view))?;
    let needle = format!("{}[]", helper);
    for line in lines.iter_mut().skip(start + 1) {
        let trimmed = line.trim_start();
        if !line.starts_with(' ') && !trimmed.is_empty() {
            break; // left the fn body
        }
        if trimmed == needle {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            *line = format!("{}later[{}]", indent, helper);
            let new_source = lines.join("\n") + "\n";
            build(&new_source).map_err(|e| format!("rewrite would not parse: {}", e))?;
            std::fs::write(filename, new_source)
                .map_err(|e| format!("could not write {}: {}", filename, e))?;
            return Ok(());
        }
    }
    Err(format!(
        "no bare `{}[]` call in {} — the call passes arguments or is part of an expression; move it into a zero-arg fn and call later[…] manually",
        helper, view
    ))
}

/// `crow test` — run only the authored test blocks in a file.
fn cmd_test_blocks(filename: &str) {
    let source = read_file(filename);
    let stmts = match build(&source) {
        Ok(s) => s,
        Err(e) => { eprintln!("{}", e); process::exit(1); }
    };
    let stmts = match lower::lower_program(&stmts) {
        Ok(s) => s,
        Err(e) => { eprintln!("{}", e); process::exit(1); }
    };
    let (issues, counts) = checker::run_test_blocks(&stmts);
    if counts.blocks == 0 {
        println!("  no test blocks in {} — add `test` after a fn, or `test name[]` anywhere", filename);
        return;
    }
    if issues.is_empty() {
        println!("\n  \x1b[32m✓ {} block{} pass\x1b[0m — {} truth{}, {} law{}, {} input{} run",
            counts.blocks, if counts.blocks == 1 { "" } else { "s" },
            counts.truths, if counts.truths == 1 { "" } else { "s" },
            counts.laws, if counts.laws == 1 { "" } else { "s" },
            counts.inputs_run, if counts.inputs_run == 1 { "" } else { "s" });
        return;
    }
    println!("\n  \x1b[33m⚠ {} failing\x1b[0m of {} block{}\n",
        issues.len(), counts.blocks, if counts.blocks == 1 { "" } else { "s" });
    for issue in &issues {
        println!("  \x1b[90m{}:{}\x1b[0m", filename, issue.line());
        println!("     {}\n", issue.summary());
    }
    process::exit(1);
}

fn run_check_pipeline(filename: &str) -> checker::CheckResult {
    let source = read_file(filename);
    let stmts = match build(&source) {
        Ok(stmts) => stmts,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    };
    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(&stmts);
    let mut result = checker::run_check(&stmts, &ctx, false);
    result.issues = checker::suppress_acknowledged(std::mem::take(&mut result.issues), &source);
    result
}

fn print_check_summary(result: &checker::CheckResult, verbose: bool) {
    let tc = result.test_counts;
    if tc.blocks > 0 {
        println!(
            "\n  tests: {} block{} | {} truth{} | {} law{} | {} input{} run",
            tc.blocks, if tc.blocks == 1 { "" } else { "s" },
            tc.truths, if tc.truths == 1 { "" } else { "s" },
            tc.laws, if tc.laws == 1 { "" } else { "s" },
            tc.inputs_run, if tc.inputs_run == 1 { "" } else { "s" },
        );
    }
    if result.issues.is_empty() {
        println!("\n  \x1b[32m✓ all good\x1b[0m — {} functions checked, {} edge cases generated",
            result.total_functions, result.edge_cases_generated);
    } else {
        println!("\n  \x1b[33m⚠ {} issue{}\x1b[0m — {} functions checked, {} edge cases generated",
            result.issues.len(),
            if result.issues.len() == 1 { "" } else { "s" },
            result.total_functions,
            result.edge_cases_generated);
    }
    if verbose {
        println!("  pure: {:?}", result.pure_fns);
        println!("  impure: {:?}", result.impure_fns);
        println!("  snapshots: {} saved, {} unchanged",
            result.snapshots_saved, result.snapshots_unchanged);
    }
}

/// Format a clickable location string like "file.crow:12 in add"
fn format_location(filename: &str, issue: &checker::Issue, fn_lines: &std::collections::BTreeMap<String, usize>) -> String {
    let func = issue.func();
    let line = issue.line();

    // For issues with a direct line number, use that
    if line > 0 {
        if func == "<top-level>" {
            return format!("{}:{}", filename, line);
        }
        return format!("{}:{} in {}", filename, line, func);
    }

    // For edge case / regression issues, look up the function's declaration line
    if func != "<top-level>" {
        if let Some(&fn_line) = fn_lines.get(func) {
            if fn_line > 0 {
                return format!("{}:{} in {}", filename, fn_line, func);
            }
        }
        return format!("{} in {}", filename, func);
    }

    filename.to_string()
}

fn cmd_check(filename: &str, verbose: bool, db_url: Option<String>, strict: bool) {
    use std::io::{self, Write, BufRead};

    let mut result = run_check_pipeline(filename);
    print_check_summary(&result, verbose);

    // Schema-aware database checks (Level 1). Advisory unless --strict.
    let source = read_file(filename);
    let resolved_db = db_url.or_else(crowfile_db_url);
    match db_check::run(filename, &source, resolved_db.as_deref()) {
        Ok(db_result) => {
            if let Some(report) =
                db_check::report::render_all(&db_result.hazards, &source)
            {
                let n = db_result.hazards.len();
                println!(
                    "\n  database checks: \x1b[33m{} finding{}\x1b[0m\n",
                    n,
                    if n == 1 { "" } else { "s" }
                );
                println!("{}", report);
                if strict {
                    process::exit(1);
                }
            } else if db_result.schema_checked {
                println!("  \x1b[32m✓ database checks clean\x1b[0m — {} SQL call{} verified against schema",
                    db_result.sql_calls, if db_result.sql_calls == 1 { "" } else { "s" });
            } else if db_result.sql_calls > 0 {
                println!(
                    "  \x1b[90m{} SQL call{} found — pass --db <url> (or set db = '…' in crowfile) for schema-aware checks\x1b[0m",
                    db_result.sql_calls,
                    if db_result.sql_calls == 1 { "" } else { "s" }
                );
            }
        }
        Err(e) => println!("  \x1b[90mdatabase checks skipped: {}\x1b[0m", e),
    }

    if result.issues.is_empty() {
        return;
    }

    // Verbose mode: dump all issues and exit (for CI)
    if verbose {
        println!();
        for (i, issue) in result.issues.iter().enumerate() {
            let loc = format_location(filename, issue, &result.fn_lines);
            println!("  {}. \x1b[90m{}\x1b[0m", i + 1, loc);
            println!("     {}", issue.summary());
            println!("     fix: {}", issue.suggest());
        }
        process::exit(1);
    }

    // Interactive mode: present issues one at a time
    let stdin = io::stdin();
    let mut idx = 0;

    while idx < result.issues.len() {
        let issue = &result.issues[idx];
        let total = result.issues.len();

        // Present the issue
        println!();
        let loc = format_location(filename, issue, &result.fn_lines);
        println!("  \x1b[1m[{}/{}]\x1b[0m \x1b[90m{}\x1b[0m", idx + 1, total, loc);
        println!("        {}", issue.summary());
        println!("        \x1b[36mfix: {}\x1b[0m", issue.suggest());

        if let Some(try_cmd) = issue.try_input() {
            println!("        try: {}", try_cmd);
        }

        // Show prompt with available commands
        let has_try = issue.try_input().is_some();
        let is_regression = matches!(issue, checker::Issue::RegressionChange { .. });
        let later_fixable = matches!(issue,
            checker::Issue::BlockingInView { path, .. } if path.len() >= 3);
        let remote_acceptable = matches!(issue, checker::Issue::RemoteQueryInView { .. });
        let mut prompt_parts = vec!["enter = recheck", "s = skip"];
        if has_try { prompt_parts.push("t = try input"); }
        if is_regression { prompt_parts.push("a = accept new"); }
        if remote_acceptable { prompt_parts.push("a = accept (deliberate)"); }
        if later_fixable { prompt_parts.push("f = fix (later[])"); }
        prompt_parts.push("q = quit");
        // No terminal (a script, CI, an editor): report and stop instead of waiting
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            println!("\n        (no terminal — not waiting for input)");
            process::exit(1);
        }
        print!("\n        {} > ", prompt_parts.join(", "));
        io::stdout().flush().unwrap();

        // Read command
        let mut input = String::new();
        match stdin.lock().read_line(&mut input) {
            Ok(0) => { println!(); break; } // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        match input.trim() {
            "" => {
                // Recheck: re-read file, re-run pipeline
                println!("\n  rechecking...");
                result = run_check_pipeline(filename);
                print_check_summary(&result, false);
                if result.issues.is_empty() {
                    break;
                }
                // Reset to first issue (the list may have changed)
                idx = 0;
            }
            "s" => {
                idx += 1;
            }
            "a" if remote_acceptable => {
                if let checker::Issue::RemoteQueryInView { line, view, .. } = issue {
                    let (line, view) = (*line, view.clone());
                    match apply_remote_accept(filename, line) {
                        Ok(()) => {
                            println!("\n  recorded: --- crow: remote-render above '{}' — rechecking...", view);
                            result = run_check_pipeline(filename);
                            print_check_summary(&result, false);
                            if result.issues.is_empty() { break; }
                            idx = 0;
                        }
                        Err(e) => println!("\n  could not apply: {}", e),
                    }
                }
            }
            "f" if later_fixable => {
                if let checker::Issue::BlockingInView { view, path, .. } = issue {
                    let helper = path[1].clone();
                    let view = view.clone();
                    match apply_later_fix(filename, &view, &helper) {
                        Ok(()) => {
                            println!("
  rewrote {}: {}[] → later[{}] — rechecking...", filename, helper, helper);
                            result = run_check_pipeline(filename);
                            print_check_summary(&result, false);
                            if result.issues.is_empty() { break; }
                            idx = 0;
                        }
                        Err(e) => println!("
  could not apply: {}", e),
                    }
                }
            }
            "t" if has_try => {
                // Run the try input through the interpreter
                if let Some(try_cmd) = issue.try_input() {
                    println!();
                    let source = read_file(filename);
                    let stmts = match build(&source) {
                        Ok(s) => s,
                        Err(e) => { eprintln!("  parse error: {}", e); continue; }
                    };
                    // Suppress panic output during try
                    let prev_hook = std::panic::take_hook();
                    std::panic::set_hook(Box::new(|_| {}));
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut interp = interpreter::Interpreter::new();
                        // Load function declarations (ignore top-level errors)
                        let _ = interp.run(&stmts);
                        let call_src = format!("log[{}]", try_cmd);
                        match build(&call_src) {
                            Ok(call_stmts) => {
                                match interp.run(&call_stmts) {
                                    Ok(_) => None,
                                    Err(e) => Some(e),
                                }
                            }
                            Err(e) => Some(format!("parse error: {}", e)),
                        }
                    }));
                    std::panic::set_hook(prev_hook);
                    match result {
                        Ok(Some(e)) => println!("  error: {}", e),
                        Ok(None) => {} // output already printed by log[]
                        Err(panic_info) => {
                            let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else {
                                "panic during execution".to_string()
                            };
                            println!("  crash: {}", msg);
                        }
                    }
                }
            }
            "a" if is_regression => {
                // Accept new behavior — skip this issue, snapshot already saved
                println!("  accepted");
                idx += 1;
            }
            "q" => break,
            other => {
                println!("  unknown command: '{}' — press enter to recheck, s to skip", other);
            }
        }
    }

    if !result.issues.is_empty() {
        process::exit(1);
    }
}

fn cmd_run(extra_args: &[String]) {
    let config = parse_crowfile();
    let binary = transpile_and_compile(&config.entry, true, config.cpu, false);
    run_binary(&binary, extra_args);
}

fn cmd_run_file(filename: &str, extra_args: &[String]) {
    let binary = transpile_and_compile(filename, true, None, false);
    run_binary(&binary, extra_args);
}

fn cmd_run_job(job_name: &str) {
    let config = parse_crowfile();
    let job_file = config.jobs.iter()
        .find(|(name, _)| name == job_name)
        .map(|(_, path)| path.clone());

    match job_file {
        Some(path) => {
            let binary = transpile_and_compile(&path, true, config.cpu, false);
            run_binary(&binary, &[]);
        }
        None => {
            eprintln!("unknown job '{}'. available jobs:", job_name);
            if config.jobs.is_empty() {
                eprintln!("  (none — add [jobs] section to crowfile)");
            } else {
                for (name, path) in &config.jobs {
                    eprintln!("  {} → {}", name, path);
                }
            }
            process::exit(1);
        }
    }
}

fn cmd_run_interpreted() {
    let config = parse_crowfile();

    let compiled_path = config.entry.replace(".crow", ".crowc");
    let stmts = if std::path::Path::new(&compiled_path).exists() {
        let bytes = std::fs::read(&compiled_path).unwrap_or_else(|e| {
            eprintln!("error reading '{}': {}", compiled_path, e);
            process::exit(1);
        });
        bincode::deserialize(&bytes).unwrap_or_else(|e| {
            eprintln!("error loading bytecode '{}': {}", compiled_path, e);
            process::exit(1);
        })
    } else {
        let source = read_file(&config.entry);
        match build(&source) {
            Ok(stmts) => stmts,
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
    };

    let mut interpreter = Interpreter::new();
    let entry_file = std::path::Path::new(&config.entry).file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| config.entry.clone());
    interpreter.set_source_file(&entry_file);
    if let Some(dir) = std::path::Path::new(&config.entry).parent() {
        if dir.as_os_str().len() > 0 {
            interpreter.set_source_dir(dir.to_path_buf());
        }
    }
    if let Some(n) = config.cpu {
        interpreter.set_cpu_count(n);
    }
    if let Some(n) = config.gpu {
        interpreter.set_gpu_count(n);
    }
    if !config.deps.is_empty() {
        let modules_dir = std::path::PathBuf::from("crow_modules");
        if !modules_dir.exists() {
            eprintln!("warning: deps defined but 'crow_modules/' directory not found");
        }
        interpreter.add_module_path(modules_dir);
        for (name, version) in &config.deps {
            let dep_file = std::path::PathBuf::from("crow_modules").join(format!("{}.crow", name));
            let dep_dir = std::path::PathBuf::from("crow_modules").join(name).join("main.crow");
            if !dep_file.exists() && !dep_dir.exists() {
                eprintln!("warning: dependency '{}' v{} not found in crow_modules/", name, version);
            }
        }
    }
    if let Err(e) = interpreter.run(&stmts) {
        eprintln!("runtime error: {}", e);
        process::exit(1);
    }
}

fn cmd_run_file_interpreted(filename: &str) {
    let stmts = if filename.ends_with(".crowc") {
        let bytes = std::fs::read(filename).unwrap_or_else(|e| {
            eprintln!("error reading '{}': {}", filename, e);
            process::exit(1);
        });
        bincode::deserialize(&bytes).unwrap_or_else(|e| {
            eprintln!("error loading bytecode '{}': {}", filename, e);
            process::exit(1);
        })
    } else {
        let source = read_file(filename);
        let stmts = match build(&source) {
            Ok(stmts) => stmts,
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        };
        // Expand user-module imports exactly as the compiled path does — the
        // lowerer needs the prefixed fns to resolve qualified named-arg calls
        let source_dir = std::path::Path::new(filename)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
        // Lower + validate exactly as the compiled path does — the two modes
        // must reject the same programs (reservation, scope, ui rules)
        // (user modules are already inlined; the interpreter must not
        // dynamically re-load them — see set_modules_preexpanded below)
        let lowered = match lower::lower_program(&stmts) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        };
        let mut vctx = type_infer::TypeContext::new();
        vctx.infer_program(&lowered);
        let mut v = validator::Validator::new(&vctx);
        if let Err(e) = v.validate(&lowered) {
            eprintln!("{}", refine_error_lines(&source, &e));
            process::exit(1);
        }
        stmts
    };

    let mut interpreter = Interpreter::new();
    interpreter.set_modules_preexpanded(true);
    let base = filename.replace(".crowc", ".crow");
    let source_file = std::path::Path::new(&base).file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| base.clone());
    interpreter.set_source_file(&source_file);
    if let Some(dir) = std::path::Path::new(&base).parent() {
        if dir.as_os_str().len() > 0 {
            interpreter.set_source_dir(dir.to_path_buf());
        }
    }
    if let Err(e) = interpreter.run(&stmts) {
        eprintln!("runtime error: {}", e);
        process::exit(1);
    }
}

/// Lenient lookup of the optional `db = 'url'` variable in the crowfile.
/// Unlike parse_crowfile this never exits — no crowfile just means no url.
fn crowfile_db_url() -> Option<String> {
    crowfile_db_url_at(std::path::Path::new("."))
}

/// Same lookup rooted at an arbitrary directory (the LSP workspace root).
pub(crate) fn crowfile_db_url_at(dir: &std::path::Path) -> Option<String> {
    let source = std::fs::read_to_string(dir.join("crowfile")).ok()?;
    let stmts = build(&source).ok()?;
    let mut interpreter = Interpreter::new();
    interpreter.run(&stmts).ok()?;
    match interpreter.get_var("db") {
        Some(val) => match &val.value {
            value::Value::Str(s) => Some(s.clone()),
            _ => None,
        },
        None => None,
    }
}

fn parse_crowfile() -> CrowfileConfig {
    let crowfile_source = match std::fs::read_to_string("crowfile") {
        Ok(s) => s,
        Err(_) => {
            eprintln!("no crowfile found. run 'crow new <name>' first");
            process::exit(1);
        }
    };

    match build(&crowfile_source) {
        Ok(stmts) => {
            let mut interpreter = Interpreter::new();
            if let Err(e) = interpreter.run(&stmts) {
                eprintln!("error in crowfile: {}", e);
                process::exit(1);
            }

            let entry = match interpreter.get_var("entry") {
                Some(val) => match &val.value {
                    value::Value::Str(s) => s.clone(),
                    _ => {
                        eprintln!("crowfile: 'entry' must be a string");
                        process::exit(1);
                    }
                },
                None => "main.crow".to_string(),
            };

            let cpu = interpreter.get_var("cpu").and_then(|val| {
                if let value::Value::Int(n) = &val.value {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            let gpu = interpreter.get_var("gpu").and_then(|val| {
                if let value::Value::Int(n) = &val.value {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            // parse deps: [math = '1.0', http = '2.3']
            let deps = interpreter.get_var("deps").map(|val| {
                if let value::Value::Map(map) = &val.value {
                    map.iter().filter_map(|(name, v)| {
                        if let value::Value::Str(ver) = &v.value {
                            Some((name.clone(), ver.clone()))
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            }).unwrap_or_default();

            // parse [jobs] section: jobs = [train = 'jobs/train.crow', ...]
            let jobs = interpreter.get_var("jobs").map(|val| {
                if let value::Value::Map(map) = &val.value {
                    map.iter().filter_map(|(name, v)| {
                        if let value::Value::Str(path) = &v.value {
                            Some((name.clone(), path.clone()))
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            }).unwrap_or_default();

            CrowfileConfig { entry, cpu, gpu, deps, jobs }
        }
        Err(e) => {
            eprintln!("error in crowfile: {}", e);
            process::exit(1);
        }
    }
}

fn read_file(filename: &str) -> String {
    match std::fs::read_to_string(filename) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading '{}': {}", filename, e);
            process::exit(1);
        }
    }
}

pub(crate) fn build(source: &str) -> Result<Vec<ast::Stmt>, String> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}


/// Statement-granular line markers make errors inside a multi-line
/// expression report the statement's first line — a 30-line view tree
/// reports `return div[` for a typo 25 lines below it. When a message
/// names a quoted identifier, walk the statement's bracket extent and
/// point at the line that actually contains it.
pub fn refine_error_line(source: &str, line_1based: usize, msg: &str) -> usize {
    let name = match msg.find('\'') {
        Some(a) => {
            let rest = &msg[a + 1..];
            match rest.find('\'') {
                Some(b) => &rest[..b],
                None => return line_1based,
            }
        }
        None => return line_1based,
    };
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return line_1based;
    }
    let lines: Vec<&str> = source.lines().collect();
    if line_1based == 0 || line_1based > lines.len() {
        return line_1based;
    }
    let start = line_1based - 1;
    let contains_word = |l: &str| -> bool {
        // Mask string literals: a name inside '...' is text, not code.
        let mut masked = String::with_capacity(l.len());
        let mut in_str = false;
        for c in l.chars() {
            if c == '\'' {
                in_str = !in_str;
                masked.push(' ');
            } else {
                masked.push(if in_str { ' ' } else { c });
            }
        }
        let l = masked.as_str();
        let bytes = l.as_bytes();
        let mut from = 0;
        while let Some(pos) = l[from..].find(name) {
            let i = from + pos;
            let before_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let end = i + name.len();
            let after_ok = end >= bytes.len()
                || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
            if before_ok && after_ok {
                return true;
            }
            from = end;
        }
        false
    };
    let mut depth = 0i32;
    for (idx, l) in lines.iter().enumerate().skip(start) {
        if idx > start && depth <= 0 {
            break;
        }
        if contains_word(l) {
            return idx + 1;
        }
        let mut in_str = false;
        for c in l.chars() {
            match c {
                '\'' => in_str = !in_str,
                '[' | '(' if !in_str => depth += 1,
                ']' | ')' if !in_str => depth -= 1,
                _ => {}
            }
        }
    }
    line_1based
}

/// Rewrite every `line N:` prefix in a (possibly multi-line) error
/// message to the refined line.
pub fn refine_error_lines(source: &str, err: &str) -> String {
    err.lines()
        .map(|entry| {
            let t = entry.trim_start();
            if let Some(rest) = t.strip_prefix("line ") {
                if let Some(colon) = rest.find(':') {
                    if let Ok(n) = rest[..colon].parse::<usize>() {
                        let refined = refine_error_line(source, n, &rest[colon..]);
                        if refined != n {
                            return entry.replacen(
                                &format!("line {}:", n),
                                &format!("line {}:", refined),
                                1,
                            );
                        }
                    }
                }
            }
            entry.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod refine_tests {
    #[test]
    fn refines_into_multiline_trees_and_leaves_correct_lines_alone() {
        let src = "x = 1\nv = div[\n    p[a],\n    p[missing_name],\n    p[b]\n]\ny = 2\n";
        // Statement starts at line 2; the offender is on line 4.
        assert_eq!(crate::refine_error_line(src, 2, "undefined variable 'missing_name'"), 4);
        // Already-correct lines stay put.
        assert_eq!(crate::refine_error_line(src, 1, "undefined variable 'x'"), 1);
        // A name inside a string literal on an earlier line must not match.
        let src2 = "v = div[\n    p['missing_name is fine here'],\n    p[missing_name]\n]\n";
        assert_eq!(crate::refine_error_line(src2, 1, "undefined variable 'missing_name'"), 3);
        // Unquoted messages pass through.
        assert_eq!(crate::refine_error_line(src, 2, "some other error"), 2);
    }
}

/// Expand `use` imports by parsing module files and inserting prefixed
/// FnDecl/StructDecl/Assign statements. This must run *before* type inference
/// so the main program's call sites can inform module function param types.
fn expand_module_imports(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    visited: &mut std::collections::HashSet<String>,
) -> Vec<ast::Stmt> {
    let mut expanded = Vec::new();
    let mut expanded_modules: Vec<(String, std::collections::BTreeMap<String, String>)> = Vec::new();
    for stmt in stmts {
        if let ast::Stmt::Use { path, alias } = stmt {
            // Built-in modules: skip file lookup
            if path == "math" || path.starts_with("math.") {
                expanded.push(stmt.clone());
                continue;
            }

            // Convert dot-separated path to file path (views.home → views/home)
            let file_rel = path.replace('.', "/");
            // `use path as alias` renames the module prefix
            let module_name = alias.clone().unwrap_or_else(|| {
                path.rsplit(['.', '/']).next()
                    .and_then(|s| if s.is_empty() { None } else { Some(s) })
                    .unwrap_or(path)
                    .to_string()
            });

            // Circular import detection
            if visited.contains(path) {
                expanded.push(stmt.clone());
                continue;
            }
            visited.insert(path.clone());

            // Try to find the module file
            let candidates = vec![
                source_dir.join(format!("{}.crow", file_rel)),
                source_dir.join(format!("crow_modules/{}.crow", file_rel)),
                source_dir.join(format!("crow_modules/{}/main.crow", file_rel)),
            ];
            let file_path = candidates.iter().find(|p| p.exists());
            let file_path = match file_path {
                Some(p) => p.clone(),
                None => {
                    expanded.push(stmt.clone());
                    continue;
                }
            };
            let source = match std::fs::read_to_string(&file_path) {
                Ok(s) => s,
                Err(_) => {
                    expanded.push(stmt.clone());
                    continue;
                }
            };
            // Parse the module
            let mut lexer = Lexer::new(&source);
            let tokens = match lexer.tokenize() {
                Ok(t) => t,
                Err(_) => continue,
            };
            let mut parser = Parser::new(tokens);
            let mod_stmts = match parser.parse() {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Recursively expand any `use` statements within the module
            let mod_dir = file_path.parent().unwrap_or(source_dir);
            let mod_stmts = expand_module_imports(&mod_stmts, mod_dir, visited);

            // Collect the module's own top-level names, then rewrite both the
            // declarations AND every reference inside module bodies — a module
            // fn must still reach its own structs/fns/vars after prefixing.
            let mut local_names: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for mod_stmt in &mod_stmts {
                match mod_stmt {
                    ast::Stmt::FnDecl { name, .. }
                    | ast::Stmt::StructDecl { name, .. }
                    | ast::Stmt::Assign { name, .. } => {
                        local_names
                            .insert(name.clone(), format!("{}_{}", module_name, name));
                    }
                    _ => {}
                }
            }

            // Rewrite module declarations with prefixed names
            for mod_stmt in &mod_stmts {
                match mod_stmt {
                    ast::Stmt::FnDecl { name, params, defaults, body, is_view } => {
                        let mut body = body.clone();
                        // Params shadow module names inside the body
                        let shadowed: std::collections::HashSet<String> =
                            params.iter().cloned().collect();
                        for st in &mut body {
                            rewrite_refs_stmt(st, &local_names, &shadowed);
                        }
                        expanded.push(ast::Stmt::FnDecl {
                            name: format!("{}_{}", module_name, name),
                            params: params.clone(),
                            defaults: defaults.clone(),
                            body,
                            is_view: *is_view,
                        });
                    }
                    ast::Stmt::StructDecl { name, fields } => {
                        expanded.push(ast::Stmt::StructDecl {
                            name: format!("{}_{}", module_name, name),
                            fields: fields.clone(),
                        });
                    }
                    ast::Stmt::Assign { name, value } => {
                        // Module-level variables become prefixed assigns
                        let mut value = value.clone();
                        rewrite_refs_expr(
                            &mut value,
                            &local_names,
                            &std::collections::HashSet::new(),
                        );
                        expanded.push(ast::Stmt::Assign {
                            name: format!("{}_{}", module_name, name),
                            value,
                        });
                    }
                    // Pass through nested Use statements so the transpiler
                    // registers those module names too
                    ast::Stmt::Use { .. } => {
                        expanded.push(mod_stmt.clone());
                    }
                    _ => {}
                }
            }
            expanded_modules.push((module_name.clone(), local_names.clone()));
        }
        // Keep the original statement (including Use, so transpiler knows about it)
        expanded.push(stmt.clone());
    }
    // Rewrite qualified refs in the importing file: home.render → home_render
    for (module_name, local_names) in &expanded_modules {
        for st in &mut expanded {
            rewrite_qualified_refs_stmt(st, module_name, local_names);
        }
    }
    expanded
}

/// Rewrite references to a module's own top-level names (fns, structs, vars)
/// to their prefixed forms inside module bodies. `shadowed` holds names bound
/// locally (params, locals) that must NOT be rewritten. String interpolation
/// contents are not rewritten (known limitation — interpolate locals, not
/// module-level names, inside module strings).
fn rewrite_refs_stmt(
    stmt: &mut ast::Stmt,
    map: &std::collections::BTreeMap<String, String>,
    shadowed: &std::collections::HashSet<String>,
) {
    match stmt {
        ast::Stmt::Assign { name, value } => {
            // The RHS first; then the target: a module-level name assigned in
            // a view or handler is the module's reactive state, so it takes
            // the prefix too (a local of the same name is prefixed alike, and
            // every read of it in the body was rewritten the same way)
            rewrite_refs_expr(value, map, shadowed);
            if !shadowed.contains(name) {
                if let Some(prefixed) = map.get(name) {
                    *name = prefixed.clone();
                }
            }
        }
        ast::Stmt::MultiAssign { names, values, .. } => {
            for v in values {
                rewrite_refs_expr(v, map, shadowed);
            }
            for n in names {
                if !shadowed.contains(n) {
                    if let Some(prefixed) = map.get(n) {
                        *n = prefixed.clone();
                    }
                }
            }
        }
        ast::Stmt::Return(Some(e)) | ast::Stmt::ExprStmt(e) => {
            rewrite_refs_expr(e, map, shadowed)
        }
        ast::Stmt::If { condition, then_body, else_body } => {
            rewrite_refs_expr(condition, map, shadowed);
            for st in then_body {
                rewrite_refs_stmt(st, map, shadowed);
            }
            if let Some(eb) = else_body {
                for st in eb {
                    rewrite_refs_stmt(st, map, shadowed);
                }
            }
        }
        ast::Stmt::For { var, iter, body } => {
            rewrite_refs_expr(iter, map, shadowed);
            let mut inner = shadowed.clone();
            inner.insert(var.clone());
            for st in body {
                rewrite_refs_stmt(st, map, &inner);
            }
        }
        ast::Stmt::While { condition, body } => {
            rewrite_refs_expr(condition, map, shadowed);
            for st in body {
                rewrite_refs_stmt(st, map, shadowed);
            }
        }
        ast::Stmt::FnDecl { params, body, .. } => {
            let mut inner = shadowed.clone();
            for p in params {
                inner.insert(p.clone());
            }
            for st in body {
                rewrite_refs_stmt(st, map, &inner);
            }
        }
        ast::Stmt::FieldAssign { object, value, .. } => {
            rewrite_refs_expr(object, map, shadowed);
            rewrite_refs_expr(value, map, shadowed);
        }
        ast::Stmt::IndexAssign { object, index, value } => {
            rewrite_refs_expr(object, map, shadowed);
            rewrite_refs_expr(index, map, shadowed);
            rewrite_refs_expr(value, map, shadowed);
        }
        ast::Stmt::Resource { count, .. } => rewrite_refs_expr(count, map, shadowed),
        _ => {}
    }
}

/// Rewrite qualified module references in the importing file:
/// `home.render` → `home_render` for every name the module defines.
/// Runs after a module is expanded so both modes see plain idents.
fn rewrite_qualified_refs_stmt(
    stmt: &mut ast::Stmt,
    module_name: &str,
    local_names: &std::collections::BTreeMap<String, String>,
) {
    let map = |e: &mut ast::Expr| rewrite_qualified_refs_expr(e, module_name, local_names);
    match stmt {
        ast::Stmt::Assign { value, .. } => map(value),
        ast::Stmt::ExprStmt(e) => map(e),
        ast::Stmt::Return(Some(e)) => map(e),
        ast::Stmt::If { condition, then_body, else_body, .. } => {
            map(condition);
            for st in then_body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
            if let Some(eb) = else_body {
                for st in eb {
                    rewrite_qualified_refs_stmt(st, module_name, local_names);
                }
            }
        }
        ast::Stmt::While { condition, body } => {
            map(condition);
            for st in body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
        }
        ast::Stmt::For { iter, body, .. } => {
            map(iter);
            for st in body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
        }
        ast::Stmt::FnDecl { body, .. } => {
            for st in body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
        }
        ast::Stmt::IndexAssign { object, index, value } => {
            map(object);
            map(index);
            map(value);
        }
        _ => {}
    }
}

fn rewrite_qualified_refs_expr(
    expr: &mut ast::Expr,
    module_name: &str,
    local_names: &std::collections::BTreeMap<String, String>,
) {
    // home.render → home_render (only names the module actually defines)
    if let ast::Expr::FieldAccess { object, field } = expr {
        if let ast::Expr::Ident(obj) = object.as_ref() {
            if obj == module_name {
                if let Some(prefixed) = local_names.get(field) {
                    *expr = ast::Expr::Ident(prefixed.clone());
                    return;
                }
            }
        }
    }
    match expr {
        ast::Expr::BinOp { left, right, .. } => {
            rewrite_qualified_refs_expr(left, module_name, local_names);
            rewrite_qualified_refs_expr(right, module_name, local_names);
        }
        ast::Expr::UnaryOp { expr: e, .. } => rewrite_qualified_refs_expr(e, module_name, local_names),
        ast::Expr::Call { callee, args } => {
            rewrite_qualified_refs_expr(callee, module_name, local_names);
            for a in args {
                rewrite_qualified_refs_expr(a, module_name, local_names);
            }
        }
        ast::Expr::FieldAccess { object, .. } => {
            rewrite_qualified_refs_expr(object, module_name, local_names);
        }
        ast::Expr::Array(items) => {
            for i in items {
                rewrite_qualified_refs_expr(i, module_name, local_names);
            }
        }
        _ => {}
    }
}

fn rewrite_refs_expr(
    expr: &mut ast::Expr,
    map: &std::collections::BTreeMap<String, String>,
    shadowed: &std::collections::HashSet<String>,
) {
    match expr {
        ast::Expr::Ident(name) => {
            if !shadowed.contains(name) {
                if let Some(prefixed) = map.get(name) {
                    *name = prefixed.clone();
                }
            }
        }
        ast::Expr::BinOp { left, right, .. } => {
            rewrite_refs_expr(left, map, shadowed);
            rewrite_refs_expr(right, map, shadowed);
        }
        ast::Expr::UnaryOp { expr: e, .. } => rewrite_refs_expr(e, map, shadowed),
        ast::Expr::Call { callee, args } => {
            rewrite_refs_expr(callee, map, shadowed);
            for a in args {
                rewrite_refs_expr(a, map, shadowed);
            }
        }
        ast::Expr::FieldAccess { object, .. } => {
            // Only the object side — the field name is not a module-level ref
            rewrite_refs_expr(object, map, shadowed);
        }
        ast::Expr::Array(items) => {
            for i in items {
                rewrite_refs_expr(i, map, shadowed);
            }
        }
        ast::Expr::Map(pairs) => {
            for (_, v) in pairs {
                rewrite_refs_expr(v, map, shadowed);
            }
        }
        ast::Expr::Named { value, .. } => rewrite_refs_expr(value, map, shadowed),
        // `name` inside a module string interpolates the module's own variable:
        // rewrite the identifiers in each span (a field name after `.` and the
        // parameters that shadow are left alone)
        ast::Expr::Str(s) => {
            if s.contains('`') {
                let mut out = String::with_capacity(s.len());
                let mut in_span = false;
                let chars: Vec<char> = s.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '`' { in_span = !in_span; out.push(c); i += 1; continue; }
                    if in_span && (c.is_alphabetic() || c == '_') && (i == 0 || chars[i - 1] != '.') {
                        let start = i;
                        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
                        let word: String = chars[start..i].iter().collect();
                        match map.get(&word) {
                            Some(prefixed) if !shadowed.contains(&word) => out.push_str(prefixed),
                            _ => out.push_str(&word),
                        }
                        continue;
                    }
                    out.push(c);
                    i += 1;
                }
                *s = out;
            }
        }
        ast::Expr::ForExpr { var, iter, body } => {
            rewrite_refs_expr(iter, map, shadowed);
            let mut inner = shadowed.clone();
            inner.insert(var.clone());
            for st in body {
                rewrite_refs_stmt(st, map, &inner);
            }
        }
        _ => {}
    }
}
