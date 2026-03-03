#!/usr/bin/env python3
"""Benchmark CSV read + load to numeric array: NumPy, Pandas, Polars."""
import time
import sys

CSV_PATH = '/tmp/bench_1gb.csv'

def bench(name, fn):
    # Warmup OS page cache on first run
    print(f'\n--- {name} ---')
    sys.stdout.flush()
    t0 = time.perf_counter()
    result = fn()
    elapsed = time.perf_counter() - t0
    print(f'  time: {elapsed:.3f}s')
    print(f'  shape: {result.shape}')
    print(f'  sum: {result.sum():.6f}')  # verify data loaded correctly
    sys.stdout.flush()
    return elapsed

# --- NumPy ---
def bench_numpy():
    import numpy as np
    return np.loadtxt(CSV_PATH, delimiter=',', skiprows=1)

# --- Pandas ---
def bench_pandas():
    import pandas as pd
    df = pd.read_csv(CSV_PATH)
    return df.values

# --- Polars ---
def bench_polars():
    import polars as pl
    df = pl.read_csv(CSV_PATH)
    return df.to_numpy()

print(f'Benchmarking CSV read from {CSV_PATH}')
import os
size_gb = os.path.getsize(CSV_PATH) / 1e9
print(f'File size: {size_gb:.3f} GB')

# Warm OS page cache
print('\nWarming page cache...')
with open(CSV_PATH, 'rb') as f:
    while f.read(1024*1024*64):
        pass
print('Cache warm.')

results = {}
results['numpy'] = bench('NumPy (loadtxt)', bench_numpy)
results['pandas'] = bench('Pandas (read_csv)', bench_pandas)

try:
    results['polars'] = bench('Polars (read_csv)', bench_polars)
except Exception as e:
    print(f'\nPolars failed: {e}')

print('\n=== Summary ===')
for name, t in sorted(results.items(), key=lambda x: x[1]):
    print(f'  {name:12s}: {t:.3f}s')

