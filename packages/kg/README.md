# kg

A knowledge graph written in Crow over Magpie (`use magpie`): nodes and
edges as two logs under a root, with history, a graph as of any
position, a node's edges read as one key's run, breadth-first walks and
paths, patterns matched as joins, rules that derive edges, and a
subgraph handed to the `graph` package for pagerank, components and
shortest paths. Each answers the same in `crow run -i` and `crow run`.

```
deps = map[kg = 'github.com/agncrow/crow/packages/kg@main']
```

`crow install` checks the package out into `crow_modules/kg`, and
`crow.lock` pins the commit it took and the hash of the files it
installed. A program imports it and calls it qualified:

```
use magpie, frame
use kg
```

## The logs

A graph lives under a root directory as three Magpie logs. `nodes` is
keyed by `id` and holds `label`, `props` (JSON text), `gone` and `at`;
`edges` is keyed by `src`, `rel`, `dst` and holds `weight`, `source`
(provenance: where the fact came from), `gone` and `at`; `back` holds
every edge again keyed by `dst`, `rel`, `src`. Each log's time column is
`at`, in milliseconds. A merged segment of `edges` is sorted by `src`,
so a node's out-edges are one run of one row group, and `back` makes
its in-edges one too. A graph of a billion edges is read a node at a
time; the whole of a small one is `edges[root]`.

## Writes

`node[root, id, label = '', props = '{}', at = -1]` writes a node and
answers its position; `retire[root, id, at = -1]` writes its tombstone.
`link[root, src, rel, dst, weight = 1.0, origin = '', at = -1]` writes an
edge forward and back and answers its position in `edges`;
`unlink[root, src, rel, dst, at = -1]` its tombstone in both. `at` is
the moment the fact holds from; -1 is now. A write is a Magpie `put`:
visible to the process at once, durable within ten milliseconds (or at
`sync[dir]` of the log).

`rule[root, f, at = -1]` applies a rule: `f` takes the edges as a frame
and answers a frame of `src`, `rel`, `dst` — the edges it derives — which
are linked with `source` 'rule'; it answers how many it linked. A rule
runs over the whole graph, so it is for a graph that fits a frame;
extraction from text is the `llm` module writing triples with `link`.

## Reads

Every read takes `upto`, a position of the edges log (or the nodes
log): the graph as it stood then; -1 is now. A read skips tombstones.

- `nodes[root, upto = -1]`, `edges[root, upto = -1]`: the newest row of
  each node, of each edge — the whole graph.
- `neighbors[root, id, rel = '', upto = -1]`: a node's out-edges, by one
  relation or all; `inbound[root, id, rel = '', upto = -1]` its in-edges,
  each as the edge is written (`src`, `rel`, `dst`).
- `history[root, id, upto = -1]`: every version of a node's out-edges,
  oldest first, the tombstones among them.
- `walk[root, origin, hops, rel = '', upto = -1]`: the nodes within
  `hops` edges, breadth first, as a frame of `id`, `hops` and `via` (the
  node each was reached from).
- `path[root, a, b, hops = 6, rel = '', upto = -1]`: the path of the
  fewest edges, as the ids along it; an empty list where none.
- `match[root, s, r, o, upto = -1]`: a pattern as three lists —
  subjects, relations, objects, a triple each — where a name beginning
  with `?` is a variable: a frame with a column per variable and a row
  per way the pattern holds. Each triple's edges are read as
  `neighbors` or `inbound` reads them (the whole graph where the triple
  names no node), and the triples are joined on the variables they
  share, so every triple after the first shares one with an earlier
  triple, and a variable appears once in a triple.
- `subgraph[root, ids, upto = -1]`: the edges among the nodes, as a
  frame of `a`, `b`, `rel`, `weight` — what the `graph` package's
  `of_edges` takes, an `Edge[a, b, weight]` per row.
- `asof[root, f, columns = []]`: the edges as they stood at each row's
  moment — `f` holds `src`, `rel`, `dst` and `at` — Magpie's `prior`
  over the edges log.

```
use magpie, frame
use kg

p = kg.node['people', 'alice', 'person', '{"title": "engineer"}']
p = kg.link['people', 'alice', 'works_at', 'acme', 1.0, 'crm']
staff = kg.inbound['people', 'acme', 'works_at']
near = kg.walk['people', 'alice', 2]
who = kg.match['people', ['?x', '?x'], ['knows', 'works_at'], ['?y', 'acme']]
```

## Held to

`kg.co`'s test blocks: the two logs and their tombstones, a read as of
a position, a node's history, a walk and a path against a graph drawn
by hand, the patterns' joins, a rule's edges, a subgraph, and the edges
as of a moment; `example.co` runs interpreted and compiled in CI with
one output. The vector index of docs/magpie-design.md's phase 10 waits
on the column of vectors (a language feature); until then an entity is
found by `search` over its text, and the edges expand it.
