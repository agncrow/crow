# retrieval

Text retrieval written in Crow, held to scikit-learn and rank_bm25 to the
last digit: the words of a text as `TfidfVectorizer`'s default tokenizer
reads them, TF-IDF weights with the smoothed idf and l2-normalised rows,
the cosine of two rows, and BM25 as `BM25Okapi` scores it (k1 = 1.5,
b = 0.75, a negative idf floored at a quarter of the average). Each
answers the same in `crow run -i` and `crow run`, and the same double as
the reference — `reference/reference.py` wrote what the references
compute for a corpus of twenty-four documents and eight queries, and
`held.co` holds the package to it.

```
deps = map[retrieval = 'github.com/agncrow/crow/packages/retrieval@main']
```

`crow install` checks the package out into `crow_modules/retrieval`, and
`crow.lock` pins the commit it took. A program imports the parts it uses:

```
use retrieval.words, retrieval.tfidf, retrieval.bm25
```

## The parts

`words` — `tokens[text]` is the lowered text's runs of two or more
letters, digits or underscores (scikit-learn's `(?u)\b\w\w+\b`; a letter
beyond ASCII separates here where the reference keeps it); `counts[toks]`
a map from each word to how often it occurs; `vocab[docs]` every word of
the tokenised documents, once, sorted; `first_seen[docs]` the same in the
order first met; `frequencies[docs]` in how many documents each occurs.

`tfidf` — `idf[docs, vocabulary]` the smoothed idf of each word,
`ln[(1 + n) / (1 + df)] + 1`; `rows[docs, vocabulary, idfs]` the fitted
documents' rows over the vocabulary, each count times its idf, each row
divided by the root of its sum of squares; `weights[toks, vocabulary,
idfs]` one row for a text transformed afterwards, a query; `cosine[a, b]`
the dot of two rows. The sums run with a fused multiply-add (`fma`), as
the reference's compiled loops run them, and in the reference's order: a
transformed row and a dot in the vocabulary's order, a fitted row in the
order the corpus first met its words (the reference sorts a fitted row's
cells by the index a word had before the vocabulary was sorted). Summed
plainly, or in the other order, a third of the weights differed in the
last digit.

`bm25` — `build[docs]` the index: the documents, their lengths, the
average length, and each word's idf, `ln[n - df + 0.5] - ln[df + 0.5]`,
a negative one replaced by 0.25 times the average (summed in the order
first seen, as the reference sums it); `scores[index, query]` a tokenised
query's score against every document, word by word in the reference's
order of operations; `top[scores, n]` the n best documents, an earlier one
first between equals (the reference leaves equals to its sort; the corpus
has none in its top three).

## Held to the references

```
crow test --packages retrieval          # held.co: every number the same
crow run packages/retrieval/example.co  # the package at work, both modes
```

`reference/reference.py` (scikit-learn 1.9, rank_bm25 0.2.2, numpy 2.5)
rewrites `reference/data/*.csv` and `reference/expected/*.csv`; floats
are written as Python prints them, the shortest text that reads back to
the same double, so a cell Crow reads is the reference's double and the
comparison is `eq`, not a tolerance.
