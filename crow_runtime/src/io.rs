/// Crow io module runtime — file reading, writing, CSV parsing.

use std::collections::BTreeMap;

/// Read entire file contents as a string.
pub fn read_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("read: {}", e))
}

/// Write string content to a file (creates or overwrites).
pub fn write_file(path: &str, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|e| format!("write: {}", e))
}

/// Append string content to a file (creates if doesn't exist).
pub fn append_file(path: &str, content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("append: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("append: {}", e))
}

// ── The core file builtins, once: read, write, exists, mkdir, ls, rm, mv,
// cp. Both modes call these; the reason texts below are the ones a
// program sees in `.reason`, so they live here and nowhere else.

// ── the effects a program calls ─────────────────────────────────────
// One function each, the row both modes call (docs/bug-classes.md §4). A
// file operation that answers nothing answers empty text, as its type
// says (the interpreter's `done` beside the compiled program's `""`).

/// `exists[path]`.
pub fn exists(path: &str) -> bool {
    fs_exists(path)
}

/// `mkdir[path]`, its parents too.
pub fn mkdir(path: &str) -> Result<String, String> {
    fs_mkdir(path).map(|_| String::new())
}

/// `ls[path]`: the names in a directory, in order.
pub fn ls(path: &str) -> Result<Vec<String>, String> {
    fs_ls(path)
}

/// `rm[path]`: a file, or a directory and what it holds.
pub fn rm(path: &str) -> Result<String, String> {
    // what the process keeps for the path goes with it (memory names a
    // path's records by the path): a log's pending rows are dropped, not
    // written into the directory a moment later
    crate::memory::forget_path(path);
    fs_rm(path).map(|_| String::new())
}

/// `mv[from, to]`.
pub fn mv(src: &str, dst: &str) -> Result<String, String> {
    fs_mv(src, dst).map(|_| String::new())
}

/// `cp[from, to]`.
pub fn cp(src: &str, dst: &str) -> Result<String, String> {
    fs_cp(src, dst).map(|_| String::new())
}

pub fn fs_read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {}", path, e))
}

/// `write[path, content]`: create or overwrite.
pub fn fs_write(path: &str, content: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|e| format!("cannot write '{}': {}", path, e))
}

/// `exists[path]`.
pub fn fs_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// `mkdir[path]`: every missing directory on the way; an existing one is fine.
pub fn fs_mkdir(path: &str) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("cannot mkdir '{}': {}", path, e))
}

/// `ls[path]`: the entry names, sorted; an entry that cannot be read is
/// an error, not a gap.
pub fn fs_ls(path: &str) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(path).map_err(|e| format!("cannot ls '{}': {}", path, e))?;
    let mut names = Vec::new();
    for entry in entries {
        let e = entry.map_err(|e| format!("ls '{}': {}", path, e))?;
        names.push(e.file_name().to_string_lossy().to_string());
    }
    names.sort();
    Ok(names)
}

/// `rm[path]`: a file, or a directory with everything in it.
pub fn fs_rm(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    r.map_err(|e| format!("cannot rm '{}': {}", path, e))
}

/// `mv[src, dst]`.
pub fn fs_mv(src: &str, dst: &str) -> Result<(), String> {
    std::fs::rename(src, dst).map_err(|e| format!("cannot mv '{}' to '{}': {}", src, dst, e))
}

/// `cp[src, dst]`.
pub fn fs_cp(src: &str, dst: &str) -> Result<(), String> {
    std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("cannot cp '{}' to '{}': {}", src, dst, e))
}

/// Parse CSV text into a Vec of BTreeMap<String, String>: the first row
/// is the headers, each row after it a map.
pub fn parse_csv_string(content: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let mut lines = content.lines();
    let header_line = lines.next().ok_or_else(|| "csv: empty file".to_string())?;
    let headers: Vec<&str> = header_line.split(',').map(|s| s.trim()).collect();
    if headers.is_empty() {
        return Err("csv: no headers found".to_string());
    }
    let mut rows = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() { continue; }
        let values: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        let mut row = BTreeMap::new();
        for (i, header) in headers.iter().enumerate() {
            let val = values.get(i).unwrap_or(&"");
            row.insert(header.to_string(), val.to_string());
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Fast numeric CSV loader — parses CSV directly into a flat Vec<f64>.
/// Returns (data, nrows, ncols). Skips the header row.
/// All values must be parseable as f64.

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_file_builtins_round_trip_and_name_the_path_in_errors() {
        let dir = std::env::temp_dir().join(format!("crow_fs_{}", std::process::id()));
        let d = dir.to_string_lossy().to_string();
        fs_mkdir(&d).unwrap();
        fs_mkdir(&d).unwrap();
        let a = format!("{}/a.txt", d);
        let b = format!("{}/b.txt", d);
        let c = format!("{}/c.txt", d);
        fs_write(&a, "hello").unwrap();
        assert!(fs_exists(&a));
        assert_eq!(fs_read(&a).unwrap(), "hello");
        fs_cp(&a, &b).unwrap();
        fs_mv(&b, &c).unwrap();
        assert!(!fs_exists(&b));
        assert_eq!(fs_ls(&d).unwrap(), vec!["a.txt".to_string(), "c.txt".to_string()]);
        let missing = format!("{}/missing.txt", d);
        let e = fs_read(&missing).unwrap_err();
        assert!(e.starts_with(&format!("cannot read '{}': ", missing)), "{}", e);
        assert!(fs_rm(&missing).unwrap_err().starts_with("cannot rm '"));
        assert!(fs_ls(&missing).unwrap_err().starts_with("cannot ls '"));
        assert!(fs_mv(&missing, &b).unwrap_err().starts_with("cannot mv '"));
        assert!(fs_cp(&missing, &b).unwrap_err().starts_with("cannot cp '"));
        fs_rm(&d).unwrap();
        assert!(!fs_exists(&d));
    }

    fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("crow_io_test_{}", name));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_read_file() {
        let p = temp_file("read.txt", "hello world");
        let result = read_file(p.to_str().unwrap());
        assert_eq!(result.unwrap(), "hello world");
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_read_file_not_found() {
        let result = read_file("/tmp/crow_io_test_nonexistent_12345.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_write_file() {
        let p = std::env::temp_dir().join("crow_io_test_write.txt");
        write_file(p.to_str().unwrap(), "test content").unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert_eq!(content, "test content");
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_append_file() {
        let p = std::env::temp_dir().join("crow_io_test_append.txt");
        let _ = std::fs::remove_file(&p);
        append_file(p.to_str().unwrap(), "first").unwrap();
        append_file(p.to_str().unwrap(), " second").unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert_eq!(content, "first second");
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_parse_csv_string() {
        let csv = "name,age,city\nalice,30,nyc\nbob,25,sf\n";
        let rows = parse_csv_string(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "alice");
        assert_eq!(rows[0]["age"], "30");
        assert_eq!(rows[0]["city"], "nyc");
        assert_eq!(rows[1]["name"], "bob");
        assert_eq!(rows[1]["age"], "25");
    }

    #[test]
    fn test_parse_csv_empty() {
        let result = parse_csv_string("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_csv_header_only() {
        let rows = parse_csv_string("a,b,c\n").unwrap();
        assert_eq!(rows.len(), 0);
    }
}

