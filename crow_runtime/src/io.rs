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

/// `read[path]`: the file's text.
// ── the effects a program calls ─────────────────────────────────────
// One function each, the row both modes call (docs/bug-classes.md §4). A
// file operation that answers nothing answers empty text, as its type
// says (the interpreter's `done` beside the compiled program's `""`).

/// `read[path]`: the file's text, or a URL's, or an S3 object's
/// (crate::sources).
pub fn read(path: &str) -> Result<String, String> {
    if crate::sources::is_remote(path) {
        return crate::sources::fetch_text(path);
    }
    fs_read(path)
}

/// `write[path, text]`: a file, or an S3 object.
pub fn write(path: &str, content: &str) -> Result<String, String> {
    if crate::sources::is_remote(path) {
        return crate::sources::put(path, content.as_bytes()).map(|_| String::new());
    }
    fs_write(path, content).map(|_| String::new())
}

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

/// `io.csv[path]`: each row a map from its column's name to its text.
pub fn csv(path: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    if crate::sources::is_remote(path) {
        return parse_csv_string(&crate::sources::fetch_text(path)?);
    }
    parse_csv(path)
}

/// `io.lines[path]`.
pub fn lines(path: &str) -> Result<Vec<String>, String> {
    if crate::sources::is_remote(path) {
        return Ok(crate::sources::fetch_text(path)?.lines().map(str::to_string).collect());
    }
    read_lines(path)
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

/// Parse a CSV file into a Vec of BTreeMap<String, String>.
/// First row is treated as headers. Each subsequent row becomes a map.
pub fn parse_csv(path: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("csv: {}", e))?;
    parse_csv_string(&content)
}

/// Parse CSV from a string (used internally and for testing).
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
pub fn load_csv_numeric(path: &str) -> Result<(Vec<f64>, usize, usize), String> {
    use std::io::{BufRead, BufReader};
    use std::fs::File;

    let file = File::open(path).map_err(|e| format!("load: {}", e))?;
    let reader = BufReader::with_capacity(8 * 1024 * 1024, file); // 8MB buffer
    let mut lines = reader.lines();

    // Read header to determine column count
    let header = lines.next()
        .ok_or_else(|| "load: empty file".to_string())?
        .map_err(|e| format!("load: {}", e))?;
    let ncols = header.split(',').count();
    if ncols == 0 {
        return Err("load: no columns found".to_string());
    }

    // Pre-allocate with estimate (assume ~100 bytes per row for 10 numeric cols)
    let file_size = std::fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0);
    let est_rows = file_size / (ncols * 12); // rough estimate
    let mut data = Vec::with_capacity(est_rows * ncols);
    let mut nrows = 0;

    for line_result in lines {
        let line = line_result.map_err(|e| format!("load: {}", e))?;
        let line = line.trim();
        if line.is_empty() { continue; }

        for (i, field) in line.split(',').enumerate() {
            if i >= ncols { break; }
            let val: f64 = field.trim().parse()
                .map_err(|_| format!("load: cannot parse '{}' as number at row {}, col {}", field.trim(), nrows + 1, i))?;
            data.push(val);
        }
        nrows += 1;
    }

    Ok((data, nrows, ncols))
}

/// Read lines from a file, returning them as a Vec<String>.
pub fn read_lines(path: &str) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("lines: {}", e))?;
    Ok(content.lines().map(|l| l.to_string()).collect())
}

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

    #[test]
    fn test_parse_csv_file() {
        let p = temp_file("data.csv", "x,y\n1,2\n3,4\n");
        let rows = parse_csv(p.to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["x"], "1");
        assert_eq!(rows[1]["y"], "4");
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_read_lines() {
        let p = temp_file("lines.txt", "line1\nline2\nline3");
        let lines = read_lines(p.to_str().unwrap()).unwrap();
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
        std::fs::remove_file(p).ok();
    }

    // ===== load_csv_numeric tests =====

    #[test]
    fn test_load_csv_numeric_basic() {
        let p = temp_file("numeric.csv", "a,b,c\n1.0,2.0,3.0\n4.0,5.0,6.0\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 2);
        assert_eq!(ncols, 3);
        assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_integers() {
        let p = temp_file("ints.csv", "x,y\n10,20\n30,40\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 2);
        assert_eq!(ncols, 2);
        assert_eq!(data, vec![10.0, 20.0, 30.0, 40.0]);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_single_row() {
        let p = temp_file("single.csv", "a,b\n7.5,8.5\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 1);
        assert_eq!(ncols, 2);
        assert_eq!(data, vec![7.5, 8.5]);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_single_column() {
        let p = temp_file("onecol.csv", "val\n1\n2\n3\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 3);
        assert_eq!(ncols, 1);
        assert_eq!(data, vec![1.0, 2.0, 3.0]);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_empty_file() {
        let p = temp_file("empty.csv", "");
        let result = load_csv_numeric(p.to_str().unwrap());
        assert!(result.is_err());
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_header_only() {
        let p = temp_file("headeronly.csv", "a,b,c\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 0);
        assert_eq!(ncols, 3);
        assert!(data.is_empty());
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_bad_value() {
        let p = temp_file("bad.csv", "a,b\n1.0,hello\n");
        let result = load_csv_numeric(p.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot parse"));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_not_found() {
        let result = load_csv_numeric("/tmp/crow_io_test_nonexistent_99999.csv");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_csv_numeric_negative_values() {
        let p = temp_file("neg.csv", "x,y\n-1.5,2.5\n3.0,-4.0\n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 2);
        assert_eq!(ncols, 2);
        assert_eq!(data, vec![-1.5, 2.5, 3.0, -4.0]);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn test_load_csv_numeric_whitespace() {
        let p = temp_file("ws.csv", "a , b\n 1.0 , 2.0 \n 3.0 , 4.0 \n");
        let (data, nrows, ncols) = load_csv_numeric(p.to_str().unwrap()).unwrap();
        assert_eq!(nrows, 2);
        assert_eq!(ncols, 2);
        assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0]);
        std::fs::remove_file(p).ok();
    }
}

