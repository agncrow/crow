# Views: the whole element vocabulary

Every HTML element Crow knows, in one view. Built by the book's
verification, not served.

## Layout containers

The five containers and their literal attributes, rendered to a page
with `document[]` and checked at two widths by the engine:

```crow
use web, ui, doc

view page_body[]
    level = 3
    return stack[
        row[h1['Title'], span['badge', color = 'muted'], justify = 'between', align = 'center'],
        grid[p['a'], p['b'], p['c'], columns = 3, gap = 'm', below = 'phone', columns_then = 1],
        overlay[scroll[p['a long tape'], height = '96px'], span['LIVE', color = 'up', anchor = 'top_right']],
        row[button['One'], button['Two'], gap = 's', wrap = 'true', below = 'phone', becomes = 'stack'],
        slider['Level', level, min = 0, max = 10, step = 1],
        gap = 'l', pad = 'l'
    ]

test layout[]
    s = app[page_body, width = 1280]
    assert[fits[s, 1280]]
    assert[len[box[s, 'grid p']] eq 3]
    assert[fits[app[page_body, width = 390], 390]]

markup = doc.document['Containers', page_body[]]
log[contains[markup, 'class="grid"']]
log[contains[markup, 'data-becomes="stack"']]
log[contains[markup, 'type="range"']]
```

```text
true
true
true
```

## Every element

```crow,norun
use web, ui

query_text = ''
prices = poll['/api/prices', 5]
config = fetch['/api/config']

view render[]
    return div[
        header[nav[a['home', href = '/'], span['·'], a['docs', href = '/docs']]],
        section[
            h1['h1'], h2['h2'], h3['h3'], h4['h4'], h5['h5'], h6['h6'],
            p['paragraph'],
            pre[code['x = 1']],
            img[src = '/logo.png', alt = 'the logo'],
            ul[li['one'], li['two']],
            details[summary['More'], p['body']],
            dropdown[summary['Actions'], stack[a['Help', href = '/help']]],
            modal[p['dialog'], open = false],
            drawer[p['panel'], open = false],
            toast[p['notice'], open = false],
            divider[],
            ol[li['first'], li['second']],
            form[
                label['search'],
                input[name = 'q'],
                textarea[name = 'notes'],
                option['a'],
                field['Query', query_text],
                button['go', class = 'primary']
            ],
            table[thead[tr[th['k'], th['v']]], tbody[tr[td['a'], td['1']]]]
        ],
        footer[p['© crow']]
    ]

page['/', render]
serve[3000]
```

`fetch[url]` loads once when the page connects; `poll[url, seconds]`
re-fetches on an interval; `field[label, var]` binds an input to reactive
state. Chapter 12 covers how they re-render.

Reference: spec section 13 (web → View Blocks, Reactive Data and Inputs).
