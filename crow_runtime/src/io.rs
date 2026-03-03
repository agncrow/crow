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
}

