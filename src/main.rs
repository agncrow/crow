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
fn cmd_test_book() {
    let dir = std::path::Path::new("docs/book/src");
    if !dir.is_dir() {
        eprintln!("error: no docs/book/src/ directory found (run from the project root)");
        process::exit(1);
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |x| x == "md"))
                .collect()
        })
        .unwrap_or_default();
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
        let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
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
    let tmp_dir = std::env::temp_dir().join("crow_book_test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let run_mode = |mode_args: &[&str], file: &std::path::Path| -> (bool, String, String) {
        let out = std::process::Command::new(&exe).args(mode_args).arg(file).output();
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
    let total = blocks.iter().filter(|b| b.mode != Mode::Ignore).count();
    println!("book: {} blocks across {} pages\n", total, files.len());
    let mut failures = 0;
    for b in &blocks {
        let label = format!("{}:{}", b.file, b.line);
        match b.mode {
            Mode::Ignore => continue,
            Mode::Run => {
                let path = tmp_dir.join("block.crow");
                let _ = std::fs::write(&path, &b.code);
                let (i_ok, i_out, i_err) = run_mode(&["run", "-i"], &path);
                let (t_ok, t_out, t_err) = run_mode(&["run"], &path);
                if !i_ok || !t_ok {
                    failures += 1;
                    println!("  FAIL  {} (execution)", label);
                    let tail = if !t_ok { t_err } else { i_err };
                    println!("        {}", tail.lines().last().unwrap_or("(no stderr)"));
                    continue;
                }
                if i_out != t_out {
                    failures += 1;
                    println!("  FAIL  {} (modes disagree)", label);
                    continue;
                }
                if let Some(exp) = &b.expected {
                    if t_out.trim_end() != exp.trim_end() {
                        failures += 1;
                        println!("  FAIL  {} (output != book text)", label);
                        println!("        expected: {:?}", exp.trim_end());
                        println!("        got:      {:?}", t_out.trim_end());
                        continue;
                    }
                }
                println!("  ok    {}", label);
            }
            Mode::NoRun => {
                let path = tmp_dir.join("block.crow");
                let _ = std::fs::write(&path, &b.code);
                let (ok, _, err) = run_mode(&["build"], &path);
                if ok {
                    println!("  ok    {} (build only)", label);
                } else {
                    failures += 1;
                    println!("  FAIL  {} (does not build)", label);
                    println!("        {}", err.lines().last().unwrap_or("(no stderr)"));
                }
            }
            Mode::Error => {
                let path = tmp_dir.join("block.crow");
                let _ = std::fs::write(&path, &b.code);
                let (ok, _, _) = run_mode(&["build"], &path);
                if !ok {
                    println!("  ok    {} (fails as documented)", label);
                } else {
                    failures += 1;
                    println!("  FAIL  {} (documented as an error, but builds)", label);
                }
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f32();
    println!();
    if failures == 0 {
        println!("\u{2713} {} book blocks verified in both modes ({:.1}s)", total, elapsed);
    } else {
        println!("\u{2717} {} of {} book blocks fail ({:.1}s)", failures, total, elapsed);
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
        write_file(&project_dir.join("main.crow"), "use web, ui\nuse views.home\n\nserve[3000]\npage['/', home_render]\n");
    } else {
        write_file(&project_dir.join("main.crow"), &format!("log['hello, {}']\n", short_name));
    }

    // --web: add views/home.crow
    if has_web {
        write_file(&project_dir.join("views/home.crow"),
            "--- home page ---\nuse ui\n\nmessage = 'welcome to crow'\n\nview home_render[]\n    return div[\n        h1['`message`']\n    ]\n");
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
        crowfile.push_str("\njobs = [");
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

    // Run type inference
    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(&stmts);

    // Run validation pass — catch user errors before transpilation
    let mut v = validator::Validator::new(&ctx);
    if let Err(e) = v.validate(&stmts) {
        eprintln!("{}", e);
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
    let status = std::process::Command::new(binary)
        .args(extra_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("error running binary: {}", e);
            process::exit(1);
        });
    process::exit(status.code().unwrap_or(1));
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
    checker::run_check(&stmts, &ctx, false)
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
        let mut prompt_parts = vec!["enter = recheck", "s = skip"];
        if has_try { prompt_parts.push("t = try input"); }
        if is_regression { prompt_parts.push("a = accept new"); }
        if later_fixable { prompt_parts.push("f = fix (later[])"); }
        prompt_parts.push("q = quit");
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
            eprintln!("{}", e);
            process::exit(1);
        }
        stmts
    };

    let mut interpreter = Interpreter::new();
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

/// Expand `use` imports by parsing module files and inserting prefixed
/// FnDecl/StructDecl/Assign statements. This must run *before* type inference
/// so the main program's call sites can inform module function param types.
fn expand_module_imports(
    stmts: &[ast::Stmt],
    source_dir: &std::path::Path,
    visited: &mut std::collections::HashSet<String>,
) -> Vec<ast::Stmt> {
    let mut expanded = Vec::new();
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
        }
        // Keep the original statement (including Use, so transpiler knows about it)
        expanded.push(stmt.clone());
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
            // Assigning creates/updates a local inside a fn body — the name is
            // shadowed from here on, but the RHS is evaluated first
            rewrite_refs_expr(value, map, shadowed);
            let _ = name;
        }
        ast::Stmt::MultiAssign { values, .. } => {
            for v in values {
                rewrite_refs_expr(v, map, shadowed);
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
