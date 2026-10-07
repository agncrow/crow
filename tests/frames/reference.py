#!/usr/bin/env python3
"""What pandas answers for each frame operation the module has.

Writes the tables the check reads (tests/frames/data/*.csv) and, for each
case, what pandas computes from them (tests/frames/expected/<case>.csv).
`tests/parity/185_frames_held_to_pandas.co` then holds the frame module
to these files, in both modes, with no Python in the loop (`crow test`
runs its test blocks; `crow test --parity` compares the two modes). Re-run after
adding a case; the tables are drawn from a seeded generator, so a re-run
rewrites the same bytes.

Needs: Python with pandas (3.0 and numpy 2.5 wrote the files checked in).

Where pandas has a choice, the one Crow's spec makes is taken: groups in
the order first seen (sort=False), a stable sort with missing cells last,
a trailing window over the rows there are (min_periods=1), a rank that
shares a rank between equal cells and skips the next (method='min'), the
population standard deviation (ddof=0), a right frame's duplicate column
named `_right`, a right or outer join compared in a sorted order,
since pandas and Crow order its rows differently (the spec says which),
groupby's own `std(ddof=0)` and `quantile` (Welford and a linear
interpolation, as Crow computes them), a rolling `first`, `last` and
`nunique` through `apply`, a time window of `(t - span, t]`, and a
moment's buckets floored in UTC.
"""
import csv
from datetime import timedelta, timezone
from pathlib import Path

import numpy as np
import pandas as pd

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
EXPECTED = HERE / "expected"
SYMBOLS = ["AAPL", "MSFT", "NVDA", "TSLA", "AMZN"]
KINDS = {"id": "int", "symbol": "str", "day": "int", "side": "str", "qty": "int",
         "price": "float", "filled": "bool", "at": "datetime", "note": "str"}


def trades(n=240, seed=7):
    """The trades table: symbols, days, sides, quantities and prices with
    missing cells, a bool, a moment and a note — enough kinds for every
    operation, and one symbol-day whose every price is missing."""
    rng = np.random.default_rng(seed)
    symbol = rng.choice(SYMBOLS, size=n, p=[0.3, 0.25, 0.2, 0.15, 0.1])
    day = rng.integers(1, 13, size=n)
    side = rng.choice(["buy", "sell"], size=n)
    qty = (rng.integers(1, 51, size=n) * 10).astype(float)
    qty[rng.random(n) < 0.08] = np.nan
    price = np.round(rng.uniform(50, 900, size=n), 3)
    price[rng.random(n) < 0.05] = np.nan
    filled = rng.random(n) < 0.7
    seconds = rng.integers(0, 86400, size=n)
    at = pd.Timestamp("2026-09-01", tz="UTC") + pd.to_timedelta((day - 1) * 86400 + seconds, unit="s")
    note = rng.choice(["ok", "late", "partial", "review", "vwap"], size=n).astype(object)
    note[rng.random(n) < 0.12] = None
    t = pd.DataFrame({"id": np.arange(1, n + 1), "symbol": symbol, "day": day, "side": side,
                      "qty": qty, "price": price, "filled": filled, "at": at, "note": note})
    # every price of TSLA on day 12 is missing: a group with nothing to aggregate
    tsla12 = t.index[-3:]
    t.loc[tsla12, "symbol"] = "TSLA"
    t.loc[tsla12, "day"] = 12
    t.loc[tsla12, "price"] = np.nan
    t.loc[tsla12, "at"] = pd.Timestamp("2026-09-12T15:00:00", tz="UTC")
    t.loc[(t.symbol == "TSLA") & (t.day == 12), "price"] = np.nan
    return t


def symbols():
    """The symbols table for joins: one symbol trades lack (META), and
    one of trades' it lacks (AMZN); `note` collides with trades' column."""
    return pd.DataFrame({
        "symbol": ["AAPL", "MSFT", "NVDA", "TSLA", "META"],
        "sector": ["devices", "cloud", "gpus", "autos", "social"],
        "lot": [100, 100, 50, 25, 10],
        "listed": pd.to_datetime(["1980-12-12", "1986-03-13", "1999-01-22", "2010-06-29", "2012-05-18"], utc=True),
        "note": ["mega", "mega", "growth", "growth", "ads"],
    })


def lots():
    """The lots table for joins on two keys: a lot per symbol and day for
    some of trades' pairs, one pair twice (a join repeats the row), and a
    pair trades lack."""
    return pd.DataFrame({
        "symbol": ["AAPL", "AAPL", "MSFT", "NVDA", "NVDA", "META"],
        "day": [1, 3, 2, 5, 5, 1],
        "lot": [100, 110, 50, 25, 26, 10],
    })


def signals():
    """The signals table for an as-of join: a signal per symbol at a moment,
    which each trade takes the latest of at or before its own."""
    at = pd.to_datetime(["2026-09-01T06:00:00", "2026-09-03T12:00:00", "2026-09-07T00:00:00", "2026-09-02T09:30:00",
                         "2026-09-10T15:00:00", "2026-09-05T05:00:00", "2026-09-12T15:00:00"], utc=True)
    return pd.DataFrame({
        "symbol": ["AAPL", "AAPL", "AAPL", "MSFT", "MSFT", "NVDA", "TSLA"],
        "at": at,
        "signal": [1, 2, 3, 4, 5, 6, 7],
    })


def write(name, df, ints=(), folder=EXPECTED):
    """One CSV as Crow reads it: an int column of whole numbers (`ints`
    names those pandas made float for their missing cells), a bool as
    true/false, a moment as ISO 8601, a missing cell empty."""
    out = df.copy().reset_index(drop=True)
    for c in out.columns:
        s = out[c]
        if c in ints:
            out[c] = s.astype("Int64")
        elif pd.api.types.is_bool_dtype(s):
            out[c] = s.map({True: "true", False: "false"})
        elif pd.api.types.is_datetime64_any_dtype(s):
            out[c] = s.apply(lambda v: "" if pd.isna(v) else v.isoformat())
        elif s.dtype == object or pd.api.types.is_string_dtype(s):
            # a bool column a join left holes in is an object column of bools and NaN
            out[c] = s.apply(lambda v: "" if (v is None or (isinstance(v, float) and np.isnan(v)) or v is pd.NA)
                             else ("true" if v is True else "false" if v is False else v))
    out.to_csv(folder / f"{name}.csv", index=False, lineterminator="\n", quoting=csv.QUOTE_MINIMAL)


def main():
    DATA.mkdir(exist_ok=True)
    EXPECTED.mkdir(exist_ok=True)
    t = trades()
    sym = symbols()
    write("trades", t, ints=["qty"], folder=DATA)
    write("symbols", sym, folder=DATA)
    lot = lots()
    sig = signals()
    write("lots", lot, folder=DATA)
    write("signals", sig, folder=DATA)
    with open(DATA / "notes.csv", "w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["id", "note"])
        w.writerow([1, "a, b"])
        w.writerow([2, 'say "hi"'])
        w.writerow([3, "plain"])
        w.writerow([4, "quoted, with \"both\""])

    # --- what the file holds
    write("meta", pd.DataFrame({"column": list(KINDS), "dtype": list(KINDS.values())}))

    # --- select, filter
    write("select_symbol_qty", t[["symbol", "qty"]], ints=["qty"])
    write("filter_big", t[t.qty > 300], ints=["qty"])
    write("filter_aapl_sell", t[(t.symbol == "AAPL") & (t.side == "sell")], ints=["qty"])
    write("filter_or_not", t[(t.price < 100) | ~(t.qty >= 50)], ints=["qty"])
    write("filter_ok", t[t.qty.notna() & t.price.notna()], ints=["qty"])
    write("filter_filled", t[t.filled], ints=["qty"])
    write("filter_day", t[t.day == 3], ints=["qty"])

    # --- sort: stable, missing cells last either way
    write("sort_qty", t.sort_values("qty", kind="stable", na_position="last"), ints=["qty"])
    write("sort_price_desc", t.sort_values("price", ascending=False, kind="stable", na_position="last"), ints=["qty"])
    write("sort_symbol", t.sort_values("symbol", kind="stable"), ints=["qty"])
    write("sort_at_desc", t.sort_values("at", ascending=False, kind="stable"), ints=["qty"])
    write("sort_filled", t.sort_values("filled", kind="stable"), ints=["qty"])
    write("head_5", t.head(5), ints=["qty"])
    write("tail_7", t.tail(7), ints=["qty"])

    # --- group: first seen first; a count counts present cells; a sum of
    # nothing is 0, a min, max or mean of nothing is missing
    g = t.groupby("symbol", sort=False)
    write("group_symbol", g.agg(qty_sum=("qty", "sum"), qty_mean=("qty", "mean"), price_min=("price", "min"),
                                price_max=("price", "max"), price_count=("price", "count"),
                                filled_sum=("filled", "sum")).reset_index(),
          ints=["qty_sum", "price_count", "filled_sum"])
    g2 = t.groupby(["symbol", "side"], sort=False)
    write("group_symbol_side", g2.agg(qty_sum=("qty", "sum"), price_mean=("price", "mean"), id_count=("id", "count")).reset_index(),
          ints=["qty_sum", "id_count"])
    g3 = t.groupby("day", sort=False)
    write("group_day", g3.agg(price_mean=("price", "mean"), qty_max=("qty", "max"), qty_min=("qty", "min"),
                              filled_max=("filled", "max")).reset_index(), ints=["qty_max", "qty_min"])
    g4 = t.groupby(["symbol", "day"], sort=False)
    write("group_symbol_day", g4.agg(price_min=("price", "min"), price_max=("price", "max"), price_mean=("price", "mean"),
                                     price_sum=("price", "sum"), price_count=("price", "count")).reset_index(),
          ints=["price_count"])

    # --- pivot: the long frame made wide, rows and columns ascending
    long = g4.agg(qty_sum=("qty", "sum")).reset_index()
    wide = long.pivot(index="symbol", columns="day", values="qty_sum").reset_index()
    wide.columns = ["symbol"] + [str(c) for c in wide.columns[1:]]
    write("pivot_qty", wide, ints=list(wide.columns[1:]))

    # --- windows over the filled frame (a missing cell stops a sum in Crow)
    tf = t.copy()
    tf["qty"] = tf.qty.fillna(0).astype(int)
    tf["price"] = tf.price.fillna(0.0)
    gs = tf.groupby("symbol", sort=False)
    base = t[["id", "symbol"]]
    write("window_running", base.assign(
        run_sum=gs.qty.cumsum(),
        run_mean=gs.price.transform(lambda s: s.expanding().mean()),
        run_min=gs.price.cummin(),
        run_max=gs.price.cummax(),
        run_count=t.groupby("symbol", sort=False).note.transform(lambda s: s.notna().cumsum()),
        run_sum_all=tf.qty.cumsum(),
    ), ints=["run_sum", "run_count", "run_sum_all"])
    write("window_trailing", base.assign(
        tr_sum=gs.qty.transform(lambda s: s.rolling(3, min_periods=1).sum()),
        tr_mean=gs.price.transform(lambda s: s.rolling(3, min_periods=1).mean()),
        tr_min=gs.price.transform(lambda s: s.rolling(3, min_periods=1).min()),
        tr_max=gs.price.transform(lambda s: s.rolling(3, min_periods=1).max()),
        tr_count=t.groupby("symbol", sort=False).note.transform(lambda s: s.notna().rolling(3, min_periods=1).sum()),
    ), ints=["tr_sum", "tr_count"])
    gt = t.groupby("symbol", sort=False)
    write("window_shift", base.assign(
        lag1=gt.price.shift(1),
        lead2=gt.price.shift(-2),
        lag_note=gt.note.shift(1),
        lag_qty=gt.qty.shift(1),
    ), ints=["lag_qty"])
    write("window_rank", base.assign(
        rank_sym=gt.price.rank(method="min"),
        rank_all=t.price.rank(method="min"),
        rank_text=t.symbol.rank(method="min"),
        rank_day=gt.day.rank(method="min"),
    ), ints=["rank_sym", "rank_all", "rank_text", "rank_day"])

    # --- joins: a's columns then b's but its key; the right frame's
    # duplicate column named _right
    write("join_inner", t.merge(sym, on="symbol", how="inner", suffixes=("", "_right")), ints=["qty"])
    write("join_left", t.merge(sym, on="symbol", how="left", suffixes=("", "_right")), ints=["qty", "lot"])
    right = t.merge(sym, on="symbol", how="right", suffixes=("", "_right"))
    write("join_right", right.sort_values("id", kind="stable", na_position="last"), ints=["id", "day", "qty"])
    outer = t.merge(sym, on="symbol", how="outer", suffixes=("", "_right"))
    write("join_outer", outer.sort_values(["symbol", "id"], kind="stable", na_position="last"), ints=["id", "day", "qty", "lot"])

    # --- stack, fill, unique, rename, a computed column, reductions
    write("stack_two", pd.concat([t.head(3), t.tail(3)]), ints=["qty"])
    write("fill_qty0", t.assign(qty=t.qty.fillna(0)), ints=["qty"])
    write("fill_note", t.assign(note=t.note.fillna("-")), ints=["qty"])
    write("fill_price", t.assign(price=t.price.fillna(0.0)), ints=["qty"])
    write("unique_symbol", pd.DataFrame({"value": pd.unique(t.symbol)}))
    write("unique_day", pd.DataFrame({"value": pd.unique(t.day)}))
    write("rename_qty", t.rename(columns={"qty": "shares"}), ints=["shares"])
    write("with_notional", tf.assign(notional=tf.qty * tf.price), ints=["qty"])
    write("reduce", pd.DataFrame({
        "rows": [len(t)],
        "notional_sum": [(tf.qty * tf.price).sum()],
        "price_mean": [tf.price.mean()],
        "price_std": [tf.price.std(ddof=0)],
        "qty_max": [int(tf.qty.max())],
        "price_min": [tf.price.min()],
    }), ints=["rows", "qty_max"])

    # --- moments: parsed, and moved to an offset
    cases = [("2026-07-14T13:30:00Z", 9 * 60), ("2026-01-05", 0), ("2026-03-29T01:30:00+01:00", -5 * 60),
             ("2026-12-31T23:59:59-08:00", 5 * 60 + 30), ("2026-06-30T12:00:00+05:30", -9 * 60 - 30)]
    rows = []
    for text, minutes in cases:
        stamp = pd.Timestamp(text)
        if stamp.tzinfo is None:
            stamp = stamp.tz_localize("UTC")  # a date alone is midnight UTC, as Crow reads it
        moved = stamp.tz_convert(timezone(timedelta(minutes=minutes)))
        sign = "+" if minutes >= 0 else "-"
        offset = "UTC" if minutes == 0 else f"{sign}{abs(minutes) // 60:02d}:{abs(minutes) % 60:02d}"
        rows.append({"input": text, "offset": offset, "output": moved.isoformat(),
                     "year": moved.year, "month": moved.month, "day": moved.day, "hour": moved.hour})
    write("datetime_tz", pd.DataFrame(rows))

    # --- the other aggregates: a median and a percentile interpolated,
    # the population deviation (groupby's Welford), the first and last
    # present cell in the column's kind, and a count of distinct cells
    more = g.agg(price_median=("price", "median"), qty_first=("qty", "first"), note_last=("note", "last"),
                 note_distinct=("note", "nunique"), at_first=("at", "first"), at_last=("at", "last")).reset_index()
    more["price_std"] = g.price.std(ddof=0).values
    more["price_var"] = g.price.var(ddof=0).values
    more["price_p90"] = g.price.quantile(0.9).values
    more["qty_p25"] = g.qty.quantile(0.25).values
    more = more[["symbol", "price_median", "price_std", "price_var", "price_p90", "qty_p25", "qty_first", "note_last", "note_distinct", "at_first", "at_last"]]
    write("group_more", more, ints=["qty_first", "note_distinct"])

    # --- windows of the other aggregates, trailing and running, and
    # over a span of time
    first_present = lambda w: w.dropna().iloc[0] if w.notna().any() else np.nan
    last_present = lambda w: w.dropna().iloc[-1] if w.notna().any() else np.nan
    write("window_more", base.assign(
        tr_median=gs.price.transform(lambda s: s.rolling(3, min_periods=1).median()),
        tr_std=gs.price.transform(lambda s: s.rolling(3, min_periods=1).std(ddof=0)),
        tr_var=gs.price.transform(lambda s: s.rolling(3, min_periods=1).var(ddof=0)),
        tr_p90=gs.price.transform(lambda s: s.rolling(3, min_periods=1).quantile(0.9, interpolation="linear")),
        run_median=gs.price.transform(lambda s: s.expanding().median()),
        run_std=gs.price.transform(lambda s: s.expanding().std(ddof=0)),
        run_p10=gs.price.transform(lambda s: s.expanding().quantile(0.1, interpolation="linear")),
        tr_first=gt.price.transform(lambda s: s.rolling(3, min_periods=1).apply(first_present, raw=False)),
        tr_last=gt.price.transform(lambda s: s.rolling(3, min_periods=1).apply(last_present, raw=False)),
        tr_distinct=gt.day.transform(lambda s: s.rolling(3, min_periods=1).apply(lambda w: w.nunique(), raw=False)),
        run_distinct=gt.day.transform(lambda s: s.expanding().apply(lambda w: w.nunique(), raw=False)),
    ), ints=["tr_distinct", "run_distinct"])
    ts = tf.sort_values("at", kind="stable").reset_index(drop=True)
    span_mean = np.empty(len(ts))
    span_sum = np.empty(len(ts))
    span_max = np.empty(len(ts))
    span_count = np.empty(len(ts))
    for _, idx in ts.groupby("symbol", sort=False).groups.items():
        sub = ts.loc[idx]
        r = sub.rolling("6h", on="at", min_periods=1)
        span_mean[idx] = r.price.mean().values
        span_sum[idx] = r.qty.sum().values
        span_max[idx] = r.price.max().values
        span_count[idx] = r.price.count().values
    whole = ts.rolling("6h", on="at", min_periods=1)
    write("window_span", ts[["id", "symbol", "at"]].assign(
        span_mean=span_mean, span_sum=span_sum, span_max=span_max, span_count=span_count,
        all_mean=whole.price.mean().values,
    ), ints=["span_sum", "span_count"])

    # --- several keys: a sort by two, a join on two (left, inner, semi,
    # anti), distinct rows by two, and an as-of join on a symbol and a moment
    write("sort_keys", t.sort_values(["symbol", "qty"], ascending=[True, False], kind="stable", na_position="last"), ints=["qty"])
    write("sort_keys_three", t.sort_values(["day", "side", "price"], kind="stable", na_position="last"), ints=["qty"])
    write("join_keys_left", t.merge(lot, on=["symbol", "day"], how="left", suffixes=("", "_right")), ints=["qty", "lot"])
    write("join_keys_inner", t.merge(lot, on=["symbol", "day"], how="inner", suffixes=("", "_right")), ints=["qty"])
    pairs = t.set_index(["symbol", "day"]).index.isin(lot.set_index(["symbol", "day"]).index)
    write("join_semi", t[pairs], ints=["qty"])
    write("join_anti", t[~pairs], ints=["qty"])
    write("unique_rows", t.drop_duplicates(subset=["symbol", "side"]), ints=["qty"])
    left = t.sort_values("at", kind="stable")
    write("join_asof", pd.merge_asof(left, sig.sort_values("at", kind="stable"), on="at", by="symbol", suffixes=("", "_right")), ints=["qty", "signal"])

    # --- buckets of moments, and bars from them
    bucketed = t.assign(bucket=t["at"].dt.floor("6h"))
    write("bucket_6h", bucketed, ints=["qty"])
    bars = bucketed.groupby(["symbol", "bucket"], sort=False).agg(
        price_first=("price", "first"), price_max=("price", "max"), price_min=("price", "min"), price_last=("price", "last"), qty_sum=("qty", "sum")).reset_index()
    write("bars_6h", bars, ints=["qty_sum"])

    # --- text over a column, and a moment's fields over one
    write("text_columns", pd.DataFrame({
        "id": t.id,
        "lower": t.note.str.lower(),
        "upper": t.symbol.str.lower().str.upper(),
        "replaced": t.note.str.replace("a", "A", regex=False),
        "subbed": t.note.str.replace(r"[aeiou]", "_", regex=True),
        "has_a": t.note.str.contains("a", regex=False, na=False),
        "starts_r": t.note.str.startswith("r", na=False),
        "ends_k": t.note.str.endswith("k", na=False),
        "hour": t["at"].dt.hour, "weekday": t["at"].dt.weekday + 1, "day": t["at"].dt.day, "month": t["at"].dt.month,
        "year": t["at"].dt.year, "minute": t["at"].dt.minute, "second": t["at"].dt.second,
    }))

    # --- unpivot, fills by direction, complete rows, a conditional
    # column and casts
    write("unpivot_qty_price", t[["id", "symbol", "qty", "price"]].melt(id_vars=["id", "symbol"], value_vars=["qty", "price"]))
    write("fill_forward", t.assign(qty=t.qty.ffill()), ints=["qty"])
    write("fill_backward", t.assign(note=t.note.bfill()), ints=["qty"])
    write("complete_all", t.dropna(), ints=["qty"])
    write("complete_qty_price", t.dropna(subset=["qty", "price"]), ints=["qty"])
    write("where_columns", base.assign(
        picked=np.where(t.filled, tf.qty, 0),
        label=np.where(t.day == 1, "one", t.symbol),
        px=np.where(t.price > 500, t.price, tf.qty * 1.0),
    ), ints=["picked"])
    write("casts", pd.DataFrame({
        "id": t.id,
        "price_int": t.price.apply(lambda x: np.nan if pd.isna(x) else float(int(x))),
        "qty_float": t.qty.astype(float),
        "filled_int": t.filled.astype(int),
    }), ints=["price_int"])

    print(f"wrote {len(list(EXPECTED.glob('*.csv')))} expected files and {len(list(DATA.glob('*.csv')))} tables")


if __name__ == "__main__":
    main()
