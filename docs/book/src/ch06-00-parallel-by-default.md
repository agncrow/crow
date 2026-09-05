# Parallel by Default

This is the chapter where Crow stops resembling Python. In most languages,
parallelism is something you *add* — threads, pools, `async`, locks. In
Crow it's what `for` already does: iterations run concurrently, on all
your cores, and you never wrote a thread.

The design has consequences, and this chapter is honest about them: what
the loop gives you, what it asks of you, and the three search builtins
that cover the patterns a parallel loop can't.

One boundary to keep in mind as you read: `for` is about **throughput** —
finishing a computation faster by using every core. Keeping an *app*
responsive while something slow happens is a different job with its own
verb, `later`, which you'll meet in the web chapter
([Slow Work and the Live Loop](ch12-04-background-work.md)).
