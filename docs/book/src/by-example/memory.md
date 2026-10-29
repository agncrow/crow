# Process memory: `use memory`

Process memory is what the standard library's modules keep between calls —
Magpie's sequencer holds the rows put since its last flush there. A program
may not import it: a program's state is its variables, and a page's its
facts. These blocks run through the book harness's door, which a program
has not, as the `sys` page's do.

```crow
use memory

struct Count [n, label]

fn bumped[c, k]
    return Count[c.n + k, 'revised']

log[recall['visits', Count[0, 'none yet']].label]
remember['visits', Count[1, 'first']]
log[revise['visits', Count[0, ''], bumped, 2].n]
log[recall['visits', Count[0, '']].label]
remember['visits/today', Count[5, 'beneath']]
forget['visits']
log[recall['visits/today', Count[0, 'forgotten']].label]
log[len[canonical['.']] gt 0]
```

```text
none yet
3
revised
forgotten
true
```

A record held for the process under a name outlives the call that put it
there and is shared by the program's threads: what a module keeps between
calls, as Magpie's sequencer keeps the rows put since its last flush. A
name holds one kind of record — a read with a default of another kind
stops, naming both — and `recall` answers the default where nothing is
held. `revise` runs its function under the name's lock, handed the record
held and the rest of its arguments, and holds what the function answers:
two threads revising one name take turns, and a revision whose function
fails leaves the record as it was. Names are a tree, so forgetting
`visits` forgets `visits/today` too, and a record named by a path
(`canonical[dir]`) goes when `rm` removes the path.
