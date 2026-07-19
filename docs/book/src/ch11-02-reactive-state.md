# Reactive State and Events

Top-level variables in a ui program are **reactive state**. Views read
them; event handlers change them; every change re-renders the page for
every connected browser. Here is the whole loop:

```crow,norun
use web, ui

count = 0

fn inc[]
    count = count + 1

fn dec[]
    count = count - 1

view render[]
    return div[
        h1['Count: `count`'],
        button['+', click = inc],
        button['-', click = dec]
    ]

serve[3000]
page['/', render]
```

Click `+` in any browser tab: `inc` runs *on the server*, `count`
changes, and every open tab shows the new value. You wrote no JavaScript,
no fetch calls, no socket code.

Events bind by **name** — `click = inc` — the functions-as-values idea
from chapter 4, never a lambda. And notice what `count` is: the one
sanctioned exception to scope isolation. Ordinary functions still can't
touch it; views and handlers can, because the runtime owns it,
synchronizes it, and knows when it changed. It's not ambient mutation —
it's a managed channel.

Data sources compose the same way: `prices = poll['/api/prices', 5]`
declares state that refreshes every five seconds and re-renders whatever
reads it.
