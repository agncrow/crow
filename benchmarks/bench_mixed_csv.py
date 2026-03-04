"""Benchmark mixed-type CSV loading: Pandas vs Polars."""
import time

path = '/tmp/bench_mixed_1gb.csv'

# --- Polars ---
import polars as pl
t0 = time.perf_counter()
df_pl = pl.read_csv(path)
t1 = time.perf_counter()
print(f'Polars:  {t1-t0:.3f}s  shape={df_pl.shape}  dtypes={df_pl.dtypes}')

# --- Pandas ---
import pandas as pd
t0 = time.perf_counter()
df_pd = pd.read_csv(path)
t1 = time.perf_counter()
print(f'Pandas:  {t1-t0:.3f}s  shape={df_pd.shape}  dtypes={list(df_pd.dtypes)}')

