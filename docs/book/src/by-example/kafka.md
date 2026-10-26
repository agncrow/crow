# The Kafka door: `use kafka`

```crow
use kafka
use frame

gone = rm['ex_kafka']
served = serve['ex_kafka', 0, rounds = 3]
log[served]
log[exists['ex_kafka']]
gone = rm['ex_kafka']
```

```text
0
true
```

`serve[root, port]` is the door: a Kafka client that connects to the port
is spoken to in the protocol a 0.10.1 broker speaks, its records landing
in the lake's logs under the root, one log per topic. The block above
runs the door's loop three rounds with nobody connecting; a producer and a
consumer group driven against a running door are tests/kafka.rs's, with
the pure-Python client. Reference: spec section 13 (kafka).
