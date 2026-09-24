# Controlling Resources

Parallelism defaults to all cores; three builtins adjust it when you need
to:

```crow,norun
cpu[8]      --- cap the next loops at 8 threads
cpu[1]      --- one core: every loop in order
gpu[]       --- enable GPU execution for tensor work
nodes[4]    --- distribute across machines (cluster runs)
```

That's the entire concurrency API. There are no threads to spawn, no
futures to await, no locks to take — the scope rule (chapter 4) plus the
read/write analysis make the safe thing the only thing.

Next: what happens when an operation fails — Crow's other big idea.
