//! Forward-only SQL migrations.
//!
//! Reads `.sql` files from a directory, sorts by filename, executes new ones.
//! Tracks applied migrations in a `_crow_migrations` table.

use crate::CrowDb;

const CREATE_MIGRATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS _crow_migrations (
    name TEXT PRIMARY KEY,
    applied_at TEXT DEFAULT CURRENT_TIMESTAMP
)";

/// Run migrations from a directory. Returns count of newly applied migrations.
pub fn run_migrations(db: &dyn CrowDb, dir: &str) -> Result<i64, String> {
    // Create tracking table
    db.exec(CREATE_MIGRATIONS_TABLE, &[])?;

    // Read and sort migration files
    let mut entries: Vec<String> = Vec::new();
    let read_dir = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read migrations directory '{}': {}", dir, e))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                entries.push(name.to_string());
            }
        }
    }
    entries.sort();

    // Get already-applied migrations
    let applied_frame = db.query("SELECT name FROM _crow_migrations", &[])?;
    let mut applied: std::collections::HashSet<String> = std::collections::HashSet::new();
    if applied_frame.nrows() > 0 {
        if let Some(col) = applied_frame.col("name") {
            match col {
                crow_runtime::frame::Column::Str(names) => {
                    for name in names {
                        applied.insert(name.clone());
                    }
                }
                _ => {}
            }
        }
    }

    // Apply new migrations
    let mut count = 0i64;
    for name in &entries {
        if applied.contains(name) {
            continue;
        }

        let path = std::path::Path::new(dir).join(name);
        let sql = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read migration '{}': {}", name, e))?;

        db.exec(&sql, &[])?;
        db.exec(
            "INSERT INTO _crow_migrations (name) VALUES (?)",
            &[serde_json::json!(name)],
        )?;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_migrations() {
        use crate::sqlite::CrowSqliteDb;

        // Create temp dir with migration files
        let dir = std::env::temp_dir().join("crow_test_migrations");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("001_create_users.sql"),
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
        ).unwrap();
        std::fs::write(
            dir.join("002_add_email.sql"),
            "ALTER TABLE users ADD COLUMN email TEXT",
        ).unwrap();

        let db = CrowSqliteDb::memory().unwrap();
        let count = run_migrations(&db, dir.to_str().unwrap()).unwrap();
        assert_eq!(count, 2);

        // Run again — should skip already applied
        let count2 = run_migrations(&db, dir.to_str().unwrap()).unwrap();
        assert_eq!(count2, 0);

        // Verify table exists with columns
        db.exec("INSERT INTO users (name, email) VALUES ('alice', 'alice@example.com')", &[]).unwrap();
        let frame = db.query("SELECT * FROM users", &[]).unwrap();
        assert_eq!(frame.nrows(), 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}

