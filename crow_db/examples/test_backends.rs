//! Quick integration test for ClickHouse and PostgreSQL backends.
//! Run: cargo run --example test_backends --features "clickhouse,postgres,sqlite" -p crow_db

use crow_db::{connect, CrowDb};

fn main() {
    test_clickhouse();
    println!();
    test_postgres();
}

fn test_clickhouse() {
    println!("=== Testing ClickHouse (native TCP, port 9000) ===");
    match connect("clickhouse://default@127.0.0.1:9000/default", None) {
        Ok(handle) => {
            println!("  Connected to ClickHouse");

            match handle.query("SELECT 1 AS num, 'hello' AS msg", &[]) {
                Ok(frame) => println!("  Query OK: {} rows, cols: {:?}", frame.nrows(), frame.columns),
                Err(e) => println!("  Query FAIL: {}", e),
            }

            match handle.exec("CREATE TABLE IF NOT EXISTS _crow_test (id UInt32, name String) ENGINE = Memory", &[]) {
                Ok(n) => println!("  CREATE TABLE OK ({})", n),
                Err(e) => println!("  CREATE TABLE FAIL: {}", e),
            }

            match handle.exec("INSERT INTO _crow_test VALUES (1, 'alice'), (2, 'bob')", &[]) {
                Ok(n) => println!("  INSERT OK ({})", n),
                Err(e) => println!("  INSERT FAIL: {}", e),
            }

            match handle.query("SELECT * FROM _crow_test ORDER BY id", &[]) {
                Ok(frame) => {
                    println!("  SELECT * OK: {} rows, cols: {:?}", frame.nrows(), frame.columns);
                    for col_idx in 0..frame.ncols() {
                        println!("    {}: {:?}", frame.columns[col_idx], frame.data[col_idx]);
                    }
                }
                Err(e) => println!("  SELECT * FAIL: {}", e),
            }

            let _ = handle.exec("DROP TABLE IF EXISTS _crow_test", &[]);

            match handle.transact(&|_tx| Ok(())) {
                Ok(()) => println!("  transact: unexpected success"),
                Err(e) => println!("  transact correctly refused: {}", e),
            }
        }
        Err(e) => println!("  Connection FAIL: {}", e),
    }
}

fn test_postgres() {
    println!("=== Testing PostgreSQL (port 5432) ===");
    match connect("postgres://localhost:5432/postgres", None) {
        Ok(handle) => {
            println!("  Connected to PostgreSQL");

            match handle.query("SELECT 1 AS num, 'hello' AS msg", &[]) {
                Ok(frame) => println!("  Query OK: {} rows, cols: {:?}", frame.nrows(), frame.columns),
                Err(e) => println!("  Query FAIL: {}", e),
            }

            match handle.exec("CREATE TABLE IF NOT EXISTS _crow_test (id INTEGER, name TEXT)", &[]) {
                Ok(n) => println!("  CREATE TABLE OK ({})", n),
                Err(e) => println!("  CREATE TABLE FAIL: {}", e),
            }

            match handle.exec("INSERT INTO _crow_test (id, name) VALUES ($1, $2)", &[
                serde_json::json!(1), serde_json::json!("alice"),
            ]) {
                Ok(n) => println!("  INSERT OK ({})", n),
                Err(e) => println!("  INSERT FAIL: {}", e),
            }

            match handle.exec("INSERT INTO _crow_test (id, name) VALUES ($1, $2)", &[
                serde_json::json!(2), serde_json::json!("bob"),
            ]) {
                Ok(n) => println!("  INSERT OK ({})", n),
                Err(e) => println!("  INSERT FAIL: {}", e),
            }

            match handle.query("SELECT * FROM _crow_test ORDER BY id", &[]) {
                Ok(frame) => {
                    println!("  SELECT * OK: {} rows, cols: {:?}", frame.nrows(), frame.columns);
                    for col_idx in 0..frame.ncols() {
                        println!("    {}: {:?}", frame.columns[col_idx], frame.data[col_idx]);
                    }
                }
                Err(e) => println!("  SELECT * FAIL: {}", e),
            }

            match handle.transact(&|_tx| Ok(())) {
                Ok(()) => println!("  transact OK"),
                Err(e) => println!("  transact FAIL: {}", e),
            }

            let _ = handle.exec("DROP TABLE IF EXISTS _crow_test", &[]);
        }
        Err(e) => println!("  Connection FAIL: {}", e),
    }
}

