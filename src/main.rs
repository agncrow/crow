// The interpreter is shared across a parallel loop's threads, so the
// compiler proves it `Sync` through every field, down to wgpu's device types
// behind the GPU kernels; on Linux (the Vulkan backend) that proof is deeper
// than the default limit of 128 (E0275 in CI, not on macOS).
#![recursion_limit = "256"]

mod ast;
mod appbook;
mod builtins;
mod ir;
mod harness;
#[cfg(test)]
mod interp_tests;
mod ir_interp;
mod lower_ir;
mod emit_rs;
mod ops;
mod baseline;
mod checker;
mod db_check;
mod derive;
mod lsp;
mod lexer;
mod lower;
mod parallel;
mod parser;
mod rust_names;
mod served;
mod compiled_harness;
mod served_parity;
mod sql_types;
mod token;
mod type_infer;
mod wireframe;
mod formatter;
mod modules;
mod value;

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
        "-v" | "--version" => {
            println!("(o.o) crow {}", env!("CARGO_PKG_VERSION"));
        }
        "help" | "--help" | "-h" => print_usage(),
        "new" => cmd_new(),
        "install" => cmd_install(),
        "fmt" => cmd_fmt(),
        "build" => cmd_build(),
        "launch" => cmd_launch(),
        "check" => {
            let mut verbose = false;
            let mut strict = false;
            let mut accept_all = false;
            let mut show: Option<Option<String>> = None;
            let mut db_url: Option<String> = None;
            let mut file: Option<String> = None;
            let mut rest = args.iter().skip(2);
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--verbose" => verbose = true,
                    "--strict" => strict = true,
                    "--accept" => accept_all = true,
                    "--show" => {
                        // --show [name]: the app's structure, or one declaration
                        let next = rest.clone().next().cloned();
                        show = Some(match next { Some(n) if !n.starts_with('-') && !is_source(&n) => { rest.next(); Some(n) } _ => None });
                    }
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
            if let Some(name) = show {
                cmd_show(&filename, name.as_deref());
                return;
            }
            cmd_check(&filename, verbose, db_url, strict, accept_all);
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
                eprintln!("usage: crow fix file.co --hazard N [--option M] [--db url]");
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
            } else if args.iter().any(|a| a == "--served") {
                process::exit(served_parity::run());
            } else if args.iter().any(|a| a == "--bench") {
                cmd_test_bench();
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
            match args.get(2).map(|s| s.as_str()) {
                Some("book") => cmd_stop_book(),
                _ => cmd_stop(),
            }
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
                if is_source(&target) || is_bytecode(&target) {
                    if target.ends_with(".crow") || target.ends_with(".crowc") { note_legacy_extension(&target); }
                    // crow run file.co arg1 arg2 ... -> transpile + compile + run
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
            // check if it's a .co or .coc file (shortcut)
            if is_source(other) || is_bytecode(other) {
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
    eprintln!("  crow build file.co   transpile a specific file");
    eprintln!("  crow run               transpile + compile + run main.co");
    eprintln!("  crow run file.co     transpile + compile + run a specific file");
    eprintln!("  crow run train         run a named job from crowfile");
    eprintln!("  crow run -i file.co  run with interpreter (slow, for debugging)");
    eprintln!("  crow stop              stop the running app (pidfile, then project binary)");
    eprintln!("  crow book [--port N]   serve the book with every code block runnable in place (default port 3008)");
    eprintln!("  crow check             auto-verify project (uses crowfile)");
    eprintln!("  crow check file.co   auto-verify a specific file");
    eprintln!("  crow check --db <url>  + schema-aware db checks (or db = '…' in crowfile)");
    eprintln!("  crow fix file.co --hazard N [--option M]   apply a fix from the check report");
    eprintln!("  crow lsp               language server over stdio (see docs/crow-lsp.md)");
    eprintln!("  crow fmt               format project (uses crowfile)");
    eprintln!("  crow fmt file.co     format a specific file");
    eprintln!("  crow fmt --check       check formatting without changing files");
    eprintln!("  crow fmt --fix-interpolation  migrate {{expr}} strings to `expr`");
    eprintln!("  crow fmt --fix-views      migrate ui-building fns to view declarations");
    eprintln!("  crow launch 4 file.co  build + run on 4 distributed processes");
    eprintln!("  crow test --parity     run tests/parity/ in both modes, diff output");
    eprintln!("  crow test --served     serve tests/served/ and the example apps in both modes, drive each page, compare");
    eprintln!("  crow test --compiled file.co  run the test blocks with the apps they open compiled");
    eprintln!("  crow test --book       execute every code block of the book in both modes");
    eprintln!("  crow test --docs       check the docs cover every compiler name (and nothing more)");
    eprintln!("  crow check --verbose   verbose output with purity and snapshot details");
    eprintln!("  crow -v                show version");
}

/// Run every tests/parity/*.co file through both execution modes and diff
/// their stdout. The compiled program and the interpreter must agree on
/// every program (design rule 11); this makes drift a test failure instead
/// of a discipline.

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

/// Write a book block to its worker's file, or say why it cannot be.
fn write_book_block(dir: &std::path::Path, code: &str) -> Result<std::path::PathBuf, String> {
    let path = dir.join("block.co");
    std::fs::write(&path, code).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    Ok(path)
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
    // block.co), and one directory per worker inside it: blocks are checked
    // in parallel — each compiled block is a cargo build, which is what makes
    // this harness slow — and reported in book order.
    // The root is persistent so each worker's cargo cache survives between
    // runs (a cold cache is a minute of dependency builds per worker); a lock
    // file refuses a second run while one is in progress. It lives under the
    // project's `target/`, not the system's temp directory: macOS deletes
    // temp files nobody has read for three days, which takes a build
    // script's output (libsqlite3-sys's `bindgen.rs`) from under a cache
    // cargo still believes is fresh, and every block that links it then
    // fails to build.
    let root = std::env::current_dir().unwrap_or_default().join("target").join("crow_book_test");
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
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(mode_args).arg(file).current_dir(dir).env("CROW_BUILD_PROFILE", "dev");
            let out = cmd.output();
            match out {
                Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()),
                Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
            }
        };
        let label = format!("{}:{}", b.file, b.line);
        // a block that cannot be written fails: running the last block's
        // file in its place ran a page that serves, and the suite hung
        let path = match write_book_block(dir, &b.code) {
            Ok(p) => p,
            Err(e) => return (false, format!("  FAIL  {} ({})\n", label, e)),
        };
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
///   4. every examples/*.co is listed in examples/README.md
fn cmd_test_docs() {
    use std::collections::BTreeSet;
    let spec = std::fs::read_to_string("docs/language-spec.md").unwrap_or_else(|_| {
        eprintln!("error: docs/language-spec.md not found (run from the project root)");
        process::exit(1);
    });
    let mut tables: Vec<(&str, Vec<&str>)> = vec![
        ("core", lower_ir::CORE_RESERVED.to_vec()),
        ("web", lower_ir::WEB_RESERVED.iter().chain(lower_ir::WEB_EXTRA_FUNCTIONS.iter()).copied().collect()),
        ("ui", lower_ir::UI_RESERVED.iter().copied().filter(|n| *n != "today").collect()),
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
    // names a program reads as values, not calls: used when read at all
    const READ_AS_VALUES: &[&str] = &["reader"];
    let read_as_value = |n: &str| {
        blocks.match_indices(n).any(|(i, _)| {
            let before = blocks[..i].chars().next_back().map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
            let after = blocks[i + n.len()..].chars().next().map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
            before && after
        })
    };
    for (table, names) in &tables {
        let uncalled: Vec<&str> = names.iter().copied()
            .filter(|n| !SPEC_ONLY.contains(n) && !blocks.contains(&format!("{}[", n)) && !(READ_AS_VALUES.contains(n) && read_as_value(n)))
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
        .filter(|t| t.ends_with("*.co"))
        .map(|t| t.trim_end_matches("*.co").to_string())
        .collect();
    let mut unlisted: Vec<String> = vec![];
    if let Ok(rd) = std::fs::read_dir("examples") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| x != "co" && x != "crow") { continue; }
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
                .filter(|p| p.extension().map_or(false, |x| x == "co" || x == "crow"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        eprintln!("error: tests/parity/ contains no .co files");
        process::exit(1);
    }

    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    let run_mode = |mode_args: &[&str], file: &std::path::Path| -> (bool, String, String) {
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(mode_args).arg(file).env("CROW_BUILD_PROFILE", "dev");
        let out = cmd.output();
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

/// `crow test --bench [--quick] [--write]`: every `tests/bench/*.co` runs
/// in both modes, the compiled one as the release build `crow build`
/// makes, and the seconds each program reports with `bench: <section>
/// <seconds>` lines are printed side by side; every other line the two
/// modes print must agree, as in the parity corpus. `--quick` shrinks the
/// sizes (the programs read BENCH_QUICK) and builds without optimisation:
/// the smoke run CI does so the programs cannot rot. `--write` records the
/// table in docs/benchmarks.md with the machine and the commit, keeping
/// the hand-written notes below the table.
fn cmd_test_bench() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let write = args.iter().any(|a| a == "--write");
    let dir = std::path::Path::new("tests/bench");
    if !dir.is_dir() {
        eprintln!("error: no tests/bench/ directory found (run from the project root)");
        process::exit(1);
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |x| x == "co"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        eprintln!("error: tests/bench/ contains no .co files");
        process::exit(1);
    }
    let data = dir.join("data");
    let _ = std::fs::create_dir_all(&data);
    let rows = if quick { 20_000 } else { 1_000_000 };
    write_bench_rows(&data.join("rows.csv"), rows);
    if quick {
        // Rust 2024: the environment is process-global, hence the block.
        unsafe { std::env::set_var("CROW_BUILD_PROFILE", "dev"); }
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    let timed = |mut cmd: std::process::Command| -> (bool, String, String, f64) {
        if quick {
            cmd.env("BENCH_QUICK", "1");
        }
        let start = std::time::Instant::now();
        match cmd.output() {
            Ok(o) => (
                o.status.success(),
                String::from_utf8_lossy(&o.stdout).to_string(),
                String::from_utf8_lossy(&o.stderr).to_string(),
                start.elapsed().as_secs_f64(),
            ),
            Err(e) => (false, String::new(), format!("failed to spawn: {}", e), 0.0),
        }
    };
    println!(
        "bench: {} programs, interpreter vs compiled{}\n",
        files.len(),
        if quick { " (quick sizes, unoptimised build)" } else { " (release build)" }
    );
    let mut table: Vec<(String, String, Option<f64>, Option<f64>)> = Vec::new();
    let mut failures = 0;
    for file in &files {
        let name = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let path = file.to_string_lossy().to_string();
        let mut icmd = std::process::Command::new(&exe);
        icmd.args(["run", "-i"]).arg(&path);
        let (i_ok, i_out, i_err, i_wall) = timed(icmd);
        // `crow build` in a subprocess, so a program that fails to compile
        // is one FAIL line and the run goes on; the binary it copies here
        // is removed once timed.
        let (c_ok, c_out, c_err, c_wall) = {
            let mut bcmd = std::process::Command::new(&exe);
            bcmd.arg("build").arg(&path);
            match bcmd.output() {
                Ok(o) if o.status.success() => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let built = stdout.lines().rev().find_map(|l| l.strip_prefix("build ok -> ./")).map(|s| s.trim().to_string());
                    match built {
                        Some(bin) => {
                            // a freshly built binary's first run pays the
                            // loader (a quarter second here); warm it once
                            // at the quick sizes so the timed run is the
                            // program, not the file system
                            let _ = std::process::Command::new(format!("./{}", bin)).env("BENCH_QUICK", "1").output();
                            let r = timed(std::process::Command::new(format!("./{}", bin)));
                            let _ = std::fs::remove_file(&bin);
                            r
                        }
                        None => (false, String::new(), "crow build printed no binary name".to_string(), 0.0),
                    }
                }
                Ok(o) => (false, String::new(), String::from_utf8_lossy(&o.stderr).to_string(), 0.0),
                Err(e) => (false, String::new(), format!("failed to spawn: {}", e), 0.0),
            }
        };
        let mut ok = true;
        if !i_ok {
            ok = false;
            println!("  FAIL  {}: interpreter exited with error: {}", name, i_err.lines().last().unwrap_or("(no stderr)"));
        }
        if !c_ok {
            ok = false;
            println!("  FAIL  {}: compiled exited with error: {}", name, c_err.lines().last().unwrap_or("(no stderr)"));
        }
        let checks = |s: &str| s.lines().filter(|l| !l.starts_with("bench: ")).map(|l| l.to_string()).collect::<Vec<_>>();
        if i_ok && c_ok && checks(&i_out) != checks(&c_out) {
            ok = false;
            let (a, b) = (checks(&i_out), checks(&c_out));
            let n = (0..a.len().max(b.len())).find(|&k| a.get(k) != b.get(k)).unwrap_or(0);
            println!("  FAIL  {}: the modes disagree on a check line", name);
            println!("        interpreter: {}", a.get(n).map(|s| s.as_str()).unwrap_or("<missing>"));
            println!("        compiled:    {}", b.get(n).map(|s| s.as_str()).unwrap_or("<missing>"));
        }
        if ok {
            println!("  ok    {}", name);
        } else {
            failures += 1;
        }
        let i_secs = bench_lines(&i_out);
        let c_secs = bench_lines(&c_out);
        let mut sections: Vec<String> = i_secs.iter().map(|(s, _)| s.clone()).collect();
        for (s, _) in &c_secs {
            if !sections.contains(s) {
                sections.push(s.clone());
            }
        }
        for s in sections {
            let iv = i_secs.iter().find(|(k, _)| k == &s).map(|(_, v)| *v);
            let cv = c_secs.iter().find(|(k, _)| k == &s).map(|(_, v)| *v);
            table.push((name.clone(), s, iv, cv));
        }
        table.push((
            name.clone(),
            "whole program".to_string(),
            if i_ok { Some(i_wall) } else { None },
            if c_ok { Some(c_wall) } else { None },
        ));
    }
    println!();
    println!("  {:<10} {:<18} {:>12} {:>12}   {}", "program", "section", "interpreter", "compiled", "compiled is");
    for (p, s, i, c) in &table {
        println!(
            "  {:<10} {:<18} {:>12} {:>12}   {}",
            p,
            s,
            i.map(fmt_secs).unwrap_or_else(|| "—".into()),
            c.map(fmt_secs).unwrap_or_else(|| "—".into()),
            speedup(*i, *c)
        );
    }
    if write {
        write_benchmarks_doc(&table, quick, rows);
        println!("\nrecorded in docs/benchmarks.md");
    }
    println!();
    if failures == 0 {
        println!("✓ {} programs ran in both modes", files.len());
    } else {
        println!("✗ {} of {} programs failed", failures, files.len());
        process::exit(1);
    }
}

fn bench_lines(out: &str) -> Vec<(String, f64)> {
    out.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("bench: ")?;
            let mut it = rest.split_whitespace();
            let name = it.next()?.to_string();
            let secs = it.next()?.parse::<f64>().ok()?;
            Some((name, secs))
        })
        .collect()
}

fn fmt_secs(s: f64) -> String {
    if s < 0.001 {
        format!("{:.0}µs", s * 1e6)
    } else if s < 1.0 {
        format!("{:.1}ms", s * 1e3)
    } else {
        format!("{:.2}s", s)
    }
}

fn speedup(i: Option<f64>, c: Option<f64>) -> String {
    match (i, c) {
        (Some(i), Some(c)) if c > 0.0 && i > 0.0 => {
            let r = i / c;
            if r >= 1.0 { format!("×{:.1} faster", r) } else { format!("×{:.1} slower", 1.0 / r) }
        }
        _ => String::new(),
    }
}

/// The frame fixture: `rows` rows of five text and five numeric columns,
/// deterministic, so a measurement is repeatable and the file need not be
/// tracked.
fn write_bench_rows(path: &std::path::Path, rows: usize) {
    use std::io::Write;
    let file = std::fs::File::create(path).unwrap_or_else(|e| {
        eprintln!("error writing {}: {}", path.display(), e);
        process::exit(1);
    });
    let mut w = std::io::BufWriter::new(file);
    let symbols = ["AAPL", "MSFT", "NVDA", "AMZN", "META", "GOOG", "TSLA", "NFLX"];
    let venues = ["NYSE", "NASDAQ", "ARCA", "BATS"];
    let _ = writeln!(w, "id,symbol,day,qty,price,side,venue,pnl,fee,note");
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 1..=rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let qty = (x % 500) as usize + 1;
        let price = 20.0 + ((x >> 9) % 20000) as f64 / 100.0;
        let pnl = (((x >> 21) % 40000) as f64 - 20000.0) / 100.0;
        let _ = writeln!(
            w,
            "{},{},2026-09-{:02},{},{:.2},{},{},{:.2},{:.2},note {}",
            i,
            symbols[i % 8],
            1 + i % 28,
            qty,
            price,
            if i % 2 == 1 { "buy" } else { "sell" },
            venues[i % 4],
            pnl,
            qty as f64 * 0.005,
            i
        );
    }
    let _ = w.flush();
}

fn write_benchmarks_doc(table: &[(String, String, Option<f64>, Option<f64>)], quick: bool, rows: usize) {
    let path = std::path::Path::new("docs/benchmarks.md");
    let notes = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.find("\n## Notes").map(|i| s[i..].to_string()))
        .unwrap_or_else(|| "\n## Notes\n\nHand-written findings go here; the table above is rewritten by `crow test --bench --write`.\n".to_string());
    let sh = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let cpu = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
            .unwrap_or_default()
    };
    let commit = sh("git", &["rev-parse", "--short", "HEAD"]);
    let date = sh("date", &["+%Y-%m-%d"]);
    let mut s = String::new();
    s.push_str("# Benchmarks\n\n");
    s.push_str(&format!(
        "*Measured by `crow test --bench --write` on {}, {} ({}), commit {}. The interpreter is `crow run -i`; compiled is the {} of the same program, timed from inside the program so a build is never counted. Sizes: {}. The programs are in `tests/bench/`; `crow test --bench --quick` runs them at small sizes in CI so they cannot rot.*\n\n",
        date,
        cpu,
        std::env::consts::ARCH,
        commit,
        if quick { "unoptimised build (`--quick`)" } else { "release build `crow build` makes" },
        if quick { format!("quick, {} csv rows", rows) } else { format!("full, {} csv rows", rows) }
    ));
    s.push_str("| Program | Section | Interpreter | Compiled | Compiled is |\n|---|---|---|---|---|\n");
    for (p, sec, i, c) in table {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            p,
            sec,
            i.map(fmt_secs).unwrap_or_else(|| "—".into()),
            c.map(fmt_secs).unwrap_or_else(|| "—".into()),
            speedup(*i, *c)
        ));
    }
    s.push_str(&notes);
    std::fs::write(path, s).unwrap_or_else(|e| {
        eprintln!("error writing docs/benchmarks.md: {}", e);
        process::exit(1);
    });
}


fn cmd_repl() {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    println!("(o.o) crow {} — type help for commands, ctrl-c to exit", env!("CARGO_PKG_VERSION"));
    let mut session = ir_interp::Session::new();
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
                            session = ir_interp::Session::new();
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
                        repl_eval(&mut session, &source, &mut accumulated_stmts);
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
                repl_eval(&mut session, trimmed, &mut accumulated_stmts);
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

fn repl_eval(session: &mut ir_interp::Session, source: &str, accumulated: &mut Vec<ast::Stmt>) {
    let source = source.trim();
    if source.is_empty() {
        return;
    }

    match build(source) {
        Ok(stmts) => {
            // Accumulate statements for :check
            accumulated.extend(stmts.clone());

            match session.feed(&stmts) {
                // an expression statement's value is printed, a done is not
                Ok(Some(val)) if !matches!(val.value, value::Value::Done) => println!("{}", val),
                Ok(_) => {}
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

    // main.co
    if has_web {
        write_file(&project_dir.join("main.co"), "use web, ui\nuse views.home\n\nserve[3000]\npage['/', home.render]\n");
    } else {
        write_file(&project_dir.join("main.co"), &format!("log['hello, {}']\n", short_name));
    }

    // --web: add views/home.co
    if has_web {
        write_file(&project_dir.join("views/home.co"),
            "--- home page ---\nuse ui\n\nmessage = 'welcome to crow'\n\nview render[]\n    return div[\n        h1['`message`']\n    ]\n");
    }

    // --ml: add jobs/train.co and lib/model.co
    if has_ml {
        write_file(&project_dir.join("jobs/train.co"),
            "use lib.model\n\nlog['training started']\ntrain[]\nlog['training complete']\n");
        write_file(&project_dir.join("lib/model.co"),
            "fn train[]\n  log['model training...']\n");
        jobs_section.push(("train".to_string(), "jobs/train.co".to_string()));
    }

    // --db: add db/schema.co and jobs/migrate.co
    if has_db {
        write_file(&project_dir.join("db/schema.co"),
            "use db\n\ntable users\n  name, str\n  email, str\n");
        write_file(&project_dir.join("jobs/migrate.co"),
            "use db\n\nlog['running migrations']\n");
        jobs_section.push(("migrate".to_string(), "jobs/migrate.co".to_string()));
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
    files.push("  main.co".to_string());
    if has_web { files.push("  views/home.co".to_string()); }
    if has_ml {
        files.push("  jobs/train.co".to_string());
        files.push("  lib/model.co".to_string());
    }
    if has_db {
        files.push("  db/schema.co".to_string());
        files.push("  jobs/migrate.co".to_string());
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

/// Transpile a .co file to Rust, compile it, and return the path to the binary.
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

    // Type inference and validation. The compiler may ask the program's
    // databases what each literal query returns, so frame cells are typed
    // natively.
    let ctx = infer_and_validate(&stmts, describing()).unwrap_or_else(|e| {
        eprintln!("{}", refine_error_lines(&source, &e));
        process::exit(1);
    });
    // A name Rust reserves (`move`, `impl`) is a Crow name like any other:
    // the compiled program renames each one it declares, after the gate has
    // spoken in the program's own names, and prints the program's name
    // wherever one reaches the reader
    let (stmts, ctx, shown) = match rust_names::rename_rust_keywords(&stmts) {
        None => (stmts, ctx, std::collections::BTreeMap::new()),
        Some((renamed, shown)) => {
            let ctx = infer_and_validate(&renamed, describing()).unwrap_or_else(|e| {
                eprintln!("{}", refine_error_lines(&source, &e));
                process::exit(1);
            });
            (renamed, ctx, shown)
        }
    };
    for note in &ctx.describe_notes {
        eprintln!("{}", note);
    }

    let entry_file = std::path::Path::new(entry).file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.to_string());
    // The Rust backend: the program lowered to the IR the interpreter runs,
    // then printed as Rust
    let mut ctx = ctx;
    let name = std::path::Path::new(&entry_file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| entry_file.clone());
    let program = lower_ir::lower(&stmts, &mut ctx, &name).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });
    let test_control = TEST_CONTROL.load(std::sync::atomic::Ordering::SeqCst);
    let emitted = emit_rs::emit_with(&program, ctx.registry.clone(), &entry_file, shown, cpu, test_control).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });
    let rust_source = emitted.source.clone();
    // `crow build --rs file.co`: the Rust the program compiles to, printed
    if PRINT_RS.load(std::sync::atomic::Ordering::SeqCst) {
        print!("{}", rust_source);
        process::exit(0);
    }

    // Create build directory
    let build_dir = std::path::PathBuf::from(".crow_build");
    let src_dir = build_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap_or_else(|e| {
        eprintln!("error creating build dir: {}", e);
        process::exit(1);
    });

    // Write Cargo.toml; a test build is a binary of its own beside the
    // program's, sharing the dependencies already built
    let project_name = if test_control { format!("{}_test", package_name(entry)) } else { package_name(entry) };
    let mut dep_list: Vec<String> = Vec::new();
    if emitted.needs_rayon {
        dep_list.push("rayon = \"1.11\"".to_string());
    }
    if emitted.needs_serde_json {
        dep_list.push("serde_json = \"1\"".to_string());
    }
    if emitted.needs_crow_runtime {
        let runtime_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_runtime");
        if emitted.web_features.contains("arrow_ipc") {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\", features = [\"arrow\"] }}", runtime_path));
        } else {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\" }}", runtime_path));
        }
    }
    if emitted.needs_regex {
        dep_list.push("regex = \"1\"".to_string());
    }
    if emitted.needs_chrono {
        dep_list.push("chrono = \"0.4\"".to_string());
    }
    if emitted.needs_crow_web {
        let web_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_web");
        let features: Vec<&str> = emitted.web_features.iter().map(|s| s.as_str()).collect();
        let features_str = features.join("\", \"");
        dep_list.push(format!(
            "crow_web = {{ path = \"{}\", features = [\"{}\"] }}",
            web_path, features_str
        ));
        // crow_web needs tokio for async runtime
        dep_list.push("tokio = { version = \"1\", features = [\"full\"] }".to_string());
        // msgpack[] needs rmp-serde in the generated project
        if emitted.web_features.contains("msgpack") {
            dep_list.push("rmp-serde = \"1\"".to_string());
        }
        // stream[] needs tokio-stream in the generated project
        if emitted.web_features.contains("stream") {
            dep_list.push("tokio-stream = \"0.1\"".to_string());
        }
    }
    if emitted.needs_crow_db {
        let db_path = concat!(env!("CARGO_MANIFEST_DIR"), "/crow_db");
        let features: Vec<&str> = emitted.db_features.iter().map(|s| s.as_str()).collect();
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
    let cargo_toml = generated_cargo_toml(&project_name, &deps);
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

    // Write the generated Rust source (always, so cargo can detect dep changes)
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
        let child = cmd.spawn().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        let handed = hand_signals_to(child.id());
        let output = child.wait_with_output().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        drop(handed);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let translated = translate_rust_compile_error(&stderr);
            eprintln!("{}", translated);
            process::exit(1);
        }
    } else {
        let mut child = cmd.spawn().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        let handed = hand_signals_to(child.id());
        let status = child.wait().unwrap_or_else(|e| {
            eprintln!("error running cargo: {}", e);
            process::exit(1);
        });
        drop(handed);
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
                // `&str` before `String`: the other order turns `&String`
                // into `stringing`
                s = s.replace("i64", "int").replace("f64", "float")
                     .replace("&str", "string").replace("String", "string")
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
            eprintln!("usage: crow fmt <file.co> or run from a project with crowfile");
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

/// `crow build --rs`: print the generated Rust instead of compiling it.
static PRINT_RS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `crow test --compiled`: the program is built to answer the test harness
/// (`crow_web::live::test_control`), under a name of its own.
static TEST_CONTROL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn cmd_build() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--rs") {
        PRINT_RS.store(true, std::sync::atomic::Ordering::SeqCst);
        let file = args.iter().skip(2).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
        transpile_and_compile(&file, true, None, false);
        return;
    }
    // `crow build --ir file.co`: the program's one typed form, printed
    // (docs/ir-design.md); nothing is compiled
    if args.iter().any(|a| a == "--ir") {
        let file = args.iter().skip(2).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
        match ir_of_file(&file, describing()) {
            Ok(text) => print!("{}", text),
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        return;
    }
    let (entry, cpu) = if args.len() > 2 {
        (args[2].clone(), None)
    } else if std::path::Path::new("crowfile").exists() {
        let config = parse_crowfile();
        (config.entry, config.cpu)
    } else {
        eprintln!("usage: crow build <file.co> or run from a project with crowfile");
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
    // crow launch <n> <file.co>
    // crow launch <n>  (uses crowfile)
    if args.len() < 3 {
        eprintln!("usage: crow launch <num_processes> [file.co]");
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
        eprintln!("usage: crow launch <num_processes> <file.co>");
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
    let handed = hand_signals_to(child.id());
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
    drop(handed);
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
/// the interpreter, the Rust backend, and the runner; this just puts a
/// button in front of them.
fn cmd_book() {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    let args: Vec<String> = std::env::args().collect();
    let chosen: Option<u16> = args.iter().position(|a| a == "--port")
        .and_then(|i| args.get(i + 1)).and_then(|p| p.parse().ok());
    // The site is Crow's own: docs/book/site.co renders every chapter
    // (use doc) with the runner built into each code block. Render it fresh
    // with the interpreter — well under a second — then serve the files.
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("crow"));
    let write_only = args.iter().any(|a| a == "--write");
    let site = if std::path::Path::new("docs/book/site.co").exists() { std::path::Path::new("docs/book/site.co") } else { std::path::Path::new("docs/book/site.co") };
    // The language's book lives on 3008; an app's book lives next to the
    // app, on its serve port plus one, so the two never collide and two
    // apps' books do not either
    let mut port: u16 = chosen.unwrap_or(3008);
    let mut what = "the Crow Book".to_string();
    let book_dir = if site.exists() {
        // the Crow Book itself, rendered by its own site program
        let rendered = std::process::Command::new(&exe).args(["run", "-i", &site.display().to_string()]).status().map(|s| s.success()).unwrap_or(false);
        let dir = std::path::PathBuf::from("docs/book/crow-site");
        if !rendered || !dir.join("index.html").exists() {
            eprintln!("error: rendering the book with docs/book/site.co failed");
            process::exit(1);
        }
        dir
    } else if std::path::Path::new("crowfile").exists() || std::path::Path::new("main.co").exists() || std::path::Path::new("main.co").exists() {
        // an app: its own book, derived from its source (app-baseline-design.md §8)
        let entry = if std::path::Path::new("crowfile").exists() { parse_crowfile().entry } else if std::path::Path::new("main.co").exists() { "main.co".to_string() } else { "main.co".to_string() };
        if chosen.is_none() { port = serve_port_of(&read_file(&entry)).map(|p| p.saturating_add(1)).unwrap_or(3009); }
        what = "the app's book".to_string();
        match build_app_book(&entry) {
            Ok(dir) => { println!("crow book: the app's book is written to {}", dir.display()); dir }
            Err(e) => { eprintln!("{}", e); process::exit(1); }
        }
    } else {
        eprintln!("error: run `crow book` inside an app (a crowfile or main.co) or the crow repository");
        process::exit(1);
    };
    if write_only { return; }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("error: cannot listen on port {}: {}", port, e);
        if e.kind() == std::io::ErrorKind::AddrInUse {
            eprintln!("  a book may already be serving there — `crow stop book`, or pick another: crow book --port {}", port + 1);
        }
        process::exit(1);
    });
    // the pid, so `crow stop book` can find this server from the project
    let _ = std::fs::create_dir_all(".crow");
    let _ = std::fs::write(".crow/book.pid", std::process::id().to_string());
    println!("crow book: http://localhost:{}/  — {} (Ctrl-C or `crow stop book` to stop)", port, what);
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
            // the runner executes code: only the book's own page reaches it
            if !book_request_allowed(&head, &method, port) {
                respond(&mut stream, "403 Forbidden", "text/plain; charset=utf-8", b"forbidden: the book answers its own page on this machine");
                return;
            }

            if method == "POST" && path == "/run" {
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                let id = req["id"].as_str().unwrap_or("anon").chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
                let code = req["code"].as_str().unwrap_or("").to_string();
                let mode = req["mode"].as_str().unwrap_or("interp").to_string();
                let dir = std::env::temp_dir().join("crow_book_play").join(&id);
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("block.co");
                let _ = std::fs::write(&file, &code);
                let (cmd_args, limit): (Vec<&str>, u64) = match mode.as_str() {
                    "compile" => (vec!["run", "block.co"], 600),
                    "test" => (vec!["test", "block.co"], 120),
                    _ => (vec!["run", "-i", "block.co"], 60),
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

/// A request the book server may act on. It must name this machine as its
/// host — a page on another site cannot, even through a DNS name it points
/// here (rebinding) — and a POST must come from the book's own page, as
/// JSON. A page elsewhere can send a simple POST to localhost without
/// asking the browser; a JSON body makes the browser ask first, and nothing
/// here says yes. The runner executes code, so a page you visit must never
/// reach it.
fn book_request_allowed(head: &str, method: &str, port: u16) -> bool {
    let header = |name: &str| {
        head.lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    };
    let this_machine = |authority: &str| {
        [format!("127.0.0.1:{}", port), format!("localhost:{}", port), format!("[::1]:{}", port)]
            .iter()
            .any(|a| a.eq_ignore_ascii_case(authority))
    };
    if !header("host").map(|h| this_machine(&h)).unwrap_or(false) {
        return false;
    }
    if method == "POST" {
        let own_page = header("origin")
            .and_then(|o| o.strip_prefix("http://").map(|a| this_machine(a)))
            .unwrap_or(false);
        let json = header("content-type")
            .map(|c| c.to_ascii_lowercase().starts_with("application/json"))
            .unwrap_or(false);
        return own_page && json;
    }
    true
}

/// The port an entry serves on: the first number in `serve[...]`.
fn serve_port_of(source: &str) -> Option<u16> {
    for line in source.lines() {
        let t = line.trim_start();
        if t.starts_with("---") { continue; }
        if let Some(rest) = t.find("serve[").map(|i| &t[i + 6..]) {
            let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = digits.parse::<u16>() { return Some(p); }
        }
    }
    None
}

/// `crow stop book`: the book this project is serving, by its pidfile.
fn cmd_stop_book() {
    if let Ok(pid_s) = std::fs::read_to_string(".crow/book.pid") {
        if let Ok(pid) = pid_s.trim().parse::<i32>() {
            let alive = unsafe { libc_kill(pid, 0) } == 0;
            if alive {
                unsafe { libc_kill(pid, 15) };
                let _ = std::fs::remove_file(".crow/book.pid");
                println!("stopped the book (pid {})", pid);
                return;
            }
        }
        let _ = std::fs::remove_file(".crow/book.pid");
    }
    println!("no book is serving from this project");
}

fn cmd_stop() {
    if let Ok(pid_s) = std::fs::read_to_string(".crow/app.pid") {
        if let Ok(pid) = pid_s.trim().parse::<i32>() {
            let alive = unsafe { libc_kill(pid, 0) } == 0;
            if alive {
                unsafe { libc_kill(pid, 15) };
                let _ = std::fs::remove_file(".crow/app.pid");
                println!("stopped (pid {})", pid);
                if let Ok(b) = std::fs::read_to_string(".crow/book.pid") {
                    if b.trim().parse::<i32>().map(|p| unsafe { libc_kill(p, 0) } == 0).unwrap_or(false) {
                        println!("the book is still serving — `crow stop book` stops it");
                    }
                }
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
    #[link_name = "signal"]
    fn libc_signal(sig: i32, handler: usize) -> usize;
}

/// The program `crow run` is running, which a signal to `crow run` is
/// handed on to.
static RUNNING_CHILD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

extern "C" fn hand_on_signal(sig: i32) {
    let pid = RUNNING_CHILD.load(std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        // kill() is safe inside a signal handler
        unsafe {
            libc_kill(pid, sig);
        }
    }
}

const HANDED_SIGNALS: [i32; 3] = [1, 2, 15]; // SIGHUP, SIGINT, SIGTERM

/// While a child crow started runs — the program `crow run` runs, the cargo
/// a build runs — a signal to crow is handed on to it, as a signal to the
/// program would stop it: `kill <pid of crow run>` used to end `crow run`
/// alone and leave its compiled program serving. crow then waits for the
/// child and exits as it did. When the guard goes, a signal is crow's own
/// again.
struct HandedSignals;

#[cfg(unix)]
fn hand_signals_to(child: u32) -> HandedSignals {
    RUNNING_CHILD.store(child as i32, std::sync::atomic::Ordering::SeqCst);
    for sig in HANDED_SIGNALS {
        unsafe {
            libc_signal(sig, hand_on_signal as extern "C" fn(i32) as usize);
        }
    }
    HandedSignals
}

#[cfg(not(unix))]
fn hand_signals_to(_child: u32) -> HandedSignals {
    HandedSignals
}

impl Drop for HandedSignals {
    fn drop(&mut self) {
        RUNNING_CHILD.store(0, std::sync::atomic::Ordering::SeqCst);
        #[cfg(unix)]
        for sig in HANDED_SIGNALS {
            // SIG_DFL
            unsafe {
                libc_signal(sig, 0);
            }
        }
    }
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

/// Tier-2 applier for stale-snapshot and remote-query-in-view: the cure the
/// language has is a derivation. A top-level `x = query[…]` becomes
/// `derive x` with the query as its body; a `rows = query[remote, …]` line
/// inside a view is hoisted above the view as `derive rows[fresh = 60]`,
/// and the view keeps reading `rows`. Only the exact single-line shapes are
/// rewritten; anything else gets the manual edit spelled out. The rewrite
/// must parse before it is written.
fn apply_derive_fix(filename: &str, issue: &checker::Issue) -> Result<String, String> {
    let source = read_file(filename);
    let (new_source, what) = match issue {
        checker::Issue::StaleSnapshot { var, source_line, .. } => derive_snapshot_rewrite(&source, var, *source_line)?,
        checker::Issue::RemoteQueryInView { view, conn, .. } => derive_hoist_rewrite(&source, view, conn)?,
        _ => return Err("not a finding a derivation cures".into()),
    };
    build(&new_source).map_err(|e| format!("rewrite would not parse: {}", e))?;
    std::fs::write(filename, &new_source).map_err(|e| format!("could not write {}: {}", filename, e))?;
    Ok(what)
}

/// The indentation a file uses: its first indented line's, else four spaces.
fn file_indent(lines: &[String]) -> String {
    lines.iter()
        .find(|l| l.starts_with(' ') && !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| *c == ' ').collect())
        .unwrap_or_else(|| "    ".to_string())
}

/// Brackets open on a line are closed on it.
fn balanced(line: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    for c in line.chars() {
        match c {
            '\'' => in_str = !in_str,
            '[' | '(' if !in_str => depth += 1,
            ']' | ')' if !in_str => depth -= 1,
            _ => {}
        }
    }
    depth == 0 && !in_str
}

/// `var = expr` at the top level, on one line, becomes `derive var` with
/// `return expr` as its body.
fn derive_snapshot_rewrite(source: &str, var: &str, line_1based: usize) -> Result<(String, String), String> {
    let mut lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();
    if line_1based == 0 || line_1based > lines.len() {
        return Err(format!("could not locate the assignment to '{}'", var));
    }
    let prefix = format!("{} = ", var);
    let line = lines[line_1based - 1].clone();
    if !line.starts_with(&prefix) || !balanced(&line) {
        return Err(format!(
            "'{}' is assigned across several lines — write `derive {}` by hand: the expression becomes its `return`",
            var, var
        ));
    }
    for (i, l) in lines.iter().enumerate() {
        if i + 1 == line_1based { continue; }
        let t = l.trim_start();
        if t.starts_with(&prefix) || (t.starts_with(var) && t[var.len()..].starts_with(", ")) {
            return Err(format!(
                "'{}' is also assigned at line {} — a derivation is never assigned; drop that assignment, or change the facts it reads instead",
                var, i + 1
            ));
        }
    }
    let indent = file_indent(&lines);
    let expr = line[prefix.len()..].to_string();
    lines.splice(line_1based - 1..line_1based, [format!("derive {}", var), format!("{}return {}", indent, expr)]);
    Ok((lines.join("\n") + "\n", format!("{} = … → derive {}", var, var)))
}

/// A single-line `name = query[conn, …]` inside `view` is hoisted above the
/// view as `derive name[fresh = 60]`; a `--- crow: remote-render` pragma
/// directly above the view goes with it, since the view no longer queries.
fn derive_hoist_rewrite(source: &str, view: &str, conn: &str) -> Result<(String, String), String> {
    let mut lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();
    let start = lines.iter().position(|l| l.starts_with(&format!("view {}[", view)) || l.starts_with(&format!("fn {}[", view)))
        .ok_or_else(|| format!("could not find view {}", view))?;
    // the body: indented lines until the next top-level line
    let mut end = start + 1;
    while end < lines.len() && (lines[end].starts_with(' ') || lines[end].trim().is_empty()) { end += 1; }
    let needle_a = format!("query[{},", conn);
    let needle_b = format!("query[{} ", conn);
    let mut found: Option<(usize, String, String)> = None;
    for i in start + 1..end {
        let l = &lines[i];
        let t = l.trim_start();
        if !(t.contains(&needle_a) || t.contains(&needle_b)) || !balanced(l) { continue; }
        if let Some(eq) = t.find(" = ") {
            let name = &t[..eq];
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                found = Some((i, name.to_string(), t[eq + 3..].to_string()));
                break;
            }
        }
    }
    let (at, name, rhs) = found.ok_or_else(|| format!(
        "the query in '{}' is not a single-line `name = query[{}, …]` — name it on one line, or write `derive name[fresh = 60]` above the view by hand",
        view, conn
    ))?;
    // what the expression names must live at the top level: the view's
    // params and locals are not there for a derivation to read
    let top_level: std::collections::BTreeSet<String> = lines.iter().filter_map(|l| {
        let l = l.trim_end();
        if l.starts_with(' ') || l.is_empty() { return None; }
        for kw in ["fn ", "view ", "derive "] {
            if let Some(rest) = l.strip_prefix(kw) {
                return Some(rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect());
            }
        }
        l.find(" = ").map(|eq| l[..eq].split(',').next().unwrap_or("").trim().to_string())
    }).collect();
    let tokens = lexer::Lexer::new(&rhs).tokenize().map_err(|e| format!("could not read the query line: {}", e))?;
    let mut prev_dot = false;
    for st in &tokens {
        if let token::Token::Ident(id) = &st.token {
            let known = prev_dot || id == "query" || id == conn || top_level.contains(id) || checker::BUILTINS.contains(&id.as_str());
            if !known {
                return Err(format!(
                    "the query reads '{}', which lives inside the view — make it a fact (a top-level variable a handler writes), then rewrite",
                    id
                ));
            }
        }
        prev_dot = matches!(st.token, token::Token::Dot);
    }
    let indent = file_indent(&lines);
    lines.remove(at);
    let mut insert_at = start;
    if start > 0 && lines[start - 1].trim() == "--- crow: remote-render" {
        lines.remove(start - 1);
        insert_at = start - 1;
    }
    lines.splice(insert_at..insert_at, [
        format!("derive {}[fresh = 60]", name),
        format!("{}return {}", indent, rhs),
        String::new(),
    ]);
    Ok((lines.join("\n") + "\n", format!(
        "{}: {} = query[…] → derive {}[fresh = 60] above the view — set fresh to how old the data may be",
        view, name, name
    )))
}

#[cfg(test)]
mod derive_fix_tests {
    use super::*;

    #[test]
    fn a_startup_snapshot_becomes_a_derivation() {
        let src = "use web, ui, db\n\nconn = db.connect['sqlite://x.db']\ntrades = query[conn, 'SELECT * FROM trades']\n\nview render[]\n    return div[table[trades]]\n\npage['/', render]\n";
        let (out, what) = derive_snapshot_rewrite(src, "trades", 4).unwrap();
        assert!(out.contains("derive trades\n    return query[conn, 'SELECT * FROM trades']\n"), "{}", out);
        assert!(what.contains("derive trades"));
        build(&out).unwrap();
    }

    #[test]
    fn a_snapshot_a_handler_also_assigns_is_refused() {
        let src = "use web, ui, db\n\nconn = db.connect['sqlite://x.db']\ntrades = query[conn, 'SELECT * FROM trades']\n\nfn clear[]\n    trades = []\n\nview render[]\n    return div[table[trades]]\n";
        let err = derive_snapshot_rewrite(src, "trades", 4).unwrap_err();
        assert!(err.contains("also assigned at line 7"), "{}", err);
    }

    #[test]
    fn a_remote_query_line_is_hoisted_above_the_view() {
        let src = "use web, ui, db\n\npg = db.connect['postgres://prod/trading']\nsym = 'IMRN'\n\n--- crow: remote-render\nview render[]\n    live = query[pg, 'SELECT * FROM trades WHERE symbol = ?', [sym]]\n    return div[table[live]]\n\npage['/', render]\n";
        let (out, what) = derive_hoist_rewrite(src, "render", "pg").unwrap();
        assert!(out.contains("derive live[fresh = 60]\n    return query[pg, 'SELECT * FROM trades WHERE symbol = ?', [sym]]\n\nview render[]\n    return div[table[live]]\n"), "{}", out);
        assert!(!out.contains("remote-render"), "the pragma goes with the query");
        assert!(what.contains("fresh = 60"));
        build(&out).unwrap();
    }

    #[test]
    fn a_query_over_a_view_local_is_refused() {
        let src = "use web, ui, db\n\npg = db.connect['postgres://prod/trading']\n\nview render[]\n    day = '2026-09-04'\n    live = query[pg, 'SELECT * FROM bars WHERE d = ?', [day]]\n    return div[table[live]]\n";
        let err = derive_hoist_rewrite(src, "render", "pg").unwrap_err();
        assert!(err.contains("'day'"), "{}", err);
    }
}

/// `crow test` — run only the authored test blocks in a file.
fn cmd_test_blocks(filename: &str) {
    let source = read_file(filename);
    let stmts = match build(&source) {
        Ok(s) => s,
        Err(e) => { eprintln!("{}", e); process::exit(1); }
    };
    // A part's tests run with the parts it imports inlined, exactly as the
    // run, build and check paths see the program
    let source_dir = std::path::Path::new(filename)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let stmts = match lower::lower_program(&stmts) {
        Ok(s) => s,
        Err(e) => { eprintln!("{}", e); process::exit(1); }
    };
    // The validator gates the harness as it gates run and build: a part
    // whose tests pass is a part the program accepts
    if let Err(e) = infer_and_validate(&stmts, describing()) {
        eprintln!("{}", refine_error_lines(&source, &e));
        process::exit(1);
    }
    // `crow test --compiled`: the apps the blocks open are the compiled
    // program (compiled_harness.rs); the blocks themselves run here
    let compiled = std::env::args().any(|a| a == "--compiled");
    if compiled && compiled_harness::opens_an_app(&stmts) {
        TEST_CONTROL.store(true, std::sync::atomic::Ordering::SeqCst);
        let binary = transpile_and_compile(filename, true, None, false);
        compiled_harness::use_binary(std::fs::canonicalize(&binary).unwrap_or(binary));
    }
    let (issues, counts) = checker::run_test_blocks(&stmts);
    if counts.blocks == 0 {
        println!("  no test blocks in {} — add `test` after a fn, or `test name[]` anywhere", filename);
        return;
    }
    if issues.is_empty() {
        // the blocks ran here; what they opened with `app[]` ran compiled
        let against = match (compiled, compiled_harness::started()) {
            (false, _) => String::new(),
            (true, 0) => ", none opening an app to compile".to_string(),
            (true, n) => format!(", the apps they opened compiled ({} run{})", n, if n == 1 { "" } else { "s" }),
        };
        println!("\n  \x1b[32m✓ {} block{} pass\x1b[0m{} — {} truth{}, {} law{}, {} input{} run",
            counts.blocks, if counts.blocks == 1 { "" } else { "s" }, against,
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

/// Everything the app's book needs, derived from the entry: the artifacts
/// and rules, the diff against the accepted baseline, the lowered program
/// (for the tests' steps) and the parts' files.
fn derive_app(filename: &str) -> Result<(baseline::Artifacts, Vec<baseline::Rule>, Vec<baseline::Change>, Option<baseline::Summary>, Vec<ast::Stmt>, std::collections::BTreeMap<String, String>), String> {
    let source = read_file(filename);
    let stmts = build(&source)?;
    let source_dir = std::path::Path::new(filename).parent().unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let lowered = lower::lower_program(&stmts).map_err(|e| format!("error: {}", e))?;
    let ctx = infer_and_validate(&lowered, describing()).map_err(|e| refine_error_lines(&source, &e))?;
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let (artifacts, rules) = baseline::derive_artifacts(&lowered, &ctx, &parts, filename);
    let (changes, summary) = match baseline::load(filename) {
        None => (Vec::new(), None),
        Some(old) => { let (c, mut s) = baseline::compare(&old, &artifacts); s.dir = baseline::store_dir(filename).display().to_string(); (c, Some(s)) }
    };
    Ok((artifacts, rules, changes, summary, lowered, parts))
}

/// The comment a file opens with, as its description.
fn leading_comment(path: &str) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("---") { out.push(rest.trim().to_string()); } else if t.is_empty() && out.is_empty() { continue; } else { break; }
    }
    out.join(" ")
}

/// `crow book` inside an app: build the app's book under .crow/book and
/// return the site directory to serve.
fn build_app_book(entry: &str) -> Result<std::path::PathBuf, String> {
    let (artifacts, rules, changes, summary, lowered, parts) = derive_app(entry)?;
    let crowfile = std::fs::read_to_string("crowfile").unwrap_or_default();
    let name = crowfile.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("name").map(|r| r.trim().trim_start_matches('=').trim().trim_matches(|c| c == '\'' || c == '"').to_string())
    }).filter(|n| !n.is_empty()).unwrap_or_else(|| {
        std::env::current_dir().ok().and_then(|d| d.file_name().map(|f| f.to_string_lossy().to_string())).unwrap_or_else(|| "the app".into())
    });
    let mut descriptions: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    descriptions.insert(entry.to_string(), leading_comment(entry));
    let root = project_root(std::path::Path::new(entry).parent().unwrap_or(std::path::Path::new(".")));
    for file in parts.values() {
        let p = root.join(file);
        descriptions.insert(file.clone(), leading_comment(&p.display().to_string()));
    }
    // a test belongs to the part its first view or handler belongs to
    let mut tests: Vec<(String, String, Vec<String>)> = Vec::new();
    for st in &lowered {
        if let ast::Stmt::TestBlock { name: Some(name), body, .. } = st {
            let steps = appbook::test_steps(body);
            let mut file = entry.to_string();
            for (prefix, f) in &parts {
                if steps.iter().any(|s| s.contains(prefix.as_str())) { file = f.clone(); break; }
            }
            tests.push((file, name.clone(), steps));
        }
    }
    let wide = read_file(entry).contains("wide = true");
    let input = appbook::Input { entry, name, artifacts: &artifacts, rules: &rules, changes: &changes, baseline: summary.as_ref(), descriptions, tests, wide };
    appbook::build(&input)
}

/// Source files end in `.co`. A `.co` file is still read this release,
/// with a note the first time one is met, so a tree renames at leisure.
fn is_source(path: &str) -> bool { path.ends_with(".co") || path.ends_with(".crow") }
fn is_bytecode(path: &str) -> bool { path.ends_with(".coc") || path.ends_with(".crowc") }
fn strip_source_ext(path: &str) -> &str {
    path.strip_suffix(".co").or_else(|| path.strip_suffix(".crow")).unwrap_or(path)
}
fn bytecode_path(entry: &str) -> String { format!("{}.coc", strip_source_ext(entry)) }
/// The source a path names: a bytecode path's `.co` (or legacy `.crow`)
/// beside it; any other path is the source already. (After the `.co`
/// rename it appended `.co` to a source too, and every interpreter trace
/// named `main.co` where the compiled program names `main`.)
fn source_of_bytecode(path: &str) -> String {
    if !is_bytecode(path) {
        return path.to_string();
    }
    let base = path.strip_suffix(".coc").or_else(|| path.strip_suffix(".crowc")).unwrap_or(path);
    let co = format!("{}.co", base);
    if std::path::Path::new(&co).exists() || !std::path::Path::new(&format!("{}.crow", base)).exists() { co } else { format!("{}.crow", base) }
}
static LEGACY_NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn note_legacy_extension(path: &str) {
    if !LEGACY_NOTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        eprintln!("note: {} — source files are `.co` now; `.crow` is still read this release (git mv to rename)", path);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;

    // The rename to `.co` keeps `.crow` readable this release; a regex slip
    // once turned every legacy branch into the new extension, so both are
    // pinned here.
    #[test]
    fn new_and_legacy_extensions_are_both_recognised() {
        assert!(is_source("a.co"));
        assert!(is_source("a.crow"));
        assert!(!is_source("a.coc"));
        assert!(is_bytecode("a.coc"));
        assert!(is_bytecode("a.crowc"));
        assert!(!is_bytecode("a.co"));
        assert_eq!(strip_source_ext("dir/a.co"), "dir/a");
        assert_eq!(strip_source_ext("dir/a.crow"), "dir/a");
        assert_eq!(strip_source_ext("dir/a"), "dir/a");
        assert_eq!(bytecode_path("dir/a.crow"), "dir/a.coc");
        assert_eq!(bytecode_path("dir/a.co"), "dir/a.coc");
        assert_eq!(source_of_bytecode("zz_no_such_file.coc"), "zz_no_such_file.co");
        assert_eq!(source_of_bytecode("zz_no_such_file.crowc"), "zz_no_such_file.co");
        assert_eq!(source_of_bytecode("main.co"), "main.co");
        assert_eq!(source_of_bytecode("dir/a.crow"), "dir/a.crow");
    }
}

/// A program's IR, printed: the same front end as run, test and check,
/// then the lowering of docs/ir-design.md §4.
fn ir_of_file(entry: &str, describe: bool) -> Result<String, String> {
    let source = read_file(entry);
    let stmts = build(&source).map_err(|e| e.to_string())?;
    let source_dir = std::path::Path::new(entry).parent().unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let stmts = lower::lower_program(&stmts)?;
    let mut ctx = infer_and_validate(&stmts, describe)?;
    for note in &ctx.describe_notes {
        eprintln!("{}", note);
    }
    let name = std::path::Path::new(entry)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.to_string());
    let program = lower_ir::lower(&stmts, &mut ctx, &name)?;
    Ok(ir::print(&program))
}

#[cfg(test)]
mod ir_tests {
    /// The IR must exist for every program the modes are held to agree on.
    #[test]
    fn every_parity_program_lowers_to_the_ir() {
        let mut failures = Vec::new();
        let mut count = 0;
        for entry in std::fs::read_dir("tests/parity").unwrap() {
            let p = entry.unwrap().path();
            if p.extension().map_or(false, |x| x == "co") {
                count += 1;
                if let Err(e) = super::ir_of_file(&p.to_string_lossy(), false) {
                    failures.push(format!("{}: {}", p.display(), e));
                }
            }
        }
        assert!(count > 40, "the corpus was not found");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

/// Type inference and validation of an expanded, lowered program: the gate
/// every pipeline passes — run, run -i, test and check — so the two modes
/// and the tools refuse the same programs with the same message.
/// The manifest of a program's build crate. A dev build (`crow run`, the
/// runners) optimizes the runtime crates and libraries a little and leaves
/// the program itself unoptimized, so a rebuild stays fast while a page's
/// render machinery runs near release speed; a release build (`crow
/// build`) optimizes everything.
/// The generated crate's name for a program's path: cargo takes letters,
/// digits, `_` and `-`, and not a leading digit — `01_counter.co` builds as
/// `app_01_counter`.
fn package_name(entry: &str) -> String {
    let flat: String = strip_source_ext(entry).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if flat.chars().next().map_or(true, |c| c.is_ascii_digit() || c == '-') {
        format!("app_{}", flat)
    } else {
        flat
    }
}

fn generated_cargo_toml(project_name: &str, deps: &str) -> String {
    format!(
        "[workspace]\n\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{}\n[profile.dev.package.\"*\"]\nopt-level = 1\n\n[profile.release]\nopt-level = 3\nlto = true\n",
        project_name, deps
    )
}

/// Whether the gate asks the program's databases what its query literals
/// return. Every pipeline — run, run -i, test, check, the book, each part
/// alone — passes the same answer, so a program refused for its columns'
/// types is refused by all of them; `CROW_NO_DESCRIBE` turns it off.
fn describing() -> bool {
    std::env::var("CROW_NO_DESCRIBE").is_err()
}

fn infer_and_validate(lowered: &[ast::Stmt], describe: bool) -> Result<type_infer::TypeContext, String> {
    let mut ctx = type_infer::TypeContext::new();
    ctx.describe_queries = describe;
    ctx.infer_program(lowered);
    // the lowering refuses what is not a program (lower_ir/checks.rs); it
    // runs here on a copy of the context, for the callers that only ask
    match lower_ir::check(lowered, &ctx) {
        Ok(()) => Ok(ctx),
        // a message names a variable as the program wrote it
        Err(lower_ir::Refusal::Invalid(errors)) => Err(lower::shown(&attribute_to_parts(&errors))),
        Err(refusal) => Err(refusal.message()),
    }
}

/// An error found in an imported part belongs to that part: it is named by
/// the part's file and its line refined against the part's own text. The
/// entry's errors are left as they are, for the caller to refine against
/// the entry.
fn attribute_to_parts(errors: &[(String, Option<String>)]) -> String {
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let files = EXPANSION_FILES.with(|f| f.borrow().clone());
    errors
        .iter()
        .map(|(msg, owner)| {
            let prefix = owner.as_ref().and_then(|o| {
                parts.keys().filter(|prefix| o.starts_with(prefix.as_str())).max_by_key(|prefix| prefix.len())
            });
            match prefix {
                Some(prefix) => {
                    let refined = files
                        .get(prefix)
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .map(|source| refine_error_lines(&source, msg))
                        .unwrap_or_else(|| msg.clone());
                    format!("{}: {}", parts[prefix], refined)
                }
                None => msg.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    // User modules are inlined exactly as the run and build paths do, so a
    // multi-file app is checked whole and a module prefix is a known name
    let source_dir = std::path::Path::new(filename)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    // Validated before it is checked: `crow check` refuses what `crow run`
    // refuses, with the same message
    let (lowered, lctx) = match lower::lower_program(&stmts) {
        Ok(lowered) => match infer_and_validate(&lowered, describing()) {
            Ok(lctx) => (lowered, lctx),
            Err(e) => {
                eprintln!("{}", refine_error_lines(&source, &e));
                process::exit(1);
            }
        },
        Err(e) => { eprintln!("error: {}", e); process::exit(1); }
    };
    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(&stmts);
    let mut result = checker::run_check(&stmts, &ctx, false);
    result.issues = checker::suppress_acknowledged(std::mem::take(&mut result.issues), &source);

    // The app baseline: what the program does, derived from the lowered
    // program and diffed against what was accepted (app-baseline-design.md)
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let (artifacts, rules) = baseline::derive_artifacts(&lowered, &lctx, &parts, filename);
    let changed = baseline::git_changed_files(source_dir);
    let dir = baseline::store_dir(filename).display().to_string();
    match baseline::load(filename) {
        None => {
            baseline::save_all(filename, &artifacts);
            result.baseline = Some(baseline::Summary { first: true, recorded: artifacts.entries.len(), dir, ..Default::default() });
        }
        Some(old) => {
            let (changes, mut summary) = baseline::compare(&old, &artifacts);
            summary.dir = dir;
            for c in changes {
                let expected = baseline::is_expected(&c.file, &changed);
                result.issues.push(checker::Issue::BaselineChange { key: c.key, file: c.file, was: c.was, now: c.now, reach: c.reach, expected });
            }
            // a new declaration is recorded as it is, like a new snapshot
            for (k, t) in &artifacts.entries {
                if !old.contains_key(k) { baseline::accept(filename, k, t); }
            }
            result.baseline = Some(summary);
        }
    }
    for r in rules {
        result.issues.push(checker::Issue::UiRule { subject: r.subject, file: r.file, detail: r.detail, fix: r.fix });
    }
    result.artifacts = Some(artifacts);
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
    let errors = result.issues.iter().filter(|i| i.is_error()).count();
    if result.issues.is_empty() {
        println!("\n  \x1b[32m✓ all good\x1b[0m — {} functions checked, {} edge cases generated",
            result.total_functions, result.edge_cases_generated);
    } else if errors > 0 {
        let rest = result.issues.len() - errors;
        println!("\n  \x1b[31m✗ {} error{}\x1b[0m{} — {} functions checked, {} edge cases generated",
            errors,
            if errors == 1 { "" } else { "s" },
            if rest > 0 { format!(", \x1b[33m{} issue{}\x1b[0m", rest, if rest == 1 { "" } else { "s" }) } else { String::new() },
            result.total_functions,
            result.edge_cases_generated);
    } else {
        println!("\n  \x1b[33m⚠ {} issue{}\x1b[0m — {} functions checked, {} edge cases generated",
            result.issues.len(),
            if result.issues.len() == 1 { "" } else { "s" },
            result.total_functions,
            result.edge_cases_generated);
    }
    if !result.derivations.is_empty() {
        println!("  derivations — what each reads, and who holds it:");
        for d in &result.derivations {
            println!("    {}", d);
        }
    }
    if let Some(b) = &result.baseline {
        if b.first {
            println!("  baseline — {} declarations recorded under {} (first check: what the app does, accepted as it is)", b.recorded, b.dir);
        } else {
            let mut parts = vec![format!("{} unchanged", b.unchanged)];
            if b.changed > 0 { parts.push(format!("\x1b[33m{} changed\x1b[0m", b.changed)); }
            if b.added > 0 { parts.push(format!("{} new", b.added)); }
            if b.removed > 0 { parts.push(format!("\x1b[33m{} removed\x1b[0m", b.removed)); }
            println!("  baseline — {} ({})", parts.join(", "), b.dir);
        }
    }
    if verbose {
        println!("  pure: {:?}", result.pure_fns);
        println!("  impure: {:?}", result.impure_fns);
        println!("  snapshots: {} saved, {} unchanged",
            result.snapshots_saved, result.snapshots_unchanged);
    }
}

/// Format a clickable location string like "file.co:12 in add"
fn format_location(filename: &str, issue: &checker::Issue, fn_lines: &std::collections::BTreeMap<String, usize>) -> String {
    let func = issue.func();
    let line = issue.line();
    // a part's declaration is on a line of the part's own file
    let part = file_of_declaration(func);
    let filename = part.as_deref().unwrap_or(filename);

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

/// The part file a declaration came from, by its module prefix (the
/// longest that fits); none for the entry file's own.
fn file_of_declaration(name: &str) -> Option<String> {
    EXPANSION_PARTS.with(|p| {
        p.borrow()
            .iter()
            .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, file)| file.clone())
    })
}

/// `crow check --show`: the app as a tree; `--show name`: one declaration's
/// artifact, matched by the end of its name (`buy` finds `sim_trading_buy`).
fn cmd_show(filename: &str, name: Option<&str>) {
    let result = run_check_pipeline(filename);
    let Some(art) = &result.artifacts else { println!("  nothing to show"); return; };
    match name {
        None => print!("{}", baseline::render_structure(art, filename)),
        Some(n) => {
            let n = n.trim();
            let mut hits: Vec<(&String, &String)> = art.entries.iter().filter(|(k, _)| {
                let decl = k.split(' ').nth(1).unwrap_or("");
                decl == n || decl.ends_with(&format!("_{}", n)) || k.as_str() == n
            }).collect();
            if hits.is_empty() {
                hits = art.entries.iter().filter(|(k, _)| k.contains(n)).collect();
            }
            if hits.is_empty() {
                println!("  nothing named '{}' — try `crow check --show` for the whole app", n);
                process::exit(1);
            }
            for (_, text) in hits { print!("{}\n", text); }
        }
    }
}

/// What the whole program says about each of its files, for the database
/// checks that read a file alone: the connections it reaches as it spells
/// them (its own, and a part's through the part's name), how many databases
/// the program talks to, and its handlers. `files` are the program's files
/// by name, the entry's first; each part is named as `EXPANSION_PARTS`
/// names it.
fn db_programs(filename: &str, files: &[(String, String)]) -> std::collections::BTreeMap<String, db_check::Program> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(stmts) = build(&read_file(filename)) else { return out };
    let source_dir = std::path::Path::new(filename).parent().unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let expanded = expand_module_imports(&stmts, source_dir, &mut visited);
    let Ok(lowered) = lower::lower_program(&expanded) else { return out };
    let mut ctx = type_infer::TypeContext::new();
    ctx.describe_queries = false;
    ctx.infer_program(&lowered);
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let databases = ctx.conn_urls.iter().map(|(_, u)| u.clone()).collect::<std::collections::BTreeSet<_>>().len();
    // a name a part declares carries the part's prefix; the entry's carry none
    let prefix_of = |name: &str| -> String { parts.keys().filter(|p| name.starts_with(p.as_str())).max_by_key(|p| p.len()).cloned().unwrap_or_default() };
    let by_file: std::collections::BTreeMap<&str, &str> = parts.iter().map(|(p, rel)| (rel.as_str(), p.as_str())).collect();
    for (name, source) in files {
        let own = if name == filename { "" } else { by_file.get(name.as_str()).copied().unwrap_or("") };
        let mut program = db_check::Program { databases, ..Default::default() };
        for (conn, url) in &ctx.conn_urls {
            if prefix_of(conn) == own {
                program.conns.push((conn[own.len()..].to_string(), url.clone()));
            }
        }
        // a part imported by name reaches its connections through that name
        if let Ok(stmts) = build(source) {
            for st in &stmts {
                if let ast::Stmt::Use { path, alias } = st {
                    let prefix = format!("{}_", path.replace(['.', '/'], "_"));
                    let spelled = alias.clone().unwrap_or_else(|| path.rsplit(['.', '/']).next().unwrap_or(path).to_string());
                    for (conn, url) in &ctx.conn_urls {
                        if prefix_of(conn) == prefix {
                            program.conns.push((format!("{}.{}", spelled, &conn[prefix.len()..]), url.clone()));
                        }
                    }
                }
            }
        }
        for h in &ctx.view_handlers {
            if prefix_of(h) == own {
                program.handlers.insert(h[own.len()..].to_string());
            }
        }
        out.insert(name.clone(), program);
    }
    out
}

fn cmd_check(filename: &str, verbose: bool, db_url: Option<String>, strict: bool, accept_all: bool) {
    use std::io::{self, Write, BufRead};

    let mut result = run_check_pipeline(filename);
    // --accept: every baseline change is intended — the baseline moves
    // without the loop, for a script or a deliberate sweep
    if accept_all {
        let mut n = 0;
        for issue in &result.issues {
            if let checker::Issue::BaselineChange { key, now, .. } = issue {
                baseline::accept(filename, key, now);
                n += 1;
            }
        }
        if n > 0 {
            println!("  accepted {} baseline change{}", n, if n == 1 { "" } else { "s" });
            result = run_check_pipeline(filename);
        }
    }
    print_check_summary(&result, verbose);

    // Schema-aware database checks (Level 1): the entry and every part it
    // imports, each against its own lines — an app keeps its queries in its
    // parts. Advisory unless --strict.
    let resolved_db = db_url.or_else(crowfile_db_url);
    let mut files: Vec<(String, String)> = vec![(filename.to_string(), read_file(filename))];
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let paths = EXPANSION_FILES.with(|f| f.borrow().clone());
    for (prefix, rel) in &parts {
        if let Some(source) = paths.get(prefix).and_then(|p| std::fs::read_to_string(p).ok()) {
            files.push((rel.clone(), source));
        }
    }
    let programs = db_programs(filename, &files);
    let (mut calls, mut checked, mut findings, mut other_db) = (0usize, false, 0usize, 0usize);
    let mut reports: Vec<(String, String)> = Vec::new();
    let mut skipped: Option<String> = None;
    for (name, source) in &files {
        let program = programs.get(name).cloned().unwrap_or_default();
        match db_check::run_in(name, source, resolved_db.as_deref(), &program) {
            Ok(db_result) => {
                calls += db_result.sql_calls;
                checked |= db_result.schema_checked;
                other_db += db_result.other_db;
                if let Some(report) = db_check::report::render_all(&db_result.hazards, source) {
                    findings += db_result.hazards.len();
                    reports.push((name.clone(), report));
                }
            }
            Err(e) => {
                skipped.get_or_insert(e);
            }
        }
    }
    if other_db > 0 {
        eprintln!("{}", db_check::other_db_note(other_db, resolved_db.as_deref().unwrap_or_default()));
    }
    if findings > 0 {
        println!("\n  database checks: \x1b[33m{} finding{}\x1b[0m\n", findings, if findings == 1 { "" } else { "s" });
        for (name, report) in &reports {
            // a part's findings are named by its file
            if name != filename {
                println!("  \x1b[90m{}\x1b[0m", name);
            }
            println!("{}", report);
        }
        if strict {
            process::exit(1);
        }
    } else if let Some(e) = skipped.filter(|_| calls == 0) {
        println!("  \x1b[90mdatabase checks skipped: {}\x1b[0m", e);
    } else if checked {
        println!("  \x1b[32m✓ database checks clean\x1b[0m — {} SQL call{} verified against schema", calls, if calls == 1 { "" } else { "s" });
    } else if calls > 0 {
        println!(
            "  \x1b[90m{} SQL call{} found — pass --db <url> (or set db = '…' in crowfile) for schema-aware checks\x1b[0m",
            calls,
            if calls == 1 { "" } else { "s" }
        );
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
        let derive_fixable = matches!(issue, checker::Issue::StaleSnapshot { .. } | checker::Issue::RemoteQueryInView { .. });
        let mut prompt_parts = vec!["enter = recheck", "s = skip"];
        if has_try { prompt_parts.push("t = try input"); }
        let is_baseline = matches!(issue, checker::Issue::BaselineChange { .. });
        if is_regression { prompt_parts.push("a = accept new"); }
        if is_baseline { prompt_parts.push("a = accept (the baseline moves)"); }
        if remote_acceptable { prompt_parts.push("a = accept (deliberate)"); }
        if derive_fixable { prompt_parts.push("d = derive (rewrite)"); }
        if later_fixable { prompt_parts.push("f = fix (later[])"); }
        prompt_parts.push("q = quit");
        // No terminal (a script, CI, an editor): report every finding, then stop
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            for (i, rest) in result.issues.iter().enumerate().skip(idx + 1) {
                println!();
                println!("  \x1b[1m[{}/{}]\x1b[0m \x1b[90m{}\x1b[0m", i + 1, total, format_location(filename, rest, &result.fn_lines));
                println!("        {}", rest.summary());
                println!("        \x1b[36mfix: {}\x1b[0m", rest.suggest());
            }
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
            "a" if is_baseline => {
                if let checker::Issue::BaselineChange { key, now, .. } = issue {
                    baseline::accept(filename, key, now);
                    println!("\n  accepted {} — rechecking...", key);
                    result = run_check_pipeline(filename);
                    print_check_summary(&result, false);
                    if result.issues.is_empty() { break; }
                    idx = 0;
                }
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
            "d" if derive_fixable => {
                match apply_derive_fix(filename, issue) {
                    Ok(what) => {
                        println!("\n  rewrote {}: {} — rechecking...", filename, what);
                        result = run_check_pipeline(filename);
                        print_check_summary(&result, false);
                        if result.issues.is_empty() { break; }
                        idx = 0;
                    }
                    Err(e) => println!("\n  could not apply: {}", e),
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
                        // the file with the call appended, run as one program
                        let call_src = format!("log[{}]", try_cmd);
                        match build(&call_src) {
                            Ok(call_stmts) => {
                                let mut all = stmts.clone();
                                all.extend(call_stmts);
                                match ir_interp::run_script(&all, filename) {
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
    if !config.deps.is_empty() {
        let modules_dir = std::path::PathBuf::from("crow_modules");
        if !modules_dir.exists() {
            eprintln!("warning: deps defined but 'crow_modules/' directory not found");
        }
        for (name, version) in &config.deps {
            let dep_file = std::path::PathBuf::from("crow_modules").join(format!("{}.co", name));
            let dep_dir = std::path::PathBuf::from("crow_modules").join(name).join("main.co");
            if !dep_file.exists() && !dep_dir.exists() {
                eprintln!("warning: dependency '{}' v{} not found in crow_modules/", name, version);
            }
        }
    }
    // the crowfile's cpu count bounds the parallel loops, as it does the
    // compiled program's; its gpu setting turns the device on
    if let Some(n) = config.cpu {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
    }
    if config.gpu.is_some() {
        crow_runtime::gpu::set_gpu_enabled(true);
    }
    // the bytecode when `crow compile` left one, else the entry's source;
    // user modules and crow_modules/ deps are expanded on the way
    let compiled_path = bytecode_path(&config.entry);
    if std::path::Path::new(&compiled_path).exists() {
        cmd_run_file_interpreted(&compiled_path);
    } else {
        cmd_run_file_interpreted(&config.entry);
    }
}

fn cmd_run_file_interpreted(filename: &str) {
    let (stmts, source) = if is_bytecode(filename) {
        let bytes = std::fs::read(filename).unwrap_or_else(|e| {
            eprintln!("error reading '{}': {}", filename, e);
            process::exit(1);
        });
        let stmts: Vec<ast::Stmt> = bincode::deserialize(&bytes).unwrap_or_else(|e| {
            eprintln!("error loading bytecode '{}': {}", filename, e);
            process::exit(1);
        });
        (stmts, String::new())
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
        (expand_module_imports(&stmts, source_dir, &mut visited), source)
    };
    // Lower + validate exactly as the compiled path does — the two modes
    // must reject the same programs (reservation, scope, ui rules)
    let lowered = match lower::lower_program(&stmts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };
    let mut ctx = match infer_and_validate(&lowered, describing()) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("{}", refine_error_lines(&source, &e));
            process::exit(1);
        }
    };
    let serves = serve_line(&lowered).is_some();
    // then the one typed form (ir-design.md), walked by the interpreter
    let base = source_of_bytecode(filename);
    let name = std::path::Path::new(&base)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| base.clone());
    let program = lower_ir::lower(&lowered, &mut ctx, &name).unwrap_or_else(|e| {
        eprintln!("{}", e);
        process::exit(1);
    });
    if serves {
        // a program that serves pages: its main runs, then the interpreter
        // renders and handles for the live runtime's sessions
        let template = match ir_interp::run_to_serve(&program, &ctx.registry, &base) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("runtime error: {}", e);
                process::exit(1);
            }
        };
        if let Err(e) = served::serve(template) {
            eprintln!("error: {}", e);
            process::exit(1);
        }
        return;
    }
    if let Err(e) = ir_interp::run(&program, &ctx.registry, &base) {
        eprintln!("runtime error: {}", e);
        process::exit(1);
    }
}

/// The line of a program's top-level `serve[…]`, when it serves pages.
fn serve_line(stmts: &[ast::Stmt]) -> Option<usize> {
    let mut line = 0;
    for s in stmts {
        match s {
            ast::Stmt::Line(n) => line = *n,
            ast::Stmt::ExprStmt(ast::Expr::Call { callee, .. }) => match callee.as_ref() {
                ast::Expr::Ident(n) if n == "serve" => return Some(line),
                ast::Expr::FieldAccess { field, .. } if field == "serve" => return Some(line),
                _ => {}
            },
            _ => {}
        }
    }
    None
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
    let it = ir_interp::run_script(&stmts, "crowfile").ok()?;
    match it.var("db") {
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
            let it = match ir_interp::run_script(&stmts, "crowfile") {
                Ok(it) => it,
                Err(e) => {
                    eprintln!("error in crowfile: {}", e);
                    process::exit(1);
                }
            };

            let entry = match it.var("entry") {
                Some(val) => match &val.value {
                    value::Value::Str(s) => s.clone(),
                    _ => {
                        eprintln!("crowfile: 'entry' must be a string");
                        process::exit(1);
                    }
                },
                None => {
                    if !std::path::Path::new("main.co").exists() && std::path::Path::new("main.co").exists() {
                        note_legacy_extension("main.co");
                        "main.co".to_string()
                    } else { "main.co".to_string() }
                }
            };

            let cpu = it.var("cpu").and_then(|val| {
                if let value::Value::Int(n) = &val.value {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            let gpu = it.var("gpu").and_then(|val| {
                if let value::Value::Int(n) = &val.value {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            // parse deps: [math = '1.0', http = '2.3']
            let deps = it.var("deps").map(|val| {
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

            // parse [jobs] section: jobs = [train = 'jobs/train.co', ...]
            let jobs = it.var("jobs").map(|val| {
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
    // a `.crow` entry still runs this release, with a note the first time
    if filename.ends_with(".crow") { note_legacy_extension(filename); }
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

/// Expand `use` imports: every module file is parsed once, its
/// declarations prefixed with its path (`sim/clock.co` → `sim_clock_`) and
/// hoisted ahead of the program, and every importer's qualified references
/// (`clock.head`, or the alias's) are rewritten to the prefixed names. A
/// module reached through two importers — the tape and the trading parts
/// both reading the clock — is one copy. This runs *before* type inference
/// so the main program's call sites can inform module function param types.
fn expand_module_imports(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    visited: &mut std::collections::HashSet<String>,
) -> Vec<ast::Stmt> {
    let mut ex = ModuleExpansion::default();
    let body = expand_into(stmts, source_dir, &mut ex, visited);
    let mut out = ex.hoisted;
    out.extend(body);
    // every part says `use db` for itself; the program imports it once
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    out.retain(|st| match st {
        ast::Stmt::Use { path, alias } => seen.insert(format!("{}|{}", path, alias.clone().unwrap_or_default())),
        _ => true,
    });
    // Each part is validated alone, as `crow test part.co` sees it — its
    // own names unprefixed, its imports inlined — so a name a part reserves
    // by what it imports, or a handler nothing in the part wires, is
    // refused whichever file is the entry
    for path in ex.done.keys() {
        if let Err(e) = validate_part_alone(path) {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
    out
}

thread_local! {
    static VALIDATED_PARTS: std::cell::RefCell<std::collections::HashSet<std::path::PathBuf>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// module prefix (`sim_session_`) → the part's project-relative file
    static EXPANSION_PARTS: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());
    /// module prefix → the part's file on disk, read back to refine an
    /// error's line against the part's own text
    static EXPANSION_FILES: std::cell::RefCell<std::collections::BTreeMap<String, std::path::PathBuf>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());
}

/// Validate one part as the entry of its own program: parsed, its imports
/// inlined, lowered and passed through the same gate as the whole app.
/// Each part is validated once per process, and reports under its
/// project-relative path.
fn validate_part_alone(path: &std::path::Path) -> Result<(), String> {
    let first = VALIDATED_PARTS.with(|v| v.borrow_mut().insert(path.to_path_buf()));
    if !first {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let root = project_root(&dir);
    let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.to_path_buf());
    let rel = path
        .strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", rel, e))?;
    let stmts = build(&source).map_err(|e| format!("{}: {}", rel, e))?;
    let mut visited = std::collections::HashSet::new();
    let expanded = expand_module_imports(&stmts, &dir, &mut visited);
    let lowered = lower::lower_program(&expanded).map_err(|e| format!("{}: error: {}", rel, e))?;
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    infer_and_validate(&lowered, describing()).map(|_| ()).map_err(|e| {
        refine_error_lines(&source, &e)
            .lines()
            .map(|l| {
                // the gate names an imported part's errors by its file; the
                // rest are this part's own
                if parts.values().any(|file| l.starts_with(&format!("{}: ", file))) {
                    l.to_string()
                } else {
                    format!("{}: {}", rel, l)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[derive(Default)]
struct ModuleExpansion {
    /// modules already expanded, by file: the local name → prefixed name map
    done: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeMap<String, String>>,
    /// every module's prefixed declarations, in dependency order
    hoisted: Vec<ast::Stmt>,
}

fn expand_into(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    ex: &mut ModuleExpansion,
    in_progress: &mut std::collections::HashSet<String>,
) -> Vec<ast::Stmt> {
    let mut expanded = Vec::new();
    // how this file spells each module it imports (the last segment or the
    // alias), with the module's names
    let mut imported: Vec<(String, std::collections::BTreeMap<String, String>)> = Vec::new();
    for stmt in stmts {
        let ast::Stmt::Use { path, alias } = stmt else {
            expanded.push(stmt.clone());
            continue;
        };
        // Built-in modules: skip file lookup
        if path == "math" || path.starts_with("math.") {
            expanded.push(stmt.clone());
            continue;
        }
        // Convert dot-separated path to file path (views.home → views/home)
        let file_rel = path.replace('.', "/");
        let spelling = alias.clone().unwrap_or_else(|| {
            path.rsplit(['.', '/']).next()
                .and_then(|s| if s.is_empty() { None } else { Some(s) })
                .unwrap_or(path)
                .to_string()
        });
        // The path is rooted at the project (the directory holding the
        // crowfile), so a part under views/ reaches a part under sim/
        // whichever file is the entry; without a crowfile the entry file's
        // directory is the root
        let root = project_root(source_dir);
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        for ext in ["co", "crow"] {
            candidates.push(root.join(format!("{}.{}", file_rel, ext)));
            candidates.push(root.join(format!("crow_modules/{}.{}", file_rel, ext)));
            candidates.push(root.join(format!("crow_modules/{}/main.{}", file_rel, ext)));
            if root != source_dir {
                candidates.push(source_dir.join(format!("{}.{}", file_rel, ext)));
            }
        }
        let Some(file_path) = candidates.iter().find(|p| p.exists()).cloned() else {
            expanded.push(stmt.clone());
            continue;
        };
        if file_path.extension().map_or(false, |e| e == "crow") { note_legacy_extension(&file_path.display().to_string()); }
        let canon = std::fs::canonicalize(&file_path).unwrap_or(file_path.clone());
        // Expanded already, through another importer: one copy, this
        // file's spelling of it
        if let Some(names) = ex.done.get(&canon) {
            imported.push((spelling, names.clone()));
            expanded.push(stmt.clone());
            continue;
        }
        // Circular import: the earlier expansion covers it
        let key = canon.to_string_lossy().to_string();
        if in_progress.contains(&key) {
            expanded.push(stmt.clone());
            continue;
        }
        in_progress.insert(key.clone());
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
        // The module's own imports are hoisted first and its references to
        // them rewritten
        let mod_dir = file_path.parent().unwrap_or(source_dir);
        let mod_stmts = expand_into(&mod_stmts, mod_dir, ex, in_progress);
        in_progress.remove(&key);

        // The prefix is the module's path: unique per file, the same from
        // every importer
        let prefix = file_rel.replace('/', "_");
        // the baseline attributes each declaration to its part's file
        EXPANSION_PARTS.with(|p| {
            let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            let rel = canon.strip_prefix(&root).map(|p| p.display().to_string()).unwrap_or_else(|_| file_path.display().to_string());
            p.borrow_mut().insert(format!("{}_", prefix), rel);
        });
        EXPANSION_FILES.with(|f| {
            f.borrow_mut().insert(format!("{}_", prefix), canon.clone());
        });
        // Collect the module's own top-level names, then rewrite both the
        // declarations AND every reference inside module bodies — a module
        // fn must still reach its own structs/fns/vars after prefixing.
        let mut local_names: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for mod_stmt in &mod_stmts {
            match mod_stmt {
                ast::Stmt::FnDecl { name, .. }
                | ast::Stmt::StructDecl { name, .. }
                | ast::Stmt::Assign { name, .. }
                | ast::Stmt::Derive { name, .. } => {
                    local_names.insert(name.clone(), format!("{}_{}", prefix, name));
                }
                ast::Stmt::MultiAssign { names, .. } => {
                    for name in names {
                        local_names.insert(name.clone(), format!("{}_{}", prefix, name));
                    }
                }
                _ => {}
            }
        }
        let none = std::collections::HashSet::new();
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
                    ex.hoisted.push(ast::Stmt::FnDecl {
                        name: format!("{}_{}", prefix, name),
                        params: params.clone(),
                        defaults: defaults.clone(),
                        body,
                        is_view: *is_view,
                    });
                }
                ast::Stmt::StructDecl { name, fields } => {
                    ex.hoisted.push(ast::Stmt::StructDecl {
                        name: format!("{}_{}", prefix, name),
                        fields: fields.clone(),
                    });
                }
                // Module-level variables become prefixed assigns
                ast::Stmt::Assign { name, value } => {
                    let mut value = value.clone();
                    rewrite_refs_expr(&mut value, &local_names, &none);
                    ex.hoisted.push(ast::Stmt::Assign {
                        name: format!("{}_{}", prefix, name),
                        value,
                    });
                }
                ast::Stmt::MultiAssign { names, values } => {
                    let mut values = values.clone();
                    for v in &mut values {
                        rewrite_refs_expr(v, &local_names, &none);
                    }
                    ex.hoisted.push(ast::Stmt::MultiAssign {
                        names: names.iter().map(|n| format!("{}_{}", prefix, n)).collect(),
                        values,
                    });
                }
                // A derivation is a part's belief: prefixed like its facts,
                // its body reading the module's own names
                ast::Stmt::Derive { name, fresh, body } => {
                    let mut body = body.clone();
                    for st in &mut body {
                        rewrite_refs_stmt(st, &local_names, &none);
                    }
                    ex.hoisted.push(ast::Stmt::Derive {
                        name: format!("{}_{}", prefix, name),
                        fresh: *fresh,
                        body,
                    });
                }
                // Pass through nested Use statements so the compiler
                // registers those module names too
                ast::Stmt::Use { .. } => {
                    ex.hoisted.push(mod_stmt.clone());
                }
                _ => {}
            }
        }
        ex.done.insert(canon, local_names.clone());
        imported.push((spelling, local_names));
        // Keep the original statement (including Use, so the compiler knows about it)
        expanded.push(stmt.clone());
    }
    // Rewrite qualified refs in the importing file: home.render → home_render
    for (spelling, local_names) in &imported {
        for st in &mut expanded {
            rewrite_qualified_refs_stmt(st, spelling, local_names);
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
        // `session.day = x` in another part's handler: an assignment to the
        // module's fact, which the validator then judges like any fact write
        ast::Stmt::FieldAssign { object, field, value } => {
            map(value);
            let target = if let ast::Expr::Ident(obj) = &*object {
                if obj == module_name { local_names.get(field).cloned() } else { None }
            } else { None };
            if let Some(prefixed) = target {
                *stmt = ast::Stmt::Assign { name: prefixed, value: value.clone() };
                return;
            }
            map(object);
        }
        ast::Stmt::MultiAssign { values, .. } => {
            for v in values {
                map(v);
            }
        }
        ast::Stmt::Derive { body, .. } | ast::Stmt::TestBlock { body, .. } => {
            for st in body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
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
        ast::Expr::Map(pairs) => {
            for (_, v) in pairs {
                rewrite_qualified_refs_expr(v, module_name, local_names);
            }
        }
        ast::Expr::Named { value, .. } => rewrite_qualified_refs_expr(value, module_name, local_names),
        ast::Expr::ForExpr { iter, body, .. } => {
            rewrite_qualified_refs_expr(iter, module_name, local_names);
            for st in body {
                rewrite_qualified_refs_stmt(st, module_name, local_names);
            }
        }
        // `clock.head` inside an interpolation span reads the module's name
        // like any other expression: rewritten to the prefixed form
        ast::Expr::Str(s) => {
            if s.contains('`') {
                let needle = format!("{}.", module_name);
                let chars: Vec<char> = s.chars().collect();
                let mut out = String::with_capacity(s.len());
                let mut in_span = false;
                let mut i = 0;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '`' { in_span = !in_span; out.push(c); i += 1; continue; }
                    if in_span && (c.is_alphabetic() || c == '_') && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '.')) {
                        let rest: String = chars[i..].iter().collect();
                        if rest.starts_with(&needle) {
                            let after = &rest[needle.len()..];
                            let field: String = after.chars().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect();
                            if let Some(prefixed) = local_names.get(&field) {
                                out.push_str(prefixed);
                                i += needle.chars().count() + field.chars().count();
                                continue;
                            }
                        }
                    }
                    out.push(c);
                    i += 1;
                }
                *s = out;
            }
        }
        _ => {}
    }
}

/// The directory a project's module paths are rooted at: the nearest
/// ancestor holding a crowfile, else the entry file's own directory.
fn project_root(source_dir: &std::path::Path) -> std::path::PathBuf {
    let start = if source_dir.as_os_str().is_empty() { std::path::Path::new(".") } else { source_dir };
    let mut dir = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    loop {
        if dir.join("crowfile").exists() {
            return dir;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod book_port_tests {
    use super::*;

    /// An app's book lives next to the app: its serve port plus one.
    /// The book's runner executes code, and a page on any site could reach
    /// it: a simple POST from `https://evil.example` ran a program while
    /// `crow book` served. Only the book's own page, on this machine, as
    /// JSON, is answered; a host name other than this machine is refused
    /// for every request (DNS rebinding).
    #[test]
    fn only_the_books_own_page_reaches_the_runner() {
        let req = |method: &str, headers: &[(&str, &str)]| {
            let mut head = format!("{} /run HTTP/1.1\r\n", method);
            for (k, v) in headers {
                head.push_str(&format!("{}: {}\r\n", k, v));
            }
            head + "\r\n"
        };
        let own = [("Host", "localhost:3008"), ("Origin", "http://localhost:3008"), ("Content-Type", "application/json")];
        assert!(book_request_allowed(&req("POST", &own), "POST", 3008));
        assert!(book_request_allowed(&req("POST", &[("Host", "127.0.0.1:3008"), ("Origin", "http://127.0.0.1:3008"), ("Content-Type", "application/json")]), "POST", 3008));
        assert!(!book_request_allowed(&req("POST", &[("Host", "localhost:3008"), ("Origin", "https://evil.example"), ("Content-Type", "text/plain")]), "POST", 3008), "a page on another site");
        assert!(!book_request_allowed(&req("POST", &[("Host", "localhost:3008"), ("Origin", "http://localhost:3008"), ("Content-Type", "text/plain")]), "POST", 3008), "a simple POST, which a browser sends from anywhere without asking");
        assert!(!book_request_allowed(&req("POST", &[("Host", "localhost:3008"), ("Content-Type", "application/json")]), "POST", 3008), "no origin");
        assert!(book_request_allowed(&req("GET", &[("Host", "localhost:3008")]), "GET", 3008));
        assert!(!book_request_allowed(&req("GET", &[("Host", "rebound.example:3008")]), "GET", 3008), "a name that points here from elsewhere");
        assert!(!book_request_allowed(&req("POST", &[("Host", "localhost:3009"), ("Origin", "http://localhost:3009"), ("Content-Type", "application/json")]), "POST", 3008), "another port's page");
    }

    #[test]
    fn the_books_port_is_next_to_the_apps() {
        assert_eq!(serve_port_of("use web, ui\nserve[3100, refresh = 1, wide = true]\npage['/', v]\n"), Some(3100));
        assert_eq!(serve_port_of("--- serve[9999] in a comment does not count\nserve[ 3000 ]\n"), Some(3000));
        assert_eq!(serve_port_of("use web\npage['/', v]\n"), None);
    }
}

#[cfg(test)]
mod build_crate_tests {
    use super::*;

    /// A dev build (`crow run`) optimizes the runtime crates and libraries,
    /// not the program: a page's render machinery ran unoptimized under
    /// `crow run`, while a rebuild compiles only the program
    #[test]
    fn a_dev_build_optimizes_the_runtime_crates() {
        assert_eq!(package_name("01_counter.co"), "app_01_counter", "a leading digit is not a crate name");
        assert_eq!(package_name("tests/parity/50_ok.co"), "tests_parity_50_ok");
        assert_eq!(package_name("my app.co"), "my_app");
        let toml = generated_cargo_toml("main", "[dependencies]\n");
        assert!(toml.contains("[profile.dev.package.\"*\"]\nopt-level = 1"), "{}", toml);
        assert!(toml.contains("[profile.release]\nopt-level = 3\nlto = true"), "{}", toml);
        assert!(toml.starts_with("[workspace]\n\n[package]\nname = \"main\""), "{}", toml);
    }
}

#[cfg(test)]
mod validation_gate_tests {
    use super::*;

    /// The gate every pipeline passes: a function reading a derivation is
    /// refused here, so `crow test` and `crow check` refuse it exactly as
    /// `crow run` does — and the derivation it should have been is accepted.
    #[test]
    fn a_function_reading_a_derivation_is_refused_by_the_gate() {
        let src = "use web, ui\nn = 1\nderive cfg\n    return n + 1\nfn band[]\n    c = cfg\n    return c * 2\nview render[]\n    return p['`band[]`']\npage['/', render]\n";
        let lowered = lower::lower_program(&build(src).expect("parses")).expect("lowers");
        let err = infer_and_validate(&lowered, false).unwrap_err();
        assert!(err.contains("derivations are not visible inside functions"), "{}", err);
        let fine = "use web, ui\nn = 1\nderive cfg\n    return n + 1\nderive band\n    c = cfg\n    return c * 2\nview render[]\n    return p['`band`']\npage['/', render]\n";
        let lowered = lower::lower_program(&build(fine).unwrap()).unwrap();
        assert!(infer_and_validate(&lowered, false).is_ok());
    }

    /// `crow run -i` serves a program that calls `serve`: its main runs and
    /// records the page and the serve it asks for, which the live runtime
    /// then serves with the interpreter rendering. A script has no `serve`.
    /// A book block the runner cannot write is a failure with the reason,
    /// never a run of whatever the worker's file held before
    #[test]
    fn a_book_block_that_cannot_be_written_fails() {
        let dir = std::env::temp_dir().join(format!("crow_book_block_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = write_book_block(&dir, "log['x']\n").unwrap_err();
        assert!(e.contains("cannot write"), "{}", e);
        std::fs::create_dir_all(&dir).unwrap();
        let p = write_book_block(&dir, "log['x']\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "log['x']\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_interpreter_serves_what_the_program_asks_for() {
        let app = "use web, ui\nn = 1\nview render[]\n    return p['`n`']\n\nserve[3100, refresh = 1, keep = true]\npage['/', render, url = ['n']]\n";
        let lowered = lower::lower_program(&build(app).unwrap()).unwrap();
        assert_eq!(serve_line(&lowered), Some(6));
        let script = lower::lower_program(&build("x = 1\nlog[x]\n").unwrap()).unwrap();
        assert_eq!(serve_line(&script), None);
        // main runs and records the page and the serve it asks for
        let (program, ctx) = ir_interp::prepare(&build(app).unwrap(), "app").unwrap();
        let it = ir_interp::run_to_serve(&program, &ctx.registry, "app.co").unwrap();
        let (path, view, url) = it.page_config.clone().expect("page recorded");
        assert_eq!((path.as_str(), view.as_str(), url), ("/", "render", vec!["n".to_string()]));
        let serve = it.serve_config.clone().expect("serve recorded");
        assert_eq!(serve.len(), 9, "the lowered serve: port, refresh, budget, theme, density, accent, wide, keep, uploads");
    }
}

#[cfg(test)]
mod module_parts_tests {
    use super::*;

    /// A part's names are reserved as its file is: a derivation named for
    /// a ui element is refused when the part is validated alone, and the
    /// message carries the part's path.
    #[test]
    fn a_part_is_validated_alone_under_its_own_reservations() {
        let dir = std::env::temp_dir().join(format!("crow_parts_alone_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sim")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("sim/clock.co"), "use web, ui\n\nbase = 0.0\n\nderive progress\n    return base * 2.0\n\nview transport[]\n    return p['`progress`']\n").unwrap();
        let path = std::fs::canonicalize(dir.join("sim/clock.co")).unwrap();
        let err = validate_part_alone(&path).unwrap_err();
        assert!(err.contains("sim/clock.co: line 5: 'progress' is a ui builtin"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A part that never imports web may name a parameter `close`: the
    /// whole program carries the entry's imports, and a part is held to its
    /// own (finch's pattern parts were refused through a ui app)
    #[test]
    fn a_part_is_not_held_to_the_entrys_reservations() {
        let dir = std::env::temp_dir().join(format!("crow_parts_resv_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/calc.co"), "fn total[close]\n    return close * 2.0\n").unwrap();
        let stmts = build("use web, ui\nuse lib.calc\nn = 0\nview v[]\n    return p['`calc.total[2.0]` `n`']\npage['/', v]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The gate names an error found in an imported part by the part's
    /// file, its line refined against the part's own text; the entry's own
    /// errors are left for the caller to refine against the entry. (A
    /// thirteen-file app used to be told only "line 60".)
    #[test]
    fn an_error_in_a_part_names_the_parts_file() {
        let dir = std::env::temp_dir().join(format!("crow_parts_attr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sim")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("sim/part.co"), "use web, ui\n\nn = 3\n\nderive d\n    return n * 2\n\nview panel[]\n    return p['`d`']\n").unwrap();
        let stmts = build("use web, ui\nuse sim.part\n\nview render[]\n    return part.panel[]\n\npage['/', render]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let _ = expand_module_imports(&stmts, &dir, &mut visited);
        let text = attribute_to_parts(&[
            ("line 6: boom".to_string(), Some("sim_part_d".to_string())),
            ("line 4: bang".to_string(), Some("render".to_string())),
        ]);
        assert_eq!(text, "sim/part.co: line 6: boom\nline 4: bang");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every gate types a query's columns as the build does: a part whose
    /// derivation gives a local an integer cell and then text is refused
    /// when validated alone, as `crow test` and `crow check` see it — they
    /// used to validate without asking the database, and passed what `crow
    /// build` refused (finch's session part). Asking never creates a SQLite
    /// file that is not there: only the program makes its database.
    #[test]
    fn every_gate_types_query_columns_as_the_build_does() {
        let dir = std::env::temp_dir().join(format!("crow_parts_describe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sim")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        let url = format!("sqlite://{}", dir.join("book.db").display());
        crow_db::connect(&url, Some(1)).unwrap().exec("CREATE TABLE nums (n INTEGER)", &[]).unwrap();
        std::fs::write(dir.join("sim/rows.co"), format!("use web, ui, db\n\nbook = db.connect['{}']\n\nderive first\n    r = query[book, 'SELECT n FROM nums']\n    x = r.n[0]\n    x = 'a'\n    return x\n\nview panel[]\n    return p['`first`']\n", url)).unwrap();
        let path = std::fs::canonicalize(dir.join("sim/rows.co")).unwrap();
        let err = validate_part_alone(&path).unwrap_err();
        assert!(err.contains("sim/rows.co: line 8: cannot reassign 'x' from int to str"), "{}", err);

        let missing = dir.join("missing.db");
        std::fs::write(dir.join("sim/fresh.co"), format!("use web, ui, db\n\nbook = db.connect['sqlite://{}']\n\nderive first\n    return query[book, 'SELECT n FROM nums']\n\nview panel[]\n    return table[first]\n", missing.display())).unwrap();
        let path = std::fs::canonicalize(dir.join("sim/fresh.co")).unwrap();
        assert!(validate_part_alone(&path).is_ok());
        assert!(!missing.exists(), "describing created the database file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A part's derivation, facts and tests survive expansion with the
    /// module prefix, and the importing file's references to them — in
    /// code and inside interpolation spans — are rewritten to match.
    #[test]
    fn a_parts_derivation_is_expanded_once_under_its_path() {
        let dir = std::env::temp_dir().join(format!("crow_parts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sim")).unwrap();
        std::fs::create_dir_all(dir.join("views")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("sim/clock.co"), "use web, ui\n\nbase = 0.0\nspeed = 1.0\nplaying = false\n\nderive head\n    return base * speed\n\nfn faster[]\n    base, speed = head, 10.0\n\nview transport[]\n    return button['10x', click = faster]\n").unwrap();
        // the tape reads the clock too: a diamond through views/sim.co
        std::fs::write(dir.join("sim/tape.co"), "use web, ui\nuse sim.clock\n\nderive marker\n    return clock.head + 1.0\n").unwrap();
        std::fs::write(dir.join("views/sim.co"), "use web, ui\nuse sim.clock\nuse sim.tape\n\nderive label\n    return 'at `clock.head` x`clock.speed` marker `tape.marker`'\n\nfn slow[]\n    clock.speed = 1.0\n\nview render[]\n    return stack[p[label], clock.transport[], button['1x', click = slow]]\n\ntest it[]\n    a = app[render]\n    click[a, clock.faster]\n").unwrap();
        let src = "use web, ui\nuse views.sim\nuse sim.clock\n\nlog[clock.speed]\npage['/', sim.render]\n";
        let stmts = build(src).unwrap();
        let mut visited = std::collections::HashSet::new();
        let expanded = expand_module_imports(&stmts, &dir, &mut visited);
        let text = format!("{:?}", expanded);
        assert_eq!(text.matches("Derive { name: \"sim_clock_head\"").count(), 1, "one copy of the clock, whichever path reached it: {}", text);
        assert!(text.contains("Derive { name: \"sim_tape_marker\""), "the tape's derivation reads the clock's: {}", text);
        assert!(text.contains("BinOp { left: Ident(\"sim_clock_head\")"), "the tape's qualified read is rewritten");
        assert!(text.contains("Derive { name: \"views_sim_label\""), "the page's derivation is prefixed by its path");
        assert!(text.contains("at `sim_clock_head` x`sim_clock_speed` marker `sim_tape_marker`"), "qualified reads inside interpolation spans are rewritten: {}", text);
        assert!(text.contains("Ident(\"sim_clock_transport\")"), "a qualified view call is rewritten");
        assert!(text.contains("names: [\"sim_clock_base\", \"sim_clock_speed\"]"), "a multi-assign in a handler takes the prefix");
        assert!(text.contains("args: [Str(\"/\"), Ident(\"views_sim_render\")]"), "the entry's page view is rewritten");
        assert!(text.contains("args: [Ident(\"sim_clock_speed\")]"), "the entry's own direct import reads the same copy: {}", text);
        assert!(text.contains("Assign { name: \"sim_clock_speed\", value: Float(1.0) }"), "a qualified assignment in another part's handler writes the fact: {}", text);
        assert!(!text.contains("Ident(\"clock_speed\")") && !text.contains("clock_clock"), "no second prefix, no second copy");
        let _ = std::fs::remove_dir_all(&dir);
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

#[cfg(test)]
mod compile_error_translation_tests {
    /// `&str` is replaced before `String`: the other order turned the
    /// `&String` in "the trait bound `String: Borrow<&String>`" into
    /// `stringing`.
    #[test]
    fn a_reference_to_a_string_reads_as_a_string() {
        let stderr = "error[E0277]: the trait bound `String: Borrow<&String>` is not satisfied\n  --> src/main.rs:65:32\n";
        let out = super::translate_rust_compile_error(stderr);
        assert!(out.contains("string: Borrow<&string>"), "{}", out);
        assert!(!out.contains("stringing"), "{}", out);
    }
}
