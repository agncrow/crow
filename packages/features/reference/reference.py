# -*- coding: utf-8 -*-
"""What the feature store is held to (held.co): feature rows pandas
computes from a seeded stream of events — a running count and a running
sum per user — and the training set pandas's `merge_asof` builds from
twelve labels against them: by user, backward (the newest feature row at
or before the label's moment), an exact moment matching, a missing cell
where none precedes.

    ~/.venvs/finch_ml/bin/python packages/features/reference/reference.py

Writes data/features.csv (user, at, clicks, spent), data/labels.csv
(user, at, label) and expected/training.csv (the labels with clicks and
spent beside them). The moments are milliseconds, strictly increasing
over the feature rows, so the newest row at a moment is also the newest
by position, which is how Magpie's `prior` breaks a tie. Every CSV has
a header row (`load` reads the first row as names); an int column with
a missing cell is written as ints and an empty cell, not as floats.
pandas 3.0.2 and numpy 2.5.2 wrote the files checked in; a re-run
rewrites the same bytes.
"""
import os

import numpy as np
import pandas as pd

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
EXPECTED = os.path.join(HERE, "expected")

USERS = ["u1", "u2", "u3", "u4"]
ROWS = 60
LABELS = 12


def features():
    rng = np.random.default_rng(7)
    steps = rng.integers(100, 2000, size=ROWS)
    at = 1000 + np.cumsum(steps)
    user = rng.choice(USERS, size=ROWS)
    amount = np.round(rng.uniform(1.0, 100.0, size=ROWS), 2)
    events = pd.DataFrame({"user": user, "at": at.astype("int64"), "amount": amount})
    rows = events.copy()
    rows["clicks"] = rows.groupby("user").cumcount() + 1
    rows["spent"] = rows.groupby("user")["amount"].cumsum()
    return rows[["user", "at", "clicks", "spent"]]


def labels(rows):
    rng = np.random.default_rng(11)
    last = int(rows["at"].max())
    # moments of their own: some before the first feature row, so a label
    # has no feature row to take
    at = rng.choice(np.arange(0, last + 5000, 7), size=LABELS, replace=False)
    user = rng.choice(USERS, size=LABELS)
    label = rng.integers(0, 2, size=LABELS)
    out = pd.DataFrame({"user": user, "at": at.astype("int64"), "label": label.astype("int64")})
    return out.sort_values("at", kind="stable").reset_index(drop=True)


def write(path, frame, ints=()):
    frame = frame.copy()
    for c in ints:
        frame[c] = frame[c].astype("Int64")
    frame.to_csv(path, index=False)


def main():
    os.makedirs(DATA, exist_ok=True)
    os.makedirs(EXPECTED, exist_ok=True)
    rows = features()
    labs = labels(rows)
    expected = pd.merge_asof(labs, rows, on="at", by="user")
    write(os.path.join(DATA, "features.csv"), rows, ints=["clicks"])
    write(os.path.join(DATA, "labels.csv"), labs)
    write(os.path.join(EXPECTED, "training.csv"), expected, ints=["clicks"])
    print(f"{len(rows)} feature rows, {len(labs)} labels, {int(expected['clicks'].isna().sum())} without a feature row")


if __name__ == "__main__":
    main()
