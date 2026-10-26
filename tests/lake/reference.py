#!/usr/bin/env python3
"""What Lucene's BM25 answers for the corpus of parity 296, computed from
the formula Lucene's BM25Similarity states (Lucene 8 and later: no
`k1 + 1` factor): idf = ln(1 + (n - df + 0.5) / (df + 0.5)), a term's
weight tf / (tf + k1 * (1 - b + b * dl / avgdl)) with k1 = 1.2 and
b = 0.75, a document's score the sum over the query's distinct words it
holds; lengths exact (Lucene keeps a byte of them, so Elasticsearch's
numbers differ past the second digit). Words are the tokens the lake
indexes: runs of letters or digits, lowercased. Prints the ranking and
the scores the parity program holds the lake to; no dependencies.
    python3 -I tests/lake/reference.py
"""
import math, re

CORPUS = [
    (1, "the quick brown fox"), (2, "a lazy dog"), (3, "the fox and the dog"),
    (4, "brown bread and butter"), (5, "fox hunting is banned"), (6, "the dog days of summer"),
    (7, "quick quick slow"), (8, "a brown dog and a brown fox"), (9, "nothing to see"),
    (10, "the end"), (11, "fox fox fox"), (12, "summer bread"),
]
QUERIES = [["fox"], ["brown", "dog"], ["the", "fox"], ["Summer", "bread, butter"]]
K1, B = 1.2, 0.75

def tokens(text):
    return re.findall(r"[0-9a-z]+", text.lower())

docs = [(i, tokens(t)) for i, t in CORPUS]
n = len(docs)
avgdl = sum(len(t) for _, t in docs) / n

def score(query):
    asked = []
    for w in query:
        for t in tokens(w):
            if t not in asked:
                asked.append(t)
    out = []
    for i, toks in docs:
        s = 0.0
        hit = False
        for w in asked:
            tf = toks.count(w)
            if tf == 0:
                continue
            hit = True
            df = sum(1 for _, d in docs if w in d)
            idf = math.log(1 + (n - df + 0.5) / (df + 0.5))
            s += idf * (tf / (tf + K1 * (1 - B + B * len(toks) / avgdl)))
        if hit:
            out.append((i, s))
    out.sort(key=lambda p: (-p[1], p[0]))
    return out

for q in QUERIES:
    ranked = score(q)[:5]
    print(q, [i for i, _ in ranked], ["%.12f" % s for _, s in ranked])
