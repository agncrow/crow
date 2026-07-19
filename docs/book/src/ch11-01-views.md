# View Blocks

A `view` is a declaration — `fn`'s sibling — whose body builds a UI tree.
Inside a view, the standard HTML vocabulary is active as functions:

```crow,norun
use web, ui

view render[]
    return div[
        h1['Market Overview'],
        p['All systems operational.'],
        ul[
            li['AAPL'],
            li['MSFT'],
            li['NVDA']
        ]
    ]

serve[3000]
page['/', render]
```

Run it and open `localhost:3000` — a server, a page, no other files.
`page[path, view]` registers a route; `serve[port]` starts the app (the
runtime manages the app object; you never thread one through).

Elements nest by nesting calls — the tree you write is the tree you get.
Attributes are named arguments: `a['docs', href = '/docs']`,
`div[..., class = 'card']`.

**The vocabulary boundary matters:** `div`, `p`, `a`, `form` are element
functions *only inside views*. Outside, they're free — `p = 0.95` is fine
in the same file — and using an element outside a view is a compile error
that says to declare a `view`. One zone, exact HTML names, zero collision
with your code. Views are also the only functions (besides event
handlers) that see reactive state — chapter 4's scope isolation stays
intact everywhere else.
