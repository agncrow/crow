#!/usr/bin/env python3
"""What scikit-learn and rank_bm25 answer for a small corpus.

Writes the corpus and the queries the package reads (data/docs.csv,
data/queries.csv) and, for each case, what the references compute
(expected/*.csv): the vocabulary with scikit-learn's smoothed idf, every
TF-IDF weight (l2-normalised rows), the cosine of each query with each
document, rank_bm25's BM25Okapi score of each query against each document,
and the top three documents of each query by that score. The package's
held.co then holds Crow to these files, in both modes, with no Python in
the loop. Floats are written as Python prints them (the shortest text that
reads back to the same double), so a cell read by Crow is the same double.

Needs: Python with scikit-learn (1.9 wrote the files checked in) and
rank_bm25 (0.2.2).

Where a reference has a choice, the one the package's README states is
taken: the default tokenizer (lowercase; tokens of two or more word
characters), smoothed idf, raw term counts, l2 rows, BM25Okapi with k1 =
1.5, b = 0.75, epsilon = 0.25, and a ranking by score with an earlier
document first between equals.
"""
import csv, re
from pathlib import Path

import numpy as np
from rank_bm25 import BM25Okapi
from sklearn.feature_extraction.text import TfidfVectorizer

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
EXPECTED = HERE / "expected"

DOCS = [
    "The quick brown fox jumps over the lazy dog",
    "A lazy dog sleeps in the sun all afternoon",
    "Markets fell sharply as the Fed raised rates again",
    "The Fed held rates steady, and markets rallied into the close",
    "Quarterly earnings beat estimates; the stock jumped 12 percent",
    "Earnings missed estimates and the stock fell 7 percent after hours",
    "A fox den was found near the river by the old mill",
    "Rivers rise after heavy rain; the mill wheel turns faster",
    "The dog chased the fox across the field until sunset",
    "Rates, yields and spreads: a primer on the bond market",
    "Bond yields climbed as traders priced in another rate rise",
    "The brown dog and the brown fox share a brown blanket",
    "Sunset over the field painted the sky orange and red",
    "An index fund tracks the market; a trader tracks the tape",
    "Tape reading is the old art of watching prints and sizes",
    "The quick fox is quick; the lazy dog is lazy",
    "Heavy rain closed the old road to the mill for a day",
    "Estimates for next quarter were raised by three analysts",
    "Three traders, one tape, and a market that would not move",
    "The afternoon sun warmed the river and the field",
    "Percent, basis points and ticks: units of the market",
    "A primer on dogs: breeds, training and the lazy afternoon nap",
    "The old mill by the river sold for 2 million after the rain",
    "Nothing here matches anything else at all",
]
QUERIES = [
    "lazy dog",
    "the fed raised rates",
    "earnings estimates percent",
    "fox river mill",
    "quick brown fox",
    "bond yields rise",
    "tape reading traders",
    "unicorn",
]
TOKEN = re.compile(r"(?u)\b\w\w+\b")


def tokens(text):
    return TOKEN.findall(text.lower())


def write(path, header, rows):
    with path.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(header)
        for r in rows:
            w.writerow(r)


def main():
    DATA.mkdir(exist_ok=True)
    EXPECTED.mkdir(exist_ok=True)
    write(DATA / "docs.csv", ["id", "text"], [(i, d) for i, d in enumerate(DOCS)])
    write(DATA / "queries.csv", ["id", "text"], [(i, q) for i, q in enumerate(QUERIES)])

    vec = TfidfVectorizer()
    X = vec.fit_transform(DOCS)
    Q = vec.transform(QUERIES)
    vocab = sorted(vec.vocabulary_, key=lambda t: vec.vocabulary_[t])
    assert vocab == sorted(vocab)
    write(EXPECTED / "vocab.csv", ["term", "idf"], [(t, repr(float(vec.idf_[j]))) for j, t in enumerate(vocab)])
    X = X.tocsr()
    rows = []
    for i in range(X.shape[0]):
        row = X.getrow(i)
        for j, v in zip(row.indices, row.data):
            rows.append((i, vocab[j], repr(float(v))))
    rows.sort(key=lambda r: (r[0], r[1]))
    write(EXPECTED / "tfidf.csv", ["doc", "term", "weight"], rows)
    C = (Q @ X.T).toarray()
    write(EXPECTED / "cosine.csv", ["query", "doc", "score"], [(q, d, repr(float(C[q, d]))) for q in range(len(QUERIES)) for d in range(len(DOCS))])

    bm = BM25Okapi([tokens(d) for d in DOCS])
    scores = {q: bm.get_scores(tokens(QUERIES[q])) for q in range(len(QUERIES))}
    write(EXPECTED / "bm25.csv", ["query", "doc", "score"], [(q, d, repr(float(scores[q][d]))) for q in range(len(QUERIES)) for d in range(len(DOCS))])
    write(EXPECTED / "bm25_idf.csv", ["term", "idf"], [(t, repr(float(bm.idf[t]))) for t in sorted(bm.idf)])
    top = []
    for q in range(len(QUERIES)):
        order = sorted(range(len(DOCS)), key=lambda d: (-scores[q][d], d))
        for r, d in enumerate(order[:3]):
            top.append((q, r, d))
    write(EXPECTED / "top.csv", ["query", "rank", "doc"], top)
    print("docs", len(DOCS), "queries", len(QUERIES), "vocab", len(vocab), "tfidf cells", len(rows), "avgdl", bm.avgdl)


if __name__ == "__main__":
    main()
