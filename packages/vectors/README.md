# vectors

A vector index written in Crow, held to faiss: a flat index over
embeddings, exact — the rows as unit vectors, a query's nearest `k` by
the inner product (the cosine, on unit rows), the positions as faiss's
`IndexFlatIP` answers them and the scores the float64 inner products.
Each answers the same in `crow run -i` and `crow run`.
`reference/reference.py` wrote what faiss and numpy compute for three
hundred random vectors and sixteen queries, and `held.co` holds the
package to it.

```
deps = map[vectors = 'github.com/agncrow/crow/packages/vectors@main']
```

`crow install` checks the package out into `crow_modules/vectors`, and
`crow.lock` pins the commit it took and the hash of the files it
installed. A program imports the part it uses:

```
use vectors.flat
```

## The parts

| Part | Functions |
|---|---|
| `flat` | `index[rows]` — a matrix of embeddings, a row each, as unit rows; `nearest[ix, q, k]` — the `k` nearest rows: `.positions` best first (an earlier row first between equals) and `.scores`, the inner products; `unit[v]`; `cosine[a, b]` |

An approximate index (HNSW, held to hnswlib's recall) is the part to
write when a corpus asks for one: a flat index over a few thousand
chunks answers in microseconds compiled, and it is exact
(docs/rag-design.md).

## Held to

`crow test packages/vectors/held.co` loads the reference's vectors and
queries and checks, for every query, that the ten nearest are the
positions faiss answered and that the scores agree with the float64
inner products to 1e-12 relative. faiss computes in float32, so what is
held to it exactly is the ranking: the reference checks that faiss's
and numpy's float64 ranking agree and that no tie lies near the tenth
before it writes anything. The reference runs in a Python environment
with `faiss-cpu` and `numpy`:

```
python3 -m venv ~/.venvs/vectors
~/.venvs/vectors/bin/pip install faiss-cpu numpy
~/.venvs/vectors/bin/python packages/vectors/reference/reference.py
```

## The example

`crow run packages/vectors/example.co` indexes five notes as
bag-of-words vectors and prints the two nearest a question with their
cosines — the same two calls a retrieval step over a model's embeddings
makes (`use llm`, `embed`).
