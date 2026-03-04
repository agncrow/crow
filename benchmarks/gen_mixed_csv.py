"""Generate a ~1GB mixed-type CSV file for benchmarking."""
import random
import string
import os

path = '/tmp/bench_mixed_1gb.csv'
target_bytes = 1_000_000_000  # 1 GB

# Columns: id(int), name(str), city(str), age(int), salary(float), score(float), dept(str), active(str), x(float), y(float)
cities = ['new_york', 'san_francisco', 'chicago', 'seattle', 'austin', 'boston', 'denver', 'miami', 'portland', 'atlanta']
depts = ['engineering', 'sales', 'marketing', 'finance', 'hr', 'ops', 'research', 'support']
names = [''.join(random.choices(string.ascii_lowercase, k=random.randint(4, 10))) for _ in range(1000)]

with open(path, 'w') as f:
    f.write('id,name,city,age,salary,score,dept,active,x,y\n')
    row_id = 0
    written = 0
    while written < target_bytes:
        name = random.choice(names)
        city = random.choice(cities)
        age = random.randint(18, 75)
        salary = round(random.uniform(30000, 200000), 2)
        score = round(random.uniform(0, 100), 4)
        dept = random.choice(depts)
        active = random.choice(['true', 'false'])
        x = round(random.uniform(-180, 180), 6)
        y = round(random.uniform(-90, 90), 6)
        line = f'{row_id},{name},{city},{age},{salary},{score},{dept},{active},{x},{y}\n'
        f.write(line)
        written += len(line)
        row_id += 1
        if row_id % 1000000 == 0:
            print(f'  {row_id} rows, {written/1e9:.3f} GB')

size = os.path.getsize(path)
print(f'Done: {row_id} rows, {size/1e9:.3f} GB')

