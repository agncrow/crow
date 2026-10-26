"""Drive the pure-Python Kafka client against a Crow door: a producer
sends three records to a topic, a consumer in a group reads them from
the start, commits, and a second consumer of the same group takes up
from the committed offset. Prints what it saw; exits non-zero on a
mismatch. Usage: driver.py <bootstrap host:port>"""
import sys
from kafka import KafkaProducer, KafkaConsumer, TopicPartition

bootstrap = sys.argv[1]
producer = KafkaProducer(bootstrap_servers=bootstrap, request_timeout_ms=10000, max_block_ms=15000)
offsets = []
for i, word in enumerate(["one", "two", "three"]):
    meta = producer.send("orders", key=b"k%d" % i, value=word.encode()).get(timeout=15)
    offsets.append(meta.offset)
producer.flush()
producer.close()
print("produced at", offsets)

consumer = KafkaConsumer("orders", bootstrap_servers=bootstrap, group_id="g1", auto_offset_reset="earliest", enable_auto_commit=False, consumer_timeout_ms=8000, request_timeout_ms=10000, session_timeout_ms=6000, heartbeat_interval_ms=2000)
seen = []
for msg in consumer:
    seen.append((msg.offset, msg.key, msg.value))
    if len(seen) == 3:
        break
print("consumed", seen)
consumer.commit()
committed = consumer.committed(TopicPartition("orders", 0))
print("committed", committed)
consumer.close()

again = KafkaConsumer("orders", bootstrap_servers=bootstrap, group_id="g1", auto_offset_reset="earliest", enable_auto_commit=False, consumer_timeout_ms=4000, request_timeout_ms=10000)
rest = [m.value for m in again]
print("after the commit", rest)
again.close()

ok = offsets == [0, 1, 2] and [v for _, _, v in seen] == [b"one", b"two", b"three"] and seen[0][1] == b"k0" and committed == 3 and rest == []
print("OK" if ok else "MISMATCH")
sys.exit(0 if ok else 1)
