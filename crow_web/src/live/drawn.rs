//! The drawn page (one-view-design.md 3.2, phase 4): a served app offers
//! every page of its own a second time, drawn, at `/_drawn<path>` — the
//! page `/execution` at `/_drawn/execution`. The drawn page is a Lory host
//! (Lory's `drawn` package): a canvas, its program built for the web, and
//! a script that opens the page's own socket with `drawn=1`. The app
//! serves its files from the folder `CROW_DRAWN` names (else `~/.crow/drawn`
//! when there is one): a path that names a file of the folder is that file,
//! and any other is the folder's `index.html`, which reads the page it
//! draws from its own address. The server holds nothing of Lory's: what it
//! serves is read from the folder at each request, as a web build is read
//! from disk. An HTML page is not touched: `/_drawn` is a path of its own.

use axum::response::IntoResponse;

/// The folder the drawn page's files are served from: `CROW_DRAWN`, else
/// `~/.crow/drawn` where it exists; none where neither names one.
pub fn folder() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("CROW_DRAWN") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    let home = std::env::var("HOME").ok()?;
    let dir = std::path::Path::new(&home).join(".crow").join("drawn");
    dir.is_dir().then_some(dir)
}

/// What an HTML page carries so a reader in a headset can step into its
/// drawn form: the drawn host's own `enter.js` (Lory's: it shows a button
/// where the browser can open an immersive session, leading to
/// `/_drawn<this page>`), named only where the folder holds one. Nothing
/// where no drawn page is served, so a page is as it was.
pub fn entry() -> &'static str {
    match folder() {
        Some(dir) if dir.join("enter.js").is_file() => "<script src=\"/_drawn/enter.js\" defer></script>",
        _ => "",
    }
}

/// The file a path under `/_drawn/` names in the folder: none for a path
/// that climbs out of it, hides (a segment starting with a dot), or names
/// no file — a page's path.
fn file_of(dir: &std::path::Path, rest: &str) -> Option<std::path::PathBuf> {
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }
    let mut at = dir.to_path_buf();
    for segment in rest.split('/') {
        if segment.is_empty() || segment == ".." || segment.starts_with('.') || segment.contains('\\') {
            return None;
        }
        at.push(segment);
    }
    at.is_file().then_some(at)
}

/// A file's media type, by its extension.
fn kind_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// What `/_drawn/<rest>` answers: the folder's file, or its page.
async fn answer(rest: String) -> axum::response::Response {
    let Some(dir) = folder() else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            "no drawn page here: CROW_DRAWN names the folder of Lory's drawn host (lory/packages/drawn/web), or link it at ~/.crow/drawn",
        )
            .into_response();
    };
    let path = file_of(&dir, &rest).unwrap_or_else(|| dir.join("index.html"));
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, kind_of(&path)), (axum::http::header::CACHE_CONTROL, "no-cache")],
            bytes,
        )
            .into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, format!("no {} in the drawn host's folder {}", path.display(), dir.display())).into_response(),
    }
}

/// The routes of the drawn page, merged into every served app.
pub fn router() -> axum::Router {
    axum::Router::new()
        .route("/_drawn", axum::routing::get(|| async { axum::response::Redirect::to("/_drawn/") }))
        .route("/_drawn/", axum::routing::get(|| answer(String::new())))
        .route(
            "/_drawn/{*rest}",
            axum::routing::get(|axum::extract::Path(rest): axum::extract::Path<String>| answer(rest)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path names a file of the folder only inside it and not hidden;
    /// a page's path names none (the folder's page answers it).
    #[test]
    fn a_path_names_a_file_of_the_folder_or_the_page() {
        let dir = std::env::temp_dir().join(format!("crow_drawn_test_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::write(dir.join("index.html"), "page").unwrap();
        std::fs::write(dir.join("build").join("main.wasm"), "wasm").unwrap();
        std::fs::write(dir.join(".secret"), "no").unwrap();
        assert_eq!(file_of(&dir, "build/main.wasm"), Some(dir.join("build").join("main.wasm")));
        assert_eq!(file_of(&dir, "index.html"), Some(dir.join("index.html")));
        assert_eq!(file_of(&dir, "execution"), None, "a page's path is the folder's page");
        assert_eq!(file_of(&dir, ""), None);
        assert_eq!(file_of(&dir, ".secret"), None, "a hidden file is not served");
        assert_eq!(file_of(&dir, "../etc/passwd"), None, "a path does not climb out");
        assert_eq!(file_of(&dir, "build/../index.html"), None);
        assert_eq!(kind_of(std::path::Path::new("a.wasm")), "application/wasm");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
