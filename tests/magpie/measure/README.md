# Magpie's measures, on small sets

The programs behind "Measured 2026-10-27, on small sets" in
docs/magpie-design.md and the article's measured results. Each prints
its own numbers; run them one at a time on a quiet machine, interpreted
(`crow run -i`; the kernels do the work either way). Every number is end
to end in the program, from the call to the value in hand, with no
result serialization: the first run cold, then the median of the warm
runs where a program says `REPS`.

| Program | What it measures | Needs |
|---|---|---|
| `vectors.co` | `nearest` over `N` vectors of width `W` from the files, against `packages/vectors`'s flat index over the same matrix in memory; the positions and the cosines must agree | nothing |
| `graph.co` | a hop over `N` nodes with `D` out-edges each: `neighbors`, `walk` of one to three hops (`packages/kg`) | nothing |
| `words.co` | a common word's top 10 over `N` texts: the batches scanned, then the merged segment's index | nothing |
| `wide.co` | a log of `N` rows by `C` float columns: the write, the merge, a scan of two columns, a lookup, a summary | nothing |
| `seq.co` | `put`, `latest` of one key and a durable `batch`, 2,000 each, beside Postgres's `INSERT` and `SELECT` by primary key (`PG`, a scratch database); Redis is asked by `redis-benchmark -p 6399 -t set,get -n 2000 -c 1 -q` beside it | Postgres; `redis-server --port 6399 --save '' --appendonly no` |
| `copy_month.co` | finch's September 2026 copied out of ClickHouse and Postgres into a Magpie log under `ROOT`, a batch a day, each log merged | finch's ClickHouse (native port 9000) and Postgres |
| `finch.co` | finch's five questions against ClickHouse as finch runs it (`clickhouse://localhost:8123/finch_trading`) and against the log `ROOT`, from the files and over rows held, five times each; the answers checked against ClickHouse's | `copy_month.co` first |

Run from this directory, so `use kg` and `use vectors.flat` resolve
through `crow_modules/`:

```
crow run -i copy_month.co
N=20000 W=64 crow run -i vectors.co
N=100000 W=64 crow run -i vectors.co
N=5000 D=10 crow run -i graph.co
N=100000 crow run -i words.co
N=200000 C=100 crow run -i wide.co
redis-server --port 6399 --save '' --appendonly no &
N=2000 crow run -i seq.co
redis-benchmark -p 6399 -t set,get -n 2000 -c 1 -q
crow run -i finch.co
```

The numbers recorded on 2026-10-27 were taken on an Apple M4 Max with
64 GB, crow 0.7.0 at the commit that added this directory, Postgres 14
and Redis 8 from Homebrew, ClickHouse as finch runs it (143,000 active
parts). Small sets teach mechanisms, not rankings: the sizes are a few
hundred thousand rows, where every structure fits in memory, and no
number here says what a billion rows would.
