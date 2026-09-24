# The Element Vocabulary

A page is composed from a closed vocabulary: the containers you met in the
layout section, the HTML elements, and a set of primitives that each add a
capability nothing else composes — an input bound to state, a disclosure,
a layer, an icon. Anything else you want is a `view` you write from these.
This chapter walks the primitives with the test harness, so every claim is
checked without a browser.

## Inputs are state

A bound input has no value of its own. The reader's change lands in the
variable, handlers read the variable, and the next render draws it. Every
kind of input binds the same way:

```crow
use web, ui

name = 'Ann'
qty = 100
notes = ''
agree = false
notify = true
side = 'buy'
mode = 'Practice'
pct = 0.4

view form[]
    return stack[
        tabs[['Scanner Replay', 'Practice'], mode],
        row[field['Name', name], field['Qty', qty], gap = 'm'],
        field['Notes', notes, lines = 3],
        row[checkbox['I agree', agree], toggle['Notify', notify], radio[['buy', 'sell'], side], gap = 'l'],
        progress[pct],
        gap = 'm'
    ]

test inputs_bind_to_state[]
    a = app[form]
    h = html[a]
    assert[contains[h, 'aria-selected="true" role="tab"']]
    assert[contains[h, '<input data-crow-input="__field:qty" type="number" value="100">']]
    assert[contains[h, '<textarea data-crow-input="__field:notes" rows="3">']]
    assert[contains[h, 'crow-toggle"><input checked="" data-crow-input="__field:notify" type="checkbox">']]
    assert[contains[h, '<progress class="crow-progress" max="1" value="0.4">']]

page['/', form]
```

`field` looks at the variable: an int or float gets a number input, and
`type = 'date'` or `'time'` asks for the browser's picker. `lines = 3` makes
a textarea. `tabs` and `radio` hold one of several strings, `checkbox` and
`toggle` a bool — the same binding drawn two ways.

## Revealing: disclosure, dropdowns, and the three layers

`details` shows its summary always and its body when open. A `dropdown` is
a button whose second child floats under it. The three layers — `modal`,
`drawer`, `toast` — sit outside the flow and show while `open` is true. Open
is state, so a handler opens and closes them:

```crow
use web, ui

show_help = false
saved = false

fn open_help[]
    show_help = true

fn close_help[]
    show_help = false

fn save[]
    saved = true

view layers[]
    return stack[
        details[summary['More filters'], p['Min price, max float'], open = true],
        dropdown[summary['Actions'], stack[button['Help', click = open_help], button['Save', click = save]]],
        modal[stack[h3['Help'], p['Click a bar to seek.'], button['Close', click = close_help]], open = show_help],
        toast[p['Saved'], open = saved, color = 'up'],
        divider[],
        gap = 'm'
    ]

test layers_follow_state[]
    a = app[layers]
    assert[contains[html[a], 'class="modal" data-open="false"']]
    click[a, open_help]
    assert[contains[html[a], 'class="modal" data-open="true"']]
    click[a, close_help]
    click[a, save]
    assert[contains[html[a], 'class="toast" data-open="true"']]

page['/', layers]
```

## Framing and detail

`nav` marks the current page, `icon` draws from a closed set in the text
colour, `tip` puts the browser's tooltip on anything, and `divider` rules
between siblings:

```crow
use web, ui

view top_bar[]
    return stack[
        nav[a['Home', href = '/'], a['Scanner', href = '/scanner'], active = '/scanner'],
        row[icon['search'], span['Search the tape', tip = 'type a symbol'], icon['star', color = 'accent'], gap = 's'],
        gap = 'm'
    ]

test the_shell[]
    a = app[top_bar]
    h = html[a]
    assert[contains[h, '<a aria-current="page" href="/scanner">']]
    assert[contains[h, 'svg class="icon icon-search"']]
    assert[contains[h, 'title="type a symbol"']]
    assert[len[box[a, 'nav a']] eq 2]

page['/', top_bar]
```

## Lists of things

A page shows many of something: past, bookmarks, trades. A `for`
expression over a view gives a list of nodes, and a child that is a list
contributes each item. That is the whole rule — no loop element, no
template syntax:

```crow
use web, ui

past = ['HSCS 2026-06-23', 'EZGO 2026-06-17', 'ANY 2026-06-01']
starting = true

fn forget[]
    past = ['HSCS 2026-06-23']
    starting = false

view session_row[s]
    return row[span[s], button[icon['x', size = 's'], click = forget, tip = 'delete'], justify = 'between']

view past_sessions[]
    rows = for s in past
        session_row[s]
    return stack[h2['Past past'], rows, button['Start', disabled = starting, class = 'primary'], gap = 's']

test a_list_is_its_items[]
    a = app[past_sessions]
    assert[len[elements[a, 'row']] eq 3]
    assert[contains[html[a], 'disabled=""']]
    click[a, forget]
    assert[len[elements[a, 'row']] eq 1]
    assert[not contains[html[a], 'disabled=""']]

page['/', past_sessions]
```

The same rule gives every row its own controls — a delete icon, a pin,
a link — which `table[frame]` cannot, since a frame's cells are values.
Use the table for data and a list of views for rows that act.

`disabled = starting` shows the other half: a boolean attribute follows
state. HTML reads presence, not the word, so the renderers write
`disabled=""` when the value is true and nothing when it is false — the
same for `hidden`, `readonly`, `required`, `checked` and `selected`.

## The look of the app

Theme, density, accent and width are settings of the app, not of the
elements. They go on `serve`:

```crow,norun
use web, ui

view render[]
    return stack[h1['Terminal'], p['dense, dark, full width']]

page['/', render]
serve[3000, theme = 'dark', density = 'compact', accent = '#3b82f6', wide = true]
```

`theme` is `'light'`, `'dark'`, or `'system'` (the default). `density =
'compact'` tightens every element's defaults for a screen full of numbers.
`accent` replaces the one colour the tokens leave to you. `wide` drops the
reading column so a dashboard uses the whole window. Nothing in the views
changes: the same tree renders under every look, and the layout engine
places it the same way.

## Records with actions

A list of records usually wants a button or two per row. `table` takes
them as `actions`, a map from label to handler; each button sends the
row's key to its handler, as a row click does. `hide` keeps a helper
column in the frame but out of the table, and a header bar is `header`.

```crow
use web, ui, frame

deleted = 0
opened = 0

fn open_row[id]
    opened = id

fn delete_row[id]
    deleted = id

view records[]
    rows = frame.new[]
    rows = with[rows, 'id', [3, 4]]
    rows = with[rows, 'name', ['alpha', 'beta']]
    return stack[
        header[h1['records', size = 'm'], nav[a['Home', href = '/'], active = '/'], gap = 'm'],
        table[rows, click = open_row, key = 'id', actions = map[Delete = delete_row], hide = ['id']]
    ]

test each_row_carries_its_actions[]
    a = app[records]
    h = html[a]
    assert[contains[h, '<header']]
    assert[contains[h, 'data-crow-click="delete_row" data-crow-args="[3]">Delete</button>']]
    assert[not contains[h, '<th class="num">id</th>']]
    click[a, delete_row[4]]
    assert[deleted eq 4]

page['/', records]
```

## One way out: widgets

The vocabulary is closed on purpose, which means a map, a rich-text
editor or a calendar cannot be written with it. Those are widgets: a
piece of the page your own JavaScript draws. `widget['map', props = …,
on = map[pick = choose]]` draws the module `widgets/map.js` in your
program's folder. The module exports two functions:

```js
// widgets/map.js
export function mount(el, props, send) {
  const b = document.createElement('button');
  b.textContent = 'Pick Paris (showing ' + props.city + ')';
  b.onclick = () => send('pick', 'Paris');
  el.appendChild(b);
}

export function update(el, props) {
  el.querySelector('button').textContent = 'Pick Paris (showing ' + props.city + ')';
}
```

The page loads the module once and calls `mount` with the element and
the props, as JSON. Whenever a render changes the props it calls
`update`, and the element's insides stay the module's. `send('pick',
value)` runs the handler that `on` names for `pick`, the way a click
would: the page offers exactly the events its widgets name, and the
handler receives the value, so it checks what it is given. In a test a
widget is its element, and a click on the handler stands for what the
widget would send:

```crow
use web, ui

place = 'London'

fn choose[city]
    place = city

view atlas[]
    return stack[
        p['showing `place`'],
        widget['map', props = map[city = place], on = map[pick = choose]]
    ]

test the_map_s_pick_moves_the_page[]
    a = app[atlas]
    click[a, choose['Paris']]
    assert[elements[a, 'p'][0].text eq 'showing Paris']
    assert[elements[a, 'div.crow-widget'][0]['data-crow-props'] eq '{"city":"Paris"}']

page['/', atlas]
log['atlas defined']
```

```text
atlas defined
```

There is no other way to put code on a page: no script tags, no inline
handlers.

