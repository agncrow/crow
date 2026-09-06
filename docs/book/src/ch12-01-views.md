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

## Tables that respond

`table[frame]` shows a frame. Give it a handler and it becomes a control:
`click = focus` sends the clicked row's key column to `fn focus[symbol]`,
`key = 'symbol'` says which column that is, and `mark = 'gate'` tints the
rows whose `gate` cell is true. The scanner in the final project is one
of these.

## A view renders wherever it is called

A view is not tied to `page[]`. Call it from a test block, from a loop
that writes pages to disk, or from another view, and it returns its
markup. Hand that markup to `document[]` from `use doc` (Crow by Example, and the standard library reference in Part II) and you
have a complete page without a server — the book you are reading is built
exactly this way. Two rules make this safe:

- **Markup passes through, text is escaped.** A string that starts and
  ends with a tag is treated as markup inside `stack[...]` or any element;
  anything else is text and is escaped.
- **A failed argument renders, it does not propagate.** Elements render
  what they are given (chapter 12.5), and so does a view: an error value
  handed to a view shows as `error: reason` in place of the tree. A page
  with one unreadable chapter is still a page.

```crow
use web, ui, doc

view card[title, body]
    return stack[h3[title], p[body], surface = 'card']

markup = doc.document['Card', card['Hello', 'a paragraph']]
log[contains[markup, '<div class="stack card"']]
broken = doc.document['Card', card['Hello', read['/nonexistent/notes.md']]]
log[contains[broken, 'error: cannot read']]
```

```text
true
true
```

**The vocabulary boundary matters:** `div`, `p`, `a`, `form` are element
functions *only inside views*. Outside, they're free — `p = 0.95` is fine
in the same file — and using an element outside a view is a compile error
that says to declare a `view`. One zone, exact HTML names, zero collision
with your code. Views are also the only functions (besides event
handlers) that see reactive state — chapter 4's scope isolation stays
intact everywhere else.
