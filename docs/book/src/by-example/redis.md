# The RESP door: `use redis`

```crow
use redis

gone = rm['ex_redis']
made = mkdir['ex_redis']
served = serve['ex_redis', 0, rounds = 3]
log[served]
gone = rm['ex_redis']
```

```text
0
```

`serve[root, port]` is the door: a Redis client that connects to the
port speaks RESP2 as redis-cli does, and the door answers from Magpie —
`SET` a put into a log under the root keyed by the key, `GET` a read of
the sequencer's image in memory, `INCR`, `MGET`, `DEL`, `KEYS` and
`DBSIZE` over the same log; `XADD` a put into a stream's own log, the
entry's id its position, `XRANGE` and `XREAD` reads by position. The
block above runs the door's loop three rounds with nobody connecting; a
client's session — `redis-cli -p 6379`, or any Redis library — is
tests/redis_door.rs, driven in both modes. Reference: spec section 13
(redis).
