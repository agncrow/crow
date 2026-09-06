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

## Sliders, helpers, and bigger state

`slider['Seek %', seek_pct, min = 0, max = 100, step = 1]` binds a range
input to `seek_pct` the way `field` binds a text input; a `Seek` button's
handler then reads it. Bound inputs keep the variable's type: the browser
sends text, and an integer variable receives an integer. When a handler
assigns to a bound variable (a row click that sets `sym`), the field shows
the new value on the next render. A handler may call a helper that assigns module
state — `fn restart[]` calling `fn rewind_to[t]` — and the helper joins the
handler class in both renderers, so its write lands in state. And state
is not only numbers and strings: an array, a map, or a whole frame can be
reactive, kept as JSON between renders. The following block is verified
by the book's tests:

```crow
use web, ui, frame

seek_pct = 50
focus = ''
universe = frame.new[]

fn pick[symbol]
    focus = symbol

fn keep[u]
    universe = u

fn load_universe[]
    u = with[frame.new[], 'symbol', ['IMRN', 'AOUT', 'CHPT']]
    keep[with[u, 'gate', [1, 0, 1]]]

view render[]
    return stack[
        slider['Seek', seek_pct, min = 0, max = 100, step = 5],
        p['focus: `focus`'],
        table[universe, click = pick, key = 'symbol', mark = 'gate'],
        button['Load', click = load_universe],
        gap = 'm'
    ]

test rows[]
    s = app[render, width = 1280]
    click[s, load_universe]
    h = html[s]
    assert[contains[h, 'type="range"']]
    assert[contains[h, 'data-crow-click="pick"']]
    assert[contains[h, 'class="marked"']]
    assert[len[elements[s, 'tr']] eq 4]
    click[s, pick['AOUT']]
    assert[s.focus eq 'AOUT']
    assert[fits[s, 390]]

page['/', render]
log['reactive page defined']
```

```text
reactive page defined
```
