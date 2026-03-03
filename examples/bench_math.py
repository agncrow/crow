"""NumPy equivalent of bench_math.crow for performance comparison."""
import time
import numpy as np

print("=== Tensor Creation ===")

t0 = time.time()
for _ in range(100):
    x = np.zeros((1000, 1000))
t1 = time.time()
print(f"zeros[1000,1000] x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.ones((1000, 1000))
t1 = time.time()
print(f"ones[1000,1000] x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.eye(1000)
t1 = time.time()
print(f"eye[1000] x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.random.randn(1000, 1000)
t1 = time.time()
print(f"randn[1000,1000] x100: {t1-t0}s")

print("=== Element-wise Ops (1M elements) ===")
a = np.random.randn(1000, 1000)

t0 = time.time()
for _ in range(10):
    x = np.exp(a)
t1 = time.time()
print(f"exp x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = np.sin(a)
t1 = time.time()
print(f"sin x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = np.sqrt(np.abs(a))
t1 = time.time()
print(f"sqrt x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = a + a
t1 = time.time()
print(f"add x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = a * a
t1 = time.time()
print(f"mul x10: {t1-t0}s")

print("=== Reductions (1M elements) ===")

t0 = time.time()
for _ in range(100):
    x = np.sum(a)
t1 = time.time()
print(f"sum x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.mean(a)
t1 = time.time()
print(f"mean x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.min(a)
t1 = time.time()
print(f"min x100: {t1-t0}s")

t0 = time.time()
for _ in range(100):
    x = np.max(a)
t1 = time.time()
print(f"max x100: {t1-t0}s")

print("=== Matmul ===")

m100 = np.random.randn(100, 100)
t0 = time.time()
for _ in range(100):
    x = m100 @ m100
t1 = time.time()
print(f"matmul 100x100 x100: {t1-t0}s")

m500 = np.random.randn(500, 500)
t0 = time.time()
for _ in range(10):
    x = m500 @ m500
t1 = time.time()
print(f"matmul 500x500 x10: {t1-t0}s")

m1000 = np.random.randn(1000, 1000)
t0 = time.time()
for _ in range(3):
    x = m1000 @ m1000
t1 = time.time()
print(f"matmul 1000x1000 x3: {t1-t0}s")

print("=== Linalg ===")

m200 = np.random.randn(200, 200)
b200 = np.random.randn(200, 1)

t0 = time.time()
for _ in range(10):
    x = np.linalg.solve(m200, b200)
t1 = time.time()
print(f"solve 200x200 x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = np.linalg.inv(m200)
t1 = time.time()
print(f"inv 200x200 x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = np.linalg.det(m200)
t1 = time.time()
print(f"det 200x200 x10: {t1-t0}s")

t0 = time.time()
for _ in range(10):
    x = np.linalg.qr(m200)
t1 = time.time()
print(f"qr 200x200 x10: {t1-t0}s")

t0 = time.time()
for _ in range(3):
    x = np.linalg.svd(m200)
t1 = time.time()
print(f"svd 200x200 x3: {t1-t0}s")

print("=== done ===")

