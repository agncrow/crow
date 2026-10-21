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
`page[path, view]` serves the view at that path; `serve[port]` starts the
app (the runtime manages the app object; you never thread one through).

Elements nest by nesting calls — the tree you write is the tree you get.
Attributes are named arguments: `a['docs', href = '/docs']`,
`div[..., class = 'card']`.

## Tables that respond

`table[frame]` shows a frame. Give it a handler and it becomes a control:
`click = focus` sends the clicked row's key column to `fn focus[symbol]`,
`key = 'symbol'` says which column that is, and `mark = 'gate'` tints the
rows whose `gate` cell is true. A list of records usually wants a button
or two per row: `actions = map[Replay = replay, Delete = ask_delete]` puts
them in a trailing cell, each sending the row's key to its handler, and
`hide = ['id']` keeps the key column in the frame but out of the table.
The scanner and the session history in the final project are these.

## A view renders wherever it is called

A view is not tied to `page[]`. Call it from a test block, from a loop
that writes pages to disk, or from another view, and it returns its
markup. Hand that markup to `document[]` from `use doc` (Crow by Example, and the standard library reference in Part II) and you
have a complete page without a server — the book you are reading is built
exactly this way. Two rules make this safe:

- **Text is text.** Markup is what a view, an element or `doc` makes, and
  it stays markup however it travels — held in a local, collected by a
  `for`, handed to another view. Every other string is text and is
  escaped, whatever it holds: a reader who types `<img src=x
  onerror=…>` into a field sees those characters, on their page and on
  everyone else's, and nothing runs.
- **A failed argument renders, it does not propagate.** Elements render
  what they are given (chapter 12.5), and so does a view: an error value
  handed to a view shows as `error: the server's log has why, as <id>` in
  place of the tree — the reader is told an id, the server's log has the
  reason. A page with one unreadable chapter is still a page.

```crow
use web, ui, doc

view card[title, body]
    return stack[h3[title], p[body], surface = 'card']

markup = doc.document['Card', card['Hello', 'a paragraph']]
log['<div class="stack card"' in markup]
broken = doc.document['Card', card['Hello', read['/nonexistent/notes.md']]]
log['error: the server' in broken]
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
