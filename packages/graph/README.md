# graph

Graphs written in Crow: a graph built from its edges, walked breadth and
depth first, its components, its strongly connected pieces and a
topological order (`graph`); shortest paths by weight, the path back, a
minimum spanning tree and the longest path of an acyclic graph
(`paths`); and a node's place in the graph — pagerank, clustering,
betweenness, closeness (`rank`). Each answers the same in `crow run -i`
and `crow run`, and the same as networkx given the same edges.

```
deps = map[graph = 'github.com/agncrow/crow/packages/graph@main']
```

`crow install` checks the package out into `crow_modules/graph`, and
`crow.lock` pins the commit it took. A program imports the parts it uses:

```
use graph.graph, graph.paths, graph.rank
```

## The graph

A node is named by text. `of_edges[edges, directed]` builds a graph from
`Edge[a, b, weight]` records — an edge runs both ways unless `directed`
— and `add[g, a, b, weight]` adds one; `add_node[g, name]` a node no
edge touches. Inside, a node has an index, the order it was added in,
and a list of `Arc[to, weight]` in the order its edges were added, so a
walk visits neighbours in that order, as networkx does.

| File | Function | Answer |
|------|----------|--------|
| `graph` | `of_edges[edges, directed]`, `add[g, a, b, weight]`, `add_node[g, name]`, `reversed[g]` | a `Graph[names, index, arcs, directed]` |
| | `neighbors[g, name]`, `degree[g, name]`, `in_degree[g, name]` | the names an arc leads to; how many arcs leave, how many reach |
| | `bfs[g, start]`, `dfs[g, start]` | the names reached, in the order found or entered |
| | `hops[g, start]` | a map from each reached name to its arcs from `start` |
| | `components[g]`, `strongly_connected[g]` | lists of names: what reaches each other, and what reaches each other both ways (Kosaraju) |
| | `topological[g]` | the names in an order every arc runs forward in, the smallest name first among those ready; an error of kind `'graph'` where there is a cycle |
| `paths` | `dijkstra[g, start]` | a `Reach`: `distance[r, name]` the shortest distance by the weights (an error of kind `'graph'` where not reached), `path[r, name]` the names from the start (empty where not reached) |
| | `spanning[g]` | a `Tree[edges, weight]`: Kruskal's minimum spanning tree, a forest where the graph is in pieces |
| | `longest[g]` | the most arcs on any path of an acyclic graph (an error where there is a cycle) |
| `rank` | `pagerank[g, damping = 0.85, rounds = 100, tolerance = 0.000001]` | a map from each name to its rank: the power iteration over the arcs' weights, a dangling node's share spread evenly, until the ranks move less than the tolerance per node |
| | `clustering[g, name]` | the share of a node's pairs of neighbours that are joined |
| | `betweenness[g]` | a map from each name: the share of shortest paths (unweighted) through it, Brandes' count normalised by the pairs |
| | `closeness[g]` | a map from each name: the inverse mean distance to it — along incoming arcs where the graph is directed — scaled by the share of nodes that reach it |

```
use graph.graph, graph.paths

g = graph.of_edges[[graph.Edge['a', 'b', 1.0], graph.Edge['b', 'c', 2.0], graph.Edge['a', 'c', 4.0]], false]
r = paths.dijkstra[g, 'a']
log[paths.path[r, 'c']]      --- ['a', 'b', 'c']
```

## Against networkx

`compare.co` takes a folder of graphs networkx measured — two undirected
graphs of 40 nodes (90 edges, and 44 edges in four pieces), two directed
graphs of 30 nodes and 70 arcs, two acyclic graphs of 25 nodes and 60
arcs, random weights — and prints, for each, the largest difference
between networkx's numbers and this package's, and whether the walks and
orders are the same (2026-10-05, networkx 3.7):

| Measure | Difference |
|---|---|
| pagerank (weighted, undirected and directed) | 0 |
| clustering, betweenness, closeness (undirected and directed) | 0 |
| hops and Dijkstra's distances from a node | 0 |
| breadth-first and depth-first orders, components and their count, strongly connected pieces, in- and out-degrees | the same |
| the spanning tree's weight and size | the same |
| the lexicographic topological order and the longest path of an acyclic graph | the same |

The orders agree because a node's arcs are kept in the order its edges
were added, as networkx's adjacency is; pagerank agrees to the last digit
because the iteration is networkx's own, dangling nodes and all.
`crow run -i packages/graph/compare.co <folder>` and `crow run` print the
same lines. The folder is written by a script beside the PySR one
(`graph_bench.py`: `networkx` in a Python environment), kept out of the
repository with the data.

## Tests

Each file has its own `test` blocks, and `example.co` builds a small
graph and prints what each function answers:

```
crow test packages/graph/graph.co
crow run -i packages/graph/example.co
crow run packages/graph/example.co
```

In the Crow repository, `crow test --packages graph` runs every file's
tests and the example in both modes, comparing the output.
