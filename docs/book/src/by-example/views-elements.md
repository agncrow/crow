# Views: the whole element vocabulary

Every HTML element Crow knows, in one view. Built by the book's
verification, not served.

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
            img[src = '/logo.png'],
            ul[li['one'], li['two']],
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
