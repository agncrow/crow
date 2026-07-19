# Aliased Imports

Any import can take an alias — and an aliased import is **qualified-only**,
exactly like Python's `import pandas as pd`. Nothing enters your bare
namespace except the alias itself:

```crow
use math as m

sum = 0
for x in [1, 2, 3]
    sum = sum + x
log[sum]
log[m.sum[[10.0, 20.0]]]
```

```text
6
30
```

Look at what happened there: with `use math as m`, the name `sum` stayed
*yours* — the accumulator is an ordinary variable — while math's `sum` is
reached as `m.sum`. The alias is the escape valve for exactly the moment
when you and a module want the same word.

Calling `sum[...]` bare in that file is a compile error that names the
fix:

```crow,error
use math as m
log[sum[[1.0, 2.0]]]    --- error: 'sum' comes from math which was
                        --- imported as 'm' — call m.sum[...]
```
