# Appendix B: Operators and Builtins

## Operators

| Kind | Operators |
|---|---|
| Arithmetic | `+  -  *  /  %  ^` (`/` floors between ints; `^` is power) |
| Element-wise (tensors) | `.*  ./  .^` |
| Comparison | `eq  ne  gt  lt  gte  lte` |
| Logical | `and  or  not` |
| Chaining | `then` |
| Range | `a..b` |

There is no `==`, `!=`, `<`, `>`, `&&`, `||`, `!`, `+=`, or ternary —
each produces an error naming the Crow form.

## Builtins

The complete builtin reference lives in the specification
(`docs/language-spec.md`, "Built-in Functions" and each module's
section). The core set is listed in Appendix A; highlights by area:

- **Strings** — `split join trim replace upper lower starts ends repeat fmt`
- **Collections** — `len contains slice push pop sort reverse concat range map set del keys values has`
- **Search** — `find any all` (named predicates, parallel early exit)
- **Background work** — `later[fn, args…]` (off-loop, skip-if-running),
  `running[fn]`
- **Moments** — `now[]`, `moment[y, m, d, hour =, minute =, second =, tz =]`,
  `calendar[t, tz =]`, `parse_time[text, tz =]`, `fmt[t, time =, tz =]`
  (a moment is epoch seconds; subtraction is elapsed seconds)
- **Verification** — `test` blocks (attached = the fn above's truths;
  named = standalone; params = generated inputs), `assert[cond]`
- **Errors** — `error[reason, kind]`, metadata `.ok .reason .kind .trace`
- **IO** — `read write mkdir ls rm mv cp env input args shell`
- **Math** (via `use math`) — reductions, tensor constructors, linear
  algebra, FFT, random, autograd `grad`
- **Frames** (via `use frame`) — `load filter sort group join stats`
- **Database** (via `use db`) — `connect query exec insert transact migrate`
- **Web/UI** (via `use web, ui`) — `serve page route`, elements in views,
  `poll fetch`
