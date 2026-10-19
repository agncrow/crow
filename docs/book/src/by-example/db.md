# Databases: `use db`

```crow
use db

conn = connect['sqlite://:memory:']
query[conn, 'CREATE TABLE t (k TEXT, v INTEGER)']
query[conn, 'INSERT INTO t VALUES (?, ?)', ['a', 1]]
query[conn, 'INSERT INTO t VALUES (?, ?)', ['b', 2]]
rows = query[conn, 'SELECT k, v FROM t ORDER BY k']
log[rows.k]
log[rows.v]

fn add_c[tx]
    query[tx, 'INSERT INTO t VALUES (?, ?)', ['c', 3]]

transact[conn, add_c]
log[len[query[conn, 'SELECT k FROM t']]]
log[describe[conn, 'SELECT k, v FROM t']]

--- a function that fails rolls its transaction back, and transact
--- answers its error
fn add_d_then_fail[tx]
    query[tx, 'INSERT INTO t VALUES (?, ?)', ['d', 4]]
    return error['not today', 'db']

r = transact[conn, add_d_then_fail]
log[r.ok]
log[r.reason]
log[len[query[conn, 'SELECT k FROM t']]]

--- bulk-load a frame
use frame
write['ex_rows.csv', 'k,v\nd,4\ne,5\n']
more = frame.load['ex_rows.csv']
log[insert[conn, 't', more]]
log[len[query[conn, 'SELECT k FROM t']]]
rm['ex_rows.csv']
```

```text
['a', 'b']
[1, 2]
3
name  type
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
```

`transact` answers what its function answers, committed — or the error of
a function that failed, its transaction rolled back. `insert` uses each
backend's bulk path (COPY, RowBinary, batched
statements) and returns the row count. Migrations are the tool's: `crow db
apply <dir> <url>`.

Reference: spec section 13 (db) and book chapter 11.
