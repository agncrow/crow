# Programming a Guessing Game

Let's write a real program before any more theory. This chapter builds the
classic first project: the computer picks a number between 1 and 100, you
guess, and it tells you *too low* or *too high* until you find it.

Small as it is, this game touches most of what makes Crow itself: reading
input, converting types, meeting your first **error value**, comparing with
word operators, and looping. Everything it introduces gets a full chapter
later — here we just use it all.

## Setting Up

```bash
crow new guessing_game
cd guessing_game
```

Open `main.crow` and replace its contents as we go. Run at any point with
`crow run` (compiled) or `crow run -i` (instant, interpreted).

## Processing a Guess

First, ask for input:

```crow,norun
raw = input['your guess: ']
log['you guessed: `raw`']
```

`input[prompt]` prints the prompt, reads a line from the keyboard, and
returns it as a string. The backticks in `` 'you guessed: `raw`' `` are
Crow's string interpolation — any expression between paired backticks is
evaluated into the string.

## A Value That Might Have Failed

The player typed a *string*; we need a *number*. `int[...]` converts:

```crow
n = int['42']
log[n]
log[n + 8]

bad = int['abc']
log[bad.ok]
log[bad.reason]
```

```text
42
50
false
cannot convert 'abc' to int
```

Look closely at the second half — this is your first encounter with Crow's
error model, and there is no `try`, no `catch`, no exception. `int['abc']`
simply **returns a value that knows it failed**. Every value in Crow
carries `.ok` and `.reason`; for successful values `.ok` is `true` and you
never think about it (notice `n + 8` just works — values auto-unwrap).
For a failure, `.ok` is `false` and `.reason` says why.

What if you ignore the failure and use `bad` anyway? Crow refuses to guess:
an error value has no truth value, so a comparison or branch on it stops
the program (or returns the error from a function) rather than silently
picking a wrong path. Chapter 7 is entirely about this; for the game, all
we need is `.ok`.

## Generating the Secret Number

```crow,norun
secret = rand[1, 100]
```

`rand[min, max]` returns a random integer in the range, inclusive.

## Comparing

Crow's comparison operators are words — `eq`, `ne`, `lt`, `gt`, `lte`,
`gte` — no shift key, no `==`-vs-`=` bugs:

```crow
secret = 62
guess = 50

if guess eq secret
    log['you got it!']
else if guess lt secret
    log['too low']
else
    log['too high']
```

```text
too low
```

(There is no `elif` keyword — `else if` is just an `if` in the `else` branch,
and it reads the same way.)

## Looping Until They Get It

`while true` with `break` and `continue` gives the game loop. Here is the
complete program:

```crow,norun
secret = rand[1, 100]
log['I am thinking of a number between 1 and 100.']

while true
    raw = input['your guess: ']
    n = int[raw]
    if not n.ok
        log['`raw` is not a number — try again']
        continue
    if n eq secret
        log['you got it — `secret`!']
        break
    if n lt secret
        log['too low']
    else
        log['too high']
```

A session looks like this:

```text
I am thinking of a number between 1 and 100.
your guess: fifty
fifty is not a number — try again
your guess: 50
too low
your guess: 75
too high
your guess: 62
you got it — 62!
```

Walk the error path once more, because it's the most Crow-shaped part:
when the player types `fifty`, `int[raw]` returns a failed value. We check
`.ok`, explain, and `continue`. If we *hadn't* checked, the very next line
— `if n eq secret` — would have refused to run on an error value and
stopped the program with the reason. Wrong input can never masquerade as
a wrong guess.

## Summary

You've used: `input` and interpolation, `int` conversion, error values and
`.ok`, `rand`, word comparisons, `if`/`else`, and `while`/`break`/
`continue`. Chapter 3 now walks the everyday material properly, and
chapter 7 returns to error values in depth.
