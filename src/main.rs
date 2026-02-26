mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;
mod transpiler;
mod type_infer;
mod value;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        cmd_repl();
        return;
    }

    match args[1].as_str() {
        "init" => cmd_init(),
        "install" => cmd_install(),
        "build" => cmd_build(),
        "run" => {
            // Check for -i flag (interpreter mode)
            if args.len() > 2 && args[2] == "-i" {
                if args.len() > 3 {
                    cmd_run_file_interpreted(&args[3]);
                } else {
                    cmd_run_interpreted();
                }
            } else if args.len() > 2 {
                // crow run file.crow arg1 arg2 ... -> transpile + compile + run
                let extra_args = args[3..].to_vec();
                cmd_run_file(&args[2], &extra_args);
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
    eprintln!("  crow init              create new project with crowfile");
    eprintln!("  crow install           install deps from crowfile into crow_modules/");
    eprintln!("  crow build             transpile to rust and compile to native binary");
    eprintln!("  crow build file.crow   transpile a specific file");
    eprintln!("  crow run               transpile + compile + run (uses crowfile)");
    eprintln!("  crow run file.crow     transpile + compile + run a specific file");
    eprintln!("  crow run -i file.crow  run with interpreter (slow, for debugging)");
}

fn cmd_repl() {
    use std::io::{self, Write, BufRead};

    println!("crow repl - type expressions or statements, ctrl-d to exit");
    let mut interpreter = Interpreter::new();
    let stdin = io::stdin();
    let mut buffer = String::new();
    let mut in_block = false;

    loop {
        // Print prompt
        if in_block {
            print!("  ");
        } else {
            print!("> ");
        }
        io::stdout().flush().unwrap();

        // Read line
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                // EOF (ctrl-d)
                println!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {}", e);
                break;
            }
        }

        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');

        // Multi-line block handling
        if in_block {
            if trimmed.is_empty() || (!trimmed.starts_with(' ') && !trimmed.starts_with('\t')) {
                // Empty line or non-indented line ends the block
                in_block = false;
                let source = std::mem::take(&mut buffer);
                repl_eval(&mut interpreter, &source);
                // If this line is non-empty and non-indented, process it below
                if trimmed.is_empty() {
                    continue;
                }
                // Fall through to process this line
            } else {
                buffer.push_str(&line);
                continue;
            }
        }

        // Check if this line expects a block to follow
        // Any line containing a block keyword (not inside a string) needs indented body
        let needs_block = {
            // Strip string literals to avoid matching keywords inside strings
            let stripped = trimmed.replace(|c: char| c == '\'', "\x00")
                .split('\x00')
                .enumerate()
                .filter(|(i, _)| i % 2 == 0) // keep non-string parts
                .map(|(_, s)| s)
                .collect::<Vec<_>>()
                .join(" ");
            stripped.split_whitespace()
                .any(|w| matches!(w, "if" | "for" | "while" | "fn" | "match" | "struct"))
        };

        if needs_block {
            in_block = true;
            buffer = line;
            continue;
        }

        // Single line - evaluate immediately
        repl_eval(&mut interpreter, trimmed);
    }
}

fn repl_eval(interpreter: &mut Interpreter, source: &str) {
    let source = source.trim();
    if source.is_empty() {
        return;
    }

    match build(source) {
        Ok(stmts) => {
            match interpreter.run(&stmts) {
                Ok(val) => {
                    // Print result if it's not Done (void)
                    if !matches!(val.result, value::Value::Done) {
                        println!("{}", val);
                    }
                }
                Err(e) => eprintln!("error: {}", e),
            }
        }
        Err(e) => eprintln!("error: {}", e),
    }
}

fn cmd_init() {
    // create crowfile and main.crow
    if std::path::Path::new("crowfile").exists() {
        eprintln!("crowfile already exists");
        process::exit(1);
    }

    let dir_name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-project".to_string());

    let crowfile = format!(
        "name = '{}'\nentry = 'main.crow'\n",
        dir_name
    );

    std::fs::write("crowfile", crowfile).unwrap_or_else(|e| {
        eprintln!("error writing crowfile: {}", e);
        process::exit(1);
    });

    if !std::path::Path::new("main.crow").exists() {
        std::fs::write("main.crow", "log['hello, crow']\n").unwrap_or_else(|e| {
            eprintln!("error writing main.crow: {}", e);
            process::exit(1);
        });
    }

    println!("initialized crow project '{}'", dir_name);
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
            // Version string - future registry support
            eprintln!("  {} v{} - registry not yet available, skipping", name, source);
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

/// Transpile a .crow file to Rust, compile it, and return the path to the binary.
/// If `quiet` is true, suppress cargo build output (used for `crow run`).
fn transpile_and_compile(entry: &str, quiet: bool) -> std::path::PathBuf {
    let source = read_file(entry);
    let stmts = match build(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    };

    // Run type inference
    let mut ctx = type_infer::TypeContext::new();
    ctx.infer_program(&stmts);

    // Transpile to Rust
    let mut transpiler = transpiler::Transpiler::new(ctx);
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
    let deps = if transpiler.needs_rayon {
        "\n[dependencies]\nrayon = \"1.10\"\n"
    } else {
        ""
    };
    let cargo_toml = format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{}\n[profile.release]\nopt-level = 3\nlto = true\n",
        project_name, deps
    );
    std::fs::write(build_dir.join("Cargo.toml"), &cargo_toml).unwrap_or_else(|e| {
        eprintln!("error writing Cargo.toml: {}", e);
        process::exit(1);
    });

    // Write transpiled Rust source
    std::fs::write(src_dir.join("main.rs"), &rust_source).unwrap_or_else(|e| {
        eprintln!("error writing main.rs: {}", e);
        process::exit(1);
    });

    // Compile with cargo (suppress output for quiet mode)
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["build", "--release"]).current_dir(&build_dir);
    if quiet {
        cmd.stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
    }
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error running cargo: {}", e);
        process::exit(1);
    });

    if !status.success() {
        eprintln!("compilation failed");
        process::exit(1);
    }

    build_dir.join("target/release").join(&project_name)
}

fn cmd_build() {
    let args: Vec<String> = std::env::args().collect();
    let entry = if args.len() > 2 {
        args[2].clone()
    } else if std::path::Path::new("crowfile").exists() {
        get_entry_file()
    } else {
        eprintln!("usage: crow build <file.crow> or run from a project with crowfile");
        process::exit(1);
    };

    let binary = transpile_and_compile(&entry, false);

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

struct CrowfileConfig {
    entry: String,
    cpu: Option<usize>,
    gpu: Option<usize>,
    deps: Vec<(String, String)>,  // (name, version)
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

fn cmd_run(extra_args: &[String]) {
    let config = parse_crowfile();
    let binary = transpile_and_compile(&config.entry, true);
    run_binary(&binary, extra_args);
}

fn cmd_run_file(filename: &str, extra_args: &[String]) {
    let binary = transpile_and_compile(filename, true);
    run_binary(&binary, extra_args);
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
        match build(&source) {
            Ok(stmts) => stmts,
            Err(e) => {
                eprintln!("{}", e);
                process::exit(1);
            }
        }
    };

    let mut interpreter = Interpreter::new();
    let base = filename.replace(".crowc", ".crow");
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

fn parse_crowfile() -> CrowfileConfig {
    let crowfile_source = match std::fs::read_to_string("crowfile") {
        Ok(s) => s,
        Err(_) => {
            eprintln!("no crowfile found. run 'crow init' first");
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
                Some(val) => match &val.result {
                    value::Value::Str(s) => s.clone(),
                    _ => {
                        eprintln!("crowfile: 'entry' must be a string");
                        process::exit(1);
                    }
                },
                None => {
                    eprintln!("crowfile: missing 'entry'");
                    process::exit(1);
                }
            };

            let cpu = interpreter.get_var("cpu").and_then(|val| {
                if let value::Value::Int(n) = &val.result {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            let gpu = interpreter.get_var("gpu").and_then(|val| {
                if let value::Value::Int(n) = &val.result {
                    Some(*n as usize)
                } else {
                    None
                }
            });

            // parse deps: ['math' = '1.0', 'http' = '2.3']
            let deps = interpreter.get_var("deps").map(|val| {
                if let value::Value::Map(map) = &val.result {
                    map.iter().filter_map(|(name, v)| {
                        if let value::Value::Str(ver) = &v.result {
                            Some((name.clone(), ver.clone()))
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            }).unwrap_or_default();

            CrowfileConfig { entry, cpu, gpu, deps }
        }
        Err(e) => {
            eprintln!("error in crowfile: {}", e);
            process::exit(1);
        }
    }
}

fn get_entry_file() -> String {
    parse_crowfile().entry
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

fn build(source: &str) -> Result<Vec<ast::Stmt>, String> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}
