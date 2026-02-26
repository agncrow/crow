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
use std::collections::HashMap;

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

const DEFAULT_REGISTRY_URL: &str = "https://raw.githubusercontent.com/vsriram/registry/main/index.json";

#[derive(serde::Deserialize, Debug)]
struct RegistryIndex {
    packages: HashMap<String, RegistryPackage>,
}

#[derive(serde::Deserialize, Debug)]
struct RegistryPackage {
    repo: String,
    versions: Vec<String>,
}

fn get_registry_url() -> String {
    std::env::var("CROW_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string())
}

fn registry_cache_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".crow").join("registry").join("index.json")
}

fn fetch_registry_index() -> Option<RegistryIndex> {
    let cache_path = registry_cache_path();
    let url = get_registry_url();

    // Try fetching fresh index with curl
    let output = std::process::Command::new("curl")
        .args(["-sfL", "--max-time", "10", &url])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json = String::from_utf8_lossy(&out.stdout);
            // Cache it
            if let Some(parent) = cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache_path, &*json);
            match serde_json::from_str(&json) {
                Ok(index) => return Some(index),
                Err(e) => {
                    eprintln!("warning: failed to parse registry index: {}", e);
                }
            }
        }
        _ => {
            eprintln!("warning: failed to fetch registry from {}", url);
        }
    }

    // Fall back to cached index
    if cache_path.exists() {
        if let Ok(json) = std::fs::read_to_string(&cache_path) {
            match serde_json::from_str(&json) {
                Ok(index) => {
                    eprintln!("  using cached registry index");
                    return Some(index);
                }
                Err(_) => {}
            }
        }
    }

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
fn transpile_and_compile(entry: &str, quiet: bool) -> std::path::PathBuf {
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
        if let ast::Stmt::Use { path } = stmt {
            let module_name = path.rsplit('/').next().unwrap_or(path).to_string();

            // Circular import detection
            if visited.contains(path) {
                expanded.push(stmt.clone());
                continue;
            }
            visited.insert(path.clone());

            // Try to find the module file
            let candidates = vec![
                source_dir.join(format!("{}.crow", path)),
                source_dir.join(format!("crow_modules/{}.crow", path)),
                source_dir.join(format!("crow_modules/{}/main.crow", path)),
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

            // Rewrite module declarations with prefixed names
            for mod_stmt in &mod_stmts {
                match mod_stmt {
                    ast::Stmt::FnDecl { name, params, body } => {
                        expanded.push(ast::Stmt::FnDecl {
                            name: format!("{}_{}", module_name, name),
                            params: params.clone(),
                            body: body.clone(),
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
                        expanded.push(ast::Stmt::Assign {
                            name: format!("{}_{}", module_name, name),
                            value: value.clone(),
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
