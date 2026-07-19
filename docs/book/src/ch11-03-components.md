# Components

A view that takes parameters is a component. Views call views; the tree
composes:

```crow,norun
use web, ui

view stat_card[label, value]
    return div[
        h3[label],
        p['`value`'],
        class = 'card'
    ]

view render[]
    return div[
        h1['Dashboard'],
        stat_card['Trades today', 128],
        stat_card['Open positions', 7]
    ]

serve[3000]
page['/', render]
```

Package components in a module and they arrive qualified, like every
module export (`use cards as c` → `c.stat_card[...]`) — your component
library takes one name from the file that imports it, exactly per
chapter 8's rules. The lowercase element vocabulary itself never grows
and never varies: elements are the language's, components are yours,
and a reader can always tell which is which at a glance.

That's the whole UI model. What remains is to use it on something real.
