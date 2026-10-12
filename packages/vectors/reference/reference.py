# -*- coding: utf-8 -*-
"""What the vectors package is held to (held.co): a flat index over random
unit vectors searched by faiss's IndexFlatIP, and the float64 inner
products at the positions it answers.

    ~/.venvs/vectors/bin/python packages/vectors/reference/reference.py

faiss computes in float32, so what is held to it exactly is the ranking
— the positions of the ten nearest — which this script checks against
numpy's float64 ranking and refuses to write where the two disagree or a
tie lies near the tenth; the scores held to 1e-12 are the float64 inner
products at those positions. Every CSV has a header row (`load` reads
the first row as names).
"""
import csv
import os

import faiss
import numpy as np

N, D, Q, K = 300, 24, 16, 10
here = os.path.dirname(os.path.abspath(__file__))
rng = np.random.default_rng(7)
X = rng.standard_normal((N, D))
queries = rng.standard_normal((Q, D))
Xu = X / np.linalg.norm(X, axis=1, keepdims=True)
Qu = queries / np.linalg.norm(queries, axis=1, keepdims=True)

# numpy, float64: the inner products (summed in C by einsum, not by the
# BLAS, whose matmul on this platform raises spurious floating-point
# flags) and their descending order
scores64 = np.einsum("qd,nd->qn", Qu, Xu)
order64 = np.argsort(-scores64, axis=1, kind="stable")[:, :K]

# faiss, float32
index = faiss.IndexFlatIP(D)
index.add(np.ascontiguousarray(Xu, dtype="float32"))
scores32, positions = index.search(np.ascontiguousarray(Qu, dtype="float32"), K)
assert (positions == order64).all(), "faiss and numpy rank the nearest alike"
for i in range(Q):
    sorted_scores = np.sort(scores64[i])[::-1]
    gaps = sorted_scores[:K] - sorted_scores[1:K + 1]
    assert gaps.min() > 1e-6, "no tie near the tenth of query %d" % i
held_scores = np.take_along_axis(scores64, positions, axis=1)
assert np.abs(held_scores - scores32).max() < 1e-5, "faiss's float32 scores agree to 1e-5"


def write(path, rows, names, fmt):
    with open(os.path.join(here, path), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(names)
        for row in rows:
            w.writerow([fmt % v for v in row])


write("data/vectors.csv", X, ["d%d" % j for j in range(D)], "%.17g")
write("data/queries.csv", queries, ["d%d" % j for j in range(D)], "%.17g")
write("expected/positions.csv", positions, ["p%d" % j for j in range(K)], "%d")
write("expected/scores.csv", held_scores, ["s%d" % j for j in range(K)], "%.17g")
print("wrote %d vectors of %d, %d queries, the %d nearest of each" % (N, D, Q, K))
