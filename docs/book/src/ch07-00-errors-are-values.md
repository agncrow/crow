# Errors Are Values

Every language must answer: *what happens when an operation fails?*
Exceptions answer with a jump — control teleports to the nearest handler,
and every function between is silently interruptible. Null answers with a
landmine — a value that detonates somewhere far from where it was made.

Crow's answer: **failure is data.** A failed operation returns a value
that knows it failed and can say why. That value flows through your
program like any other — until something tries to *act* on it, at which
point Crow refuses to guess. No `try`, no `catch`, no null, no unwinding.

You've been using this model since chapter 2. This chapter makes it
precise, in four steps: what every value carries, how failures travel,
why an error can never decide a branch, and how to handle one on purpose.
