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
mod int_width;
#[cfg(test)]
mod interp_tests;
mod ir_interp;
mod driver;
mod ir_verify;
mod lower_ir;
mod emit_rs;
mod ops;
mod autograd_bridge;
mod baseline;
mod states;
mod checker;
mod db_check;
mod derive;
mod diagnostic;
mod errors_corpus;
mod dev;
mod lsp;
mod kernel;
mod fuzz;
mod lexer;
mod lower;
mod parallel;
mod packages;
mod parser;
mod rehearsal;
mod home;
mod rust_names;
mod served;
mod compiled_harness;
mod served_parity;
mod selfcheck;
#[cfg(test)]
mod totality;
mod table;
mod effects;
mod token;
mod type_infer;
mod wireframe;
mod formatter;
mod freeze;
mod modules;
mod value;

use lexer::Lexer;
use parser::Parser;
use std::process;

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

/// The interpreter's allocator (spec §13, what stays native): the system
/// allocator on macOS reads the clock in its free path and frees across
/// threads slowly, and an interpreted program allocates for nearly every
/// value; mimalloc halved the CPU of revcomp and k-nucleotide run on every
/// core.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // the interpreter's frames to the recursion limit need a deeper stack
    // than the main thread's (crow_runtime::stops::STACK); the thread is
    // named for the main thread, so a stop on it exits as one there does
    match std::thread::Builder::new().name("main".to_string()).stack_size(crow_runtime::stops::STACK).spawn(crow_main) {
        Ok(h) => {
            if h.join().is_err() {
                process::exit(101);
            }
        }
        Err(e) => {
            eprintln!("crow: cannot start: {}", e);
            process::exit(1);
        }
    }
}

fn crow_main() {
    load_dotenv();
    // a panic in the program's run is its stop, reported by the guard around
    // the run (crow_runtime::stops); a panic of crow's own prints as Rust's
    crow_runtime::stops::quiet_under_guards();
    let args: Vec<String> = std::env::args().collect();

    // a program run from its source reconciles the tables it declares (spec,
    // db, The schema a program declares): crow's own run of it, and every
    // program crow starts, open those connections in the crowfile's mode or
    // reconciling; a program `crow build` makes starts in the crowfile's
    // mode, or verifies
    let runs = match args.get(1).map(|a| a.as_str()) {
        None | Some("run" | "test" | "check" | "launch" | "db") => true,
        Some(a) => is_source(a) || is_bytecode(a),
    };
    if runs && std::env::var_os("CROW_SCHEMA").is_none() {
        let mode = crowfile_schema_or_stop().unwrap_or(crow_db::schema::Mode::Reconcile);
        // Rust 2024: the environment is process-global; nothing else runs yet
        unsafe { std::env::set_var("CROW_SCHEMA", mode.name()) };
    }

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
        // `crow setup`: the Rust toolchain and the runtime fetched for
        // compiled builds; `--check` says what is present (home.rs)
        "setup" => {
            let check = args.iter().any(|a| a == "--check");
            if let Err(e) = home::setup(check) {
                eprintln!("(x.x) {}", e);
                process::exit(1);
            }
        }
        // `crow upgrade`: the binary the install line put in the home
        // replaced by the latest release, or the one `--version` names, held
        // to its published hash; `--check` says what is newer (home.rs)
        "upgrade" => {
            if let Err(e) = home::upgrade(&args[2..]) {
                eprintln!("(x.x) {}", e);
                process::exit(1);
            }
        }
        "fmt" => cmd_fmt(),
        "build" => cmd_build(),
        "launch" => cmd_launch(),
        "db" => cmd_db(&args[2..]),
        "check" => {
            // `crow check --self`: Crow held to itself (src/selfcheck.rs)
            if args.iter().any(|a| a == "--self") {
                process::exit(selfcheck::cmd_self_check(&args[2..]));
            }
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
                    "--record-fixtures" => RECORD_FIXTURES.store(true, std::sync::atomic::Ordering::SeqCst),
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
        // Crow in Jupyter: `crow kernel install`, and the kernel Jupyter starts
        "kernel" => {
            kernel::cmd_kernel(&args);
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
            // No --hazard: what the program is refused for is fixed first,
            // where its messages name the fix; then the check runs and
            // prints the numbered menu, so the N crow fix asks for is
            // discoverable from crow fix itself.
            if let (Some(filename), None) = (&file, hazard_n) {
                if let Some(code) = fix_refusals(filename) {
                    process::exit(code);
                }
                let source = read_file(filename);
                let url = db_url.clone().or_else(crowfile_db_url);
                match db_check::run(filename, &source, url.as_deref()) {
                    Ok(result) => {
                        let ordered = db_check::report::ordered(&result.hazards);
                        if ordered.is_empty() {
                            // (a program with no database has no hazards to speak of)
                            if url.is_some() || source.lines().any(|l| l.trim_start().starts_with("use") && l.contains("db")) {
                                println!("no database hazards to fix");
                            }
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
            // the harnesses check every program's IR before running it
            // (ir_verify): a lowering that disagrees with itself fails its
            // program as the bug it is, in whichever suite reaches it
            if args.iter().any(|a| matches!(a.as_str(), "--parity" | "--served" | "--book" | "--bench" | "--packages" | "--modules" | "--fuzz" | "--errors")) && std::env::var_os("CROW_IR_VERIFY").is_none() {
                // Rust 2024: the environment is process-global; nothing else
                // runs yet
                unsafe { std::env::set_var("CROW_IR_VERIFY", "strict") };
            }
            if args.iter().any(|a| a == "--parity") {
                // `crow test --parity 97 shapes`: only the programs whose
                // names hold one of the words
                let only: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
                cmd_test_parity(&only);
            } else if args.iter().any(|a| a == "--errors") {
                // `crow test --errors [--bless] [words]`: the wrong programs
                // of tests/errors, their refusals held to what was reviewed
                // and to the rules every message keeps
                process::exit(errors_corpus::cmd_test_errors(&args[2..]));
            } else if args.iter().any(|a| a == "--served") {
                // `crow test --served formats`: only the pages whose paths
                // hold one of the words
                let only: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
                process::exit(served_parity::run(&only));
            } else if args.iter().any(|a| a == "--fuzz") && args.iter().any(|a| a == "--mistakes") {
                // `crow test --fuzz 20 --mistakes`: generated programs, one
                // mistake planted in each, each refusal held to the rules
                process::exit(errors_corpus::cmd_test_mistakes(&args[2..]));
            } else if args.iter().any(|a| a == "--fuzz") {
                // `crow test --fuzz 20 --seed 7`: programs generated from a
                // seed, run in both modes
                fuzz::cmd_test_fuzz(&args[2..]);
            } else if args.iter().any(|a| a == "--packages") {
                // `crow test --packages models`: only the packages named
                let only: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
                cmd_test_packages(&only);
            } else if args.iter().any(|a| a == "--modules") {
                // `crow test --modules [llm]`: every standard module written
                // in Crow, each its own tests (crate::modules::MODULES)
                let only: Vec<String> = args[2..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
                cmd_test_modules(&only);
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
                if args.iter().any(|a| a == "--fixtures") {
                    // `crow test --fixtures`: no database is reached — each
                    // read is what `crow check --record-fixtures` kept, and
                    // the clock and the random draws are as the recording
                    // had them, for this process and the compiled apps its
                    // tests open (crow_db::fixtures)
                    let root = project_root(std::path::Path::new(&filename).parent().unwrap_or(std::path::Path::new(".")));
                    let crow_dir = std::fs::canonicalize(&root).unwrap_or(root).join(".crow");
                    // Rust 2024: the environment is process-global; nothing
                    // else runs yet
                    let misses = std::env::temp_dir().join(format!("crow_replay_misses_{}", process::id()));
                    let _ = std::fs::write(&misses, "");
                    unsafe {
                        std::env::set_var("CROW_REPLAY", &crow_dir);
                        std::env::set_var("CROW_REPLAY_MISSES", &misses);
                        std::env::set_var("CROW_CLOCK", ir_interp::REHEARSAL_EPOCH.to_string());
                        std::env::set_var("CROW_SEED", "0");
                    }
                }
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
        // crow dev [file.co]: served by the interpreter, reloaded as its
        // files change
        "dev" => {
            dev::cmd_dev(args.get(2).map(String::as_str));
        }
        "run" => {
            // the flags ahead of the file or job: `-i` interprets, `--opt N`
            // is the compiled program's optimization level (run_flags)
            let (interp, opt, rest) = run_flags(&args[2..]).unwrap_or_else(|e| {
                eprintln!("{}", e);
                process::exit(1);
            });
            let no_compiler = !interp && rest.first().is_none_or(|t| is_source(t)) && !home::can_compile();
            if no_compiler {
                // both modes answer the same by design; the difference is
                // speed, so a machine with no compiler yet runs the program
                // interpreted and is told how to get one
                eprintln!("(o.o) no compiler here yet: running interpreted — `crow setup` fetches the Rust toolchain and the runtime (about 175 MB, once), and `crow run` compiles from then on");
            }
            if interp || no_compiler {
                if let Some(file) = rest.first() {
                    cmd_run_file_interpreted(file);
                } else {
                    cmd_run_interpreted();
                }
            } else if let Some(target) = rest.first() {
                if is_source(target) || is_bytecode(target) {
                    // crow run file.co arg1 arg2 ... -> transpile + compile + run
                    let extra_args = rest[1..].to_vec();
                    cmd_run_file(target, &extra_args, opt);
                } else {
                    // crow run <job_name> -> look up in crowfile [jobs]
                    cmd_run_job(target, opt);
                }
            } else {
                // crow run (uses crowfile) -> transpile + compile + run
                cmd_run(&[], opt);
            }
        }
        other => {
            // check if it's a .co or .coc file (shortcut)
            if is_source(other) && !home::can_compile() {
                eprintln!("(o.o) no compiler here yet: running interpreted — `crow setup` fetches the Rust toolchain and the runtime (about 175 MB, once), and `crow run` compiles from then on");
                cmd_run_file_interpreted(other);
            } else if is_source(other) || is_bytecode(other) {
                let extra_args = args[2..].to_vec();
                cmd_run_file(other, &extra_args, 0);
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
    eprintln!("  crow setup             fetch the Rust toolchain and the runtime for compiled builds (--check: what is present)");
    eprintln!("  crow upgrade           replace this crow with the latest release, verified (--version X: that release; --check: what is newer)");
    eprintln!("  crow build             transpile to rust and compile to native binary");
    eprintln!("  crow build file.co   transpile a specific file");
    eprintln!("  crow run               transpile + compile + run main.co");
    eprintln!("  crow run file.co     transpile + compile + run a specific file");
    eprintln!("  crow run --opt 2 file.co  the same, the program optimized at level 2 (0 to 3; 0 unless said)");
    eprintln!("  crow run train         run a named job from crowfile");
    eprintln!("  crow run -i file.co  run with interpreter (slow, for debugging)");
    eprintln!("  crow stop              stop the running app (pidfile, then project binary)");
    eprintln!("  crow dev [file.co]     serve with the interpreter, reloading as files change (sessions kept)");
    eprintln!("  crow book [--port N]   serve the book with every code block runnable in place (default port 3008)");
    eprintln!("  crow check             auto-verify project (uses crowfile)");
    eprintln!("  crow check file.co   auto-verify a specific file");
    eprintln!("  crow check --db <url>  + schema-aware db checks (or db = '…' in crowfile)");
    eprintln!("  crow check --record-fixtures  the scenarios read the real databases; keep what each statement answers");
    eprintln!("  crow fix file.co --hazard N [--option M]   apply a fix from the check report");
    eprintln!("  crow db diff [file.co] the SQL that makes each database hold the tables the program declares (a migration draft)");
    eprintln!("  crow lsp               language server over stdio (see docs/crow-lsp.md)");
    eprintln!("  crow kernel install    make Crow a Jupyter kernel (notebooks, JupyterLab, VS Code)");
    eprintln!("  crow fmt               format project (uses crowfile)");
    eprintln!("  crow fmt file.co     format a specific file");
    eprintln!("  crow fmt --check       check formatting without changing files");
    eprintln!("  crow fmt --fix-interpolation  migrate {{expr}} strings to `expr`");
    eprintln!("  crow fmt --fix-views      migrate ui-building fns to view declarations");
    eprintln!("  crow launch 4 file.co  build + run on 4 distributed processes");
    eprintln!("  crow test --parity     run tests/parity/ in both modes, diff output (words after it pick programs by name)");
    eprintln!("  crow test --packages   in the crow repository: each package's tests, and its example in both modes");
    eprintln!("  crow test --fuzz [n]   n generated programs run in both modes; one they disagree on is kept (--seed s repeats a run)");
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
    // the book's page on `sys` shows the standard library's own layer: its
    // blocks run through the harness's door, which a program has not
    // SAFETY: set before the blocks' processes are spawned, on one thread
    unsafe { std::env::set_var("CROW_SYS", "1") };
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
                // it fails as the book says, with Crow's refusal: a lowering
                // that disagrees with itself, or Rust that does not compile,
                // is a bug in Crow, not the documented error
                let (ok, _, err) = run_mode(&["build"], &path);
                if ok {
                    (false, format!("  FAIL  {} (documented as an error, but builds)\n", label))
                } else if err.contains("a bug in Crow") || err.contains("error[E") {
                    (false, format!("  FAIL  {} (fails, but as a bug in Crow)\n        {}\n", label, err.lines().find(|l| l.contains("bug in Crow") || l.contains("error[E")).unwrap_or("")))
                } else {
                    (true, format!("  ok    {} (fails as documented)\n", label))
                }
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

/// The spec's reference-table rows (`| `name[…` |`) that name no function
/// of the section they are in: Core Builtins' rows name core builtins, and
/// a module's section of the Standard Library its own functions. Checked
/// against every table at once, math's `rand[d1, d2, …]` had passed as
/// core's `rand`, an int, from when math's functions became callable
/// without `math.`, which gave core the name.
fn phantom_rows(spec: &str, tables: &[(&str, Vec<&str>)]) -> Vec<String> {
    let keywords = ["map", "struct", "fn", "view", "test", "use"];
    // a module's own form under a core builtin's name, where core's takes
    // other kinds: web's `json[map]` answers a response (`json[text]`
    // parses), and `push[queue, fn]` queues a task (`push[list, x]`)
    const OWN_FORMS: &[(&str, &str)] = &[("web", "json"), ("web", "push")];
    let (mut h2, mut h3, mut h4) = (String::new(), String::new(), String::new());
    let mut phantom = Vec::new();
    for (i, line) in spec.lines().enumerate() {
        if let Some(h) = line.strip_prefix("## ") {
            (h2, h3, h4) = (h.to_string(), String::new(), String::new());
            continue;
        }
        if let Some(h) = line.strip_prefix("### ") {
            (h3, h4) = (h.to_string(), String::new());
            continue;
        }
        if let Some(h) = line.strip_prefix("#### ") {
            h4 = h.to_string();
            continue;
        }
        let Some(section) = reference_tables(&h2, &h3, &h4) else { continue };
        let l = line.trim_start();
        let Some(rest) = l.strip_prefix("| `") else { continue };
        let name: String = rest.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
        let after = rest[name.len()..].chars().next().unwrap_or(' ');
        if after != '[' || name.is_empty() || keywords.contains(&name.as_str()) {
            continue;
        }
        let of_section = tables.iter().filter(|(t, _)| section.contains(t)).any(|(_, names)| names.contains(&name.as_str()))
            || OWN_FORMS.iter().any(|(module, n)| section.contains(module) && *n == name);
        if of_section {
            continue;
        }
        let elsewhere: Vec<&str> = tables.iter().filter(|(_, names)| names.contains(&name.as_str())).map(|(t, _)| *t).collect();
        phantom.push(if elsewhere.is_empty() {
            format!("line {}: `{}[` is documented but the compiler has no such builtin", i + 1, name)
        } else {
            format!("line {}: `{}[` is documented under {} but is {}'s", i + 1, name, section.join(" and "), elsewhere.join(" and "))
        });
    }
    phantom
}

/// The compiler's tables a reference section's rows name: Core Builtins'
/// the core's, each module's section of the Standard Library the
/// module's (web's the ui names, the elements, the controls and charts
/// too); none outside the two reference sections.
fn reference_tables(h2: &str, h3: &str, h4: &str) -> Option<Vec<&'static str>> {
    if h2.starts_with("11.") {
        return Some(vec!["core"]);
    }
    if !h2.starts_with("13.") {
        return None;
    }
    let sub = |module: &str| h4.contains(&format!("`{}`", module));
    // the web section documents the page too (the elements, the controls,
    // the charts), through its Layout section
    if h3 == "web" || h3.starts_with("Layout") {
        return Some(vec!["web", "ui", "element", "control", "chart"]);
    }
    // a module's section; a sub-module documented under its heading
    // (`#### … (sub-module: \`math.stats\`)`) is its own table
    if let Some(m) = modules::by_path(h3).filter(|m| m.docs.is_some()) {
        let child = modules::MODULES.iter().find(|c| c.docs.is_some() && modules::parent_of(c.path) == Some(m.path) && sub(c.path));
        return Some(vec![child.map_or(m.path, |c| c.path)]);
    }
    // the section's opening, before any module's heading
    let mut every = vec!["core", "element", "control"];
    every.extend(modules::MODULES.iter().filter(|m| m.docs.is_some()).map(|m| m.path));
    Some(every)
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
    // the compiler's tables: the core's names, the elements and controls,
    // and every documented module's (crate::modules::MODULES) — its rows
    // and forms, and those of the modules documented under it
    let mut tables: Vec<(&str, Vec<&str>)> = vec![
        ("core", lower_ir::CORE_RESERVED.to_vec()),
        ("element", lower::UI_ELEMENTS.to_vec()),
        ("control", lower::UI_CONTROLS.to_vec()),
    ];
    for m in modules::MODULES.iter().filter(|m| m.docs.is_some()) {
        tables.push((m.path, lower_ir::names_of(m.path)));
    }
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
    let phantom = phantom_rows(&spec, &tables);
    if phantom.is_empty() {
        println!("  ok    every `name[` table row is a compiler name of its section");
    } else {
        failures += phantom.len();
        for p in &phantom { println!("  FAIL  {}", p); }
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
    for m in modules::MODULES.iter().filter(|m| m.docs.is_some()).map(|m| m.path) {
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
        "test",                                             // a keyword
        "app", "click",                                     // test-harness names (used in test blocks, not run blocks)
        "contains", "has",                                  // the rows behind `x in xs` and `key in m`: written as the operator
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
            if p.extension().map_or(true, |x| x != "co") { continue; }
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
    if let Some(m) = section("### math") { let ex: Vec<(usize, usize)> = [stats, nn].iter().flatten().copied().collect(); add("math", "math", "use math", "math", modules::by_path("math").and_then(|m| m.docs).unwrap_or_default(), one(Some(m)), ex); }
    if stats.is_some() { add("math-stats", "math.stats", "use math.stats", "math.stats", modules::by_path("math.stats").and_then(|m| m.docs).unwrap_or_default(), one(stats), vec![]); }
    if nn.is_some() { add("math-nn", "math.nn", "use math.nn", "math.nn", modules::by_path("math.nn").and_then(|m| m.docs).unwrap_or_default(), one(nn), vec![]); }
    if let Some(f) = section("### frame") { add("frame", "frame", "use frame", "frame", modules::by_path("frame").and_then(|m| m.docs).unwrap_or_default(), one(Some(f)), vec![]); }
    if let Some(d) = section("### db") { add("db", "db", "use db", "db", modules::by_path("db").and_then(|m| m.docs).unwrap_or_default(), one(Some(d)), vec![]); }
    if let Some(l) = section("### llm") { add("llm", "llm", "use llm", "llm", modules::by_path("llm").and_then(|m| m.docs).unwrap_or_default(), one(Some(l)), vec![]); }
    if let Some(i) = section("### io") { add("io", "io", "use io", "io", modules::by_path("io").and_then(|m| m.docs).unwrap_or_default(), one(Some(i)), vec![]); }
    if let Some(d) = section("### doc") { add("doc", "doc", "use doc", "doc", modules::by_path("doc").and_then(|m| m.docs).unwrap_or_default(), one(Some(d)), vec![]); }
    if let Some(b) = section("### bits") { add("bits", "bits", "use bits", "bits", modules::by_path("bits").and_then(|m| m.docs).unwrap_or_default(), one(Some(b)), vec![]); }
    if let Some(c) = section("### chart") { add("chart", "chart", "use chart", "chart", modules::by_path("chart").and_then(|m| m.docs).unwrap_or_default(), one(Some(c)), vec![]); }
    if let Some(y) = section("### sys") { add("sys", "sys", "use sys", "sys", modules::by_path("sys").and_then(|m| m.docs).unwrap_or_default(), one(Some(y)), vec![]); }
    // web splits where the spec does: the view side (`use ui`) and the server side (`use web`)
    // (the spec's `### web` runs, through the `### Layout` section, up to `### db`)
    if let (Some((w0, _)), Some((d0, _))) = (section("### web"), section("### db")) {
        let server = (w0..d0).find(|&i| lines[i].trim_end() == "#### Core (Server)").unwrap_or(d0);
        add("ui", "ui", "use ui", "ui", modules::by_path("ui").and_then(|m| m.docs).unwrap_or_default(), vec![(w0, server, false)], vec![]);
        if server < d0 { add("web", "web", "use web", "web", modules::by_path("web").and_then(|m| m.docs).unwrap_or_default(), vec![(server, d0, true)], vec![]); }
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

/// `crow test --packages`: the packages the repository ships (packages/)
/// held to the language's own bar — every file's test blocks pass, and each
/// package's `example.co` runs in both modes with the same output. The
/// example is the compiled side: a test block runs in the interpreter, only
/// the apps it opens compiled.
/// `crow test --modules`: the standard modules written in Crow, each its
/// own `test` blocks (`crow test src/modules/<name>.co`), from the
/// manifest — a module added is tested with no line written here.
fn cmd_test_modules(only: &[String]) {
    let files: Vec<(String, std::path::PathBuf)> = modules::MODULES
        .iter()
        .filter(|m| m.source.is_some())
        .map(|m| {
            let name = if m.path.is_empty() { "core" } else { m.path.rsplit('.').next().unwrap_or(m.path) };
            (if m.path.is_empty() { "core".to_string() } else { m.path.to_string() }, std::path::Path::new("src/modules").join(format!("{name}.co")))
        })
        .filter(|(path, _)| only.is_empty() || only.iter().any(|w| path.contains(w.as_str())))
        .collect();
    if files.iter().any(|(_, f)| !f.exists()) {
        eprintln!("error: the modules' sources are under src/modules/ (run from the repository's root)");
        process::exit(1);
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    let last = |text: &str| text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    let start = std::time::Instant::now();
    let mut failures = 0;
    println!("modules: {} written in Crow, each its own test blocks\n", files.len());
    for (path, file) in &files {
        let (ok, out, err) = match std::process::Command::new(&exe).arg("test").arg(file).env("CROW_BUILD_PROFILE", "dev").output() {
            Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
        };
        if ok {
            println!("  ok    {} ({})", path, last(&out));
        } else {
            failures += 1;
            println!("  FAIL  {}\n{}\n{}", path, out.trim_end(), err.trim_end());
        }
    }
    if failures == 0 {
        println!("\n✓ {} modules' tests pass ({:.1}s)", files.len(), start.elapsed().as_secs_f64());
    } else {
        println!("\n✗ {} of {} modules failed ({:.1}s)", failures, files.len(), start.elapsed().as_secs_f64());
        process::exit(1);
    }
}

fn cmd_test_packages(only: &[String]) {
    let dir = std::path::Path::new("packages");
    let mut packages: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    packages.retain(|p| only.is_empty() || only.iter().any(|w| p.file_name().is_some_and(|n| n.to_string_lossy() == w.as_str())));
    packages.sort();
    if packages.is_empty() {
        eprintln!("error: no packages under packages/ match (run from the repository's root)");
        process::exit(1);
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("error locating crow binary: {}", e);
        process::exit(1);
    });
    let run = |args: &[&str], file: &std::path::Path| -> (bool, String, String) {
        match std::process::Command::new(&exe).args(args).arg(file).env("CROW_BUILD_PROFILE", "dev").output() {
            Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => (false, String::new(), format!("failed to spawn: {}", e)),
        }
    };
    let last = |text: &str| text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    let start = std::time::Instant::now();
    let (mut checked, mut failures) = (0, 0);
    println!("packages: {} under packages/, test blocks and each example in both modes\n", packages.len());
    for package in &packages {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(package)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "co")).collect())
            .unwrap_or_default();
        files.sort();
        for file in &files {
            let name = file.display().to_string();
            if file.file_name().is_some_and(|n| n == "example.co") {
                checked += 1;
                let (i_ok, i_out, i_err) = run(&["run", "-i"], file);
                let (c_ok, c_out, c_err) = run(&["run"], file);
                if i_ok && c_ok && i_out == c_out {
                    println!("  ok    {} (both modes, {} lines)", name, i_out.lines().count());
                } else {
                    failures += 1;
                    println!("  FAIL  {}", name);
                    if !i_ok { println!("        interpreter: {}", last(&i_err)); }
                    if !c_ok { println!("        compiled: {}", last(&c_err)); }
                    if i_ok && c_ok {
                        let at = i_out.lines().zip(c_out.lines()).position(|(a, b)| a != b).unwrap_or(i_out.lines().count().min(c_out.lines().count()));
                        println!("        line {}: interpreter {:?}, compiled {:?}", at + 1, i_out.lines().nth(at).unwrap_or(""), c_out.lines().nth(at).unwrap_or(""));
                    }
                }
                continue;
            }
            if !std::fs::read_to_string(file).unwrap_or_default().lines().any(|l| l.starts_with("test ")) {
                continue;
            }
            checked += 1;
            let (ok, out, err) = run(&["test"], file);
            if ok {
                println!("  ok    {} ({})", name, last(&out).trim_start_matches("\x1b[32m").replace("\x1b[0m", ""));
            } else {
                failures += 1;
                println!("  FAIL  {}\n        {}", name, if last(&out).is_empty() { last(&err) } else { last(&out) });
            }
        }
        if !package.join("example.co").exists() {
            failures += 1;
            println!("  FAIL  {}: no example.co — a package shows each of its files at work, in both modes", package.display());
        }
    }
    if failures == 0 {
        println!("\n✓ {} checks pass across {} packages ({:.1}s)", checked, packages.len(), start.elapsed().as_secs_f64());
    } else {
        println!("\n✗ {} of {} checks fail", failures, checked);
        process::exit(1);
    }
}

fn cmd_test_parity(only: &[String]) {
    let dir = std::path::Path::new("tests/parity");
    if !dir.is_dir() {
        eprintln!("error: no tests/parity/ directory found (run from the project root)");
        process::exit(1);
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |x| x == "co"))
                .filter(|p| only.is_empty() || only.iter().any(|w| p.to_string_lossy().contains(w.as_str())))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        eprintln!("error: no .co files in tests/parity/ match");
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
            // the same silence is no match: a program that logs nothing
            // compares nothing, and would pass with every builtin wrong
            if i_out.trim().is_empty() {
                failures += 1;
                println!("  FAIL  {}", name);
                println!("        prints nothing in either mode: a program that logs nothing compares nothing");
                continue;
            }
            println!("  ok    {}", name);
            continue;
        }
        // both modes stopping at the same point with the same words is
        // parity too (a crash's last line, a backtrace's note, never is; nor
        // is the lowering disagreeing with itself — the program never ran,
        // and that had passed as both modes stopping alike)
        let i_last = i_err.lines().last();
        let crow_bug = i_last.is_some_and(|l| l.contains("a bug in Crow, not in this program"));
        if !i_ok && !t_ok && !crow_bug && i_out == t_out && i_last.is_some() && i_last == t_err.lines().last() {
            println!("  ok    {} (both stop: {})", name, i_last.unwrap_or_default());
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

                // a paste arrives as one reading of many lines: a whole cell,
                // run as it is (it had waited for a blank line behind the
                // continuation prompt, and looked like nothing happening)
                if !in_block && trimmed.contains('\n') {
                    repl_eval(&mut session, trimmed, &mut accumulated_stmts);
                    continue;
                }

                // Multi-line block handling: a line indented, or an `else`
                // under its `if`, goes on with the block
                let goes_on = trimmed.starts_with(' ') || trimmed.starts_with('\t') || trimmed.starts_with("else") || trimmed.starts_with("elif");
                if in_block {
                    if trimmed.is_empty() || !goes_on {
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
                    // (the words the parser opens a block with)
                    stripped.split_whitespace()
                        .any(|w| matches!(w, "if" | "for" | "while" | "fn" | "view" | "test" | "struct" | "derive"))
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
            // a program that serves pages is run from its file: the console
            // runs statements and shows their values, and serves nothing
            if serve_line(&stmts).is_some() {
                eprintln!("(o.o) the console serves no pages: save the program as a file, then `crow run -i app.co` serves it, or `crow dev app.co` reloads it as it changes");
            }
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
    // what crow builds and installs is rebuilt from the project: crow.lock,
    // committed, is what makes `crow install` the same everywhere
    write_file(&project_dir.join(".gitignore"), "crow_modules/\n.crow_build/\n");

    println!("created project '{}'", short_name);
    let mut files: Vec<String> = Vec::new();
    files.push("  crowfile".to_string());
    files.push("  main.co".to_string());
    files.push("  .gitignore".to_string());
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

/// `crow install [--update [name …]]`: the crowfile's deps into
/// `crow_modules/`, each pinned by `crow.lock` (packages.rs; spec, Package
/// Manager).
fn cmd_install() {
    let config = parse_crowfile();
    if config.deps.is_empty() {
        println!("no deps in crowfile");
        return;
    }
    let args: Vec<String> = std::env::args().skip(2).collect();
    let updating = args.iter().any(|a| a == "--update");
    let named: Vec<String> = args.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let root = project_root(&std::env::current_dir().unwrap_or_default());
    match packages::install(&root, &config.deps, &named, updating && named.is_empty(), &crowfile_deps) {
        Ok(said) => {
            for line in said {
                println!("  {}", line);
            }
            println!("done");
        }
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    }
}

/// Whether a function came from a dependency (`crow_modules/…`): its
/// prefix's part is a package's file. A package is held by its own tests;
/// the app's check probes the app's functions, not a dependency's (an
/// example's `crow check` had reported the vectors package's `cosine` of
/// two lengths six times).
pub(crate) fn from_dependency(fn_name: &str) -> bool {
    // (a package linked into crow_modules is recorded by its path as the
    // program names it, absolute where the link leaves the project)
    EXPANSION_PARTS.with(|p| p.borrow().iter().any(|(prefix, rel)| fn_name.starts_with(prefix.as_str()) && rel.split('/').any(|c| c == "crow_modules")))
}

/// Transpile a .co file to Rust, compile it, and return the path to the binary.
/// If `quiet` is true, suppress cargo build output (used for `crow run`).
/// If `release` is true, compile with --release (optimized). Otherwise, use debug mode (fast compile).
/// `opt` is the dev build's optimization level (`crow run --opt N`; 0 unless
/// said); a release build ignores it.
fn transpile_and_compile(entry: &str, quiet: bool, cpu: Option<usize>, release: bool, opt: u8) -> std::path::PathBuf {
    // Verification harnesses (`crow test --book`, `--parity`) set
    // CROW_BUILD_PROFILE=dev: a checked program only has to run, not ship,
    // and skipping optimization + LTO is most of the per-block build time.
    // CROW_DEBUGINFO=1: the `debuginfo` profile — dev with debug info — in a
    // target directory of its own, for a Rust backtrace with file and line
    // when a program panics (a bug in Crow, never a Crow error). The plain
    // dev build carries no debug info: macOS keeps an object file per unit
    // for it, and the harness caches had grown by the hundred gigabytes.
    let debuginfo = std::env::var("CROW_DEBUGINFO").map(|v| v == "1").unwrap_or(false);
    let release = release && !debuginfo && std::env::var("CROW_BUILD_PROFILE").map(|v| v != "dev").unwrap_or(true);
    let profile_dir = if release { "release" } else if debuginfo { "debuginfo" } else { "debug" };
    let source = read_file(entry);
    let stmts = build(&source).unwrap_or_else(|e| refuse(entry, &source, &e));

    // Expand module imports before type inference so call sites inform param types
    let source_dir = std::path::Path::new(entry)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);

    // Resolve named arguments to positional form (shared with the interpreter)
    let stmts = lower::lower_program(&stmts).unwrap_or_else(|e| refuse(entry, &source, &e));

    let entry_file = std::path::Path::new(entry).file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.to_string());
    let name = std::path::Path::new(&entry_file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| entry_file.clone());
    // Type inference, and the lowering to the IR the interpreter runs,
    // which refuses what is not a program. The compiler may ask the
    // program's databases what each literal query returns, so frame cells
    // are typed natively.
    let (ctx, program) = infer_and_lower(&stmts, describing(), &name).unwrap_or_else(|e| refuse(entry, &source, &e));
    // A name Rust reserves (`move`, `impl`) is a Crow name like any other:
    // the compiled program renames each one it declares, after the gate has
    // spoken in the program's own names, and prints the program's name
    // wherever one reaches the reader
    let (ctx, program, shown) = match rust_names::rename_rust_keywords(&stmts) {
        None => (ctx, program, std::collections::BTreeMap::new()),
        Some((renamed, shown)) => {
            let (ctx, program) = infer_and_lower(&renamed, describing(), &name).unwrap_or_else(|e| refuse(entry, &source, &e));
            (ctx, program, shown)
        }
    };
    for note in &ctx.describe_notes {
        eprintln!("{}", note);
    }
    // The Rust backend: the program's IR printed as Rust
    let test_control = TEST_CONTROL.load(std::sync::atomic::Ordering::SeqCst);
    let emitted = emit_rs::emit_with(&program, ctx.registry.clone(), &entry_file, shown, cpu, test_control, crowfile_schema_or_stop()).unwrap_or_else(|e| {
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
    // every program allocates with mimalloc, as the interpreter does (spec
    // §13, what stays native): the system allocator had cost compiled
    // binarytrees nine times the CPU
    // the runtime crates the program links, wherever this install keeps
    // them (home.rs): what is missing is fetched first, once, saying so
    home::ensure_for_build().unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    });
    let rt = home::runtime().unwrap_or_else(|| {
        eprintln!("error: a compiled build needs the runtime, which is not on this machine: `crow setup` fetches it");
        process::exit(1);
    });
    let rt_dir = rt.dir().display().to_string();
    let mut dep_list: Vec<String> = vec!["mimalloc = { version = \"0.1\", default-features = false }".to_string()];
    if emitted.needs_rayon {
        dep_list.push("rayon = \"1.11\"".to_string());
    }
    if emitted.needs_serde_json {
        dep_list.push("serde_json = \"1\"".to_string());
    }
    if emitted.needs_serde {
        dep_list.push("serde = { version = \"1\", features = [\"derive\"] }".to_string());
    }
    if emitted.needs_crow_runtime {
        let runtime_path = format!("{}/crow_runtime", rt_dir);
        // what the program's paths may name (sources), and Arrow for a
        // response in it
        // (a response as Arrow asks crow_web's arrow_ipc, which carries the
        // runtime's arrow with it)
        let features: Vec<String> = emitted.runtime_features.iter().cloned().collect();
        if features.is_empty() {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\" }}", runtime_path));
        } else {
            dep_list.push(format!("crow_runtime = {{ path = \"{}\", features = [{}] }}", runtime_path, features.iter().map(|f| format!("\"{}\"", f)).collect::<Vec<_>>().join(", ")));
        }
    }
    if emitted.needs_regex {
        dep_list.push("regex = \"1\"".to_string());
    }
    if emitted.needs_crow_web {
        let web_path = format!("{}/crow_web", rt_dir);
        let features: Vec<&str> = emitted.web_features.iter().map(|s| s.as_str()).collect();
        // (a program that only makes a response from bytes needs none)
        if features.is_empty() {
            dep_list.push(format!("crow_web = {{ path = \"{}\" }}", web_path));
        } else {
            dep_list.push(format!("crow_web = {{ path = \"{}\", features = [\"{}\"] }}", web_path, features.join("\", \"")));
        }
        // crow_web needs tokio for async runtime
        dep_list.push("tokio = { version = \"1\", features = [\"full\"] }".to_string());
    }
    if emitted.needs_crow_db {
        let db_path = format!("{}/crow_db", rt_dir);
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
    let cargo_toml = generated_cargo_toml(&project_name, &deps, opt);
    std::fs::write(build_dir.join("Cargo.toml"), &cargo_toml).unwrap_or_else(|e| {
        eprintln!("error writing Cargo.toml: {}", e);
        process::exit(1);
    });
    if rt.vendored() {
        let cargo_dir = build_dir.join(".cargo");
        let _ = std::fs::create_dir_all(&cargo_dir);
        std::fs::write(cargo_dir.join("config.toml"), home::vendored_config(&rt.dir().join("vendor"))).unwrap_or_else(|e| {
            eprintln!("error writing .cargo/config.toml: {}", e);
            process::exit(1);
        });
        let lock = build_dir.join("Cargo.lock");
        if !lock.exists() {
            if let Ok(text) = std::fs::read_to_string(rt.dir().join("Cargo.lock")) {
                let _ = std::fs::write(&lock, text);
            }
        }
    }

    // Check if source has changed — skip compilation if binary is up-to-date
    let main_rs_path = src_dir.join("main.rs");
    let binary_path = build_dir.join("target").join(profile_dir).join(&project_name);

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
    let mut cmd = home::cargo();
    if release {
        cmd.args(["build", "--release"]).current_dir(&build_dir);
    } else if debuginfo {
        cmd.args(["build", "--profile", "debuginfo"]).current_dir(&build_dir);
    } else {
        cmd.args(["build"]).current_dir(&build_dir);
    }
    // the binary lands where the run looks for it, whatever target
    // directory the environment names (a CARGO_TARGET_DIR set for other
    // projects had sent it elsewhere, and `crow run` found nothing to run)
    // (absolute: cargo runs inside the build directory)
    cmd.env("CARGO_TARGET_DIR", std::env::current_dir().unwrap_or_default().join(&build_dir).join("target"));
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

/// What a build says when the program's Rust does not compile. Crow's own
/// checks refuse a program before any Rust is written, so the Rust compiler
/// refusing what Crow accepted is a bug in Crow: said as one, after the
/// compiler's first error and where it points, so it can be reported. (The
/// compiler's words had been reworded as if they were the program's — a
/// borrow of a moved value in the printed Rust read "value was already used
/// and cannot be accessed again".) A division by a literal zero, or a
/// literal that overflows, the compiler sees before the program runs: that
/// one is the program's.
fn translate_rust_compile_error(stderr: &str) -> String {
    if stderr.contains("attempt to divide") && stderr.contains("by zero") {
        return "error: cannot divide by zero".to_string();
    }
    if stderr.contains("attempt to calculate the remainder") && stderr.contains("a divisor of zero") {
        return "error: cannot divide by zero".to_string();
    }
    if stderr.contains("arithmetic operation overflowed") || stderr.contains("overflow") && stderr.contains("attempt to") {
        return "error: arithmetic overflow".to_string();
    }
    let lines: Vec<&str> = stderr.lines().collect();
    let first = lines.iter().position(|l| l.starts_with("error") && !l.contains("could not compile") && !l.contains("aborting due to"));
    let said = match first {
        Some(i) => {
            let at = lines[i + 1..].iter().take(3).find(|l| l.trim_start().starts_with("-->")).map(|l| format!("\n  {}", l.trim())).unwrap_or_default();
            format!("{}{}", lines[i], at)
        }
        None => stderr.trim().to_string(),
    };
    format!("{}\nerror: the compiled program does not build — the Rust compiler refused what Crow accepted, a bug in Crow, not in this program (`crow run -i` runs it meanwhile)", said)
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

/// `crow check --record-fixtures`: each scenario reads the real databases,
/// and what each statement answers is kept in `.crow/fixtures`.
static RECORD_FIXTURES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn cmd_build() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--rs") {
        PRINT_RS.store(true, std::sync::atomic::Ordering::SeqCst);
        let file = args.iter().skip(2).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
        transpile_and_compile(&file, true, None, false, 0);
        return;
    }
    // `crow build --types file.co`: every type inference found, each
    // body's locals included — the oracle inference's rewrite is held to
    if args.iter().any(|a| a == "--types") {
        let file = args.iter().skip(2).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
        match types_of_file(&file, describing()) {
            Ok(text) => print!("{}", text),
            Err(e) => refuse(&file, &read_file(&file), &e),
        }
        return;
    }
    // `crow build --ir file.co`: the program's one typed form, printed
    // (docs/ir-design.md); nothing is compiled
    if args.iter().any(|a| a == "--ir") {
        let file = args.iter().skip(2).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
        match ir_of_file(&file, describing()) {
            Ok(text) => print!("{}", text),
            Err(e) => refuse(&file, &read_file(&file), &e),
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

    let binary = transpile_and_compile(&entry, false, cpu, true, 0);

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
    let binary = transpile_and_compile(&entry, true, None, true, 0);

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

/// Whether a process's working directory is `here` (lsof): what `crow
/// stop` asks before it signals one, so another project's app — or a
/// process that took a stale pidfile's number since — is never touched.
fn runs_in(pid: i32, here: &str) -> bool {
    // (Linux names a process's working directory in /proc; a Mac, lsof)
    if let Ok(cwd) = std::fs::read_link(format!("/proc/{}/cwd", pid)) {
        return cwd.to_string_lossy() == here;
    }
    std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
        .lines()
        .any(|l| l.strip_prefix('n') == Some(here))
}

fn cmd_stop() {
    let here = std::env::current_dir().unwrap_or_default();
    let here_s = here.to_string_lossy().to_string();
    if let Ok(pid_s) = std::fs::read_to_string(".crow/app.pid") {
        if let Ok(pid) = pid_s.trim().parse::<i32>() {
            let alive = unsafe { libc_kill(pid, 0) } == 0 && runs_in(pid, &here_s);
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
    // Candidates: the cargo-built binary AND the copied ./main that
    // `crow build` announces, and a program the interpreter serves — `crow
    // run -i`, and `crow dev`'s watcher with it, so the watcher does not
    // start it again (the interpreter is crow itself: no pidfile and no
    // build named it, and `crow stop` had said nothing was running).
    // Over-matching is safe — every candidate is cwd-verified against this
    // project before any signal is sent.
    let me = std::process::id() as i32;
    let mut pids = String::new();
    for pattern in [".crow_build/target", "^\\./main( |$)", "/main$", "crow (run -i|dev)( |$)"] {
        if let Ok(o) = std::process::Command::new("pgrep").args(["-f", pattern]).output() {
            pids.push_str(&String::from_utf8_lossy(&o.stdout));
            pids.push('\n');
        }
    }
    let mut stopped = 0;
    let mut seen: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    for pid_s in pids.split_whitespace() {
        let Ok(pid) = pid_s.parse::<i32>() else { continue };
        if pid == me || !seen.insert(pid) {
            continue;
        }
        if runs_in(pid, &here_s) {
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
            let known = prev_dot || id == "query" || id == conn || top_level.contains(id) || lower_ir::BUILTINS.contains(&id.as_str());
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
    let stmts = build(&source).unwrap_or_else(|e| refuse(filename, &source, &e));
    // A part's tests run with the parts it imports inlined, exactly as the
    // run, build and check paths see the program
    let source_dir = std::path::Path::new(filename)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let lowered = lower::lower_program(&stmts).unwrap_or_else(|e| refuse(filename, &source, &e));
    // The validator gates the harness as it gates run and build: a part
    // whose tests pass is a part the program accepts
    if let Err(e) = infer_and_validate(&lowered, describing()) {
        refuse(filename, &source, &e);
    }
    // `crow test --compiled`: the apps the blocks open are the compiled
    // program (compiled_harness.rs); the blocks themselves run here
    let compiled = std::env::args().any(|a| a == "--compiled");
    if compiled && compiled_harness::opens_an_app(&lowered) {
        TEST_CONTROL.store(true, std::sync::atomic::Ordering::SeqCst);
        let binary = transpile_and_compile(filename, true, None, false, 0);
        compiled_harness::use_binary(std::fs::canonicalize(&binary).unwrap_or(binary));
    }
    // the blocks are prepared from the statements as written: the shared
    // pipeline lowers them itself, and lowered twice, `window[…, by = …]`
    // and `request[…, timeout = …]` — their options filled in once — were
    // refused as too many arguments in every test block
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
        let against = if crow_runtime::fixtures::replaying().is_some() { format!("{}, the databases and the requests answered from .crow/fixtures", against) } else { against };
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

/// The app's scenarios (app-baseline-design.md §9, B3): each named test in
/// its files that opens the app (`app[…]` — a test of pure functions leaves
/// no state, and is a truth `crow test` runs), run in a rehearsal — no connection opened, queries answered
/// from `.crow/fixtures` or from rows generated from `.crow/schema`, the
/// clock at the rehearsal's epoch and the random draws seeded. Each is an
/// artifact saying how it ended, what it wrote, which statements were
/// answered with generated rows, which assertions did not hold on that
/// data, and the facts it changed; key → (file, text).
fn scenario_artifacts(entry: &str, parts: &std::collections::BTreeMap<String, String>) -> std::collections::BTreeMap<String, (String, String)> {
    let entry_dir = std::path::Path::new(entry).parent().unwrap_or(std::path::Path::new("."));
    let root = project_root(entry_dir);
    let mut files: Vec<String> = parts.values().cloned().collect();
    let entry_rel = std::fs::canonicalize(entry)
        .ok()
        .and_then(|p| p.strip_prefix(&root).ok().map(|r| r.display().to_string()))
        .unwrap_or_else(|| entry.to_string());
    files.push(entry_rel);
    files.sort();
    files.dedup();
    let mut out = std::collections::BTreeMap::new();
    for file in files {
        let path = root.join(&file);
        let Ok(source) = std::fs::read_to_string(&path) else { continue };
        let Ok(parsed) = build(&source) else { continue };
        if !parsed.iter().any(|s| matches!(s, ast::Stmt::TestBlock { name: Some(_), .. })) {
            continue;
        }
        let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let stmts = expand_module_imports(&parsed, &dir, &mut std::collections::HashSet::new());
        let decls = checker::declarations(&stmts);
        let stem = file.trim_end_matches(".co").replace('/', "_");
        fn opens_app(stmts: &[ast::Stmt]) -> bool {
            let mut found = false;
            for st in stmts {
                let mut st = st.clone();
                visit_exprs_mut(&mut st, &mut |e| {
                    if matches!(e, ast::Expr::Call { callee, .. } if matches!(callee.as_ref(), ast::Expr::Ident(n) if n == "app")) {
                        found = true;
                    }
                });
            }
            found
        }
        for st in &stmts {
            let ast::Stmt::TestBlock { name: Some(name), params, body } = st else { continue };
            if !params.is_empty() {
                continue;
            }
            let recording = RECORD_FIXTURES.load(std::sync::atomic::Ordering::SeqCst);
            // a test that opens no app leaves no state and is no scenario; a
            // recording runs it all the same, for what its requests and
            // statements answer — `crow test --fixtures` replays every test
            let scenario = opens_app(body);
            if !scenario && !recording {
                continue;
            }
            let r = if recording { rehearsal::begin_recording(&root) } else { rehearsal::begin(&root) };
            crow_runtime::rng::set_seed(0);
            let (result, changed_facts, mailed) = ir_interp::run_scenario(&decls, &format!("test {}", name), body);
            crow_runtime::rng::clear_seed();
            rehearsal::end();
            if recording {
                match rehearsal::keep_fixtures(&r) {
                    Ok(kept) => {
                        for (sql, rows, answered) in kept {
                            let head: String = sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
                            if rows < answered {
                                println!("  recorded {}… — the first {} of {} rows", head, rows, answered);
                            } else {
                                println!("  recorded {}… — {} rows", head, rows);
                            }
                        }
                    }
                    Err(e) => eprintln!("  the fixtures of {} were not kept: {}", name, e),
                }
                for line in crow_runtime::fixtures::take_kept() {
                    println!("  recorded {}", line);
                }
            }
            if !scenario {
                continue;
            }
            let short = |l: &str| -> String { if l.chars().count() > 200 { format!("{}…", l.chars().take(200).collect::<String>()) } else { l.to_string() } };
            // the store reads a key back from the first line, up to ` (`
            let key = format!("scenario {}_{}", stem, name);
            let mut text = format!("{} ({})\n", key, file);
            match &result {
                Ok(()) => text.push_str("  ran to its end\n"),
                Err(e) => text.push_str(&format!("  stopped: {}\n", short(e.lines().next().unwrap_or("")))),
            }
            let section = |text: &mut String, title: &str, lines: &[String]| {
                if !lines.is_empty() {
                    text.push_str(&format!("  {}:\n", title));
                    for l in lines {
                        text.push_str(&format!("    {}\n", short(l)));
                    }
                }
            };
            section(&mut text, "wrote", &r.effects.lock().map(|e| e.clone()).unwrap_or_default());
            section(&mut text, "mailed", &mailed);
            section(&mut text, "answered with generated rows", &r.generated.lock().map(|g| g.clone()).unwrap_or_default());
            section(&mut text, "did not hold on rehearsal data", &r.unheld.lock().map(|u| u.clone()).unwrap_or_default());
            section(&mut text, "changed", &changed_facts);
            // (a stopped handler's notice holds a random id, which no two
            // rehearsals share)
            out.insert(key, (file.clone(), crow_web::core::response::ids_read_past(&text)));
        }
    }
    out
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
    let (mut artifacts, rules) = baseline::derive_artifacts(&lowered, &ctx, &parts, filename);
    wireframe::layout_artifacts(&mut artifacts, filename, read_file(filename).contains("wide = true"));
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

/// Source files end in `.co`, bytecode in `.coc`.
fn is_source(path: &str) -> bool { path.ends_with(".co") }
fn is_bytecode(path: &str) -> bool { path.ends_with(".coc") }
fn strip_source_ext(path: &str) -> &str { path.strip_suffix(".co").unwrap_or(path) }
fn bytecode_path(entry: &str) -> String { format!("{}.coc", strip_source_ext(entry)) }
/// The source a path names: a bytecode path's `.co` beside it; any other
/// path is the source already. (After the `.co` rename it appended `.co`
/// to a source too, and every interpreter trace named `main.co` where the
/// compiled program names `main`.)
fn source_of_bytecode(path: &str) -> String {
    match path.strip_suffix(".coc") {
        Some(base) => format!("{}.co", base),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod fix_tests {
    /// `crow fix`: the fixes a refusal names, applied round after round (the
    /// split line lets `totl` be read as `total`'s slip), the file left a
    /// program; one whose mistakes ask for the author is left refused, as it
    /// was
    #[test]
    fn crow_fix_mends_what_the_messages_name() {
        let dir = std::env::temp_dir().join(format!("crow_fix_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("a.co");
        std::fs::write(&file, "xs = ['a']\nif len[xs] == 1\n    log[lenn[xs]]\npush[xs, 'b']\ntotal = 0 n = 1\nlog[totl]\n").unwrap();
        let path = file.display().to_string();
        assert_eq!(super::fix_refusals(&path), None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "xs = ['a']\nif len[xs] eq 1\n    log[len[xs]]\nxs = push[xs, 'b']\ntotal = 0\nn = 1\nlog[total]\n");
        std::fs::write(&file, "log[missing]\n").unwrap();
        assert_eq!(super::fix_refusals(&path), Some(1));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "log[missing]\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;

    // `.crow` and `.crowc` were read for a release after the rename to
    // `.co`; they are no longer Crow files.
    #[test]
    fn source_and_bytecode_extensions() {
        assert!(is_source("a.co"));
        assert!(!is_source("a.coc"));
        assert!(!is_source("a.crow"));
        assert!(is_bytecode("a.coc"));
        assert!(!is_bytecode("a.co"));
        assert!(!is_bytecode("a.crowc"));
        assert_eq!(strip_source_ext("dir/a.co"), "dir/a");
        assert_eq!(strip_source_ext("dir/a"), "dir/a");
        assert_eq!(strip_source_ext("dir/a.crow"), "dir/a.crow");
        assert_eq!(bytecode_path("dir/a.co"), "dir/a.coc");
        assert_eq!(source_of_bytecode("dir/a.coc"), "dir/a.co");
        assert_eq!(source_of_bytecode("main.co"), "main.co");
    }
}

/// A program's IR, printed: the same front end as run, test and check,
/// then the lowering of docs/ir-design.md §4.
/// Every type inference found for a program, as `crow build --types`
/// prints it (`type_infer::dump_types`).
fn types_of_file(entry: &str, describe: bool) -> Result<String, String> {
    let source = read_file(entry);
    let stmts = build(&source).map_err(|e| e.to_string())?;
    let source_dir = std::path::Path::new(entry).parent().unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let stmts = lower::lower_program(&stmts)?;
    let mut ctx = infer_and_validate(&stmts, describe)?;
    ctx.record_envs = true;
    let name = std::path::Path::new(entry).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| entry.to_string());
    lower_ir::lower(&stmts, &mut ctx, &name)?;
    Ok(type_infer::dump_types(&ctx))
}

fn ir_of_file(entry: &str, describe: bool) -> Result<String, String> {
    program_of_file(entry, describe).map(|program| ir::print(&program))
}

/// A program's IR, lowered as every pipeline lowers it.
fn program_of_file(entry: &str, describe: bool) -> Result<ir::Program, String> {
    program_of_source(&read_file(entry), entry, describe)
}

/// A program's IR from its text, as if read from `entry`: the same pipeline
/// (`crow check --self` lowers thousands of cases without writing them).
pub(crate) fn program_of_source(source: &str, entry: &str, describe: bool) -> Result<ir::Program, String> {
    let stmts = build(source).map_err(|e| e.to_string())?;
    let source_dir = std::path::Path::new(entry).parent().unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    let stmts = lower::lower_program(&stmts)?;
    let name = std::path::Path::new(entry)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| entry.to_string());
    let (ctx, program) = infer_and_lower(&stmts, describe, &name)?;
    for note in &ctx.describe_notes {
        eprintln!("{}", note);
    }
    Ok(program)
}

#[cfg(test)]
mod ir_tests {
    /// The IR must exist for every program the modes are held to agree on,
    /// lowered as every pipeline lowers it: its queries' columns asked of
    /// its databases. (On a thread with room: the debug build's lowering
    /// frames are large, and the 43-entry map literal of parity 48 lowers
    /// as a chain that deep — it overflowed the test thread's 2 MB on
    /// 2026-10-08 while every release suite passed; the release binary's
    /// main thread has 8 MB, as `crow run` has.)
    #[test]
    fn every_parity_program_lowers_to_the_ir() {
        let lowered = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let mut failures = Vec::new();
                let mut count = 0;
                for entry in std::fs::read_dir("tests/parity").unwrap() {
                    let p = entry.unwrap().path();
                    if p.extension().map_or(false, |x| x == "co") {
                        count += 1;
                        if let Err(e) = super::ir_of_file(&p.to_string_lossy(), true) {
                            failures.push(format!("{}: {}", p.display(), e));
                        }
                    }
                }
                (count, failures)
            })
            .expect("a thread for the corpus")
            .join()
            .expect("the corpus lowers without a panic");
        let (count, failures) = lowered;
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
/// build`) optimizes everything. A dev build carries no debug info (macOS
/// keeps an object file per unit for it, and the harness caches had grown
/// by the hundred gigabytes); the `debuginfo` profile, dev as cargo defines
/// it, is built by `CROW_DEBUGINFO=1` into its own `target/debuginfo`, for a
/// Rust backtrace with file and line when a program panics.
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

/// The dev build (`crow run`): the program at `opt` — 0 unless `--opt`
/// says, so a large program rebuilds quickly after an edit (finch's crate:
/// 3 s at 0, 14 s at 1), while a program whose own loops are the work runs
/// many times faster at 1 (fannkuch: 12×) — and the runtime crates at 1 at
/// least, following `opt` above it, so a higher level reaches the kernels
/// too. The release build (`crow build`) is level 3 with LTO. The program
/// and Crow's own crates check every add for overflow in both; the crates
/// beneath them are built as their authors release them, unchecked (with
/// checks, a vectorizable loop of theirs runs one item at a time).
fn generated_cargo_toml(project_name: &str, deps: &str, opt: u8) -> String {
    let crates = opt.max(1);
    let mut checked = String::new();
    for krate in ["crow_runtime", "crow_web", "crow_db"] {
        if deps.contains(&format!("{} = ", krate)) {
            for profile in ["dev", "release"] {
                checked.push_str(&format!("\n[profile.{}.package.{}]\noverflow-checks = true\n", profile, krate));
            }
        }
    }
    format!(
        "[workspace]\n\n[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{}\n[profile.dev]\ndebug = 0\nopt-level = {}\n\n[profile.dev.package.\"*\"]\nopt-level = {}\noverflow-checks = false\n\n[profile.debuginfo]\ninherits = \"dev\"\ndebug = true\n\n[profile.debuginfo.package.\"*\"]\nopt-level = 1\n\n[profile.release]\nopt-level = 3\nlto = true\noverflow-checks = true\n\n[profile.release.package.\"*\"]\noverflow-checks = false\n{}",
        project_name, deps, opt, crates, checked
    )
}

/// Whether the gate asks the program's databases what its query literals
/// return. Every pipeline — run, run -i, test, check, the book, each part
/// alone — passes the same answer, so a program refused for its columns'
/// types is refused by all of them; `CROW_NO_DESCRIBE` turns it off.
pub(crate) fn describing() -> bool {
    std::env::var("CROW_NO_DESCRIBE").is_err()
}

/// The program inferred and lowered once (F5 S10): the lowering is the gate
/// — it refuses what is not a program, each error owned by the part it is
/// in — and its IR is the one the backends read, so a pipeline that runs or
/// builds a program lowers it once.
fn infer_and_lower(lowered: &[ast::Stmt], describe: bool, name: &str) -> Result<(type_infer::TypeContext, ir::Program), String> {
    let mut ctx = type_infer::TypeContext::new();
    ctx.describe_queries = describe;
    ctx.infer_program(lowered);
    match lower_ir::lower_refusing(lowered, &mut ctx, name) {
        Ok(program) => Ok((ctx, program)),
        // a message names a variable as the program wrote it
        Err(lower_ir::Refusal::Invalid(errors)) => Err(lower::shown(&attribute_to_parts(&errors))),
        Err(refusal) => Err(refusal.message()),
    }
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
    let stmts = build(&source).unwrap_or_else(|e| refuse(filename, &source, &e));
    // User modules are inlined exactly as the run and build paths do, so a
    // multi-file app is checked whole and a module prefix is a known name
    let source_dir = std::path::Path::new(filename)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = expand_module_imports(&stmts, source_dir, &mut visited);
    // Validated before it is checked: `crow check` refuses what `crow run`
    // refuses, with the same message; the lowering it refuses by is the
    // one each part's reach is read from
    let (lowered, lctx, program) = match lower::lower_program(&stmts) {
        Ok(lowered) => match infer_and_lower(&lowered, describing(), "check") {
            Ok((lctx, program)) => (lowered, lctx, program),
            Err(e) => refuse(filename, &source, &e),
        },
        Err(e) => refuse(filename, &source, &e),
    };
    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(&stmts);
    let mut result = checker::run_check(&stmts, &ctx, false);
    result.issues = checker::suppress_acknowledged(std::mem::take(&mut result.issues), &source);

    // The app baseline: what the program does, derived from the lowered
    // program and diffed against what was accepted (app-baseline-design.md)
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let (mut artifacts, rules) = baseline::derive_artifacts(&lowered, &lctx, &parts, filename);
    wireframe::layout_artifacts(&mut artifacts, filename, source.contains("wide = true"));
    // what each part can touch (second-half-design.md §1): a part that
    // starts writing files or calling a shell is a change to accept
    for (key, (file, text)) in baseline::reach_entries(&program, &parts, filename) {
        artifacts.files.insert(key.clone(), file);
        artifacts.entries.insert(key, text);
    }
    for (key, (file, text)) in scenario_artifacts(filename, &parts) {
        artifacts.files.insert(key.clone(), file);
        artifacts.entries.insert(key, text);
    }
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
    if let Some(art) = &result.artifacts {
        let reach: Vec<(&String, &String)> = art.entries.iter().filter(|(k, _)| k.starts_with("reach ")).collect();
        if !reach.is_empty() {
            println!("  reach — what each part can touch:");
            for (key, text) in reach {
                let caps: Vec<&str> = text.lines().skip(1).map(|l| l.trim().split(':').next().unwrap_or("")).collect();
                println!("    {}: {}", key.trim_start_matches("reach "), caps.join(", "));
            }
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

/// The module prefix of the part a declaration came from (the longest
/// that fits); none for the entry file's own. Inside that part's
/// functions its own facts and derivations carry the prefix, so a name
/// without it is never the part's own.
pub(crate) fn part_prefix_of_declaration(name: &str) -> Option<String> {
    EXPANSION_PARTS.with(|p| {
        p.borrow()
            .iter()
            .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, _)| prefix.clone())
    })
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

/// The flags `crow run` takes ahead of the file or job: `-i` (interpret)
/// and `--opt N` (the compiled program's optimization level, 0 to 3; 0
/// unless said). Answers whether to interpret, the level, and what
/// follows — the file or job, then the program's own arguments, untouched.
fn run_flags(args: &[String]) -> Result<(bool, u8, Vec<String>), String> {
    let mut interp = false;
    let mut opt: Option<u8> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => interp = true,
            "--opt" => {
                let level = args.get(i + 1).and_then(|v| v.parse::<u8>().ok()).filter(|n| *n <= 3);
                match level {
                    Some(n) => {
                        opt = Some(n);
                        i += 1;
                    }
                    None => {
                        return Err("crow run --opt takes an optimization level, 0 to 3 — `crow run --opt 2 file.co` (0 unless said; `crow build` makes the fully optimized binary)".to_string());
                    }
                }
            }
            _ => break,
        }
        i += 1;
    }
    if interp && opt.is_some() {
        return Err("crow run -i interprets the program and builds nothing: --opt belongs to the compiled run — `crow run --opt 2 file.co`".to_string());
    }
    Ok((interp, opt.unwrap_or(0), args[i..].to_vec()))
}

fn cmd_run(extra_args: &[String], opt: u8) {
    let config = parse_crowfile();
    let binary = transpile_and_compile(&config.entry, true, config.cpu, false, opt);
    run_binary(&binary, extra_args);
}

fn cmd_run_file(filename: &str, extra_args: &[String], opt: u8) {
    let binary = transpile_and_compile(filename, true, None, false, opt);
    run_binary(&binary, extra_args);
}

fn cmd_run_job(job_name: &str, opt: u8) {
    let config = parse_crowfile();
    let job_file = config.jobs.iter()
        .find(|(name, _)| name == job_name)
        .map(|(_, path)| path.clone());

    match job_file {
        Some(path) => {
            let binary = transpile_and_compile(&path, true, config.cpu, false, opt);
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
    // a dependency not installed is said before the program runs
    let root = project_root(&std::env::current_dir().unwrap_or_default());
    for (name, spec) in &config.deps {
        if !root.join("crow_modules").join(name).exists() {
            eprintln!("warning: '{}' ({}) is not installed — run crow install", name, spec);
        }
    }
    // the crowfile's cpu count bounds the parallel loops, as it does the
    // compiled program's (crow_runtime::cores); its gpu setting turns the
    // device on
    if let Some(n) = config.cpu {
        crow_runtime::cores::set_default(n);
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
        let stmts = build(&source).unwrap_or_else(|e| refuse(filename, &source, &e));
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
    let lowered = lower::lower_program(&stmts).unwrap_or_else(|e| refuse(filename, &source, &e));
    let serves = serve_line(&lowered).is_some();
    // then the one typed form (ir-design.md), walked by the interpreter
    let base = source_of_bytecode(filename);
    let name = std::path::Path::new(&base)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| base.clone());
    let (ctx, program) = infer_and_lower(&lowered, describing(), &name).unwrap_or_else(|e| refuse(filename, &source, &e));
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
    // a script's output in blocks when it goes to a file or a pipe, as the
    // compiled script's (crow_runtime::display::buffer_output)
    crow_runtime::display::buffer_output();
    let ran = ir_interp::run(&program, &ctx.registry, &base);
    crow_runtime::display::flush_output();
    if let Err(e) = ran {
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
    crowfile_text_at(dir, "db")
}

/// A text variable of the crowfile in `dir`, read leniently: no crowfile,
/// or no such variable, is `None`.
fn crowfile_text_at(dir: &std::path::Path, var: &str) -> Option<String> {
    let source = std::fs::read_to_string(dir.join("crowfile")).ok()?;
    let stmts = build(&source).ok()?;
    let it = ir_interp::run_script(&stmts, "crowfile").ok()?;
    match it.var(var) {
        Some(val) => match &val.value {
            value::Value::Str(s) => Some(s.clone()),
            _ => None,
        },
        None => None,
    }
}

/// The crowfile's `schema`: the mode a program opens the connections it
/// declares tables on in (spec, db, The schema a program declares).
fn crowfile_schema() -> Result<Option<crow_db::schema::Mode>, String> {
    match crowfile_text_at(std::path::Path::new("."), "schema") {
        Some(s) => crow_db::schema::Mode::parse(&s).map(Some).map_err(|e| format!("crowfile: schema = '{}': {}", s, e)),
        None => Ok(None),
    }
}

/// The crowfile's `schema`, for a command that runs or builds a program:
/// a mode that is not one stops the command.
fn crowfile_schema_or_stop() -> Option<crow_db::schema::Mode> {
    crowfile_schema().unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        process::exit(1);
    })
}

/// `crow db diff [file.co]`: for each connection the program declares
/// tables on and opens with a url it writes, the SQL that would make its
/// database hold what the program declares — what reconciling would run,
/// and, commented out, what it will not — a draft for a migration. Only
/// the catalog is read.
fn cmd_db(args: &[String]) {
    // `crow db describe <url> <sql>`: the columns the statement answers,
    // as JSON — what the compiler asks of a database whose driver is
    // written in Crow, from a process of its own (docs/db-in-crow-design.md)
    if args.first().map(|s| s.as_str()) == Some("describe") {
        let (Some(url), Some(sql)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: crow db describe <url> <sql>");
            process::exit(1);
        };
        // (the columns that can hold NULL marked as the lowering marks them:
        // the provider's own describe of a live connection)
        let answered = driver::tooling_ready().and_then(|_| crow_db::describe::of_url(url, sql));
        match answered {
            Ok(cols) => println!("{}", serde_json::Value::Array(cols.into_iter().map(|(n, k)| serde_json::json!([n, k])).collect())),
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
        return;
    }
    if args.first().map(|s| s.as_str()) != Some("diff") {
        eprintln!("usage: crow db diff [file.co]");
        process::exit(1);
    }
    if let Err(e) = driver::tooling_ready() {
        eprintln!("error: {}", e);
        process::exit(1);
    }
    let file = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| parse_crowfile().entry);
    let program = program_of_file(&file, false).unwrap_or_else(|e| refuse(&file, &read_file(&file), &e));
    let plan = lower_ir::schema::plan(&program);
    if plan.is_empty() {
        println!("-- {} declares no tables on its connections", file);
        return;
    }
    let mut failed = false;
    for (var, url, declared) in plan {
        let Some(url) = url else {
            println!("-- {}: opens with a url the program computes, not read\n", lower::shown(&var));
            continue;
        };
        let shown = crow_db::schema::shown_url(&url);
        let declared: Vec<&str> = declared.iter().map(|s| s.as_str()).collect();
        match crow_db::schema::diff(&url, &declared) {
            Ok(sql) if sql.is_empty() => println!("-- {} ({}): the database holds what the program declares\n", shown, lower::shown(&var)),
            Ok(sql) => println!("-- {} ({})\n{}", shown, lower::shown(&var), sql),
            Err(e) => {
                eprintln!("error: {} ({}): {}", shown, lower::shown(&var), e);
                failed = true;
            }
        }
    }
    if failed {
        process::exit(1);
    }
}

/// The dependencies a package's crowfile names, from its directory: none
/// when it has no crowfile. (A package's own dependencies install beside
/// it; packages.rs.)
pub(crate) fn crowfile_deps(dir: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    let path = dir.join("crowfile");
    let Ok(source) = std::fs::read_to_string(&path) else { return Ok(Vec::new()) };
    let stmts = build(&source).map_err(|e| format!("{}: {}", path.display(), e))?;
    let it = ir_interp::run_script(&stmts, "crowfile").map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(deps_in(&it))
}

/// `deps = map[models = 'github.com/…@v1', mine = '../mine']`, as a ran
/// crowfile holds it.
fn deps_in(it: &ir_interp::IrInterp) -> Vec<(String, String)> {
    it.var("deps")
        .map(|val| {
            if let value::Value::Map(map) = &val.value {
                map.iter()
                    .filter_map(|(name, v)| if let value::Value::Str(ver) = &v.value { Some((name.clone(), ver.clone())) } else { None })
                    .collect::<Vec<_>>()
            } else {
                vec![]
            }
        })
        .unwrap_or_default()
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
                None => "main.co".to_string(),
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
            let deps = deps_in(&it);

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
    match std::fs::read_to_string(filename) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading '{}': {}", filename, e);
            process::exit(1);
        }
    }
}

thread_local! {
    /// The comment written above each function, by the function's name as
    /// the lowering knows it (a part's under its prefix): what a tool's
    /// schema says the function does (spec, llm: Tools are the functions).
    static FN_DOCS: std::cell::RefCell<std::collections::BTreeMap<String, String>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// The comment lines directly above each `fn` of a source, noted under
/// the function's name with the prefix its file's functions carry: the
/// sentence the code already carries, which a tool's schema quotes.
pub(crate) fn note_fn_docs(prefix: &str, source: &str) {
    let lines: Vec<&str> = source.lines().collect();
    FN_DOCS.with(|d| {
        let mut docs = d.borrow_mut();
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.strip_prefix("fn ") else { continue };
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() {
                continue;
            }
            let mut above: Vec<String> = Vec::new();
            let mut k = i;
            while k > 0 {
                k -= 1;
                let Some(text) = lines[k].strip_prefix("---") else { break };
                above.push(text.trim().to_string());
            }
            above.reverse();
            let doc = above.join(" ").trim().to_string();
            docs.insert(format!("{}{}", prefix, name), doc);
        }
    });
}

/// The comment above a function, by its name; none where there is none.
pub(crate) fn fn_doc(name: &str) -> Option<String> {
    FN_DOCS.with(|d| d.borrow().get(name).filter(|s| !s.is_empty()).cloned())
}

pub(crate) fn build(source: &str) -> Result<Vec<ast::Stmt>, String> {
    note_fn_docs("", source);
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
/// What a program is refused for, as the front end says it (`crow run`'s
/// parse, expansion, lowering and checks, on the text given); `None` when
/// it is a program.
fn refusal_of(filename: &str, source: &str) -> Option<String> {
    let stmts = match build(source) {
        Ok(s) => s,
        Err(e) => return Some(e),
    };
    let dir = std::path::Path::new(filename).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let mut visited = std::collections::HashSet::new();
    let stmts = match expand_module_imports_checked(&stmts, dir, &mut visited) {
        Ok(s) => s,
        Err(e) => return Some(e),
    };
    let lowered = match lower::lower_program(&stmts) {
        Ok(l) => l,
        Err(e) => return Some(e),
    };
    let name = std::path::Path::new(filename).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    match infer_and_lower(&lowered, describing(), &name) {
        Ok(_) => None,
        Err(e) => Some(refine_error_lines(source, &e)),
    }
}

/// `crow fix file.co`: the fixes the program's refusal names, applied in
/// place — a slip's nearest name, a missing import, another language's
/// habit in its Crow form (diagnostic::fixes, as the editor's quick fixes
/// read them) — round after round, since a parse error hides the ones
/// after it; then what was changed, and what is left for the author.
/// `Some(exit status)` when the program is still refused; `None` when it is
/// a program, for the database check to run.
fn fix_refusals(filename: &str) -> Option<i32> {
    let original = read_file(filename);
    let mut source = original.clone();
    // each change at its line, kept where the lines a later change adds
    // above it move it
    let mut changed: Vec<(usize, String)> = Vec::new();
    for _ in 0..50 {
        let Some(text) = refusal_of(filename, &source) else { break };
        let (next, done) = diagnostic::apply(&source, &diagnostic::fixes(&source, &text));
        if done.is_empty() || next == source {
            break;
        }
        // the lines this round adds: an import inserted before its line,
        // a statement split after its line
        let adds: Vec<(usize, bool, usize)> = done
            .iter()
            .filter(|f| f.with.contains('\n'))
            .map(|f| (f.line, f.start == 0 && f.end == 0 && f.with.ends_with('\n'), f.with.matches('\n').count()))
            .collect();
        let moved = |line: usize, own: Option<&diagnostic::Fix>| -> usize {
            line + adds
                .iter()
                .filter(|(at, before, _)| if *before { *at <= line } else { *at < line })
                .filter(|(at, before, _)| own.is_none_or(|f| !(f.line == *at && *before && f.start == 0 && f.end == 0 && f.with.ends_with('\n'))))
                .map(|(_, _, n)| n)
                .sum::<usize>()
        };
        for c in changed.iter_mut() {
            c.0 = moved(c.0, None);
        }
        changed.extend(done.iter().map(|f| (moved(f.line, Some(f)), f.what.clone())));
        source = next;
    }
    changed.sort();
    if source != original {
        if let Err(e) = std::fs::write(filename, &source) {
            eprintln!("error: cannot write {}: {}", filename, e);
            return Some(1);
        }
        println!("crow fix: {} change{} to {}", changed.len(), if changed.len() == 1 { "" } else { "s" }, filename);
        for (line, what) in &changed {
            println!("  {}:{}: {}", filename, line, what);
        }
    }
    match refusal_of(filename, &source) {
        Some(text) => {
            if changed.is_empty() {
                println!("crow fix: nothing in {} it can fix — each mistake below asks for its author", filename);
            } else {
                println!("\nleft for you:");
            }
            eprintln!("{}", shown_refusal(filename, &source, &text));
            Some(1)
        }
        None => {
            if changed.is_empty() {
                println!("crow fix: {} is a program — nothing to fix", filename);
            } else {
                println!("{} is a program now", filename);
            }
            None
        }
    }
}

/// A refusal, printed the one way every command prints it — the file, line
/// and column of what the program wrote, the message, the line underlined
/// (diagnostic.rs; spec, Error Messages) — and the program not run.
fn refuse(entry: &str, source: &str, err: &str) -> ! {
    eprintln!("{}", shown_refusal(entry, source, err));
    process::exit(1);
}

/// A refusal's text as `refuse` prints it: its lines refined into the
/// statement, a part's excerpt read from the part's file.
fn shown_refusal(entry: &str, source: &str, err: &str) -> String {
    let refined = refine_error_lines(source, err);
    let dir = std::path::Path::new(entry).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let root = project_root(dir);
    let read = |part: &str| std::fs::read_to_string(root.join(part)).or_else(|_| std::fs::read_to_string(dir.join(part))).ok();
    diagnostic::render(entry, source, &refined, &read)
}

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
/// Whether a file may import `sys`, the standard library's own layer: the
/// modules the compiler ships (src/modules), and the harnesses' door
/// (`CROW_SYS=1`, for the tests and the book that show it). A program
/// reaches what `sys` builds — `request`, `socket`, the readers — never
/// `sys` itself (spec, The standard library's layer).
fn sys_allowed(dir: &std::path::Path) -> bool {
    if std::env::var_os("CROW_SYS").is_some() {
        return true;
    }
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let here = canon(dir);
    home::module_source_dirs().iter().any(|standard| here.starts_with(canon(standard)))
}

fn sys_refusal(line: usize) -> String {
    format!(
        "line {}: `sys` is the standard library's own layer — a program serves with serve[…], reads with request[…], read[…] and socket[…], and connects with connect[…]",
        line
    )
}

fn expand_module_imports(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    visited: &mut std::collections::HashSet<String>,
) -> Vec<ast::Stmt> {
    expand_module_imports_checked(stmts, source_dir, visited).unwrap_or_else(|e| {
        // a part's refusal names the part, whose text it is read from
        let root = project_root(source_dir);
        let read = |part: &str| std::fs::read_to_string(root.join(part)).or_else(|_| std::fs::read_to_string(source_dir.join(part))).ok();
        eprintln!("{}", diagnostic::render("", "", &e, &read));
        process::exit(1);
    })
}

/// The program with its imports inlined, or the first error a part it
/// imports has when validated alone — for a caller that goes on after it
/// (the language server, which had exited on it).
pub(crate) fn expand_module_imports_checked(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    visited: &mut std::collections::HashSet<String>,
) -> Result<Vec<ast::Stmt>, String> {
    // the entry's own imports and their lines: a part's, hoisted ahead of
    // the entry, carries no line of the entry's
    let use_key = |path: &str, alias: &Option<String>| format!("{}|{}", path, alias.clone().unwrap_or_default());
    let mut own_lines: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut line = 0usize;
    for st in stmts {
        match st {
            ast::Stmt::Line(n) => line = *n,
            ast::Stmt::Use { path, alias } => {
                if path == "sys" && !sys_allowed(source_dir) {
                    return Err(sys_refusal(line));
                }
                if path.starts_with("db.") && !sys_allowed(source_dir) {
                    return Err(format!("line {}: `{}` is the driver's own, which `use db` carries — a program connects with connect[…]", line, path));
                }
                own_lines.entry(use_key(path, alias)).or_insert(line);
            }
            _ => {}
        }
    }
    // what the program's databases answer is kept in its project
    SCHEMA_DIR.with(|d| *d.borrow_mut() = Some(project_root(source_dir).join(".crow").join("schema")));
    let mut ex = ModuleExpansion::default();
    let mut body = expand_into(stmts, source_dir, &mut ex, visited);
    if !ex.refused.is_empty() {
        return Err(ex.refused.join("\n"));
    }
    separate_part_locals(&mut body, &mut ex.hoisted);
    let hoisted = ex.hoisted.len();
    let mut all = ex.hoisted;
    all.extend(body);
    // every part says `use db` for itself; the program imports it once — at
    // the entry's line when the entry imports it too, so an import reserves
    // names by the line it is made on (a part's has none: it reserves
    // names in the part alone)
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<ast::Stmt> = Vec::with_capacity(all.len());
    for (i, st) in all.into_iter().enumerate() {
        if let ast::Stmt::Use { path, alias } = &st {
            let key = use_key(path, alias);
            if !seen.insert(key.clone()) {
                continue;
            }
            if i < hoisted {
                if let Some(l) = own_lines.get(&key).filter(|l| **l > 0) {
                    out.push(ast::Stmt::Line(*l));
                    out.push(st);
                    out.push(ast::Stmt::Line(0));
                    continue;
                }
            }
        }
        out.push(st);
    }
    // Each part is validated alone, as `crow test part.co` sees it — its
    // own names unprefixed, its imports inlined — so a name a part reserves
    // by what it imports, or a handler nothing in the part wires, is
    // refused whichever file is the entry
    for path in ex.done.keys() {
        validate_part_alone_from(path, ex.written.get(path).map(|d| d.as_path()))?;
    }
    // a part's function is typed from its tests' calls whichever file is
    // the entry, though the tests are the part's alone (set last, after
    // each part validated alone set its own)
    let tested: TestedCalls = ex
        .done
        .iter()
        .flat_map(|(path, names)| {
            let calls = PART_TEST_CALLS.with(|c| c.borrow().get(path).cloned().unwrap_or_default());
            calls.into_iter().map(|(f, types)| {
                let f = names.get(&f).cloned().unwrap_or(f);
                (f, types.iter().map(|t| spelled_as(t, names)).collect())
            })
        })
        .collect();
    TESTED_BY_PARTS.with(|t| *t.borrow_mut() = tested);
    // an entry that does not import ui keeps its top-level variables plain
    // however its parts' state is held: a script that imports a part runs
    // its top level in order (set last, after each part validated alone
    // set it for itself)
    let is_ui = |s: &ast::Stmt| matches!(s, ast::Stmt::Use { path, .. } if path == "ui" || path == "web.ui");
    let plain = !stmts.iter().any(is_ui) && out.iter().any(is_ui);
    PLAIN_ENTRY.with(|p| p.set(plain));
    // (set last, after each part validated alone set its own)
    UNRESOLVED_USES.with(|u| *u.borrow_mut() = ex.unresolved);
    // the standard modules only a part imports: the entry's own imports
    // are what its names resolve against
    let entry_has: std::collections::BTreeSet<String> = stmts
        .iter()
        .filter_map(|s| if let ast::Stmt::Use { path, .. } = s { Some(path) } else { None })
        .flat_map(|p| modules_of_import(p))
        .collect();
    let part_only: Vec<String> = out
        .iter()
        .filter_map(|s| if let ast::Stmt::Use { path, .. } = s { Some(path) } else { None })
        .filter(|p| lower_ir::is_standard_module(p) || p.starts_with("web."))
        .flat_map(|p| modules_of_import(p))
        .filter(|m| !entry_has.contains(m))
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    PART_ONLY_IMPORTS.with(|p| *p.borrow_mut() = part_only);
    Ok(out)
}

/// Where the program being built keeps what its databases answered: the
/// project's `.crow/schema`, once a file of it has been expanded.
pub(crate) fn schema_dir() -> Option<std::path::PathBuf> {
    SCHEMA_DIR.with(|d| d.borrow().clone())
}

/// What the tests of the entry expanded last's parts call the parts'
/// functions with (`TypeContext::type_tested`).
pub(crate) fn tested_by_parts() -> TestedCalls {
    TESTED_BY_PARTS.with(|t| t.borrow().clone())
}

/// A type a part alone gave, as a program importing the part spells it: the
/// part's own structs by their prefixed names.
fn spelled_as(t: &type_infer::Ty, names: &std::collections::BTreeMap<String, String>) -> type_infer::Ty {
    use type_infer::Ty as T;
    let named = |n: &String| names.get(n).cloned().unwrap_or_else(|| n.clone());
    let spelled = |t: &T| Box::new(spelled_as(t, names));
    match t {
        T::Record(n) => T::Record(named(n)),
        T::OptRecord(n) => T::OptRecord(named(n)),
        T::List(e) => T::List(spelled(e)),
        T::Map(k, e) => T::Map(*k, spelled(e)),
        T::Result(e) => T::Result(spelled(e)),
        T::Tensor(e, dims) => T::Tensor(spelled(e), dims.clone()),
        T::Fn(params, ret) => T::Fn(params.iter().map(|p| spelled_as(p, names)).collect(), spelled(ret)),
        other => other.clone(),
    }
}

/// A top-level name of an entry file that does not import ui, in a program
/// whose parts do: a plain variable of the entry's top level, not state.
pub(crate) fn plain_entry_name(name: &str) -> bool {
    PLAIN_ENTRY.with(|p| p.get()) && part_prefix_of_declaration(name).is_none()
}

/// Each part is validated alone, so nothing in a part's bodies can mean a
/// name the entry binds at its top level. A local, loop variable or
/// parameter that a part's function, view or derivation binds under such a
/// name is renamed with the part's prefix as a suffix (`c` in
/// `sim/session.co` becomes `c__sim_session`), so no later stage — the
/// types, the facts, the runtime's scopes — takes the one for the other.
/// A keyword argument naming a renamed parameter follows it.
fn separate_part_locals(entry: &mut [ast::Stmt], hoisted: &mut [ast::Stmt]) {
    use std::collections::{BTreeMap, BTreeSet};
    let mut top: BTreeSet<String> = BTreeSet::new();
    top_level_names(entry, &mut top);
    if top.is_empty() {
        return;
    }
    let mut params_renamed: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for st in hoisted.iter_mut() {
        let decl = match st {
            ast::Stmt::FnDecl { name, .. } | ast::Stmt::Derive { name, .. } | ast::Stmt::Assign { name, .. } => name.clone(),
            ast::Stmt::MultiAssign { names, .. } => names.first().cloned().unwrap_or_default(),
            _ => continue,
        };
        let Some(prefix) = part_prefix_of_declaration(&decl) else { continue };
        let mut bound: BTreeSet<String> = BTreeSet::new();
        match &*st {
            ast::Stmt::FnDecl { params, body, .. } => {
                bound.extend(params.iter().cloned());
                bound_names(body, &mut bound);
            }
            ast::Stmt::Derive { body, .. } => bound_names(body, &mut bound),
            ast::Stmt::Assign { value, .. } => bound_names_expr(value, &mut bound),
            ast::Stmt::MultiAssign { values, .. } => values.iter().for_each(|v| bound_names_expr(v, &mut bound)),
            _ => {}
        }
        let suffix = prefix.trim_end_matches('_').to_string();
        let map: BTreeMap<String, String> = bound
            .into_iter()
            .filter(|n| top.contains(n))
            .map(|n| {
                let new = format!("{}__{}", n, suffix);
                (n, new)
            })
            .collect();
        if map.is_empty() {
            continue;
        }
        match st {
            ast::Stmt::FnDecl { name, params, body, .. } => {
                let mut renamed = BTreeMap::new();
                for p in params.iter_mut() {
                    if let Some(new) = map.get(p) {
                        renamed.insert(p.clone(), new.clone());
                        *p = new.clone();
                    }
                }
                body.iter_mut().for_each(|s| rename_locals(s, &map));
                if !renamed.is_empty() {
                    params_renamed.insert(name.clone(), renamed);
                }
            }
            ast::Stmt::Derive { body, .. } => body.iter_mut().for_each(|s| rename_locals(s, &map)),
            ast::Stmt::Assign { value, .. } => rename_locals_expr(value, &map),
            ast::Stmt::MultiAssign { values, .. } => values.iter_mut().for_each(|v| rename_locals_expr(v, &map)),
            _ => {}
        }
    }
    if params_renamed.is_empty() {
        return;
    }
    for st in entry.iter_mut().chain(hoisted.iter_mut()) {
        visit_exprs_mut(st, &mut |e| {
            if let ast::Expr::Call { callee, args } = e {
                if let ast::Expr::Ident(f) = callee.as_ref() {
                    if let Some(renamed) = params_renamed.get(f) {
                        for a in args.iter_mut() {
                            if let ast::Expr::Named { name, .. } = a {
                                if let Some(new) = renamed.get(name) {
                                    *name = new.clone();
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

/// The names a file binds at its top level: its variables (in the top
/// level's loops and branches too), functions, structs and derivations.
fn top_level_names(stmts: &[ast::Stmt], out: &mut std::collections::BTreeSet<String>) {
    for st in stmts {
        match st {
            ast::Stmt::Assign { name, .. }
            | ast::Stmt::FnDecl { name, .. }
            | ast::Stmt::StructDecl { name, .. }
            | ast::Stmt::Derive { name, .. } => {
                out.insert(name.clone());
            }
            ast::Stmt::MultiAssign { names, .. } => out.extend(names.iter().cloned()),
            ast::Stmt::For { var, body, .. } => {
                out.insert(var.clone());
                top_level_names(body, out);
            }
            ast::Stmt::While { body, .. } => top_level_names(body, out),
            ast::Stmt::If { then_body, else_body, .. } => {
                top_level_names(then_body, out);
                if let Some(e) = else_body {
                    top_level_names(e, out);
                }
            }
            _ => {}
        }
    }
}

/// The names a body binds: what it assigns and the variables of its loops.
fn bound_names(stmts: &[ast::Stmt], out: &mut std::collections::BTreeSet<String>) {
    for st in stmts {
        match st {
            ast::Stmt::Assign { name, value } => {
                out.insert(name.clone());
                bound_names_expr(value, out);
            }
            ast::Stmt::MultiAssign { names, values } => {
                out.extend(names.iter().cloned());
                values.iter().for_each(|v| bound_names_expr(v, out));
            }
            ast::Stmt::For { var, iter, body } => {
                out.insert(var.clone());
                bound_names_expr(iter, out);
                bound_names(body, out);
            }
            ast::Stmt::While { condition, body } => {
                bound_names_expr(condition, out);
                bound_names(body, out);
            }
            ast::Stmt::If { condition, then_body, else_body } => {
                bound_names_expr(condition, out);
                bound_names(then_body, out);
                if let Some(e) = else_body {
                    bound_names(e, out);
                }
            }
            ast::Stmt::Return(Some(e)) | ast::Stmt::ExprStmt(e) => bound_names_expr(e, out),
            _ => {}
        }
    }
}

/// The variables of the `for` expressions inside an expression, and what
/// their bodies assign.
fn bound_names_expr(e: &ast::Expr, out: &mut std::collections::BTreeSet<String>) {
    match e {
        ast::Expr::ForExpr { var, iter, body } => {
            out.insert(var.clone());
            bound_names_expr(iter, out);
            bound_names(body, out);
        }
        ast::Expr::BinOp { left, right, .. } => {
            bound_names_expr(left, out);
            bound_names_expr(right, out);
        }
        ast::Expr::UnaryOp { expr, .. } => bound_names_expr(expr, out),
        ast::Expr::Call { callee, args } => {
            bound_names_expr(callee, out);
            args.iter().for_each(|a| bound_names_expr(a, out));
        }
        ast::Expr::FieldAccess { object, .. } => bound_names_expr(object, out),
        ast::Expr::Array(items) => items.iter().for_each(|i| bound_names_expr(i, out)),
        ast::Expr::Map(pairs) => pairs.iter().for_each(|(_, v)| bound_names_expr(v, out)),
        ast::Expr::Named { value, .. } => bound_names_expr(value, out),
        _ => {}
    }
}

/// Rename a body's bindings and every read of them, strings' spans
/// included; keyword names, fields and map keys are not variables.
fn rename_locals(st: &mut ast::Stmt, map: &std::collections::BTreeMap<String, String>) {
    let to = |n: &mut String| {
        if let Some(new) = map.get(n.as_str()) {
            *n = new.clone();
        }
    };
    match st {
        ast::Stmt::Assign { name, value } => {
            rename_locals_expr(value, map);
            to(name);
        }
        ast::Stmt::MultiAssign { names, values } => {
            values.iter_mut().for_each(|v| rename_locals_expr(v, map));
            names.iter_mut().for_each(to);
        }
        ast::Stmt::Return(Some(e)) | ast::Stmt::ExprStmt(e) => rename_locals_expr(e, map),
        ast::Stmt::If { condition, then_body, else_body } => {
            rename_locals_expr(condition, map);
            then_body.iter_mut().for_each(|s| rename_locals(s, map));
            if let Some(eb) = else_body {
                eb.iter_mut().for_each(|s| rename_locals(s, map));
            }
        }
        ast::Stmt::For { var, iter, body } => {
            rename_locals_expr(iter, map);
            to(var);
            body.iter_mut().for_each(|s| rename_locals(s, map));
        }
        ast::Stmt::While { condition, body } => {
            rename_locals_expr(condition, map);
            body.iter_mut().for_each(|s| rename_locals(s, map));
        }
        ast::Stmt::FieldAssign { object, value, .. } => {
            rename_locals_expr(object, map);
            rename_locals_expr(value, map);
        }
        ast::Stmt::IndexAssign { object, index, value } => {
            rename_locals_expr(object, map);
            rename_locals_expr(index, map);
            rename_locals_expr(value, map);
        }
        _ => {}
    }
}

fn rename_locals_expr(e: &mut ast::Expr, map: &std::collections::BTreeMap<String, String>) {
    match e {
        ast::Expr::Ident(n) => {
            if let Some(new) = map.get(n.as_str()) {
                *n = new.clone();
            }
        }
        // a string's spans read the names as the body does
        ast::Expr::Str(_) => rewrite_refs_expr(e, map, &std::collections::HashSet::new()),
        ast::Expr::BinOp { left, right, .. } => {
            rename_locals_expr(left, map);
            rename_locals_expr(right, map);
        }
        ast::Expr::UnaryOp { expr, .. } => rename_locals_expr(expr, map),
        ast::Expr::Call { callee, args } => {
            rename_locals_expr(callee, map);
            args.iter_mut().for_each(|a| rename_locals_expr(a, map));
        }
        ast::Expr::FieldAccess { object, .. } => rename_locals_expr(object, map),
        ast::Expr::Array(items) => items.iter_mut().for_each(|i| rename_locals_expr(i, map)),
        ast::Expr::Map(pairs) => pairs.iter_mut().for_each(|(_, v)| rename_locals_expr(v, map)),
        ast::Expr::Named { value, .. } => rename_locals_expr(value, map),
        ast::Expr::ForExpr { var, iter, body } => {
            rename_locals_expr(iter, map);
            if let Some(new) = map.get(var.as_str()) {
                *var = new.clone();
            }
            body.iter_mut().for_each(|s| rename_locals(s, map));
        }
        _ => {}
    }
}

/// Every expression in a statement, bodies and tests included, children
/// before their parent.
fn visit_exprs_mut(st: &mut ast::Stmt, f: &mut dyn FnMut(&mut ast::Expr)) {
    fn expr(e: &mut ast::Expr, f: &mut dyn FnMut(&mut ast::Expr)) {
        match e {
            ast::Expr::BinOp { left, right, .. } => {
                expr(left, f);
                expr(right, f);
            }
            ast::Expr::UnaryOp { expr: inner, .. } => expr(inner, f),
            ast::Expr::Call { callee, args } => {
                expr(callee, f);
                args.iter_mut().for_each(|a| expr(a, f));
            }
            ast::Expr::FieldAccess { object, .. } => expr(object, f),
            ast::Expr::Array(items) => items.iter_mut().for_each(|i| expr(i, f)),
            ast::Expr::Map(pairs) => pairs.iter_mut().for_each(|(k, v)| {
                expr(k, f);
                expr(v, f);
            }),
            ast::Expr::Named { value, .. } => expr(value, f),
            ast::Expr::ForExpr { iter, body, .. } => {
                expr(iter, f);
                body.iter_mut().for_each(|s| visit_exprs_mut(s, f));
            }
            _ => {}
        }
        f(e);
    }
    match st {
        ast::Stmt::Assign { value, .. } | ast::Stmt::ExprStmt(value) | ast::Stmt::Return(Some(value)) => expr(value, f),
        ast::Stmt::MultiAssign { values, .. } => values.iter_mut().for_each(|v| expr(v, f)),
        ast::Stmt::FnDecl { body, .. } | ast::Stmt::Derive { body, .. } | ast::Stmt::TestBlock { body, .. } => {
            body.iter_mut().for_each(|s| visit_exprs_mut(s, f))
        }
        ast::Stmt::If { condition, then_body, else_body } => {
            expr(condition, f);
            then_body.iter_mut().for_each(|s| visit_exprs_mut(s, f));
            if let Some(eb) = else_body {
                eb.iter_mut().for_each(|s| visit_exprs_mut(s, f));
            }
        }
        ast::Stmt::For { iter, body, .. } => {
            expr(iter, f);
            body.iter_mut().for_each(|s| visit_exprs_mut(s, f));
        }
        ast::Stmt::While { condition, body } => {
            expr(condition, f);
            body.iter_mut().for_each(|s| visit_exprs_mut(s, f));
        }
        ast::Stmt::FieldAssign { object, value, .. } => {
            expr(object, f);
            expr(value, f);
        }
        ast::Stmt::IndexAssign { object, index, value } => {
            expr(object, f);
            expr(index, f);
            expr(value, f);
        }
        _ => {}
    }
}

thread_local! {
    /// The entry being expanded imports no ui while a part does
    static PLAIN_ENTRY: std::cell::Cell<bool> = std::cell::Cell::new(false);
    /// The project's record of what its databases answered (`.crow/schema`)
    static SCHEMA_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = std::cell::RefCell::new(None);
    /// Each part validated alone, by its file: the fingerprint of its text
    /// and its imports' when it was, and what the validation found. A part
    /// is validated again only once one of them changes (a language
    /// server's parts change under it; a run's never do).
    static VALIDATED_PARTS: std::cell::RefCell<PartValidations> = std::cell::RefCell::new(std::collections::HashMap::new());
    /// module prefix (`sim_session_`) → the part's project-relative file
    static EXPANSION_PARTS: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());
    /// module prefix → the part's file on disk, read back to refine an
    /// error's line against the part's own text
    static EXPANSION_FILES: std::cell::RefCell<std::collections::BTreeMap<String, std::path::PathBuf>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());
    /// part file → what its tests call its functions with, as the part
    /// alone typed them (its own names unprefixed)
    static PART_TEST_CALLS: std::cell::RefCell<std::collections::BTreeMap<std::path::PathBuf, TestedCalls>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// The same for every part of the entry expanded last, spelled as the
    /// entry spells each part
    static TESTED_BY_PARTS: std::cell::RefCell<TestedCalls> = const { std::cell::RefCell::new(Vec::new()) };
    /// a module's record, prefixed (`sessions_Day`) → the name it was
    /// declared by (`Day`), which a value of it shows
    static DECLARED_RECORDS: std::cell::RefCell<std::collections::BTreeMap<String, String>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// The name a record was declared by: a module's `Day`, not the
/// `sessions_Day` its expansion made of it.
pub(crate) fn declared_record_name(name: &str) -> String {
    DECLARED_RECORDS.with(|d| d.borrow().get(name).cloned()).unwrap_or_else(|| name.to_string())
}

/// A function's name and the types a test calls it with, call by call.
type TestedCalls = Vec<(String, Vec<type_infer::Ty>)>;

/// Each part validated alone: its fingerprint then, and what it found.
type PartValidations = std::collections::HashMap<std::path::PathBuf, (u64, Result<(), String>)>;

/// A part's fingerprint: its text and the texts of the parts it imports,
/// transitively — what its validation depends on.
fn part_fingerprint(path: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    fn walk(path: &std::path::Path, root: &std::path::Path, seen: &mut std::collections::BTreeSet<std::path::PathBuf>, h: &mut std::collections::hash_map::DefaultHasher) {
        if !seen.insert(path.to_path_buf()) {
            return;
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        path.hash(h);
        text.hash(h);
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("use ") else { continue };
            for used in rest.split(',') {
                let name = used.split_whitespace().next().unwrap_or("");
                let file = root.join(format!("{}.co", name.replace('.', "/")));
                if file.exists() {
                    walk(&file, root, seen, h);
                }
            }
        }
    }
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let root = project_root(dir);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    walk(path, &root, &mut std::collections::BTreeSet::new(), &mut h);
    h.finish()
}

/// Validate one part as the entry of its own program: parsed, its imports
/// inlined, lowered and passed through the same gate as the whole app.
/// A part is validated once per fingerprint (`VALIDATED_PARTS`), and
/// reports under its project-relative path.
#[cfg(test)]
fn validate_part_alone(path: &std::path::Path) -> Result<(), String> {
    validate_part_alone_from(path, None)
}

/// A part validated alone, its imports read from `written` — its directory
/// as its importer reached it, which says a linked package is one — or
/// from where it lies.
fn validate_part_alone_from(path: &std::path::Path, written: Option<&std::path::Path>) -> Result<(), String> {
    let print = part_fingerprint(path);
    if let Some((seen, found)) = VALIDATED_PARTS.with(|v| v.borrow().get(path).cloned())
        && seen == print
    {
        return found;
    }
    // (an import that reaches back to this part while it is validated
    // finds it good, as it had)
    VALIDATED_PARTS.with(|v| v.borrow_mut().insert(path.to_path_buf(), (print, Ok(()))));
    let found = validate_part_alone_now(path, written);
    VALIDATED_PARTS.with(|v| v.borrow_mut().insert(path.to_path_buf(), (print, found.clone())));
    found
}

fn validate_part_alone_now(path: &std::path::Path, written: Option<&std::path::Path>) -> Result<(), String> {
    let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    // (its imports read from where its importer reached it)
    let import_dir = written.map(|d| d.to_path_buf()).unwrap_or_else(|| dir.clone());
    let root = project_root(&dir);
    let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.to_path_buf());
    let rel = path
        .strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", rel, e))?;
    let stmts = build(&source).map_err(|e| format!("{}: {}", rel, e))?;
    let mut visited = std::collections::HashSet::new();
    let expanded = expand_module_imports_checked(&stmts, &import_dir, &mut visited)?;
    let lowered = lower::lower_program(&expanded).map_err(|e| format!("{}: error: {}", rel, e))?;
    let parts = EXPANSION_PARTS.with(|p| p.borrow().clone());
    let ctx = infer_and_validate(&lowered, describing()).map_err(|e| {
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
    })?;
    // what its tests call its functions with, for the programs importing it
    PART_TEST_CALLS.with(|c| c.borrow_mut().insert(path.to_path_buf(), ctx.tested));
    Ok(())
}

#[derive(Default)]
struct ModuleExpansion {
    /// modules already expanded, by file: the local name → prefixed name map
    done: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeMap<String, String>>,
    /// each module's directory as its importer's path spelled it, by file: a
    /// linked package lies elsewhere, and only this path says it is one
    written: std::collections::BTreeMap<std::path::PathBuf, std::path::PathBuf>,
    /// every module's prefixed declarations, in dependency order
    hoisted: Vec<ast::Stmt>,
    /// how deep in the parts the expansion is: the entry is 0
    depth: usize,
    /// the entry's imports of a module there is no file and no standard
    /// module for, with the line of each
    unresolved: Vec<(usize, String)>,
    /// the parts that do not lex or parse, each with its own refusal (a
    /// part dropped silently had left its names undefined in the entry:
    /// "undefined variable 'sim'" for a `contains[…]` in views/sim.co)
    refused: Vec<String>,
}

thread_local! {
    /// The entry's imports the expansion found nothing for: the checks
    /// refuse each where the program's other refusals are made (spec,
    /// Error Messages). Set when a program is expanded.
    pub(crate) static UNRESOLVED_USES: std::cell::RefCell<Vec<(usize, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The standard modules the entry's parts import and the entry does
    /// not: each file's imports are its own (spec, Modules), so a name the
    /// entry reaches only through a part's import is refused for the
    /// import, as it would be with no import at all. Set when a program is
    /// expanded (the program still links every module its parts import).
    pub(crate) static PART_ONLY_IMPORTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The standard modules an import makes names from: its own, and `math`
/// for `use math.stats`, `web` for `use web.auth`.
fn modules_of_import(path: &str) -> Vec<String> {
    let mut out = vec![path.to_string()];
    if path.starts_with("math.") {
        out.push("math".to_string());
    }
    if path.starts_with("web.") {
        out.push("web".to_string());
    }
    out
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
    let mut line = 0usize;
    for stmt in stmts {
        if let ast::Stmt::Line(n) = stmt {
            line = *n;
        }
        let ast::Stmt::Use { path, alias } = stmt else {
            expanded.push(stmt.clone());
            continue;
        };
        // Built-in modules: skip file lookup (the standard library's own
        // sources, src/modules/*.co, are inlined by the lowering, never
        // read as a module of the program's — a file beside them would be;
        // a `web.` module the web has none of is the checks' to refuse)
        if lower_ir::is_standard_module(path) || path.starts_with("web.") {
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
        // a package reaches the packages installed beside it — its own
        // dependencies, which `crow install` puts in the same crow_modules/
        // (read from the directory as its path spells it: a linked
        // package's root is where it really lies)
        let beside = package_dir(source_dir).and_then(|p| p.parent().map(|m| m.to_path_buf()));
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        candidates.push(root.join(format!("{}.co", file_rel)));
        candidates.push(root.join(format!("crow_modules/{}.co", file_rel)));
        candidates.push(root.join(format!("crow_modules/{}/main.co", file_rel)));
        if let Some(modules) = &beside {
            candidates.push(modules.join(format!("{}.co", file_rel)));
            candidates.push(modules.join(format!("{}/main.co", file_rel)));
        }
        if root != source_dir {
            candidates.push(source_dir.join(format!("{}.co", file_rel)));
        }
        let Some(file_path) = candidates.iter().find(|p| p.exists()).cloned() else {
            // no file and no standard module: the entry's import is refused
            // with its line (a part's is the part's, validated alone)
            if ex.depth == 0 {
                ex.unresolved.push((line, path.clone()));
            }
            expanded.push(stmt.clone());
            continue;
        };
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
        // Parse the module: one that does not is the program's refusal,
        // named by the part's file
        // (a package linked into crow_modules lies elsewhere: it is named
        // by the path the program reaches it through)
        let shown_part = std::fs::canonicalize(&file_path)
            .ok()
            .zip(std::fs::canonicalize(&root).ok())
            .and_then(|(c, r)| file_path.strip_prefix(&r).or_else(|_| c.strip_prefix(&r)).ok().map(|p| p.display().to_string()))
            .unwrap_or_else(|| file_path.display().to_string());
        let mut lexer = Lexer::new(&source);
        let tokens = match lexer.tokenize() {
            Ok(t) => t,
            Err(e) => {
                ex.refused.push(format!("{}: {}", shown_part, e));
                expanded.push(stmt.clone());
                continue;
            }
        };
        let mut parser = Parser::new(tokens);
        let mod_stmts = match parser.parse() {
            Ok(s) => s,
            Err(e) => {
                ex.refused.push(format!("{}: {}", shown_part, e));
                expanded.push(stmt.clone());
                continue;
            }
        };
        // The module's own imports are hoisted first and its references to
        // them rewritten
        let mod_dir = file_path.parent().unwrap_or(source_dir);
        // (a module of the program's may not open the standard library's
        // layer either: the door is the compiler's own modules')
        // (refused as a part's refusals are, never by leaving the process:
        // a language server or a test lowering such a part had exited)
        if !sys_allowed(mod_dir) {
            let mut at = 0usize;
            let mut opens = false;
            for st in &mod_stmts {
                match st {
                    ast::Stmt::Line(n) => at = *n,
                    ast::Stmt::Use { path, .. } if path == "sys" => {
                        ex.refused.push(format!("{}: {}", file_path.display(), sys_refusal(at)));
                        opens = true;
                        break;
                    }
                    _ => {}
                }
            }
            if opens {
                continue;
            }
        }
        ex.depth += 1;
        let mod_stmts = expand_into(&mod_stmts, mod_dir, ex, in_progress);
        ex.depth -= 1;
        in_progress.remove(&key);

        // The prefix is the module's path: unique per file, the same from
        // every importer. A package's own import is spelled inside the
        // package, so its path there carries the package's name: `use b` in
        // package `pk` is `pk.b` to the program, never the program's `b`
        // (read from the path as spelled, before a link is followed: a
        // package linked into crow_modules is a package all the same)
        let package = package_dir(source_dir).or_else(|| package_dir(&root)).and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
        let prefix = match &package {
            Some(pk) => format!("{}_{}", pk, file_rel.replace('/', "_")),
            None => file_rel.replace('/', "_"),
        };
        note_fn_docs(&format!("{}_", prefix), &source);
        // the baseline attributes each declaration to its part's file
        EXPANSION_PARTS.with(|p| {
            let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            // (a linked package's file lies outside the root: the path the
            // program reaches it through, crow_modules/…, names it — the
            // baseline's keys had held the machine's absolute path)
            let rel = file_path
                .strip_prefix(&root)
                .or_else(|_| canon.strip_prefix(&root))
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| file_path.display().to_string());
            let rel = match &package {
                Some(pk) => format!("crow_modules/{}/{}", pk, rel),
                None => rel,
            };
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
                    DECLARED_RECORDS.with(|d| d.borrow_mut().insert(format!("{}_{}", prefix, name), name.clone()));
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
        ex.written.insert(canon.clone(), mod_dir.to_path_buf());
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
pub(crate) fn rewrite_refs_stmt(
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
            // `change[a, 'session.day', v]` in a test names a part's fact
            // as the code does: the harness sets it by its prefixed name
            if matches!(callee.as_ref(), ast::Expr::Ident(n) if n == "change") {
                if let Some(ast::Expr::Str(s)) = args.get_mut(1) {
                    let field = s.strip_prefix(module_name).and_then(|rest| rest.strip_prefix('.')).map(str::to_string);
                    if let Some(prefixed) = field.and_then(|f| local_names.get(&f)) {
                        *s = prefixed.clone();
                    }
                }
            }
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
pub(crate) fn project_root(source_dir: &std::path::Path) -> std::path::PathBuf {
    let start = if source_dir.as_os_str().is_empty() { std::path::Path::new(".") } else { source_dir };
    // an installed package is rooted at itself: its files reach each other
    // by their paths inside it, whatever the program beside it holds
    if let Some(package) = package_dir(start) {
        return std::fs::canonicalize(&package).unwrap_or(package);
    }
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

/// The installed package a directory lies in — `crow_modules/<name>`, as
/// the path spells it (a linked package's own path does not say it) — or
/// none.
fn package_dir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().ok()?.join(dir) };
    dir.ancestors()
        .find(|a| a.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "crow_modules"))
        .map(|a| a.to_path_buf())
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
mod docs_check_tests {
    use super::*;

    /// A reference row names a function of its own section's table: math's
    /// `rand[d1, d2, …]` had passed as core's `rand`, an int, checked
    /// against every table at once.
    #[test]
    fn a_reference_row_names_a_function_of_its_own_section() {
        let tables: Vec<(&str, Vec<&str>)> = vec![
            ("core", vec!["json", "len", "rand"]),
            ("web", vec!["serve"]),
            ("math", vec!["randn", "uniform"]),
            ("math.stats", vec!["mean"]),
        ];
        let spec = [
            "## 11. Core Builtins",
            "| `rand[n]` | an int |",
            "| `len[x]` | a length |",
            "## 13. Standard Library",
            "### math",
            "#### Random",
            "| `rand[d1, d2]` | a tensor |",
            "| `uniform[low, high, d1]` | a tensor |",
            "#### Statistics (sub-module: `math.stats`)",
            "| `mean[x]` | the mean |",
            "| `uniform[low, high]` | math's, not stats' |",
            "### web",
            "| `json[map]` | a response: web's own form of the name |",
            "| `serve[port]` | serves |",
            "| `ghost[x]` | nothing |",
            "## 14. Parallelism",
            "| `ghost[x]` | not a reference section |",
        ]
        .join("\n");
        assert_eq!(
            phantom_rows(&spec, &tables),
            vec![
                "line 7: `rand[` is documented under math but is core's".to_string(),
                "line 11: `uniform[` is documented under math.stats but is math's".to_string(),
                "line 15: `ghost[` is documented but the compiler has no such builtin".to_string(),
            ]
        );
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
        let toml = generated_cargo_toml("main", "[dependencies]\n", 0);
        assert!(toml.contains("[profile.dev.package.\"*\"]\nopt-level = 1"), "{}", toml);
        assert!(toml.contains("[profile.dev]\ndebug = 0\nopt-level = 0\n"), "a dev build carries no debug info and leaves the program at 0: {}", toml);
        // `--opt 1` optimizes the program; `--opt 3` lifts the runtime crates with it
        assert!(generated_cargo_toml("main", "", 1).contains("[profile.dev]\ndebug = 0\nopt-level = 1\n\n[profile.dev.package.\"*\"]\nopt-level = 1\n"));
        assert!(generated_cargo_toml("main", "", 3).contains("[profile.dev]\ndebug = 0\nopt-level = 3\n\n[profile.dev.package.\"*\"]\nopt-level = 3\n"));
        assert!(toml.contains("[profile.debuginfo]\ninherits = \"dev\"\ndebug = true\n\n[profile.debuginfo.package.\"*\"]\nopt-level = 1\n"), "CROW_DEBUGINFO=1 builds dev with debug info: {}", toml);
        assert!(toml.contains("[profile.release]\nopt-level = 3\nlto = true\noverflow-checks = true"), "{}", toml);
        // the crates beneath Crow's are built unchecked, Crow's own checked
        assert!(toml.contains("[profile.release.package.\"*\"]\noverflow-checks = false"), "{}", toml);
        assert!(!toml.contains("crow_runtime]"), "no runtime among the dependencies, none named: {}", toml);
        let with_runtime = generated_cargo_toml("main", "[dependencies]\ncrow_runtime = { path = \"/x\" }\n", 0);
        assert!(with_runtime.contains("[profile.release.package.crow_runtime]\noverflow-checks = true") && with_runtime.contains("[profile.dev.package.crow_runtime]\noverflow-checks = true"), "{}", with_runtime);
        assert!(toml.starts_with("[workspace]\n\n[package]\nname = \"main\""), "{}", toml);
    }

    #[test]
    fn run_takes_its_flags_ahead_of_the_file() {
        let a = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(run_flags(&a("file.co x y")).unwrap(), (false, 0, a("file.co x y")));
        assert_eq!(run_flags(&a("--opt 3 file.co")).unwrap(), (false, 3, a("file.co")));
        assert_eq!(run_flags(&a("-i file.co")).unwrap(), (true, 0, a("file.co")));
        assert_eq!(run_flags(&a("--opt 2")).unwrap(), (false, 2, Vec::<String>::new()));
        assert_eq!(run_flags(&[]).unwrap(), (false, 0, Vec::<String>::new()));
        assert!(run_flags(&a("--opt 4 file.co")).unwrap_err().contains("0 to 3"));
        assert!(run_flags(&a("--opt file.co")).unwrap_err().contains("0 to 3"));
        assert!(run_flags(&a("-i --opt 2 file.co")).unwrap_err().contains("interprets"));
        // the program's own arguments follow the file, untouched
        assert_eq!(run_flags(&a("file.co --opt 2")).unwrap(), (false, 0, a("file.co --opt 2")));
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
        assert!(err.contains("'cfg' is a derivation, which a function does not see"), "{}", err);
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
        let app = "use web, ui\nn = 1\nview render[]\n    return p['`n`']\n\nserve[3100, refresh = 1, keep = true]\npage['/', render, url = ['n']]\npage['/n/:n', render]\n";
        let lowered = lower::lower_program(&build(app).unwrap()).unwrap();
        assert_eq!(serve_line(&lowered), Some(6));
        let script = lower::lower_program(&build("x = 1\nlog[x]\n").unwrap()).unwrap();
        assert_eq!(serve_line(&script), None);
        // main runs and records the page and the serve it asks for
        let (program, ctx) = ir_interp::prepare(&build(app).unwrap(), "app").unwrap();
        let it = ir_interp::run_to_serve(&program, &ctx.registry, "app.co").unwrap();
        // every page, in order (the last alone had been served)
        assert_eq!(it.pages, vec![
            ("/".to_string(), "render".to_string(), vec!["n".to_string()]),
            ("/n/:n".to_string(), "render".to_string(), vec![]),
        ]);
        let serve = it.serve_config.clone().expect("serve recorded");
        assert_eq!(serve.len(), 11, "the lowered serve: port, refresh, budget, theme, density, accent, wide, keep, uploads, cors, files");
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
    /// Each file's imports are its own: the entry reaches a module only
    /// through its own `use`, not a part's — bare (`join`), qualified
    /// (`frame.new`), a core name whose form for these arguments is the
    /// module's (`max` of a column), and web's `route` — while the part's
    /// own functions keep their own import, and what the part declares is
    /// reached qualified as before
    #[test]
    fn a_parts_import_does_not_serve_the_entry() {
        let dir = std::env::temp_dir().join(format!("crow_parts_imports_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/helpers.co"), "use frame, math\n\nfn table_of[xs]\n    return frame.new[x = xs]\n\nfn spread[df]\n    return max[abs[df.x]]\n").unwrap();
        let check = |src: &str| -> Result<(), String> {
            let stmts = build(src).unwrap();
            let mut visited = std::collections::HashSet::new();
            let stmts = expand_module_imports_checked(&stmts, &dir, &mut visited)?;
            let lowered = lower::lower_program(&stmts).map_err(|e| format!("error: {}", e))?;
            infer_and_validate(&lowered, false).map(|_| ())
        };
        let leaking = "use lib.helpers\n\ndf = frame.new[node = ['a'], value = [1.0]]\nlog[len[helpers.table_of[[3.0]]]]\nlog[len[join[df, df, 'node']]]\nlog[max[abs[df.value - 1.0]]]\nlog[helpers.spread[helpers.table_of[[2.0]]]]\n";
        let err = check(leaking).unwrap_err();
        assert!(err.contains("line 3: frame.new[] is frame's: add `use frame`"), "{}", err);
        assert!(err.contains("line 6: abs[] and max[] of numbers are math's: add `use math`"), "{}", err);
        // (asked once per module: join's line adds nothing)
        assert_eq!(err.matches("add `use frame`").count(), 1, "{}", err);
        assert!(!err.contains("with"), "the entry's own form, not its rewrite: {}", err);
        let own = leaking.replacen("use lib.helpers\n", "use lib.helpers, frame, math\n", 1);
        assert!(check(&own).is_ok(), "{:?}", check(&own).err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A standard module written in Crow keeps its own imports inside its
    /// functions: the ClickHouse driver's `frame.new[]`, reached through a
    /// part, had been refused as the entry's — "frame.new[] is frame's: add
    /// `use frame`" at the entry's call — in an app whose parts alone import
    /// frame (finch)
    #[test]
    fn a_standard_modules_code_keeps_its_own_imports() {
        let dir = std::env::temp_dir().join(format!("crow_parts_module_imports_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/data.co"), "use db, frame\n\nfn count[url]\n    c = connect[url]\n    t = query[c, 'SELECT 1 AS n']\n    return len[t]\n").unwrap();
        let stmts = build("use lib.data\n\nlog[data.count['clickhouse://localhost:9000/default']]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports_checked(&stmts, &dir, &mut visited).unwrap();
        let lowered = lower::lower_program(&stmts).unwrap();
        let inlined = |s: &ast::Stmt| matches!(s, ast::Stmt::FnDecl { name, .. } if crate::modules::crow_scope_of(name).is_some_and(|m| m.contains("clickhouse")));
        assert!(lowered.iter().any(inlined), "the driver is inlined, its frame.new[] with it");
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A test's call refines a parameter the program's calls typed with
    /// an unknown inside: `fit`, which nothing calls, hands `step` a list
    /// of unknown items; the test hands it a list of tensors, which the
    /// gradient inside `gradients` then needs (it had been left
    /// `List(Unknown)`, and grad refused the loss's weights as "a value of
    /// a kind grad does not differentiate" — the gpt package)
    #[test]
    fn a_tests_call_refines_what_an_uncalled_function_left_unknown() {
        let dir = std::env::temp_dir().join(format!("crow_tests_refine_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'refine'\n").unwrap();
        std::fs::write(
            dir.join("lib/gm.co"),
            "use math\n\nfn loss[x, w0, w1]\n    return sum[(x * w0 * w1) .* (x * w0 * w1)]\n\nfn gradients[x, ws]\n    g = grad[loss]\n    every = g[x, ws[0], ws[1]]\n    return slice[every, 1, 3]\n\nfn init[]\n    ws = []\n    ws = push[ws, [[0.5, 0.0], [0.0, 0.5]]]\n    ws = push[ws, [[1.0, 0.0], [0.0, 1.0]]]\n    return ws\n",
        )
        .unwrap();
        let src = "use math\nuse lib/gm\n\nfn step[x, ws, lr]\n    grads = gm.gradients[x, ws]\n    moved = for i in range[len[ws]]\n        ws[i] - grads[i] * lr\n    return moved\n\nfn fit[x, ws, steps, lr]\n    weights = ws\n    i = 0\n    while i lt steps\n        weights = step[x, weights, lr]\n        i = i + 1\n    return weights\n\ntest a_step_lowers[]\n    x = [[1.0, 2.0], [3.0, 4.0]]\n    moved = step[x, gm.init[], 0.01]\n    len[moved] eq 2\n";
        let stmts = build(src).unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports_checked(&stmts, &dir, &mut visited).unwrap();
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A standard module's function, inlined into a `use ui` program, keeps
    /// its own names: llm's `embed` stacks its rows with math's `stack`,
    /// which the element check had read as the ui element outside a view
    /// ("ui element 'stack' can only be used inside a view block" — the
    /// rag example's handler)
    #[test]
    fn a_modules_stack_in_a_ui_program_is_maths() {
        let src = "use web, ui, llm\n\nquestion = ''\ncount = 0\n\nfn wanted[]\n    e = embed[reach['openai://127.0.0.1:1/x'], [question]]\n    if e.ok\n        count = len[e]\n\nview home[]\n    return stack[field['Question', question], button['Ask', click = wanted]]\n\npage['/', home]\nserve[3000]\n";
        let stmts = build(src).unwrap();
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
    }

    /// A standard module's local named like a fact of the program is the
    /// module's own: llm's `embed` grows `rows = []`, and a `use ui`
    /// program with a fact `rows = 0` had typed the module's list as that
    /// int ("push[] takes a list and a value … got an int and numbers")
    #[test]
    fn a_modules_local_named_like_a_fact_is_its_own() {
        let src = "use web, ui, llm\n\nquestion = ''\nrows = 0\n\nfn wanted[]\n    e = embed[reach['openai://127.0.0.1:1/x'], [question]]\n    if e.ok\n        rows = len[e]\n\nview home[]\n    return stack[field['Question', question], button['Ask', click = wanted]]\n\npage['/', home]\nserve[3000]\n";
        let stmts = build(src).unwrap();
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
    }

    /// A part that does not parse is the program's refusal, named by the
    /// part's file and line (it had been dropped silently, and the entry
    /// told its name was undefined)
    #[test]
    fn a_part_that_does_not_parse_is_refused_by_its_own_file() {
        let dir = std::env::temp_dir().join(format!("crow_parts_parse_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("views")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("views/sim.co"), "fn render[]\n    return 1 y = 2\n").unwrap();
        let stmts = build("use views.sim\nlog[sim.render[]]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let err = expand_module_imports_checked(&stmts, &dir, &mut visited).unwrap_err();
        assert!(err.starts_with("views/sim.co: line 2, col 14: a line holds one statement"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// A part's function is its own in the whole program: its locals and
    /// parameters are not the entry's facts or derivations of those names,
    /// nor the functions another part imports (finch's detectors, in the
    /// app: a local `ch` beside the entry's connection was refused as a type
    /// change and made the function a handler, a parameter `secs` read as
    /// the entry's derivation, a local list `rsi` as chart's function where
    /// the scanner imports chart, a parameter `vwap` was reserved)
    #[test]
    fn a_parts_locals_are_its_own_in_the_whole_program() {
        let dir = std::env::temp_dir().join(format!("crow_parts_own_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(
            dir.join("lib/calc.co"),
            "fn spread[ts, secs, vwap]\n    ch = []\n    rsi = []\n    for t in ts\n        ch = push[ch, t * secs - vwap]\n        rsi = push[rsi, t]\n    first = rsi[0]\n    return ch[0] + first\n",
        )
        .unwrap();
        std::fs::write(dir.join("lib/scan.co"), "use math, chart\nfn smooth[xs]\n    return sum[xs]\n").unwrap();
        let stmts = build("use web, ui\nuse lib.calc\nuse lib.scan\nch = 'conn'\nderive secs\n    return len[ch]\nview v[]\n    return p['`calc.spread[[1.0, 2.0], 2.0, 0.5]` `scan.smooth[[1.0, 2.0]]` `secs`']\npage['/', v]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let mut ctx = checked.unwrap();
        let program = lower_ir::lower(&lowered, &mut ctx, "parts").unwrap();
        let text = ir::print(&program);
        assert!(text.contains("pure lib_calc_spread("), "a plain function, not a handler:\n{}", text);
        assert!(text.contains("let ch__lib_calc: List(Float) = []"), "{}", text);
        assert!(text.contains("secs__lib_calc: Float"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A part's function only its own tests call is typed from their calls
    /// whichever file is the entry: the entry's build leaves the part's
    /// tests out, and the function's calls had been run by name there
    /// (finch's `cut`, `minute` and `channel_tape`, in the app)
    #[test]
    fn a_parts_function_its_tests_call_is_typed_in_the_whole_program() {
        let dir = std::env::temp_dir().join(format!("crow_parts_tested_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(
            dir.join("lib/tape.co"),
            "struct Tape [t, px]\nfn cut[tape, upto]\n    return Tape[min[tape.t, upto], abs[tape.px]]\ntest cuts[]\n    assert[cut[Tape[3.0, -1.0], 2.0].t eq 2.0]\n",
        )
        .unwrap();
        let stmts = build("use lib.tape\nlog['ready']\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let lowered = lower::lower_program(&stmts).unwrap();
        let mut ctx = infer_and_validate(&lowered, false).unwrap();
        let program = lower_ir::lower(&lowered, &mut ctx, "parts").unwrap();
        let text = ir::print(&program);
        assert!(text.contains("lib_tape_cut(tape: Record(lib_tape_Tape), upto: Float)"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A part's function may name a local as the entry names a derivation:
    /// the local is the function's (finch's book walk kept its `fills`
    /// beside trading's `derive fills`, and was refused as assigning a
    /// derivation)
    #[test]
    fn a_parts_local_may_share_a_derivations_name() {
        let dir = std::env::temp_dir().join(format!("crow_parts_derived_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/walk.co"), "fn gather[xs]\n    fills = []\n    for x in xs\n        fills = push[fills, x * 2.0]\n    return fills\n").unwrap();
        let stmts = build("use web, ui\nuse lib.walk\nderive fills\n    return walk.gather[[1.0, 2.0]]\nview v[]\n    return p['`len[fills]`']\npage['/', v]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let mut ctx = checked.unwrap();
        let program = lower_ir::lower(&lowered, &mut ctx, "parts").unwrap();
        let text = ir::print(&program);
        assert!(text.contains("let fills__lib_walk: List(Float) = []"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing in a part's bodies is the entry's top-level name of the same
    /// spelling: a derivation's local `c` beside the entry's `c` was refused
    /// as writing a fact, a handler's local `r` was typed as the entry's
    /// text, and a handler's parameter `s` typed the entry's `s` (in a
    /// check script importing finch's parts, and silently wrong where the
    /// types agreed). A keyword naming a renamed parameter follows it.
    #[test]
    fn a_parts_locals_are_not_the_entrys_names() {
        let dir = std::env::temp_dir().join(format!("crow_parts_locals_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(
            dir.join("lib/state.co"),
            "use web, ui\nbase = 2\nderive doubled\n    c = base * 2\n    return c\nfn reset[]\n    r = 3\n    base = r\nfn set_to[s]\n    base = s\nview show[]\n    return row[p['`doubled`'], button['reset', click = reset], button['four', click = set_to[4]]]\n",
        )
        .unwrap();
        let stmts = build("use web, ui\nuse lib.state\nc = 5\nr = 'text'\ns = 'hello'\nfn seven[]\n    state.set_to[s = 7]\nview v[]\n    return stack[p['`c` `r` `s`'], state.show[], button['seven', click = seven]]\npage['/', v]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let lowered = lower::lower_program(&stmts).unwrap();
        let checked = infer_and_validate(&lowered, false);
        assert!(checked.is_ok(), "{:?}", checked.err());
        let mut ctx = checked.unwrap();
        let program = lower_ir::lower(&lowered, &mut ctx, "parts").unwrap();
        let text = ir::print(&program);
        assert!(text.contains("let c__lib_state: Int"), "{}", text);
        assert!(text.contains("let r__lib_state: Int"), "{}", text);
        assert!(text.contains("s__lib_state: Int"), "{}", text);
        assert!(text.contains("fact c: Int") && text.contains("fact r: Str") && text.contains("fact s: Str"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a database answers about a statement is kept in the project's
    /// `.crow/schema`, and a build reads it back when no database answers:
    /// a ClickHouse that was down typed every cell as text, and the compiled
    /// finch refused `entry` as a float given text
    #[test]
    fn a_query_is_typed_from_what_its_database_last_answered() {
        let dir = std::env::temp_dir().join(format!("crow_schema_record_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'schema'\n").unwrap();
        let db = dir.join("t.db");
        let h = crow_db::connect(&format!("sqlite://{}", db.display()), None).unwrap();
        h.exec("CREATE TABLE bars (sym TEXT, n INTEGER, px REAL)", &[]).unwrap();
        // answered: the columns are typed, and the answer is kept
        let src = format!("use db\nbook = connect['sqlite://{}']\nr = query[book, 'SELECT sym, n, px FROM bars']\n", db.display());
        let stmts = expand_module_imports(&build(&src).unwrap(), &dir, &mut std::collections::HashSet::new());
        let ctx = infer_and_validate(&lower::lower_program(&stmts).unwrap(), true).unwrap();
        let cols = ctx.vars.get("r").and_then(|t| t.frame_cols().cloned().flatten()).map(|c| c.cols.clone()).unwrap_or_default();
        // columns not declared NOT NULL: their cells can be missing
        assert_eq!(cols.get("n"), Some(&type_infer::Ty::Result(Box::new(type_infer::Ty::Int))), "{:?}", cols);
        let kept: Vec<_> = std::fs::read_dir(dir.join(".crow/schema")).unwrap().collect();
        assert_eq!(kept.len(), 1, "one statement kept");
        // not answered: a server that is not there types it from the record
        let text = std::fs::read_to_string(kept[0].as_ref().unwrap().path()).unwrap();
        let unreachable = "clickhouse://127.0.0.1:1/none";
        let src = format!("use db\nbook = connect['{}']\nr = query[book, 'SELECT sym, n, px FROM bars']\n", unreachable);
        let stmts = expand_module_imports(&build(&src).unwrap(), &dir, &mut std::collections::HashSet::new());
        let ctx = infer_and_validate(&lower::lower_program(&stmts).unwrap(), true).unwrap();
        let cols = ctx.vars.get("r").and_then(|t| t.frame_cols().cloned().flatten()).map(|c| c.cols.clone()).unwrap_or_default();
        assert_eq!(cols.get("px"), Some(&type_infer::Ty::Result(Box::new(type_infer::Ty::Float))), "{:?}\n{}", cols, text);
        assert!(text.contains("\"float?\""), "the record keeps that a column can hold NULL: {}", text);
        assert!(ctx.describe_notes.iter().any(|n| n.contains("typed from .crow/schema")), "{:?}", ctx.describe_notes);
        // not asked (CROW_NO_DESCRIBE): no database answers either, so the
        // record types it too — and asking nothing writes nothing
        let src = format!("use db\nbook = connect['sqlite://{}']\nr = query[book, 'SELECT sym, n, px FROM bars']\n", db.display());
        let stmts = expand_module_imports(&build(&src).unwrap(), &dir, &mut std::collections::HashSet::new());
        let ctx = infer_and_validate(&lower::lower_program(&stmts).unwrap(), false).unwrap();
        let cols = ctx.vars.get("r").and_then(|t| t.frame_cols().cloned().flatten()).map(|c| c.cols.clone()).unwrap_or_default();
        assert_eq!(cols.get("sym"), Some(&type_infer::Ty::Result(Box::new(type_infer::Ty::Str))), "{:?}", cols);
        assert_eq!(std::fs::read_dir(dir.join(".crow/schema")).unwrap().count(), 1, "one statement kept, as before");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `crow build --types` prints each body's locals beside its signature:
    /// the oracle inference's rewrite is held to
    #[test]
    fn the_types_dump_shows_each_body() {
        let dir = std::env::temp_dir().join(format!("crow_types_dump_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.co");
        std::fs::write(&file, "fn half[x]\n    y = x / 2.0\n    return y\nlog[half[3]]\n").unwrap();
        let text = types_of_file(&file.display().to_string(), false).unwrap();
        assert!(text.contains("functions:\n  half(x: Int) -> Float\n    x: Int\n    y: Float\n"), "{}", text);
        assert!(text.contains("\nspecializations:\n") && text.contains("\ntop:\n"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A test block's program types its queries' columns as the program
    /// does: a NULL cell of a column that can hold one is a missing cell, an
    /// error of reason `missing`, as `crow run` reads it. (The test's
    /// program was lowered without asking the database, so its cells were
    /// dynamic — a NULL one JSON's null — and a handler handed one was
    /// copied for a dynamic value.)
    #[test]
    fn a_test_types_its_queries_as_the_program_does() {
        let dir = std::env::temp_dir().join(format!("crow_test_describes_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        let h = crow_db::connect(&format!("sqlite://{}", db.display()), None).unwrap();
        h.exec("CREATE TABLE bars (sym TEXT NOT NULL, px REAL)", &[]).unwrap();
        h.exec("INSERT INTO bars (sym) VALUES ('AB')", &[]).unwrap();
        let src = format!("use db\nbook = connect['sqlite://{}']\ntest a_null_cell_is_missing[]\n    r = query[book, 'SELECT sym, px FROM bars']\n    px = r.px[0]\n    assert[not px.ok and px.reason eq 'missing', 'px is `px.reason`']\n", db.display());
        let stmts = expand_module_imports(&build(&src).unwrap(), &dir, &mut std::collections::HashSet::new());
        let stmts = lower::lower_program(&stmts).unwrap();
        let (issues, counts) = checker::run_test_blocks(&stmts);
        assert_eq!(counts.blocks, 1);
        assert!(issues.is_empty(), "{:?}", issues);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A named test that opens the app runs as a scenario in a rehearsal
    /// (B3): nothing connects — a sqlite file that is not there stays not
    /// there, a ClickHouse that is not up answers from rows generated from
    /// what it answered at build time — a write is recorded with its
    /// payload, and an assertion that depends on real data is reported
    #[test]
    fn a_named_test_rehearses_with_no_connection() {
        let dir = std::env::temp_dir().join(format!("crow_rehearsal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".crow/schema")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'rehearsal'\n").unwrap();
        let db = dir.join("never.db");
        let sql = "SELECT sym, px FROM bars WHERE d = ?";
        std::fs::write(
            dir.join(".crow/schema").join(format!("{}.json", crow_runtime::fixtures::fnv(&["ch", sql]))),
            serde_json::json!({ "via": "ch", "sql": sql, "columns": [["sym", "str"], ["px", "float"]] }).to_string(),
        )
        .unwrap();
        let src = format!(
            "use web, ui, db\nbook = connect['sqlite://{}']\nch = connect['clickhouse://127.0.0.1:1/none']\nsaid = ''\nn = 0\nfn save[]\n    exec[book, 'INSERT INTO notes (text) VALUES (?)', ['hello']]\n    r = query[ch, '{}', ['x']]\n    n = len[r]\n    w = write['{}', 'a note']\n    said = 'saved `len[r]`'\nview v[]\n    return stack[p['`said`'], button['save', click = save]]\npage['/', v]\ntest saving_writes_a_note[]\n    a = app[v]\n    click[a, save]\n    assert[n eq 7, 'real data has seven']\n",
            db.display(),
            sql,
            dir.join("out.txt").display()
        );
        let main = dir.join("main.co");
        std::fs::write(&main, src).unwrap();
        let out = scenario_artifacts(&main.display().to_string(), &std::collections::BTreeMap::new());
        let (file, text) = out.get("scenario main_saving_writes_a_note").cloned().unwrap_or_else(|| panic!("{:?}", out.keys().collect::<Vec<_>>()));
        assert_eq!(file, "main.co");
        assert!(text.contains("ran to its end"), "{}", text);
        assert!(text.contains(&format!("wrote:\n    sqlite://{} INSERT INTO notes (text) VALUES (?) [\"hello\"]", db.display())), "{}", text);
        assert!(text.contains("answered with generated rows:\n    SELECT sym, px FROM bars WHERE d = ?"), "{}", text);
        assert!(text.contains("did not hold on rehearsal data:") && text.contains("real data has seven"), "{}", text);
        assert!(text.contains("n = 3") && text.contains("said = saved 3"), "{}", text);
        assert!(!db.exists(), "the rehearsal opened the sqlite file");
        assert!(text.contains("write[") && !dir.join("out.txt").exists(), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An entry that does not import ui is a script, whatever its parts
    /// import: its top level runs in order (a check script importing
    /// finch's session part computed `n = len[xs]` after the loop that
    /// filled `xs` as 0, its variables having become state)
    #[test]
    fn a_script_importing_a_ui_part_runs_its_top_level_in_order() {
        let dir = std::env::temp_dir().join(format!("crow_parts_script_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/state.co"), "use web, ui\nbase = 2\nfn twice[x]\n    return x * 2\nview show[]\n    return p['`base`']\n").unwrap();
        let stmts = build("use lib.state\nxs = []\nfor i in range[3]\n    xs = push[xs, state.twice[i]]\nn = len[xs]\nlog['`n`']\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let it = ir_interp::run_script(&stmts, "parts").unwrap();
        assert!(matches!(it.var("n").map(|v| &v.value), Some(crate::value::Value::Int(3))), "{:?}", it.var("n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An installed package's own imports are its own files: `use b` in
    /// package `pk` is `crow_modules/pk/b.co`, beside a program with a
    /// `b.co` of its own, which keeps its own (the package's import had
    /// resolved to the program's file and run its code)
    #[test]
    fn a_packages_imports_are_its_own_files() {
        let dir = std::env::temp_dir().join(format!("crow_package_own_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("crow_modules/pk")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'app'\n").unwrap();
        std::fs::write(dir.join("crow_modules/pk/a.co"), "use b\nfn g[]\n    return b.f[]\n").unwrap();
        std::fs::write(dir.join("crow_modules/pk/b.co"), "fn f[]\n    return 'package b'\n").unwrap();
        std::fs::write(dir.join("b.co"), "fn f[]\n    return 'program b'\n").unwrap();
        let stmts = build("use b\nuse pk.a\nx = a.g[]\ny = b.f[]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let it = ir_interp::run_script(&stmts, "app").unwrap();
        assert!(matches!(it.var("x").map(|v| &v.value), Some(crate::value::Value::Str(s)) if s == "package b"), "{:?}", it.var("x"));
        assert!(matches!(it.var("y").map(|v| &v.value), Some(crate::value::Value::Str(s)) if s == "program b"), "{:?}", it.var("y"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A test sets a part's fact by the name the code reads it by:
    /// `change[a, 'calc.rate', 2.0]` (the harness knew it only by its
    /// prefixed name)
    #[test]
    fn a_test_changes_a_parts_fact_by_its_qualified_name() {
        let dir = std::env::temp_dir().join(format!("crow_parts_change_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/calc.co"), "use web, ui\nrate = 1.0\nview show[]\n    return p['`rate`']\n").unwrap();
        let stmts = build("use web, ui\nuse lib.calc\nview v[]\n    return calc.show[]\npage['/', v]\ntest t[]\n    a = app[v]\n    change[a, 'calc.rate', 2.0]\n    assert['2' in html[a]]\n").unwrap();
        let mut visited = std::collections::HashSet::new();
        let stmts = expand_module_imports(&stmts, &dir, &mut visited);
        let text = format!("{:?}", stmts);
        assert!(text.contains("Str(\"lib_calc_rate\")"), "{}", text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An import a part makes reserves names in the part, not in the entry:
    /// the entry may name a parameter `vwap` beside a part that imports
    /// chart (it was refused "provided by chart (imported on line 0)");
    /// the entry's own import still reserves it, with its line
    #[test]
    fn a_parts_import_reserves_nothing_in_the_entry() {
        let dir = std::env::temp_dir().join(format!("crow_parts_import_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("crowfile"), "name = 'parts'\n").unwrap();
        std::fs::write(dir.join("lib/scan.co"), "use math, chart\nfn smooth[xs]\n    return sum[xs]\n").unwrap();
        let check = |entry: &str| {
            let stmts = build(entry).unwrap();
            let mut visited = std::collections::HashSet::new();
            let stmts = expand_module_imports(&stmts, &dir, &mut visited);
            let lowered = lower::lower_program(&stmts).unwrap();
            infer_and_validate(&lowered, false).map(|_| ())
        };
        let ok = check("use lib.scan\nfn spread[vwap]\n    return vwap * 2.0\nlog[spread[1.0]]\nlog[scan.smooth[[1.0]]]\n");
        assert!(ok.is_ok(), "{:?}", ok.err());
        let own = check("use chart\nuse lib.scan\nfn spread[vwap]\n    return vwap * 2.0\nlog[spread[1.0]]\nlog[scan.smooth[[1.0]]]\n");
        assert!(own.as_ref().is_err_and(|e| e.contains("'vwap' is provided by chart (imported on line 1)")), "{:?}", own);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `use sys` is the standard library's: a program's file is refused
    /// before it runs, with the way a program reaches what `sys` builds
    #[test]
    fn a_program_may_not_import_sys() {
        // SAFETY: the door is read by this process alone, in this test
        unsafe { std::env::remove_var("CROW_SYS") };
        let stmts = crate::build("use sys\nb = bytes['x']\nlog[len[b]]\n").expect("parses");
        let dir = std::env::temp_dir();
        let mut visited = std::collections::HashSet::new();
        let out = expand_module_imports_checked(&stmts, &dir, &mut visited);
        assert!(out.as_ref().is_err_and(|e| e.contains("line 1: `sys` is the standard library's own layer")), "{:?}", out.map(|_| ()));
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
        crow_db::connect(&url, Some(1)).unwrap().exec("CREATE TABLE nums (n INTEGER NOT NULL)", &[]).unwrap();
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
    /// Rust the compiler refuses is a bug in Crow, said as one after the
    /// compiler's own first error and where it points (a moved value had
    /// read "value was already used and cannot be accessed again")
    #[test]
    fn rust_that_does_not_build_is_a_bug_in_crow() {
        let stderr = "   Compiling app v0.1.0\nerror[E0382]: borrow of moved value: `x`\n  --> src/main.rs:65:32\n   |\nerror: could not compile `app`\n";
        let out = super::translate_rust_compile_error(stderr);
        assert!(out.starts_with("error[E0382]: borrow of moved value: `x`\n  --> src/main.rs:65:32\n"), "{}", out);
        assert!(out.lines().last().is_some_and(|l| l.contains("a bug in Crow, not in this program")), "{}", out);
        assert_eq!(super::translate_rust_compile_error("error: this operation will panic at runtime\nattempt to divide `1_i64` by zero"), "error: cannot divide by zero");
    }
}
