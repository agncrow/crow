#!/usr/bin/env python3
"""Generate a ~1GB CSV file with 10 columns of random floats."""
import random
import sys
import os

random.seed(42)
outpath = '/tmp/bench_1gb.csv'
cols = ['c0','c1','c2','c3','c4','c5','c6','c7','c8','c9']
target_bytes = 1_000_000_000  # ~1GB

with open(outpath, 'w') as f:
    f.write(','.join(cols) + '\n')
    written = 0
    rows = 0
    while written < target_bytes:
        line = ','.join(f'{random.uniform(-100, 100):.6f}' for _ in range(10))
        f.write(line + '\n')
        written += len(line) + 1
        rows += 1
        if rows % 1_000_000 == 0:
            print(f'  {rows/1e6:.0f}M rows, {written/1e9:.2f} GB', file=sys.stderr)

actual = os.path.getsize(outpath)
print(f'Generated {outpath}: {rows} rows, {actual/1e9:.3f} GB')

