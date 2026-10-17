# Crow Math Library Design

Crow is a data science language. The math library must be powerful enough to build deep learning frameworks like PyTorch from scratch, including distributed training and GPU computing. The runtime is pure Rust — no C/C++ dependencies. CPU math is backed by `faer` (fastest pure Rust linear algebra), GPU compute by `wgpu` (cross-platform WebGPU), and the tensor type is our own.

## Layers

### Layer 0: Array

The foundation. Not nested arrays in the interpreter sense — a contiguous block of memory with shape, stride, and dtype. Every numerical operation in ML and scientific computing flows through arrays.

```
--- literals
x = [1, 2, 3]                             --- 1D
m = [[1, 2], [3, 4]]                      --- 2D

--- creation
z = zeros[3, 4]                            --- 3x4 of zeros
o = ones[2, 3]                             --- 2x3 of ones
r = uniform[0.0, 1.0, 2, 3]                --- 2x3 uniform random
f = full[3, 4, 7.0]                        --- 3x4 filled with 7.0

--- ranges
a = arange[0, 10, 2]                      --- [0, 2, 4, 6, 8]
l = linspace[0, 1, 5]                      --- [0, 0.25, 0.5, 0.75, 1.0]

--- special matrices
i = eye[3]                                 --- 3x3 identity
d = diag[x]                                --- diagonal matrix from vector
d = diag[m]                                --- extract diagonal from matrix

--- constants
pi                                         --- 3.14159...
e                                          --- 2.71828...
inf                                        --- infinity
nan                                        --- not a number
```

Properties: `shape`, `dtype`, `device`, `grad`.

### Layer 1: Array Operations

Element-wise math, reductions, reshaping, sorting, combining, and broadcasting.

```
--- element-wise arithmetic
c = a + b
c = a - b
c = a .* b                                --- element-wise multiply
c = a ./ b                                --- element-wise divide
c = a .^ b                                --- element-wise power

--- element-wise math
c = exp[a]
c = log[a]
c = sqrt[a]
c = abs[a]
c = sign[a]
c = floor[a]
c = ceil[a]
c = round[a]
c = clip[a, 0, 1]                         --- clamp values
c = mod[a, b]                              --- modular arithmetic

--- trigonometry
c = sin[a]
c = cos[a]
c = tan[a]
c = asin[a]
c = acos[a]
c = atan[a]
c = atan2[y, x]
c = sinh[a]
c = cosh[a]
c = tanh[a]

--- reductions
s = sum[x]                                 --- total
s = sum[x, axis = 0]                       --- along axis
m = mean[x]
mx = max[x, axis = 1]
mn = min[x]
p = prod[x]                               --- product

--- selection
i = argmax[x]                              --- index of max
i = argmin[x]                              --- index of min
c = where[cond, a, b]                      --- conditional: a if true, b if false

--- sorting
s = sort[x]                                --- sorted array
i = argsort[x]                             --- indices that would sort
t = topk[x, 5]                             --- top 5 values
u = unique[x]                              --- unique values

--- cumulative
c = cumsum[x]                              --- running sum
c = cumprod[x]                             --- running product

--- shape
r = reshape[x, [2, 3]]
t = trans[x]
f = flatten[x]
s = slice[x, 0, 3]

--- combining
c = concat[a, b, axis = 0]                --- join along axis
s = stack[arrays, axis = 0]               --- new axis

--- advanced indexing
y = x[indices]                             --- gather by index array

--- broadcasting
--- shapes [3, 1] + [1, 4] → [3, 4] automatically
```

### Layer 2: Linear Algebra

What makes ML possible.

```
--- core
c = a * b                                 --- matmul
t = trans[m]
i = inv[m]
d = det[m]

--- decompositions
u, s, v = svd[m]
q, r = qr[m]
l = cholesky[m]
vals, vecs = eigen[m]

--- solving
x = solve[a, b]                            --- solve Ax = b

--- norms
n = norm[x]                                --- L2 norm
n = norm[x, p = 1]                         --- L1 norm
```

### Layer 3: Random & Distributions

Reproducible random number generation and sampling from distributions.

```
--- seeding
seed[42]                                   --- set global seed

--- sampling
r = uniform[0.0, 1.0, 3, 4]                --- uniform [0, 1)
n = normal[0, 1, [3, 4]]                  --- normal distribution
u = uniform[5, 10, [3, 4]]                --- uniform [5, 10)
p = poisson[3.0, [100]]                   --- poisson distribution
b = bernoulli[0.5, [100]]                 --- coin flips

--- array operations
s = shuffle[x]                             --- random permutation
c = choice[x, 10]                          --- random sample of 10
p = permutation[n]                         --- random permutation of 0..n
```

### Layer 4: Signal Processing

FFT and related transforms. Native implementations — can't build efficiently from array ops.

```
--- FFT
f = fft[x]                                --- 1D FFT (returns complex)
x = ifft[f]                               --- inverse FFT
f = fft2[m]                               --- 2D FFT

--- convolution (mathematical, not NN)
c = conv[a, b]                             --- 1D convolution
c = conv2d[a, b]                           --- 2D convolution

--- windowing
w = hamming[n]                             --- Hamming window
w = hanning[n]                             --- Hanning window
```

### Layer 5: Special Functions & Complex Numbers

Mathematical functions that can't be built from basic ops, and complex number support (required by FFT).

```
--- special functions
g = gamma[x]                               --- gamma function
e = erf[x]                                --- error function
b = bessel[x, n]                           --- Bessel function
f = factorial[n]                           --- n!
c = choose[n, k]                           --- binomial coefficient

--- complex numbers
c = complex[3, 4]                          --- 3 + 4i
r = real[c]                                --- real part
i = imag[c]                                --- imaginary part
m = magnitude[c]                           --- |c|
p = phase[c]                               --- angle
c = conjugate[c]                           --- complex conjugate
```

### Layer 6: Sparse Arrays

Different data structure for matrices that are mostly zeros. Native implementation — can't fake with dense arrays.

```
--- creation
s = sparse[dense_array]                    --- dense to sparse
s = sparse[rows, cols, vals, shape]        --- COO format
d = dense[s]                               --- sparse to dense

--- operations (same syntax as dense)
c = s * v                                  --- sparse matmul
c = s + s2                                 --- sparse add
x = solve[s, b]                            --- sparse solve
```

Sparse arrays use the same operators as dense arrays. The runtime dispatches to sparse-optimized algorithms automatically.

### Layer 7: Autograd

What makes deep learning possible. Computational graph that records operations and computes gradients automatically.

```
fn loss[w, x, y]
  return mean[(x * w - y) .* (x * w - y)]

dloss = grad[loss]                         --- returns gradient function
g = dloss[w, x, y]                         --- computes gradients
w = w - 0.01 * g
```

No manual derivative computation. `grad[f]` transforms a function into its gradient function. Composable: `grad[grad[f]]` for second derivatives.

### Layer 8: GPU Computing

Array operations on GPU. Crow uses `gpu` as a language-level directive — the programmer specifies *intent*, the compiler decides what runs on GPU. No per-array device placement, no manual transfers.

```
gpu[]                    --- all GPUs on this machine
gpu[2]                   --- 2 GPUs on this machine

w = randn[784, 128] .* 0.01

--- compiler auto-dispatches numeric ops to GPU
h = relu[data * w + b]
```

The compiler decides what is GPU-eligible (numeric ops on arrays) and what falls back to CPU (string ops, I/O). GPU compute uses `wgpu` (WebGPU standard) with WGSL compute shaders. The runtime auto-detects the backend: Apple Silicon → Metal, Linux → Vulkan, Windows → DX12. All from one set of shaders — no platform-specific code. Backend can be pinned in the `crowfile` with `gpu_backend = 'metal'`.

### Layer 9: Distributed Computing

*Superseded (2026-10-11, 2026-10-16): the process count is the launch's — `crow launch <n> file.co` starts the processes and each reads its rank with `env['CROW_RANK', '0']`; `nodes` is no builtin (spec, Design Note "the process count is the launch's"). The directives below were not built.*

Training across multiple machines. Crow uses `nodes` as a directive — same philosophy as `gpu`. Specify intent, the compiler handles data sharding, gradient synchronization, and communication.

```
gpu[]                    --- all GPUs, this machine
gpu[2]                   --- 2 GPUs, this machine
nodes[4]                 --- 4 machines
nodes[4, gpu = 2]        --- 4 machines, 2 GPUs each
```

The user writes the same training loop. The compiler inserts allreduce, data sharding, and gradient sync automatically.

```
--- single machine
gpu[]

fn loss[w, x, y]
  return mean[(x * w - y) .* (x * w - y)]

dloss = grad[loss]
for i in range[1000]
  g = dloss[w, x, y]
  w = w - 0.01 * g

--- distributed across 4 machines, same code
nodes[4]
gpu[]

for i in range[1000]
  g = dloss[w, x, y]
  w = w - 0.01 * g
```

No explicit `allreduce`, `broadcast`, `scatter`, or `gather` calls. The compiler knows the training pattern and inserts the right communication. Data scientists write math, not distributed systems code.

## How Other Languages Do It

### Python (NumPy + PyTorch/JAX)

Python itself does nothing fast. NumPy is a C extension providing ndarray. PyTorch is a C++ engine (libtorch) with Python bindings. All heavy computation is C/C++/CUDA — Python is glue. Autograd in PyTorch is tape-based, implemented in C++. JAX takes a different approach: functional transformations (`jax.grad`, `jax.vmap`, `jax.jit`) that compile to XLA. Python's advantage is ecosystem, not language design.

**Lesson for Crow:** Avoid the "two language problem." Crow transpiles to Rust — the output language is fast enough. No C extensions needed. Pure Rust dependencies (faer, wgpu, rustfft) mean `cargo build` works everywhere with no system library installation.

### Julia

The most relevant comparison. Arrays are first-class. Multiple dispatch means `A * B` dispatches to BLAS automatically based on types. BLAS/LAPACK are in the stdlib. Autograd via Zygote.jl uses *source-to-source transformation* — `gradient(f, x)` transforms the function `f` into its derivative. GPU via CUDA.jl. The key insight: Julia is fast enough that libraries are written *in Julia*, not C.

**Lesson for Crow:** Like Julia, Crow's transpiled output is fast enough. Write everything in Crow + Rust, no FFI escape hatches. Source-level autograd is possible but complex — functional transformation is simpler.

### R

Vectors and matrices are first-class (everything is a vector). BLAS/LAPACK in base R. But R is slow for iteration, so heavy ML uses C/C++ extensions (Rcpp). Deep learning is mostly done by calling Python via reticulate. R wasn't designed for DL.

**Lesson for Crow:** First-class arrays are right, but the language must be fast enough to avoid needing escape hatches. R failed here.

### MATLAB

Everything is a matrix. The entire language is built around linear algebra. BLAS/LAPACK *are* the runtime. GPU via Parallel Computing Toolbox. Great design for numerical computing but proprietary and not built for modern distributed training.

**Lesson for Crow:** MATLAB proves that making arrays/matrices the core type works beautifully for scientific computing. The API design (concise, math-oriented) is worth studying.

### JAX

Functional approach: pure functions, no side effects. `grad(f)` returns the gradient *function*. `vmap(f)` auto-vectorizes. `jit(f)` compiles to XLA. Composable transformations — you can `grad(grad(f))` for second derivatives.

**Lesson for Crow:** JAX's `grad[f]` as function transformation fits Crow's minimalist philosophy perfectly. No mutable tape, no hidden state, no OOP. Just functions in, functions out. This is the autograd design Crow should adopt.

### Mojo

Python syntax, compiled via MLIR. Aims to be "Python but fast." Tensors via MAX engine. Still early, but the MLIR approach to GPU compilation is interesting.

### Swift for TensorFlow (dead)

Built differentiable programming into the compiler. `@differentiable` attribute on functions, compiler auto-generates derivatives. Technically brilliant — the most principled approach — but commercially failed when Google pulled support.

**Lesson for Crow:** Compiler-level autograd is the dream but risky. Function-level transformation (JAX-style) gets 90% of the benefit with 10% of the complexity.

### Comparison Table

| Language | Array/Tensor | Autograd | GPU | Perf strategy |
|----------|--------|----------|-----|---------------|
| Python | Library (C) | Runtime tape (C++) | Library (CUDA) | Everything in C/C++ |
| Julia | First-class | Source transform | Library | Language is fast enough |
| R | First-class | External | External | C extensions |
| MATLAB | First-class | Toolbox | Toolbox | Built-in BLAS |
| JAX | Library (C) | Function transform | XLA | Compiler |
| Mojo | Library | TBD | MLIR | Compiler |
| Swift4TF | Library | Compiler | TBD | Compiler (dead) |

### Crow's Position

Crow is in the **Julia camp** — transpiles to a fast language (Rust), so no "two language problem." Combined with **JAX-style functional autograd** — `grad[f]` returns a gradient function, composable and stateless. The runtime is pure Rust with zero C/C++ dependencies — `faer` for CPU math (often faster than OpenBLAS), `wgpu` for cross-platform GPU (Metal/Vulkan/DX12), and our own tensor type. This gives Crow:

- Julia's advantage: everything written in one language, no C extensions
- JAX's advantage: clean functional autograd, no mutable tape
- MATLAB's advantage: arrays as first-class citizens
- None of Python's disadvantage: no glue language, no performance cliff
- Rust's advantage: zero C/C++ deps, `cargo build` works everywhere, memory safety

## Library Structure

One `math` library with sub-modules:

```
use math                --- core: arrays, ops, linalg
use math.stats          --- statistics, distributions, hypothesis testing
use math.signal         --- FFT, convolution, filtering, windowing
use math.optimize       --- minimize, root finding, curve fitting
use math.sparse         --- sparse arrays
use math.cluster        --- k-means, PCA, t-SNE
use math.nn             --- neural network layers, activations, loss functions
use math.series         --- time series, rolling, autocorrelation
```

`use math` gives you arrays, element-wise ops, linalg, autograd, random, and GPU/distributed directives — everything needed for most work. Sub-modules are imported only when needed.

## Architecture

### Design Decision: Pure Rust (faer + wgpu + own tensor)

Crow owns its runtime. No C/C++ dependencies, no framework wrappers. `cargo build` just works everywhere.

**Why not burn/candle?** Crow is a language, not an application. Building on top of another framework means Crow becomes a syntax layer over someone else's design decisions. If that framework changes its API, deprecates a feature, or makes a design choice that conflicts with Crow's semantics — we're stuck. Languages that own their runtime (Julia) outlast languages that wrap someone else's (Swift for TensorFlow, dead).

**Why faer?** Pure Rust linear algebra that's often faster than OpenBLAS. No BLAS/LAPACK FFI, no C dependencies. Best-in-class CPU math performance. It's our BLAS — it does the hard math (decompositions, matmul, solves), we own everything above it.

**Why wgpu?** Cross-platform GPU compute via the WebGPU standard (W3C). Works on Mac (Metal), Linux (Vulkan), Windows (DX12) — no CUDA installation required. One set of WGSL compute shaders runs everywhere. Stable API backed by a spec, not a company's roadmap.

**Why our own tensor?** The tensor type is the core of a numerical computing language. Shape, stride, broadcasting, memory layout, autograd taping — these must match Crow's semantics exactly. We can't retrofit another library's tensor to work the way Crow needs.

### Runtime Stack

```
┌─────────────────────────────────────┐
│  Crow language                      │  ← syntax, transpiler, interpreter
├─────────────────────────────────────┤
│  crow_runtime                       │  ← our crate
│    ├── CrowTensor                   │  ← our tensor type (shape, stride, data)
│    ├── broadcasting                 │  ← shape expansion logic
│    ├── autograd                     │  ← tape-based, JAX-style grad[f]
│    ├── element-wise ops             │  ← SIMD-vectorized loops
│    ├── reductions                   │  ← sum, mean, max along axes
│    ├── sorting                      │  ← sort, argsort, topk, unique
│    ├── creation                     │  ← zeros, ones, eye, linspace, arange
│    ├── reshaping                    │  ← reshape, trans, flatten, concat
│    ├── random                       │  ← rand crate + rand_distr
│    ├── complex                      │  ← num-complex crate
│    └── dispatch                     │  ← CPU vs GPU routing
├─────────────────────────────────────┤
│  CPU backend                        │
│    ├── faer                         │  ← matmul, SVD, QR, cholesky, eigen, solve
│    ├── rustfft                      │  ← FFT, IFFT, FFT2
│    └── sprs                         │  ← sparse arrays (COO/CSR)
├─────────────────────────────────────┤
│  GPU backend                        │
│    └── wgpu                         │  ← WGSL compute shaders
│         ├── Metal (Mac)             │
│         ├── Vulkan (Linux)          │
│         └── DX12 (Windows)          │
├─────────────────────────────────────┤
│  Distributed backend                │
│    └── custom (TCP/RDMA)            │  ← allreduce, scatter, gather
└─────────────────────────────────────┘
```

### CrowTensor Type

The core data structure. Immutable, reference-counted storage with shape/stride metadata for zero-copy views.

```rust
struct CrowTensor {
    data: Arc<TensorStorage>,  // shared immutable data (reference counted)
    shape: Vec<usize>,         // [3, 4] = 3 rows, 4 cols
    strides: Vec<usize>,       // element offsets per dimension
    offset: usize,             // where this view starts in the data
}

enum TensorStorage {
    Cpu(Vec<f64>),             // f64 on CPU — always
    Gpu(GpuBuffer),            // f32 on GPU — wgpu buffer
}
```

#### Design Decisions

**Data types: f64 on CPU, f32 on GPU.** The user never sees dtypes. `[1, 2, 3]` is always f64 on CPU. When `gpu[]` is active, tensors are created as f32 on GPU automatically. Conversion at the device boundary is automatic and invisible. No `dtype`, no `.float()`, no `.half()`. One way.

- f64 handles integers exactly up to 2^53
- f64 on GPU is fine on data center hardware (A100/H100: 1:2 ratio vs f32)
- f32 on GPU is the default because consumer GPUs cripple f64 (1:32-64 ratio)

**Views: Immutable + Arc.** `trans[m]` and `slice[m, 0, 3]` create new `CrowTensor` values that share the same `Arc<TensorStorage>` with different shape/strides/offset. Zero copy. Operations like `m + 1` always produce a new tensor with new storage. No mutation of existing data, ever.

```
m = [[1, 2], [3, 4]]
t = trans[m]

m.data ──→ Arc (ref count: 2) ──→ [1.0, 2.0, 3.0, 4.0]
t.data ──┘

m: shape [2,2], strides [2,1], offset 0    ← reads row-major
t: shape [2,2], strides [1,2], offset 0    ← reads column-major, same data
```

When all references are dropped, ref count hits 0 and memory is freed. Deterministic, no GC.

**Memory pool.** Operations reuse freed buffers instead of hitting the system allocator:

```rust
struct TensorPool {
    free_buffers: Vec<Vec<f64>>,   // recycled allocations by size bucket
}
```

In training loops with millions of iterations, no buffer is ever truly freed and reallocated. Same pattern as PyTorch's CUDA caching allocator.

**Non-contiguous data.** After `trans[m]`, strides are swapped but data order is unchanged. Before BLAS/faer calls (matmul, SVD, etc.), `make_contiguous()` copies data into the expected layout. Transpose is O(1) to create; the first BLAS operation on it may trigger an O(n) copy. This is how NumPy and PyTorch work.

#### Memory Layout

Row-major (C order), matching Rust's default. A 3×4 matrix stores 12 f64 values contiguously. `strides` enables views (transpose, slice) without copying data.

#### Two-Tier Value System

```
Value::Array(Vec<CrowValue>)     --- general arrays: ['hello', true]
Value::NumArray(CrowTensor)      --- numeric arrays: contiguous f64, shaped
```

Type is decided at creation and never changes. Arrays are homogeneous per the spec. `[1, 2, 3]` → `NumArray`. `['a', 'b']` → `Array`. No mixed types, no type changes mid-program.

**In the transpiler:** Type inference detects numeric arrays and generates `CrowTensor` code directly. No runtime dispatch needed — the type is known at compile time.

#### Codebase Structure

Separate `crow_runtime` crate. Transpiled code depends on the runtime only, not the compiler.

```
crow/
  Cargo.toml              ← workspace root
  crow_lang/              ← compiler, interpreter, checker, transpiler
    Cargo.toml
    src/
  crow_runtime/           ← tensor type, ops, GPU, memory pool
    Cargo.toml
    src/
      lib.rs
      tensor.rs
      ops.rs
      gpu.rs
      pool.rs
```

Generated binaries (`crow build`) depend on `crow_runtime` only — not the parser, type inference, or checker.

### Performance

With faer + SIMD + contiguous memory:

| Operation | 1M elements | 1B elements | vs NumPy |
|-----------|------------|------------|---------|
| Element-wise add | ~0.5ms | ~500ms | Same (both SIMD) |
| Matmul 1000×1000 | ~50ms | N/A | Same (both BLAS-level) |
| Sum reduction | ~0.3ms | ~300ms | Same |
| Sort | ~20ms | ~20s | Same (both O(n log n)) |
| Memory per f64 | 8 bytes | 8 bytes | Same |

faer is often *faster* than OpenBLAS (which NumPy uses) because it's designed for modern CPU architectures with better cache utilization.

### Pure Rust Dependencies

| Crate | Purpose | Lines of glue |
|-------|---------|--------------|
| `faer` | Matmul, decompositions, solve, norm | ~300 |
| `wgpu` | GPU compute shaders (Metal/Vulkan/DX12) | ~1500 |
| `rustfft` | FFT, IFFT, FFT2 | ~200 |
| `num-complex` | Complex number type | ~100 |
| `rand` + `rand_distr` | Random distributions | ~200 |
| `sprs` | Sparse arrays (COO/CSR) | ~200 |

Total pure Rust. No C/C++ compilation. No system library installation. `cargo build` works on every platform.

### What We Build (~4000 lines)

| Component | Effort | Description |
|-----------|--------|-------------|
| CrowTensor struct | ~500 lines | Shape, stride, data, views, contiguous storage |
| Broadcasting | ~200 lines | Shape expansion, iterator logic |
| Element-wise ops | ~300 lines | +, -, *, exp, sin, etc. with SIMD |
| Reductions | ~200 lines | sum, mean, max, min along axes |
| Sorting & selection | ~200 lines | sort, argsort, topk, unique, where |
| Creation functions | ~200 lines | zeros, ones, eye, linspace, arange, full, diag |
| Reshaping & combining | ~200 lines | reshape, trans, flatten, concat, stack |
| Autograd | ~800 lines | Tape-based, JAX-style grad[f] |
| GPU kernels (WGSL) | ~1500 lines | Compute shaders for core ops |

### What We Use From Crates

| Component | Crate | Why not build ourselves |
|-----------|-------|----------------------|
| Matmul, LU, SVD, QR, Cholesky, Eigen, Solve | `faer` | Decades of numerical analysis research. Getting these right and fast is a multi-year effort. |
| FFT | `rustfft` | O(n log n) FFT algorithms are well-understood but tricky to optimize. |
| Sparse storage formats | `sprs` | COO/CSR conversion, sparse matmul — specialized algorithms. |
| Complex numbers | `num-complex` | Standard type, no reason to reinvent. |
| Random distributions | `rand` + `rand_distr` | Statistically correct sampling is subtle. |
| GPU abstraction | `wgpu` | WebGPU API over Metal/Vulkan/DX12. We write shaders, not driver code. |

### What Can Be Built in Crow

These are algorithms on top of primitives — pure Crow code.

| Component | Why buildable |
|-----------|--------------|
| NN layers (linear, conv, attention) | Functions on arrays |
| Optimizers (SGD, Adam, AdamW) | ~10-20 lines each, just array math |
| Loss functions | Functions on arrays |
| Training loops | Pure control flow |
| Data loading + batching | File I/O + slicing |
| Learning rate schedulers | Simple math |
| Model serialization | Array → file I/O |
| Statistics (median, std, corr) | Reductions + sorting |
| Interpolation (linear, spline) | Array ops + loops |
| Numerical methods (ODE, root finding) | Loops + arithmetic |
| Clustering (k-means, PCA) | Array ops + linalg |
| Distance metrics | Array ops + norm |
| Time series (rolling, autocorr) | Slicing + reductions |
| Set operations (union, intersect) | Sort + unique |

### Autograd Design

Three approaches exist in the wild:

**Tape-based (PyTorch style).** Every op records itself on a global tape. `loss.backward()` replays in reverse. Requires the runtime to intercept every operation. Mutable, imperative, hidden state.

**Source-to-source (Julia/Zygote style).** The compiler transforms the function into its derivative at compile time. Powerful but extremely complex to implement.

**Functional transformation (JAX style).** `grad[f]` takes a function and returns a new function that computes gradients. No tape, no hidden state, no mutation. Composable: `grad[grad[f]]` for second derivatives.

**Crow's choice: JAX-style functional transformation.** This fits Crow's philosophy perfectly — functions in, functions out. No mutable tape, no hidden state. The transpiler can optimize gradient functions at compile time since the computation graph is known statically.

```
fn loss[w, x, y]
  return mean[(x * w - y) .* (x * w - y)]

dloss = grad[loss]                         --- returns gradient function
g = dloss[w, x, y]                         --- computes gradients
w = w - 0.01 * g
```

Zero overhead for non-gradient code — `grad` is only applied where explicitly requested.

## Implementation Order

1. **N-dimensional arrays** — shape, stride, dtype, contiguous memory, creation (`zeros`, `ones`, `rand`, `full`, `arange`, `linspace`, `eye`, `diag`)
2. **Element-wise ops** — `+`, `-`, `.*`, `./`, `.^`, `exp`, `log`, `sqrt`, `abs`, `sign`, `floor`, `ceil`, `round`, `clip`, `mod`
3. **Trigonometry** — `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sinh`, `cosh`, `tanh`
4. **Reductions** — `sum`, `mean`, `max`, `min`, `prod`, `argmax`, `argmin` (with axis)
5. **Sorting & selection** — `sort`, `argsort`, `topk`, `unique`, `where`
6. **Cumulative ops** — `cumsum`, `cumprod`
7. **Reshaping & combining** — `reshape`, `trans`, `flatten`, `slice`, `concat`, `stack`
8. **Broadcasting** — automatic shape expansion
9. **Advanced indexing** — gather by index array
10. **Matmul (`*`) + BLAS** — matrix multiplication, backed by BLAS
11. **Linear algebra** — `inv`, `det`, `svd`, `qr`, `cholesky`, `eigen`, `solve`, `norm`
12. **Random & distributions** — `seed`, `normal`, `uniform`, `poisson`, `bernoulli`, `shuffle`, `choice`
13. **Complex numbers** — `complex`, `real`, `imag`, `magnitude`, `phase`, `conjugate`
14. **FFT** — `fft`, `ifft`, `fft2`, `conv` (mathematical)
15. **Special functions** — `gamma`, `erf`, `bessel`, `factorial`, `choose`
16. **Sparse arrays** — `sparse`, `dense`, sparse matmul/solve
17. **Autograd** — `grad[f]` functional transformation, gradient computation
18. **GPU** — `gpu[]`/`gpu[n]` directive, compiler auto-dispatch, backend detection
19. **Distributed** — `nodes[n]` directive, compiler-managed data sharding and gradient sync

## Advanced Distributed Patterns

*Superseded (2026-10-11, 2026-10-16): the process count is the launch's — `crow launch <n> file.co` starts the processes and each reads its rank with `env['CROW_RANK', '0']`; `nodes` is no builtin (spec, Design Note "the process count is the launch's"). The directives below were not built.*

`nodes[n]` handles data parallelism — same model, different data on each device. This covers most training. For larger models, Crow provides strategy directives:

```
--- data parallelism (default)
nodes[4]

--- ZeRO-style sharding
nodes[4, shard = 'full']

--- tensor parallelism
nodes[4, tensor = 8]

--- pipeline parallelism
nodes[4, split = 'pipeline', stages = 4]

--- mix (what Llama/GPT actually use)
nodes[64, tensor = 8, pipeline = 8]
```

Same philosophy as `gpu[]` — specify intent, compiler decides how. The training code never changes.

### ZeRO / FSDP Sharding

Shard optimizer states, gradients, and parameters across devices to fit larger models in memory. Instead of every GPU holding a full copy of the model, each GPU holds a slice. Gradients are reduced and parameters are gathered on demand.

```
nodes[4, shard = 'full']

--- same training code
dloss = grad[loss]
for batch in batches
  g = dloss[w, batch.x, batch.y]
  w = w - 0.001 * g
```

The compiler inserts gather/scatter operations around forward and backward passes. The user sees no difference.

**Priority: next after `nodes[n]`.** Biggest practical impact — lets you train models 4-8x larger with the same hardware.

### Tensor Parallelism

Split one layer's weights across devices. A 16K×16K matrix becomes four 16K×4K shards, each GPU computes its piece of the matmul, results are combined.

```
nodes[4, tensor = 4]
```

The compiler analyzes matrix operations and decides how to partition weight matrices across devices. Column-parallel for the first linear layer, row-parallel for the second — matching the Megatron-LM strategy.

**Priority: after ZeRO.** Needed for models too large for one GPU's memory even with sharding.

### Pipeline Parallelism (Deferred)

Split a model into stages across devices. GPU 0 runs layers 1-10, GPU 1 runs layers 11-20. Micro-batches keep all GPUs busy:

```
Time →
GPU 0: [micro1] [micro2] [micro3]  [idle]   [back3] [back2] [back1]
GPU 1:  [idle]  [micro1] [micro2] [micro3]  [back2] [back1]  [idle]
GPU 2:  [idle]   [idle]  [micro1] [micro2]  [back1]  [idle]   [idle]
```

Multiple scheduling strategies exist (GPipe, PipeDream, 1F1B, interleaved) each with different memory/compute tradeoffs. The compiler must understand model structure, manage activation memory, and implement a scheduler.

**Deferred because:** High implementation complexity. Only matters for models too large to fit on one GPU even with tensor parallelism — 100B+ parameters. When implemented:

```
nodes[4, split = 'pipeline', stages = 4]
```

### Mixture of Experts (Deferred)

N "expert" sub-networks with a router that decides which expert processes each token. Experts live on different devices, so tokens are routed across the network via all-to-all communication.

**Deferred because:** Used by a handful of frontier models (GPT-4, Mixtral, Switch Transformer). The routing, load balancing, and all-to-all communication is specialized. 99% of users training dense models don't need it. When implemented:

```
nodes[8, split = 'expert', experts = 8]
```

### Custom Kernels (Escape Hatch)

Sometimes compiler-generated GPU code isn't fast enough. Flash Attention is the classic example — it fuses softmax + matmul into one kernel that never materializes the full N×N attention matrix, giving 2-4x speedup.

This can't be expressed in Crow (or Python, or Julia). It requires GPU-specific code managing memory hierarchy and tiling. The escape hatch is Rust:

```
--- crowfile
native = ['kernels/flash_attention.rs']
```

```
--- main.co
out = flash_attention[q, k, v]     --- calls the Rust kernel
```

Custom kernels are a build concern, not a language concern. Long-term, the compiler should recognize common patterns (e.g., `softmax[q * trans[k] ./ sqrt[d]] * v`) and auto-fuse them.

### Priority Summary

| Pattern | When needed | Complexity | Status |
|---|---|---|---|
| Data parallelism | Always | Low | `nodes[n]` ✅ |
| ZeRO/FSDP sharding | Large models | Medium | Next |
| Tensor parallelism | Very large models | High | After ZeRO |
| Pipeline parallelism | Huge models (100B+) | Very high | Deferred |
| Mixture of Experts | Frontier models | Very high | Deferred |
| Custom kernels | Perf-critical ops | Escape hatch | Rust FFI |

## Examples

### Linear Regression

```
x = [1, 2, 3, 4, 5]
y = [2, 4, 6, 8, 10]

w = 0.0
b = 0.0

fn loss[w, b, x, y]
  pred = x .* w + b
  return mean[(pred - y) .* (pred - y)]

dloss = grad[loss]

for i in range[1000]
  g_w, g_b = dloss[w, b, x, y]
  w = w - 0.01 * g_w
  b = b - 0.01 * g_b

log['w = `w`, b = `b`']
```

### Neural Network (2-Layer, From Scratch)

```
w = [
  w1 = randn[784, 128] .* 0.01,
  b1 = zeros[128],
  w2 = randn[128, 10] .* 0.01,
  b2 = zeros[10]
]

fn relu[x]
  return max[x, 0]

fn model[w, x]
  h = relu[x * w.w1 + w.b1]
  return h * w.w2 + w.b2

fn cross_entropy[logits, labels]
  probs = exp[logits] ./ sum[exp[logits], axis = 1]
  return -mean[sum[labels .* log[probs], axis = 1]]

fn loss[w, x, y]
  return cross_entropy[model[w, x], y]

dloss = grad[loss]
```

### Training Loop with Batching

```
data = read['mnist.csv']
batches = chunk[data, 32]

for epoch in range[10]
  total = 0.0
  for batch in batches
    g = dloss[w, batch.x, batch.y]
    w = w - 0.001 * g                     --- map arithmetic
    total = total + loss[w, batch.x, batch.y]
  log['epoch `epoch` loss = `total / len[batches]`']
```

### GPU Training

```
gpu[]

w = [
  w1 = randn[784, 128] .* 0.01,
  b1 = zeros[128]
]

--- compiler auto-dispatches numeric ops to GPU
h = relu[data * w.w1 + w.b1]
```

### Distributed Training

```
nodes[4]
gpu[]

--- same training code, compiler handles distribution
dloss = grad[loss]
for i in range[1000]
  g = dloss[w, x, y]
  w = w - 0.001 * g
```

### Transformer Attention (Core of LLMs)

```
fn attention[q, k, v]
  d = shape[k][-1]
  scores = q * trans[k] ./ sqrt[d]
  weights = softmax[scores, axis = -1]
  return weights * v

fn multi_head[x, wq, wk, wv, wo, heads]
  q = reshape[x * wq, [-1, heads, 64]]
  k = reshape[x * wk, [-1, heads, 64]]
  v = reshape[x * wv, [-1, heads, 64]]
  out = attention[q, k, v]
  return reshape[out, [-1, heads * 64]] * wo
```

No classes, no decorators, no `self`, no `nn.Module`. Just functions on arrays. `grad[f]` turns any function into its gradient. GPU is a directive, not a code rewrite. Distributed is a directive, not a framework.

## Comparison with Other Languages

### Simplicity

How easy is it to write ML code?

| Language | Rating | Why |
|---|---|---|
| **Crow** | ★★★★★ | `grad[f]`, map arithmetic, directives. Minimal ceremony. |
| JAX | ★★★★ | Functional and clean, but Python boilerplate (`@jax.jit`, `jax.tree_map`) |
| MATLAB | ★★★★ | Great for linear algebra, weak for modern DL |
| Julia | ★★★½ | Clean but multiple dispatch can be confusing, Zygote has sharp edges |
| PyTorch | ★★★ | `loss.backward()`, `optimizer.step()`, `optimizer.zero_grad()`, `.to(device)`, `nn.Module`, `self.` everywhere |
| R | ★★ | Not designed for DL, everything is a workaround |

Crow 2-layer neural network — 10 lines:

```
w = [w1 = randn[784, 128] .* 0.01, b1 = zeros[128],
     w2 = randn[128, 10] .* 0.01, b2 = zeros[10]]

fn loss[w, x, y]
  h = relu[x * w.w1 + w.b1]
  return cross_entropy[h * w.w2 + w.b2, y]

dloss = grad[loss]
for batch in batches
  g = dloss[w, batch.x, batch.y]
  w = w - 0.001 * g
```

PyTorch equivalent — 20 lines:

```
class Model(nn.Module):
    def __init__(self):
        super().__init__()
        self.w1 = nn.Parameter(torch.randn(784, 128) * 0.01)
        self.b1 = nn.Parameter(torch.zeros(128))
        self.w2 = nn.Parameter(torch.randn(128, 10) * 0.01)
        self.b2 = nn.Parameter(torch.zeros(10))

    def forward(self, x):
        h = torch.relu(x @ self.w1 + self.b1)
        return h @ self.w2 + self.b2

model = Model().cuda()
optimizer = torch.optim.SGD(model.parameters(), lr=0.001)
criterion = nn.CrossEntropyLoss()

for batch_x, batch_y in dataloader:
    batch_x, batch_y = batch_x.cuda(), batch_y.cuda()
    optimizer.zero_grad()
    loss = criterion(model(batch_x), batch_y)
    loss.backward()
    optimizer.step()
```

### GPU / Distributed

How easy is it to scale from laptop to cluster?

| Language | Rating | Why |
|---|---|---|
| **Crow** | ★★★★★ | `gpu[]`, `nodes[4]`. One line. Code doesn't change. |
| JAX | ★★★★ | `pmap` is clean but axis names and `PartitionSpec` get complex |
| DeepSpeed | ★★★½ | Config-driven but still wraps PyTorch's complexity |
| PyTorch | ★★½ | `DDP`, `FSDP`, `init_process_group`, `DistributedSampler`... |
| Julia | ★★ | MPI.jl is manual, CUDA.jl works but no unified story |
| MATLAB | ★½ | Parallel Computing Toolbox, proprietary, limited |
| R | ★ | Barely exists |

Scaling the same training code:

| Step | Crow | PyTorch |
|---|---|---|
| Single GPU | Add `gpu[]` | `.cuda()` on model + every tensor |
| Multi-GPU | `gpu[4]` | `DDP(model)` + `DistributedSampler` + `init_process_group` + launch script |
| Multi-node | Add `nodes[4]` | All of the above + `torchrun` + env vars + NCCL config |
| ZeRO sharding | `nodes[4, shard = 'full']` | Switch to `FSDP` wrapper or install DeepSpeed + config JSON |

### Autograd

| Language | Rating | Why |
|---|---|---|
| **Crow** | ★★★★★ | `grad[f]` — one concept, composable, no hidden state |
| JAX | ★★★★★ | Same approach — `jax.grad(f)`. Crow borrowed this. |
| Swift4TF | ★★★★½ | Compiler-level, beautiful design. Dead project. |
| Julia/Zygote | ★★★★ | Source-to-source is powerful but fragile |
| PyTorch | ★★★ | Works but: mutable tape, `zero_grad()`, `retain_graph`, `no_grad()` |
| MATLAB | ★★ | Deep Learning Toolbox, limited |
| R | ★ | torch-for-R wraps PyTorch |

### Weight Management

| Language | Rating | Why |
|---|---|---|
| **Crow** | ★★★★★ | Map arithmetic: `w = w - lr * g`. One line for all params. |
| JAX | ★★★★ | Pytrees, but needs `jax.tree_map` — more verbose |
| PyTorch | ★★★ | `model.parameters()` iterator, tied to `nn.Module` OOP |
| Julia | ★★½ | Manual, Flux.jl has `Flux.params()` but not as clean |
| MATLAB | ★★ | Manual per-variable |
| R | ★ | Manual |

### Performance Potential

| Language | Rating | Why |
|---|---|---|
| Julia | ★★★★★ | JIT-compiled, LLVM, as fast as C |
| Mojo | ★★★★½ | MLIR, designed for speed, early but promising |
| **Crow** | ★★★★ | Transpiles to Rust, faer for CPU math (faster than OpenBLAS), wgpu for GPU — verified with matmul, reduction, element-wise shaders |
| JAX | ★★★★ | XLA compiler, very fast for compiled workloads |
| PyTorch | ★★★½ | C++ backend is fast, Python overhead on control flow |
| MATLAB | ★★★ | BLAS is fast, everything else is slow |
| R | ★½ | Slow unless calling C |

Crow gets ★★★★ not ★★★★★ because it's young. The design produces fast code (Rust output, faer BLAS, GPU dispatch via wgpu). GPU matmul, parallel reduction, and element-wise ops are verified end-to-end with benchmarks.

### Ecosystem

| Language | Rating | Why |
|---|---|---|
| Python | ★★★★★ | HuggingFace, pretrained models, datasets, tutorials, Stack Overflow |
| JAX | ★★★½ | Growing, but smaller than PyTorch |
| R | ★★★ | Strong for statistics, weak for DL |
| Julia | ★★★ | Growing, Flux.jl, good for science |
| MATLAB | ★★½ | Proprietary, shrinking |
| Mojo | ★ | Very early |
| **Crow** | ☆ | Doesn't exist yet |

Crow's biggest weakness. Python's ecosystem took 15 years to build.

### Overall

| Language | Simplicity | Scaling | Autograd | Weights | Performance | Ecosystem | **Total** |
|---|---|---|---|---|---|---|---|
| **Crow** | 5 | 5 | 5 | 5 | 4 | 0 | **24/30** |
| JAX | 4 | 4 | 5 | 4 | 4 | 3.5 | **24.5/30** |
| PyTorch | 3 | 2.5 | 3 | 3 | 3.5 | 5 | **20/30** |
| Julia | 3.5 | 2 | 4 | 2.5 | 5 | 3 | **20/30** |
| MATLAB | 4 | 1.5 | 2 | 2 | 3 | 2.5 | **15/30** |
| R | 2 | 1 | 1 | 1 | 1.5 | 3 | **9.5/30** |

Crow ties with JAX on design quality. Crow wins on simplicity and scaling. JAX wins on ecosystem and proven performance. PyTorch wins only on ecosystem — its developer experience is mediocre.

Crow's math library design is best-in-class. The language-level directives (`gpu[]`, `nodes[n]`, `grad[f]`, map arithmetic) are cleaner than anything else. But design is cheap — execution is everything. The gap between this document and a working system that can train a real model is enormous.

## Implementation Status: Function Coverage Rating

Honest assessment of what's actually implemented and working, compared to mature languages.

**Function count:** ~170 functions/features across 13 categories (math, linalg, stats, signal, special, complex, sparse, NN, frames, autograd, distributions, I/O, shape ops).

### Category-by-Category

| Category | Crow | NumPy/SciPy | Julia | R | MATLAB |
|----------|------|-------------|-------|---|--------|
| Tensor creation | ★★★★ | ★★★★★ | ★★★★★ | ★★★★ | ★★★★★ |
| Element-wise ops | ★★★★ | ★★★★★ | ★★★★★ | ★★★★★ | ★★★★★ |
| Reductions | ★★★★ | ★★★★★ | ★★★★★ | ★★★★★ | ★★★★★ |
| Linear algebra | ★★★★½ | ★★★★★ | ★★★★★ | ★★★ | ★★★★★ |
| Statistics | ★★★★ | ★★★★★ | ★★★★★ | ★★★★★ | ★★★★ |
| Signal processing | ★★★★ | ★★★★★ | ★★★★ | ★★ | ★★★★★ |
| Special functions | ★★★ | ★★★★★ | ★★★★★ | ★★★★ | ★★★★★ |
| Random/distributions | ★★★½ | ★★★★★ | ★★★★★ | ★★★★★ | ★★★★ |
| Sparse | ★★★ | ★★★★★ | ★★★★★ | ★★★ | ★★★★★ |
| Complex numbers | ★★★ | ★★★★★ | ★★★★★ | ★★★★ | ★★★★★ |
| DataFrames | ★★★★ | ★★★★★ (pandas) | ★★★★ (DataFrames.jl) | ★★★★★ | ★★ |
| Autograd | ★★★★ | ★★★★★ (JAX) | ★★★★★ | ★ | ★ |
| GPU | ★★★ | ★★★★★ (CuPy) | ★★★★ | ★★ | ★★★★ |
| NN layers | ★★★ | ★★★★★ (PyTorch) | ★★★★ (Flux) | ★★ | ★★★ |
| Syntax ergonomics | ★★★★★ | ★★★ | ★★★★ | ★★★ | ★★★★ |

### Strengths

- **Syntax**: `W * x + b` for matmul, `.^` for element-wise power, `sort[x]` as universal builtin — cleaner than all competitors.
- **Operator design**: MATLAB/Julia-style `*` vs `.*` is the right convention for data science.
- **Error handling**: Automatic `.ok`/`.reason` propagation — no silent NaN poisoning (NumPy), no exceptions (Julia).
- **Parallel by default**: `for` loops auto-parallelize via rayon. No threading boilerplate.
- **Linear algebra**: 13 operations (solve, inv, det, svd, qr, cholesky, eigen, norm, dot, pinv, lstsq, triu, tril) — competitive with R, approaching MATLAB's core set.
- **DataFrames**: Full frame module with load, save, select, filter, sort, group, join, stack, head, tail, fill, unique, rename, stats, datetime, tz — 16 operations covering the core data science workflow.
- **Distributions**: 8 distributions (normal, uniform, poisson, bernoulli, gamma, beta, exponential + randn) cover ~90% of practical use cases.

### Weaknesses

- **Statistics depth**: 13 functions vs SciPy's ~200. Core tests covered (chi-square, ANOVA, KS, distribution fitting) but many specialized tests still missing.
- **Signal processing**: Good coverage (FFT, conv, windows, STFT, spectrogram, lowpass/highpass/bandpass) but no adaptive filters, no wavelet transforms.
- **GPU**: Directive works (`gpu[n]`) with verified WGSL shaders for matmul, parallel reduction (sum), and all element-wise ops. Benchmarks available. Memory management with OOM detection. Still ★★★ not ★★★★★ — missing conv2d kernel, multi-GPU coordination, and large-scale production benchmarks.
- **Ecosystem**: No package manager, no community packages, no pretrained models.

### Overall

| Language | Breadth | Depth | Ergonomics | Overall |
|----------|---------|-------|------------|---------|
| Python (NumPy+SciPy+pandas) | 10/10 | 10/10 | 6/10 | 9/10 |
| Julia | 9/10 | 9/10 | 8/10 | 9/10 |
| MATLAB | 9/10 | 10/10 | 7/10 | 8/10 |
| R | 8/10 | 9/10 (stats) | 5/10 | 7/10 |
| **Crow** | 6/10 | 5/10 | 9/10 | 6/10 |

### Remaining Gaps (Priority Order)

1. ~~**`rank`, `cond`**~~ ✅ Implemented via SVD
2. ~~**Distribution fitting**~~ ✅ `fit[data, 'normal']` → `{mu, sigma}` (normal, exponential, uniform, poisson)
3. ~~**More stats tests**~~ ✅ chi-square, ANOVA, KS test
4. ~~**Signal depth**~~ ✅ STFT, spectrogram, lowpass/highpass/bandpass filters
5. **Optimization** — `minimize`, `root`, `curve_fit` (not yet implemented)
6. **Advanced ML** — beyond basic NN layers (not yet implemented)
