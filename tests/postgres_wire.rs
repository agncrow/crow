//! The Postgres driver written in Crow (docs/db-in-crow-design.md), in
//! both modes, against a scratch cluster of the test's own: a query's rows
//! typed as the server names them, parameters typed by their kinds, a
//! command's count, a frame copied in, `describe`, the server's refusal in
//! its own words, `transact` committing and rolling back, and a declared
//! column added as the connection opens (the advisory lock); SCRAM-SHA-256
//! and md5 beside trust. A machine without Postgres's binaries skips it.

use std::process::Command;

mod common;
use common::Scratch;

const PROGRAM: &str = "use db, frame

port = args[][0]
user = args[][1]
conn = connect['postgres://`user`:pw@127.0.0.1:`port`/postgres']
rows = query[conn, 'SELECT n AS id, n::float8 / 4 AS x, n::text AS s, (n = 2) AS even, now() > now() - interval \\'1 day\\' AS fresh FROM generate_series(1, 3) AS n']
log[rows]
log[int[rows.id[2]] * 2]
typed = query[conn, 'SELECT ?::int8 + 1 AS n, ?::text AS t, ? AS f, ? AS b, NULL::int8 AS gone', [41, 'hi', 2.5, true]]
log[typed]
made = query[conn, 'CREATE TABLE IF NOT EXISTS wire (k TEXT, v INT, w FLOAT8)']
log[len[made]]
log[len[query[conn, 'DELETE FROM wire RETURNING k']]]
log[len[query[conn, 'INSERT INTO wire (k, v) VALUES (?, ?), (?, ?) RETURNING k', ['a', 1, 'b', 2]]]]
log[len[query[conn, 'UPDATE wire SET v = v + 1 RETURNING k']]]
f = frame.new[k = ['c', 'd,e'], v = [3, 4], w = [0.5, 1.5]]
log[insert[conn, 'wire', f]]
back = query[conn, 'SELECT k, v, w FROM wire ORDER BY k']
log[back]
log[describe[conn, 'SELECT k, v, w, now() AS t FROM wire WHERE v = ?']]
bad = query[conn, 'SELECT nope FROM wire']
log[bad.ok]
log[bad.reason]
fn double[tx]
    query[tx, 'UPDATE wire SET v = v * 2']
transact[conn, double]
fn fails[tx]
    query[tx, 'UPDATE wire SET v = 100']
    return error['no', 'db']
r = transact[conn, fails]
log[r.ok]
log[query[conn, 'SELECT sum(v) AS total FROM wire']]
";

#[test]
fn the_postgres_driver_written_in_crow_speaks_to_a_cluster_in_both_modes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_postgres_wire");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let Some(db) = Scratch::new(dir.join("cluster")) else { return };
    // roles authenticated by SCRAM-SHA-256 and by md5, ahead of trust for everyone else
    let hba = db.dir.join("pg_hba.conf");
    let was = std::fs::read_to_string(&hba).unwrap();
    std::fs::write(&hba, format!("host all scram 127.0.0.1/32 scram-sha-256\nhost all md5u 127.0.0.1/32 md5\n{}", was)).unwrap();
    db.stop();
    db.start();
    assert!(db.psql("SET password_encryption = 'scram-sha-256'; CREATE ROLE scram WITH LOGIN PASSWORD 'pw'; SET password_encryption = 'md5'; CREATE ROLE md5u WITH LOGIN PASSWORD 'pw'; GRANT ALL ON SCHEMA public TO scram, md5u"), "the roles");
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    for user in ["crow", "scram", "md5u"] {
        // the table as an older program left it: the program's declaration
        // adds `w` as the connection opens (crow runs from source under
        // `reconcile`), through the advisory lock, as the user who owns it
        let mut said: Vec<String> = Vec::new();
        for mode in [&["run", "-i"][..], &["run"][..]] {
            assert!(db.psql(&format!("DROP TABLE IF EXISTS wire; SET ROLE {}; CREATE TABLE wire (k TEXT, v INT)", user)), "the table for {}", user);
            let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(db.port.to_string()).arg(user).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            assert!(out.status.success(), "{} {:?} failed: {}\n{}", user, mode, stdout, stderr);
            assert!(stdout.contains("[3 rows x 5 cols]\n6\n"), "the rows, typed\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("42  hi  2.5  true"), "typed parameters\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("[1 rows x 5 cols]\n0\n0\n2\n2\n2\n"), "a CREATE answers no rows; the rows a DELETE, an INSERT and an UPDATE read back; a COPY's count\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("d,e"), "a copied cell with a comma\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("[4 rows x 3 cols]\n"), "the rows copied in beside the rows inserted\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("datetime"), "describe types the server's columns\n{} {:?}: {}", user, mode, stdout);
            // (and marks those that can hold NULL, as the compiler reads them:
            // `k` and `w` are declared without NOT NULL — it had marked none)
            let described = |name: &str, kind: &str| stdout.lines().any(|l| l.trim_start().starts_with(&format!("{} ", name)) && l.trim_end().ends_with(kind));
            assert!(described("k", "str?") && described("w", "float?"), "describe marks the columns that can hold NULL\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.contains("\nfalse\npostgres query error: column \"nope\" does not exist\n"), "the server's words\n{} {:?}: {}", user, mode, stdout);
            // a: 2, b: 3, c: 3, d,e: 4, doubled by the transaction that committed; the one that failed rolled back
            assert!(stdout.contains("\nfalse\ntotal\n"), "the failed transaction's value\n{} {:?}: {}", user, mode, stdout);
            assert!(stdout.ends_with("   24\n\n[1 rows x 1 cols]\n"), "transactions\n{} {:?}: {}", user, mode, stdout);
            said.push(stdout);
        }
        assert_eq!(said[0], said[1], "the modes agree for {}", user);
    }
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}
