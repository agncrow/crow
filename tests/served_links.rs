//! The served harness (`crow test --served`) serves each reference app
//! from a copy of its sources, and an app's installed package — a link
//! under its crow_modules — comes with it: the copy had held only the
//! app's files, so an app over a package (the magpie example) exited
//! before serving in both modes, "2 mistakes".

use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
}

#[test]
fn an_apps_linked_package_is_served_with_it() {
    let root = std::env::temp_dir().join(format!("crow_served_links_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let app = root.join("examples/app");
    std::fs::create_dir_all(root.join("pkgsrc/pk")).unwrap();
    std::fs::create_dir_all(app.join("crow_modules")).unwrap();
    std::fs::write(root.join("pkgsrc/pk/crowfile"), "name = 'pk'\n").unwrap();
    std::fs::write(root.join("pkgsrc/pk/main.co"), "fn greeting[]\n    return 'hello from the package'\n").unwrap();
    std::fs::write(app.join("crowfile"), "name = 'app'\n").unwrap();
    std::fs::write(app.join("main.co"), "use web, ui\nuse pk\n\nview home[]\n    return div[pk.greeting[]]\n\nserve[3497]\npage['/', home]\n").unwrap();
    std::fs::write(app.join("main.events"), "[]\n").unwrap();
    std::os::unix::fs::symlink("../../../pkgsrc/pk", app.join("crow_modules/pk")).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    let out = Command::new(env!("CARGO_BIN_EXE_crow"))
        .args(["test", "--served"])
        .current_dir(&root)
        .env("CROW_BUILD_PROFILE", "dev")
        .output()
        .expect("the harness runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && stdout.contains("ok    examples/app"), "{}\n{}", stdout, String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&root);
}
