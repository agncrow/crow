"""Two consumers of one group against a Crow door, and a gzip producer:
the producer sends six records compressed; the two consumers join the
same group (the second joins while the first runs, so the first hears of
the rebalance and rejoins), and between them read every record exactly
once; each commits. Prints what was seen; exits non-zero on a mismatch.
Usage: driver2.py <bootstrap host:port>"""
import sys, threading, time
from kafka import KafkaProducer, KafkaConsumer

bootstrap = sys.argv[1]
producer = KafkaProducer(bootstrap_servers=bootstrap, compression_type="gzip", request_timeout_ms=10000, max_block_ms=15000)
for i in range(6):
    producer.send("zipped", key=b"k%d" % i, value=b"v%d" % i).get(timeout=15)
producer.flush()
producer.close()
print("produced 6, gzip")

seen = {}
lock = threading.Lock()

def read(name, delay):
    time.sleep(delay)
    c = KafkaConsumer("zipped", bootstrap_servers=bootstrap, group_id="pair", auto_offset_reset="earliest", enable_auto_commit=False, consumer_timeout_ms=6000, request_timeout_ms=10000, session_timeout_ms=6000, heartbeat_interval_ms=1000)
    got = []
    for m in c:
        got.append((m.offset, m.value))
        c.commit()
    with lock:
        seen[name] = got
    c.close()

a = threading.Thread(target=read, args=("a", 0.0))
b = threading.Thread(target=read, args=("b", 2.0))
a.start(); b.start(); a.join(); b.join()
all_values = sorted(v for got in seen.values() for _, v in got)
print("consumer a read", len(seen.get("a", [])), "consumer b read", len(seen.get("b", [])))
ok = all_values == [b"v%d" % i for i in range(6)]
print("OK" if ok else "MISMATCH", all_values)
sys.exit(0 if ok else 1)
