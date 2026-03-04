"""Generate a CSV with datetime + numeric columns for benchmarking gaps 1-5."""
import random
import os

path = '/tmp/bench_datetime.csv'
N = 5_000_000  # 5M rows — enough for meaningful timing

random.seed(42)

with open(path, 'w') as f:
    f.write('ts,age,salary,score\n')
    for i in range(N):
        # Random datetime in 2020-2025
        year = random.randint(2020, 2025)
        month = random.randint(1, 12)
        day = random.randint(1, 28)
        hour = random.randint(0, 23)
        minute = random.randint(0, 59)
        second = random.randint(0, 59)
        offset_h = random.choice([-5, 0, 5, 8])
        sign = '+' if offset_h >= 0 else '-'
        ts = f'{year}-{month:02d}-{day:02d}T{hour:02d}:{minute:02d}:{second:02d}{sign}{abs(offset_h):02d}:00'
        age = random.randint(18, 75)
        # 5% missing (empty) for age
        age_str = '' if random.random() < 0.05 else str(age)
        salary = round(random.uniform(30000, 200000), 2)
        score = round(random.uniform(0, 100), 4)
        f.write(f'{ts},{age_str},{salary},{score}\n')
        if (i + 1) % 1_000_000 == 0:
            print(f'  {(i+1)//1_000_000}M rows')

size = os.path.getsize(path)
print(f'Done: {N} rows, {size/1e6:.1f} MB')

