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
