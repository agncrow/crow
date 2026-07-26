# Test Blocks

In most languages, testing is a second discipline: a framework to install,
a directory to arrange, fixtures to learn. In Crow, verification is part of
writing a function. A `test` block right under a `fn` states what that
function does — and what it can never do:

```crow
fn fee[notional]
    if notional lt 0
        return error['negative notional', 'validation']
    return max[1.0, notional * 0.001]
test[n]
    fee[50000.0] eq 50.0
    fee[100.0] eq 1.0
    fee[-5.0].kind eq 'validation'
    if n gte 0.0
        fee[n] gte 1.0

log['fee is defined']
```

```text
fee is defined
```

Notice what running that program printed: just the log line. Test blocks
never execute when a program runs — `crow run` skips them, `crow build`
strips them from the binary entirely. They run when you ask for
verification:

```text
$ crow test pricing.crow

  ✓ 1 block passes — 3 truths, 1 law, 6 inputs run
```

## Truths: bare booleans

Each plain boolean line in a test block is a *truth* — something that must
be so. The first three lines above are **examples**: exact facts at inputs
you chose. `fee[100.0] eq 1.0` records a *decision* (the fee floor is one
dollar), and the `.kind` line pins the error path — error values are
ordinary values, so verifying them needs nothing special.

When a truth fails, you see both sides:

```text
  ⚠ 1 failing of 1 block

  pricing.crow:5
     fee tests — line 6: assert failed: eq
    left:  0.1
    right: 1.0
```

Someone "simplified" `fee` and dropped the floor; the truth caught it with
the actual values, not just a red X.

## Laws: generated inputs

The block's parameter — `test[n]` — is not an argument anyone passes. It
declares a **generated input**: the checker invents many values for `n`,
boundary cases first, and checks the lines that use it against every one.
`fee[n] gte 1.0` is a *law*: true not at a point, but everywhere.

The `if n gte 0.0` line is doing honest work. Without it, the checker
finds `n = -1.0` — where `fee` returns an error, and errors have no truth
value, so the law fails loudly. That failure is correct: the law as first
written claimed too much. A precondition in a test block is just an `if`.

Examples pin decisions; laws pin relationships. Most functions only need
examples. Write a law when your domain hands you one — "doubling the
quantity doubles the price", "encoding then decoding returns the
original."

## Named tests: scenarios

A `test` with a name stands alone — it isn't about one function, it's
about functions working together. Setup, sequences, a database:

```crow
use db

fn book_fee[conn, amount]
    exec[conn, 'INSERT INTO fees (amount) VALUES (?)', [amount]]

test fees_survive_storage[]
    conn = connect['sqlite://:memory:']
    exec[conn, 'CREATE TABLE fees (amount REAL)']
    book_fee[conn, 18.95]
    rows = query[conn, 'SELECT amount FROM fees']
    assert[rows.amount[0] eq 18.95]

log['module loaded']
```

```text
module loaded
```

Inside scenarios you use `assert` for each step. `assert` is the one
verification word that also ships in production — a runtime guard — which
is exactly why the block keyword is `test`, not `assert`: one word runs in
your program, the other never does.

A named test with parameters is a *stateful law* — `test webhook_twice[fill]`
reads as "for any fill…" and the checker generates the fills.

## Testing a live app

Views and handlers are tested through a headless session. `app[render]`
opens one, `click` runs a handler, `a.field` reads reactive state, and
`html[a]` renders the current view:

```crow,norun
use web, ui

status = 'ready'
count = 0

fn slow_refresh[]
    count = count + 1
    status = 'updated'

fn go[]
    status = 'loading'
    later[slow_refresh]

view render[]
    return div[h1['Live book'], p['status: `status` count: `count`'],
               button['refresh', click = go]]

test refresh_flow[]
    a = app[render]
    assert[contains[html[a], 'ready']]
    click[a, go]
    assert[a.status eq 'updated']
    assert[contains[html[a], 'count: 1']]

page['/', render]
serve[3000]
```

The `later[]` call is the interesting part. In production it schedules
`slow_refresh` off the live loop; in a test it runs inline, so the line
after `click` already sees the final state. Background work is
deterministic in tests and concurrent in production — the same code,
verified without waiting or flakiness.

## The one rule about where things live

**Verification never creates methods.** A test body calls your real
functions — that is what testing is — but you never define a function
*for* a test. Bindings, sequences, conditions: they all live inside the
block. If it's in the program's namespace, it's the program; if it's under
`test`, it's the rules. The boundary is the syntax, so nobody has to
police it.

## The verbs

`crow test` runs the blocks you wrote. `crow check` runs those *plus*
everything you didn't write: automatically generated edge cases for your
pure functions, determinism contracts, and — with a database url — the
concurrency hazard checks from the databases chapter. Tests are the part
of verification you author; check is all of it.
