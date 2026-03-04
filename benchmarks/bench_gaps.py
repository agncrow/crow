"""Benchmark gaps 1-5 with pandas and polars for comparison."""
import time
import pandas as pd
import polars as pl
import numpy as np

path = '/tmp/bench_datetime.csv'

# ===== PANDAS =====
print("=" * 50)
print("PANDAS")
print("=" * 50)

t0 = time.time()
pdf = pd.read_csv(path)
pdf['ts'] = pd.to_datetime(pdf['ts'], utc=True)
t1 = time.time()
print(f"Load + parse datetime: {t1-t0:.3f}s  shape: {pdf.shape}")

# Gap 1: DateTime properties
t0 = time.time()
for _ in range(100):
    y = pdf['ts'].dt.year
    m = pdf['ts'].dt.month
    d = pdf['ts'].dt.day
    h = pdf['ts'].dt.hour
t1 = time.time()
print(f"\nGap 1 - datetime props x100: {t1-t0:.3f}s")
print(f"  sample: year={y.iloc[0]} month={m.iloc[0]} day={d.iloc[0]} hour={h.iloc[0]}")

# Gap 2: Vectorized comparison
t0 = time.time()
for _ in range(10):
    mask = pdf['age'] > 50
t1 = time.time()
print(f"\nGap 2 - age > 50 x10: {t1-t0:.3f}s")
print(f"  matches: {mask.sum()}")

# Gap 3: Column arithmetic
t0 = time.time()
for _ in range(10):
    raised = pdf['salary'] * 1.1
t1 = time.time()
print(f"\nGap 3 - salary * 1.1 x10: {t1-t0:.3f}s")

t0 = time.time()
for _ in range(10):
    bonus = pdf['salary'] + 5000
t1 = time.time()
print(f"  salary + 5000 x10: {t1-t0:.3f}s")

t0 = time.time()
for _ in range(10):
    halved = pdf['salary'] / 2
t1 = time.time()
print(f"  salary / 2 x10: {t1-t0:.3f}s")

# Gap 4: Frame to numpy array
t0 = time.time()
for _ in range(10):
    arr = pdf[['age', 'salary', 'score']].to_numpy()
t1 = time.time()
print(f"\nGap 4 - to_numpy[3cols] x10: {t1-t0:.3f}s  shape: {arr.shape}")

# Gap 5: Validity bitmap
t0 = time.time()
for _ in range(10):
    valid = pdf['age'].notna()
t1 = time.time()
print(f"\nGap 5 - age.notna() x10: {t1-t0:.3f}s")

# ===== POLARS =====
print("\n" + "=" * 50)
print("POLARS")
print("=" * 50)

t0 = time.time()
pldf = pl.read_csv(path)
# Parse datetime column
pldf = pldf.with_columns(pl.col('ts').str.to_datetime(format="%Y-%m-%dT%H:%M:%S%z").alias('ts'))
t1 = time.time()
print(f"Load: {t1-t0:.3f}s  shape: {pldf.shape}")

# Gap 1: DateTime properties
t0 = time.time()
for _ in range(100):
    y = pldf['ts'].dt.year()
    m = pldf['ts'].dt.month()
    d = pldf['ts'].dt.day()
    h = pldf['ts'].dt.hour()
t1 = time.time()
print(f"\nGap 1 - datetime props x100: {t1-t0:.3f}s")
print(f"  sample: year={y[0]} month={m[0]} day={d[0]} hour={h[0]}")

# Gap 2: Vectorized comparison
t0 = time.time()
for _ in range(10):
    mask = pldf['age'] > 50
t1 = time.time()
print(f"\nGap 2 - age > 50 x10: {t1-t0:.3f}s")
print(f"  matches: {mask.sum()}")

# Gap 3: Column arithmetic
t0 = time.time()
for _ in range(10):
    raised = pldf['salary'] * 1.1
t1 = time.time()
print(f"\nGap 3 - salary * 1.1 x10: {t1-t0:.3f}s")

t0 = time.time()
for _ in range(10):
    bonus = pldf['salary'] + 5000
t1 = time.time()
print(f"  salary + 5000 x10: {t1-t0:.3f}s")

t0 = time.time()
for _ in range(10):
    halved = pldf['salary'] / 2
t1 = time.time()
print(f"  salary / 2 x10: {t1-t0:.3f}s")

# Gap 4: Frame to numpy
t0 = time.time()
for _ in range(10):
    arr = pldf.select(['age', 'salary', 'score']).to_numpy()
t1 = time.time()
print(f"\nGap 4 - to_numpy[3cols] x10: {t1-t0:.3f}s  shape: {arr.shape}")

# Gap 5: Validity bitmap
t0 = time.time()
for _ in range(10):
    valid = pldf['age'].is_not_null()
t1 = time.time()
print(f"\nGap 5 - age.is_not_null() x10: {t1-t0:.3f}s")

print("\nDone")

