#!/usr/bin/env python3
"""Frame benchmark: Pandas vs Polars — same operations as bench_frame.co."""
import time
import sys

CSV_PATH = '/tmp/bench_mixed_1gb.csv'

def bench(label, fn, n=1):
    sys.stdout.flush()
    t0 = time.perf_counter()
    for _ in range(n):
        result = fn()
    elapsed = time.perf_counter() - t0
    print(f'  {label}: {elapsed:.3f}s')
    sys.stdout.flush()
    return elapsed

# Warm page cache
print('Warming page cache...')
with open(CSV_PATH, 'rb') as f:
    while f.read(64 * 1024 * 1024):
        pass
print('Cache warm.\n')

# ============================================================
# PANDAS
# ============================================================
import pandas as pd
print('=== PANDAS ===')

t0 = time.perf_counter()
df_pd = pd.read_csv(CSV_PATH)
t1 = time.perf_counter()
print(f'  load: {t1-t0:.3f}s  shape={df_pd.shape}')

bench('select x10', lambda: df_pd[['name', 'age', 'salary']], n=10)
bench('head x100', lambda: df_pd.head(1000), n=100)
bench('sort', lambda: df_pd.sort_values('salary'))
bench('filter', lambda: df_pd[df_pd['salary'] > 100000])
bench('group', lambda: df_pd.groupby('dept').agg({'salary': 'mean'}))

# Join: self-join with grouped result
dept_means_pd = df_pd.groupby('dept').agg({'salary': 'mean'}).reset_index()
bench('join', lambda: df_pd.merge(dept_means_pd, on='dept', how='inner'))

bench('rename x10', lambda: df_pd.rename(columns={'salary': 'pay'}), n=10)
bench('fill x10', lambda: df_pd['age'].fillna(0), n=10)
bench('unique x100', lambda: df_pd['dept'].unique(), n=100)
bench('stats', lambda: df_pd.describe())

# Stack
h1m_pd = df_pd.head(1000000)
bench('stack', lambda: pd.concat([df_pd, h1m_pd], ignore_index=True))

# Save
bench('save', lambda: df_pd.to_csv('/tmp/bench_pandas_save.csv', index=False))

del df_pd
print()

# ============================================================
# POLARS
# ============================================================
import polars as pl
print('=== POLARS ===')

t0 = time.perf_counter()
df_pl = pl.read_csv(CSV_PATH)
t1 = time.perf_counter()
print(f'  load: {t1-t0:.3f}s  shape={df_pl.shape}')

bench('select x10', lambda: df_pl.select(['name', 'age', 'salary']), n=10)
bench('head x100', lambda: df_pl.head(1000), n=100)
bench('sort', lambda: df_pl.sort('salary'))
bench('filter', lambda: df_pl.filter(pl.col('salary') > 100000))
bench('group', lambda: df_pl.group_by('dept').agg(pl.col('salary').mean()))

# Join: self-join with grouped result
dept_means_pl = df_pl.group_by('dept').agg(pl.col('salary').mean())
bench('join', lambda: df_pl.join(dept_means_pl, on='dept', how='inner'))

bench('rename x10', lambda: df_pl.rename({'salary': 'pay'}), n=10)
bench('fill x10', lambda: df_pl.with_columns(pl.col('age').fill_null(0)), n=10)
bench('unique x100', lambda: df_pl['dept'].unique(), n=100)
bench('stats', lambda: df_pl.describe())

# Stack
h1m_pl = df_pl.head(1000000)
bench('stack', lambda: pl.concat([df_pl, h1m_pl]))

# Save
bench('save', lambda: df_pl.write_csv('/tmp/bench_polars_save.csv'))

print('\n=== Done ===')

