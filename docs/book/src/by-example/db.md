# Databases: `use db`

```crow
use db

conn = connect['sqlite://:memory:']
exec[conn, 'CREATE TABLE t (k TEXT, v INTEGER)']
exec[conn, 'INSERT INTO t VALUES (?, ?)', ['a', 1]]
exec[conn, 'INSERT INTO t VALUES (?, ?)', ['b', 2]]
rows = query[conn, 'SELECT k, v FROM t ORDER BY k']
log[rows.k]
log[rows.v]

fn add_c[tx]
    exec[tx, 'INSERT INTO t VALUES (?, ?)', ['c', 3]]

transact[conn, add_c]
log[len[query[conn, 'SELECT k FROM t']]]
log[describe[conn, 'SELECT k, v FROM t']]

--- a function that fails rolls its transaction back, and transact
--- answers its error
fn add_d_then_fail[tx]
    exec[tx, 'INSERT INTO t VALUES (?, ?)', ['d', 4]]
    return error['not today', 'db']

r = transact[conn, add_d_then_fail]
log[r.ok]
log[r.reason]
log[len[query[conn, 'SELECT k FROM t']]]

--- bulk-load a frame, and apply .sql migrations from a directory
use frame
write['ex_rows.csv', 'k,v\nd,4\ne,5\n']
more = frame.load['ex_rows.csv']
log[insert[conn, 't', more]]
log[len[query[conn, 'SELECT k FROM t']]]
mkdir['ex_migrations']
write['ex_migrations/001_audit.sql', 'CREATE TABLE audit (note TEXT)']
log[migrate[conn, 'ex_migrations']]
log[migrate[conn, 'ex_migrations']]
rm['ex_rows.csv']
rm['ex_migrations']
```

```text
['a', 'b']
[1, 2]
3
name  kind
 str   str
----  ----
   k   str
   v   int

[2 rows x 2 cols]
false
not today
3
2
5
1
0
```

`transact` answers what its function answers, committed — or the error of
a function that failed, its transaction rolled back. `insert` uses each
backend's bulk path (COPY, RowBinary, batched
statements) and returns the row count; `migrate` applies new files in name
order, records them in `_crow_migrations`, and returns how many it ran —
zero the second time.

Reference: spec section 13 (db) and book chapter 11.
