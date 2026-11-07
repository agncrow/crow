//! `crow build --target web` (spec, "Building for the web"): the program
//! compiled to WebAssembly, and the page that runs it. The program's Rust
//! is the emitter's, built as a `cdylib` for `wasm32-unknown-unknown`; the
//! page is `crow.js` and `index.html` (crow_runtime/web/), which give the
//! program what `crow_runtime::host` imports and call `crow_start` and
//! `crow_frame`. What a page in a browser cannot do — a file, a socket, a
//! process, a database, mail, a wait, a server — is refused here, by the
//! kinds of the rows the program reaches (`Effect::in_browser`), before
//! anything is built.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The page's script, written beside the program's module.
const SCRIPT: &str = include_str!("../crow_runtime/web/crow.js");
/// The page itself: `{name}` is the program's module.
const PAGE: &str = include_str!("../crow_runtime/web/index.html");

/// The triple the program is built for.
pub const TRIPLE: &str = "wasm32-unknown-unknown";

/// The builtins the program reaches that a page in a browser cannot run,
/// each with what it does, in the order of their names: none for a program
/// the web target builds.
pub fn unreachable_in_browser(program: &crate::ir::Program) -> Vec<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for part in &program.parts {
        let reached = crate::ir::reached_fns(part);
        for f in part.fns.iter().filter(|f| reached.contains(&f.name)) {
            names.extend(f.effects.iter().cloned());
        }
        for d in &part.derives {
            names.extend(d.effects.iter().cloned());
        }
        for effects in part.main_effects.values() {
            names.extend(effects.iter().cloned());
        }
    }
    let mut refused = Vec::new();
    for name in names {
        let kinds = crate::table::effects_of(&name);
        if let Some(k) = kinds.into_iter().find(|k| !k.in_browser()) {
            refused.push(format!("{}[] {}", crate::table::shown_effect(&name), k.does()));
        }
    }
    refused
}

/// The refusal of a program that reaches what a page cannot run.
pub fn refusal(refused: &[String]) -> String {
    format!(
        "crow build --target web: a page in a browser cannot run what this program reaches — {}. A program built for the web writes to the console, reads the clock, draws random numbers and draws on its page (spec, Building for the web)",
        refused.join("; ")
    )
}

/// The generated manifest, made a WebAssembly module: a library the page
/// loads, built from the same `src/main.rs`.
pub fn cargo_toml(manifest: &str) -> String {
    let manifest = manifest.replacen("edition = \"2021\"\n", "edition = \"2021\"\nautobins = false\n", 1);
    format!("{}\n[lib]\npath = \"src/main.rs\"\ncrate-type = [\"cdylib\"]\n", manifest)
}

/// The build's own configuration: the target, getrandom's source in a
/// browser (`crow_runtime::host::page`), and a stack as deep as a browser
/// gives a module (8 MB; the program's thread natively has 64).
pub fn cargo_config(vendored: Option<String>) -> String {
    let mut config = vendored.unwrap_or_default();
    if !config.is_empty() && !config.ends_with('\n') {
        config.push('\n');
    }
    config.push_str(&format!(
        "\n[build]\ntarget = \"{}\"\n\n[target.{}]\nrustflags = [\"--cfg\", \"getrandom_backend=\\\"custom\\\"\", \"-C\", \"link-arg=-zstack-size=8388608\"]\n",
        TRIPLE, TRIPLE
    ));
    config
}

/// Where cargo leaves the module: the package's name with `_` for `-`.
pub fn module_path(target_dir: &Path, profile_dir: &str, project_name: &str) -> PathBuf {
    target_dir.join(TRIPLE).join(profile_dir).join(format!("{}.wasm", project_name.replace('-', "_")))
}

/// The page, written into `dir`: the module, `crow.js` and `index.html`.
pub fn write_page(module: &Path, name: &str, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {}", dir.display(), e))?;
    let wasm = format!("{}.wasm", name);
    std::fs::copy(module, dir.join(&wasm)).map_err(|e| format!("cannot copy the module to {}: {}", dir.display(), e))?;
    std::fs::write(dir.join("crow.js"), SCRIPT).map_err(|e| format!("cannot write crow.js: {}", e))?;
    std::fs::write(dir.join("index.html"), PAGE.replace("{name}", name)).map_err(|e| format!("cannot write index.html: {}", e))?;
    Ok(())
}

/// What a build without the target's standard library says, in place of
/// the Rust compiler's words.
pub fn missing_target(stderr: &str) -> Option<String> {
    let missing = stderr.contains("can't find crate for `core`") || stderr.contains("can't find crate for `std`") || stderr.contains("target may not be installed");
    missing.then(|| format!("error: building for the web needs Rust's standard library for {t}, which this toolchain lacks: `rustup target add {t}` adds it", t = TRIPLE))
}

/// What of the environment the module's build leaves out, given the
/// environment as `var` reads it: under the coverage run, cargo-llvm-cov's
/// compiler wrapper (`RUSTC_WRAPPER`) adds `-C instrument-coverage` to the
/// workspace's crates — a native program built in the run is meant to link
/// the instrumented runtime, so both modes count — but WebAssembly has no
/// profiler to link (`can't find crate for profiler_builtins`), and the
/// failed build had failed tests/web_target.rs, which ended `cargo test`
/// before the runtime crates' own tests ran (crow_runtime's coverage read
/// 57% against its floor of 87, 2026-11-07). Any other wrapper (a cache)
/// stays.
pub fn left_out(var: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
    if var("CARGO_LLVM_COV").is_some() {
        vec!["RUSTC_WRAPPER", "__CARGO_LLVM_COV_RUSTC_WRAPPER", "__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS", "__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES"]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_coverage_runs_wrapper_is_left_out_of_a_modules_build_and_another_is_kept() {
        let coverage = |v: &str| match v {
            "CARGO_LLVM_COV" => Some("1".to_string()),
            "RUSTC_WRAPPER" => Some("cargo-llvm-cov".to_string()),
            _ => None,
        };
        assert!(super::left_out(coverage).contains(&"RUSTC_WRAPPER"));
        let cache = |v: &str| if v == "RUSTC_WRAPPER" { Some("sccache".to_string()) } else { None };
        assert!(super::left_out(cache).is_empty());
    }

    use super::*;

    #[test]
    fn the_manifest_becomes_a_module_built_from_main() {
        let m = cargo_toml("[package]\nname = \"tri\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        assert!(m.contains("autobins = false"));
        assert!(m.ends_with("[lib]\npath = \"src/main.rs\"\ncrate-type = [\"cdylib\"]\n"));
    }

    #[test]
    fn the_configuration_names_the_target_and_keeps_the_vendored_sources() {
        let c = cargo_config(Some("[source.crates-io]\nreplace-with = \"vendored\"".to_string()));
        assert!(c.starts_with("[source.crates-io]\nreplace-with = \"vendored\"\n"));
        assert!(c.contains("target = \"wasm32-unknown-unknown\""));
        assert!(c.contains("getrandom_backend=\\\"custom\\\""));
    }

    #[test]
    fn a_missing_standard_library_is_said_as_the_command_that_adds_it() {
        let said = missing_target("error[E0463]: can't find crate for `core`\n  = note: the `wasm32-unknown-unknown` target may not be installed").unwrap_or_default();
        assert!(said.contains("rustup target add wasm32-unknown-unknown"));
        assert_eq!(missing_target("error: something else"), None);
    }

    #[test]
    fn the_page_names_the_module() {
        assert!(PAGE.contains("{name}"));
        assert!(SCRIPT.contains("crow_frame"));
    }
}
