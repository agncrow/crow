mod ast;
mod interpreter;
mod lexer;
mod parser;
mod token;
mod value;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "init" => cmd_init(),
        "build" => cmd_build(),
        "run" => {
            if args.len() > 2 {
                // crow run file.crow
                cmd_run_file(&args[2]);
            } else {
                // crow run (uses crowfile)
                cmd_run();
            }
        }
        other => {
            // check if it's a .crow file (shortcut: crow file.crow)
            if other.ends_with(".crow") {
                cmd_run_file(other);
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
    eprintln!("  crow init              create new project with crowfile");
    eprintln!("  crow build             compile and check for errors");
    eprintln!("  crow run               build + run (uses crowfile)");
    eprintln!("  crow run file.crow     run a specific file");
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

fn cmd_build() {
    let entry = get_entry_file();
    let source = read_file(&entry);
    // build = lex + parse (check for errors)
    match build(&source) {
        Ok(_) => println!("build ok"),
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

fn cmd_run() {
    let entry = get_entry_file();
    cmd_run_file(&entry);
}

fn cmd_run_file(filename: &str) {
    let source = read_file(filename);
    match build(&source) {
        Ok(stmts) => {
            let mut interpreter = Interpreter::new();
            if let Err(e) = interpreter.run(&stmts) {
                eprintln!("runtime error: {}", e);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

fn get_entry_file() -> String {
    let crowfile_source = match std::fs::read_to_string("crowfile") {
        Ok(s) => s,
        Err(_) => {
            eprintln!("no crowfile found. run 'crow init' first");
            process::exit(1);
        }
    };

    // parse crowfile to find entry
    match build(&crowfile_source) {
        Ok(stmts) => {
            let mut interpreter = Interpreter::new();
            if let Err(e) = interpreter.run(&stmts) {
                eprintln!("error in crowfile: {}", e);
                process::exit(1);
            }
            // look for the 'entry' variable
            match interpreter.get_var("entry") {
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
            }
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

fn build(source: &str) -> Result<Vec<ast::Stmt>, String> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}
