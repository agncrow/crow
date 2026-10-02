use std::sync::Arc;

use rand::RngExt;
use rand_distr::{Distribution, StandardNormal};

use crate::gpu::{self, GpuBuffer, GpuContext};
use crate::rng;

/// Chunked sum with 4 accumulators for SIMD auto-vectorization.
#[inline]
fn chunked_sum(data: &[f64]) -> f64 {
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    let mut acc = [0.0f64; 4];
    for chunk in chunks {
        acc[0] += chunk[0];
        acc[1] += chunk[1];
        acc[2] += chunk[2];
        acc[3] += chunk[3];
    }
    let mut total = acc[0] + acc[1] + acc[2] + acc[3];
    for &v in remainder {
        total += v;
    }
    total
}

/// Chunked reduction with 4 accumulators for SIMD auto-vectorization.
#[inline]
fn chunked_reduce(data: &[f64], init: f64, f: impl Fn(f64, f64) -> f64) -> f64 {
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    let mut acc = [init; 4];
    for chunk in chunks {
        acc[0] = f(acc[0], chunk[0]);
        acc[1] = f(acc[1], chunk[1]);
        acc[2] = f(acc[2], chunk[2]);
        acc[3] = f(acc[3], chunk[3]);
    }
    let mut result = f(f(acc[0], acc[1]), f(acc[2], acc[3]));
    for &v in remainder {
        result = f(result, v);
    }
    result
}

/// Storage backend for tensor data.
#[derive(Debug)]
pub enum TensorStorage {
    /// Contiguous f64 data on CPU.
    Cpu(Vec<f64>),
    /// f32 data on GPU via wgpu.
    Gpu(GpuBuffer),
}

/// A multidimensional numeric array with zero-copy views.
///
/// Data is immutable and reference-counted via `Arc`. Transpose and slice
/// create new `CrowTensor` values that share the same underlying storage
/// with different shape/strides/offset. Arithmetic operations always
/// produce a new tensor with new storage.
#[derive(Debug, Clone)]
pub struct CrowTensor {
    data: Arc<TensorStorage>,
    shape: Vec<usize>,
    strides: Vec<usize>,
    offset: usize,
}

/// A tensor crosses a serialized boundary — reactive state, a JSON
/// response — as its shape and its values in row-major order; it is read
/// back from that, or from a list of numbers (a vector) or a list of equal
/// rows (a matrix), as the interpreter writes one.
impl serde::Serialize for CrowTensor {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("CrowTensor", 2)?;
        st.serialize_field("shape", self.shape())?;
        st.serialize_field("data", &self.to_vec())?;
        st.end()
    }
}

impl<'de> serde::Deserialize<'de> for CrowTensor {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Shaped { shape: Vec<usize>, data: Vec<f64> },
            Vector(Vec<f64>),
            Rows(Vec<Vec<f64>>),
        }
        match Wire::deserialize(d)? {
            Wire::Shaped { shape, data } => {
                if data.len() != CrowTensor::numel(&shape) {
                    return Err(serde::de::Error::custom("tensor shape does not match its data"));
                }
                Ok(CrowTensor::from_vec(data, shape))
            }
            Wire::Vector(data) => {
                let n = data.len();
                Ok(CrowTensor::from_vec(data, vec![n]))
            }
            Wire::Rows(rows) => {
                let cols = rows.first().map_or(0, |r| r.len());
                if rows.iter().any(|r| r.len() != cols) {
                    return Err(serde::de::Error::custom("a tensor's rows are not the same length"));
                }
                let n = rows.len();
                Ok(CrowTensor::from_vec(rows.into_iter().flatten().collect(), vec![n, cols]))
            }
        }
    }
}

impl CrowTensor {
    // ── Helpers ──────────────────────────────────────────────────────

    /// Compute row-major (C-order) strides for a given shape.
    pub fn row_major_strides(shape: &[usize]) -> Vec<usize> {
        let mut strides = vec![1; shape.len()];
        for i in (0..shape.len().saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * shape[i + 1];
        }
        strides
    }

    /// Total number of elements described by the shape.
    pub fn numel(shape: &[usize]) -> usize {
        shape.iter().product()
    }

    // ── Accessors ───────────────────────────────────────────────────

    /// Shape of the tensor.
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    /// Strides of the tensor.
    pub fn strides(&self) -> &[usize] {
        &self.strides
    }

    /// Number of dimensions.
    pub fn ndim(&self) -> usize {
        self.shape.len()
    }

    /// Total number of elements.
    pub fn len(&self) -> usize {
        Self::numel(&self.shape)
    }

    /// Whether the tensor has zero elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the data is contiguous in row-major order.
    pub fn is_contiguous(&self) -> bool {
        self.strides == Self::row_major_strides(&self.shape)
    }

    /// The tensor's data on the CPU: its own storage, or a copy downloaded
    /// from the GPU. An operation with no shader reads a GPU tensor this
    /// way (`t[0]`, a broadcast, `diag` of a GPU matrix panicked).
    fn cpu_data(&self) -> std::borrow::Cow<'_, [f64]> {
        match self.data.as_ref() {
            TensorStorage::Cpu(v) => std::borrow::Cow::Borrowed(v),
            TensorStorage::Gpu(buf) => match crate::gpu::GpuContext::get() {
                Some(ctx) => std::borrow::Cow::Owned(buf.to_f64(ctx)),
                None => crate::stops::halt(0, "the GPU holding a tensor went away"),
            },
        }
    }

    /// The tensor's data, offset and strides, as a fused chain reads its
    /// elements in place (crate::fused).
    pub(crate) fn strided(&self) -> (std::borrow::Cow<'_, [f64]>, usize, &[usize]) {
        (self.cpu_data(), self.offset, &self.strides)
    }

    /// Transfer tensor data from GPU to CPU.
    /// If already on CPU, returns self (no-op).
    pub fn to_cpu(&self) -> CrowTensor {
        match self.data.as_ref() {
            TensorStorage::Cpu(_) => self.clone(),
            TensorStorage::Gpu(gpu_buf) => {
                // a tensor on the GPU was put there through its context, which
                // lives as long as the process: without it the machine failed
                let Some(ctx) = crate::gpu::GpuContext::get() else {
                    crate::stops::halt(0, "the GPU holding a tensor went away");
                };
                let data = gpu_buf.to_f64(ctx);
                CrowTensor {
                    data: Arc::new(TensorStorage::Cpu(data)),
                    shape: self.shape.clone(),
                    strides: Self::row_major_strides(&self.shape),
                    offset: 0,
                }
            }
        }
    }

    /// Transfer tensor data from CPU to GPU.
    /// If already on GPU, returns self (no-op); with no GPU on the machine
    /// the tensor stays on the CPU (spec, GPU Support: placed on the GPU
    /// "when a GPU is available").
    pub fn to_gpu(&self) -> CrowTensor {
        match self.data.as_ref() {
            TensorStorage::Gpu(_) => self.clone(),
            TensorStorage::Cpu(_) => {
                let Some(ctx) = crate::gpu::GpuContext::get() else {
                    return self.clone();
                };
                let cpu_data = self.to_vec(); // contiguous row-major
                let gpu_buf = GpuBuffer::from_f64(ctx, &cpu_data);
                CrowTensor {
                    data: Arc::new(TensorStorage::Gpu(gpu_buf)),
                    shape: self.shape.clone(),
                    strides: Self::row_major_strides(&self.shape),
                    offset: 0,
                }
            }
        }
    }

    /// Check if tensor is on GPU.
    pub fn is_gpu(&self) -> bool {
        matches!(self.data.as_ref(), TensorStorage::Gpu(_))
    }

    /// If GPU mode is enabled, move tensor to GPU. Otherwise return self.
    pub fn maybe_to_gpu(self) -> CrowTensor {
        if gpu::is_gpu_enabled() && !self.is_gpu() {
            if GpuContext::get().is_some() {
                return self.to_gpu();
            }
        }
        self
    }

    /// Get the GPU buffer, if tensor is on GPU.
    pub fn gpu_buffer(&self) -> Option<&GpuBuffer> {
        match self.data.as_ref() {
            TensorStorage::Gpu(buf) => Some(buf),
            _ => None,
        }
    }

    /// Create a CrowTensor from a GpuBuffer with a given shape.
    pub fn from_gpu_with_shape(buf: GpuBuffer, shape: Vec<usize>) -> CrowTensor {
        let strides = Self::row_major_strides(&shape);
        CrowTensor {
            data: Arc::new(TensorStorage::Gpu(buf)),
            shape,
            strides,
            offset: 0,
        }
    }

    /// Return a contiguous copy of the data in row-major order.
    /// If already contiguous, returns a cheap clone (Arc).
    pub fn to_contiguous(&self) -> CrowTensor {
        if self.is_contiguous() {
            return self.clone();
        }
        let data = self.to_vec();
        Self::from_vec(data, self.shape.clone())
    }

    /// Collect all elements into a Vec<f64> in row-major order.
    /// If tensor is on GPU, downloads to CPU first.
    pub fn to_vec(&self) -> Vec<f64> {
        // Transparently handle GPU tensors
        if self.is_gpu() {
            return self.to_cpu().to_vec();
        }
        let n = self.len();
        let mut out = Vec::with_capacity(n);
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            out.extend_from_slice(&cpu[self.offset..self.offset + n]);
        } else {
            // General strided iteration
            let mut indices = vec![0usize; self.ndim()];
            for _ in 0..n {
                let physical = self.offset
                    + indices
                        .iter()
                        .zip(self.strides.iter())
                        .map(|(i, s)| i * s)
                        .sum::<usize>();
                out.push(cpu[physical]);
                // Increment indices (row-major order: last dim fastest)
                for d in (0..self.ndim()).rev() {
                    indices[d] += 1;
                    if indices[d] < self.shape[d] {
                        break;
                    }
                    indices[d] = 0;
                }
            }
        }
        out
    }

    /// Get a single element by flat index (row-major order).
    pub fn get_flat(&self, flat_idx: usize) -> f64 {
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            return cpu[self.offset + flat_idx];
        }
        // Convert flat index to multi-dimensional indices
        let mut remaining = flat_idx;
        let mut physical = self.offset;
        for d in 0..self.ndim() {
            let stride_rm = Self::row_major_strides(&self.shape);
            let idx = remaining / stride_rm[d];
            remaining %= stride_rm[d];
            physical += idx * self.strides[d];
        }
        cpu[physical]
    }

    // ── Creation ────────────────────────────────────────────────────

    /// Create a tensor from a flat Vec and shape.
    pub fn from_vec(data: Vec<f64>, shape: Vec<usize>) -> Self {
        let expected = Self::numel(&shape);
        assert_eq!(
            data.len(),
            expected,
            "data length {} does not match shape {:?} (expected {})",
            data.len(),
            shape,
            expected
        );
        let strides = Self::row_major_strides(&shape);
        CrowTensor {
            data: Arc::new(TensorStorage::Cpu(data)),
            shape,
            strides,
            offset: 0,
        }
    }

    /// Create a scalar tensor (0-dimensional).
    pub fn scalar(value: f64) -> Self {
        Self::from_vec(vec![value], vec![])
    }

    /// Create a tensor filled with zeros.
    pub fn zeros(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        Self::from_vec(vec![0.0; n], shape).maybe_to_gpu()
    }

    /// Create a tensor filled with ones.
    pub fn ones(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        Self::from_vec(vec![1.0; n], shape).maybe_to_gpu()
    }

    /// Create a tensor filled with a constant value.
    pub fn full(shape: Vec<usize>, value: f64) -> Self {
        let n = Self::numel(&shape);
        Self::from_vec(vec![value; n], shape).maybe_to_gpu()
    }

    /// Create a 1D tensor with values from start (inclusive) to end (exclusive), stepping by step.
    pub fn arange(start: f64, end: f64, step: f64) -> Self {
        assert!(step != 0.0, "arange: step cannot be zero");
        let mut data = Vec::new();
        let mut v = start;
        if step > 0.0 {
            while v < end {
                data.push(v);
                v += step;
            }
        } else {
            while v > end {
                data.push(v);
                v += step;
            }
        }
        let n = data.len();
        Self::from_vec(data, vec![n]).maybe_to_gpu()
    }

    /// Create a 1D tensor with `n` evenly spaced values from start to end (inclusive).
    pub fn linspace(start: f64, end: f64, n: usize) -> Self {
        if n == 0 {
            return Self::from_vec(vec![], vec![0]);
        }
        if n == 1 {
            return Self::from_vec(vec![start], vec![1]);
        }
        let step = (end - start) / (n - 1) as f64;
        let data: Vec<f64> = (0..n).map(|i| start + step * i as f64).collect();
        Self::from_vec(data, vec![n]).maybe_to_gpu()
    }

    /// Create an identity matrix of size n×n.
    pub fn eye(n: usize) -> Self {
        let mut data = vec![0.0; n * n];
        for i in 0..n {
            data[i * n + i] = 1.0;
        }
        Self::from_vec(data, vec![n, n]).maybe_to_gpu()
    }

    /// Create a diagonal matrix from a 1D tensor, or extract the diagonal of
    /// a 2D tensor; any other rank is the reason.
    pub fn diag(tensor: &CrowTensor) -> Result<Self, String> {
        if tensor.ndim() == 1 {
            // 1D → diagonal matrix
            let n = tensor.len();
            let mut data = vec![0.0; n * n];
            for i in 0..n {
                data[i * n + i] = tensor.get_flat(i);
            }
            Ok(Self::from_vec(data, vec![n, n]))
        } else if tensor.ndim() == 2 {
            // 2D → extract diagonal
            let n = tensor.shape[0].min(tensor.shape[1]);
            let cpu = tensor.cpu_data();
            let data: Vec<f64> = (0..n)
                .map(|i| {
                    let physical = tensor.offset + i * tensor.strides[0] + i * tensor.strides[1];
                    cpu[physical]
                })
                .collect();
            Ok(Self::from_vec(data, vec![n]))
        } else {
            Err(format!("diag: expected a 1D or 2D tensor, got {}D", tensor.ndim()))
        }
    }

    /// Create a tensor with uniform random values in [0, 1).
    pub fn rand(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| r.random::<f64>());
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with standard normal random values (mean=0, std=1).
    pub fn randn(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| {
            let v: f64 = StandardNormal.sample(r);
            v
        });
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with uniform random values in [low, high).
    pub fn rand_range(shape: Vec<usize>, low: f64, high: f64) -> Self {
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| r.random::<f64>() * (high - low) + low);
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with normal random values (custom mean and std).
    pub fn normal(shape: Vec<usize>, mean: f64, std: f64) -> Self {
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| {
            let v: f64 = StandardNormal.sample(r);
            v * std + mean
        });
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with Poisson-distributed random values.
    /// (its parameters checked: the reason, never a panic)
    pub fn poisson(shape: Vec<usize>, lambda: f64) -> Result<Self, String> {
        use rand_distr::Poisson;
        let dist: Poisson<f64> = Poisson::new(lambda).map_err(|_| "poisson: lambda must be > 0".to_string())?;
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| {
            let v: f64 = dist.sample(r);
            v
        });
        Ok(Self::from_vec(data, shape).maybe_to_gpu())
    }

    /// Create a tensor with Bernoulli-distributed random values (0.0 or 1.0).
    /// (its parameters checked: the reason, never a panic)
    pub fn bernoulli(shape: Vec<usize>, p: f64) -> Result<Self, String> {
        use rand_distr::Bernoulli;
        let dist = Bernoulli::new(p).map_err(|_| "bernoulli: p must be in [0, 1]".to_string())?;
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| if dist.sample(r) { 1.0 } else { 0.0 });
        Ok(Self::from_vec(data, shape).maybe_to_gpu())
    }

    /// Create a tensor with Gamma-distributed random values.
    /// (its parameters checked: the reason, never a panic)
    pub fn gamma_dist(shape: Vec<usize>, alpha: f64, beta: f64) -> Result<Self, String> {
        use rand_distr::Gamma;
        let dist: Gamma<f64> = Gamma::new(alpha, beta).map_err(|_| "gamma_dist: alpha and beta must be > 0".to_string())?;
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| dist.sample(r));
        Ok(Self::from_vec(data, shape).maybe_to_gpu())
    }

    /// Create a tensor with Beta-distributed random values.
    /// (its parameters checked: the reason, never a panic)
    pub fn beta_dist(shape: Vec<usize>, alpha: f64, beta: f64) -> Result<Self, String> {
        use rand_distr::Beta;
        let dist: Beta<f64> = Beta::new(alpha, beta).map_err(|_| "beta_dist: alpha and beta must be > 0".to_string())?;
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| dist.sample(r));
        Ok(Self::from_vec(data, shape).maybe_to_gpu())
    }

    /// Create a tensor with Exponential-distributed random values.
    /// (its parameters checked: the reason, never a panic)
    pub fn exponential(shape: Vec<usize>, lambda: f64) -> Result<Self, String> {
        use rand_distr::Exp;
        let dist: Exp<f64> = Exp::new(lambda).map_err(|_| "exponential: lambda must be 0 or more".to_string())?;
        let n = Self::numel(&shape);
        let data = rng::gen_random_vec(n, |r| dist.sample(r));
        Ok(Self::from_vec(data, shape).maybe_to_gpu())
    }

    /// Shuffle tensor elements randomly (returns new tensor).
    pub fn shuffle(&self) -> Self {
        use rand::seq::SliceRandom;
        let mut data = self.to_vec();
        rng::with_rng(|r| data.shuffle(r));
        Self::from_vec(data, self.shape.clone())
    }

    /// Randomly sample n elements from the tensor (without replacement).
    pub fn choice(&self, n: usize) -> Self {
        use rand::seq::index::sample;
        let len = self.len();
        assert!(n <= len, "choice: n ({}) must be <= tensor length ({})", n, len);
        let flat = self.to_vec();
        let mut result = Vec::with_capacity(n);
        rng::with_rng(|r| {
            let indices = sample(r, len, n);
            for i in indices {
                result.push(flat[i]);
            }
        });
        Self::from_vec(result, vec![n])
    }

    /// Random permutation of 0..n as a tensor.
    pub fn permutation(n: usize) -> Self {
        use rand::seq::SliceRandom;
        let mut data: Vec<f64> = (0..n).map(|i| i as f64).collect();
        rng::with_rng(|r| data.shuffle(r));
        Self::from_vec(data, vec![n])
    }

    // ── Reshaping & Views ───────────────────────────────────────────

    /// Transpose: swap the last two dimensions. Zero-copy (shared Arc).
    pub fn transpose(&self) -> CrowTensor {
        assert!(
            self.ndim() >= 2,
            "transpose: need at least 2 dimensions, got {}",
            self.ndim()
        );
        let n = self.ndim();
        let mut new_shape = self.shape.clone();
        let mut new_strides = self.strides.clone();
        new_shape.swap(n - 2, n - 1);
        new_strides.swap(n - 2, n - 1);
        CrowTensor {
            data: Arc::clone(&self.data),
            shape: new_shape,
            strides: new_strides,
            offset: self.offset,
        }
    }

    /// Reshape to a new shape. Zero-copy if contiguous, copies otherwise.
    pub fn reshape(&self, new_shape: Vec<usize>) -> CrowTensor {
        let new_numel = Self::numel(&new_shape);
        assert_eq!(
            self.len(),
            new_numel,
            "reshape: cannot reshape {} elements into shape {:?} ({} elements)",
            self.len(),
            new_shape,
            new_numel
        );
        if self.is_contiguous() {
            // Zero-copy: same data, new shape
            CrowTensor {
                data: Arc::clone(&self.data),
                shape: new_shape.clone(),
                strides: Self::row_major_strides(&new_shape),
                offset: self.offset,
            }
        } else {
            // Must copy to make contiguous first
            let data = self.to_vec();
            Self::from_vec(data, new_shape)
        }
    }

    /// Flatten to 1D.
    pub fn flatten(&self) -> CrowTensor {
        self.reshape(vec![self.len()])
    }

    /// Slice along a given axis. Zero-copy.
    pub fn slice(&self, axis: usize, start: usize, end: usize) -> CrowTensor {
        assert!(axis < self.ndim(), "slice: axis {} out of range", axis);
        assert!(
            end <= self.shape[axis],
            "slice: end {} exceeds axis size {}",
            end,
            self.shape[axis]
        );
        assert!(start <= end, "slice: start {} > end {}", start, end);
        let mut new_shape = self.shape.clone();
        new_shape[axis] = end - start;
        let new_offset = self.offset + start * self.strides[axis];
        CrowTensor {
            data: Arc::clone(&self.data),
            shape: new_shape,
            strides: self.strides.clone(),
            offset: new_offset,
        }
    }
}

// ── Element-wise operations ─────────────────────────────────────────────

impl CrowTensor {
    /// Apply a unary function element-wise, producing a new tensor.
    pub fn map(&self, f: impl Fn(f64) -> f64) -> CrowTensor {
        let data: Vec<f64> = self.to_vec().into_iter().map(f).collect();
        Self::from_vec(data, self.shape.clone())
    }

    /// Apply a binary function element-wise with broadcasting.
    pub fn zip_with(&self, other: &CrowTensor, f: impl Fn(f64, f64) -> f64) -> CrowTensor {
        // Same shape — fast path
        if self.shape == other.shape {
            let a = self.to_vec();
            let b = other.to_vec();
            let data: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| f(x, y)).collect();
            return Self::from_vec(data, self.shape.clone());
        }
        // Broadcasting
        let out_shape = Self::broadcast_shape(&self.shape, &other.shape);
        let n = Self::numel(&out_shape);
        let mut data = Vec::with_capacity(n);
        let out_strides = Self::row_major_strides(&out_shape);
        let (a_data, b_data) = (self.cpu_data(), other.cpu_data());
        for flat in 0..n {
            let mut remaining = flat;
            let mut a_idx = 0usize;
            let mut b_idx = 0usize;
            let a_ndim = self.ndim();
            let b_ndim = other.ndim();
            let out_ndim = out_shape.len();
            for d in 0..out_ndim {
                let idx = remaining / out_strides[d];
                remaining %= out_strides[d];
                // Map to source dimensions (right-aligned)
                let a_d = (d + a_ndim).wrapping_sub(out_ndim);
                let b_d = (d + b_ndim).wrapping_sub(out_ndim);
                if a_d < a_ndim {
                    let a_dim_idx = if self.shape[a_d] == 1 { 0 } else { idx };
                    a_idx += a_dim_idx * self.strides[a_d];
                }
                if b_d < b_ndim {
                    let b_dim_idx = if other.shape[b_d] == 1 { 0 } else { idx };
                    b_idx += b_dim_idx * other.strides[b_d];
                }
            }
            let a_val = a_data[self.offset + a_idx];
            let b_val = b_data[other.offset + b_idx];
            data.push(f(a_val, b_val));
        }
        Self::from_vec(data, out_shape)
    }

    /// Compute broadcast-compatible output shape.
    #[expect(clippy::panic, reason = "an invariant, never a user's argument: every caller checks `meets` first — try_elementwise for the operators, the math, nn and special entry points for the builtins (docs/bug-classes.md §3)")]
    fn broadcast_shape(a: &[usize], b: &[usize]) -> Vec<usize> {
        let max_ndim = a.len().max(b.len());
        let mut out = Vec::with_capacity(max_ndim);
        for i in 0..max_ndim {
            let da = if i < max_ndim - a.len() {
                1
            } else {
                a[i - (max_ndim - a.len())]
            };
            let db = if i < max_ndim - b.len() {
                1
            } else {
                b[i - (max_ndim - b.len())]
            };
            if da == db {
                out.push(da);
            } else if da == 1 {
                out.push(db);
            } else if db == 1 {
                out.push(da);
            } else {
                panic!(
                    "broadcast: incompatible shapes {:?} and {:?} at dimension {}",
                    a, b, i
                );
            }
        }
        out
    }

    // Convenience element-wise ops

    /// Build a CrowTensor from a GpuBuffer, inheriting shape from self.
    fn from_gpu_buffer(&self, buf: GpuBuffer) -> CrowTensor {
        CrowTensor {
            data: Arc::new(TensorStorage::Gpu(buf)),
            shape: self.shape.clone(),
            strides: Self::row_major_strides(&self.shape),
            offset: 0,
        }
    }

    /// GPU-dispatched binary op, or CPU fallback.
    fn gpu_binary_or_cpu(&self, other: &CrowTensor, op: gpu::BinaryOp, cpu_fn: impl Fn(f64, f64) -> f64) -> CrowTensor {
        // the GPU kernel takes two buffers of one shape; two shapes that
        // broadcast are the CPU's (the kernel asserted one length: a panic)
        if let (Some(a_buf), Some(b_buf), true) = (self.gpu_buffer(), other.gpu_buffer(), self.shape == other.shape) {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_elementwise_binary(ctx, a_buf, b_buf, op);
                return self.from_gpu_buffer(out);
            }
        }
        self.zip_with(other, cpu_fn)
    }

    /// GPU-dispatched unary op, or CPU fallback.
    fn gpu_unary_or_cpu(&self, op: gpu::UnaryOp, cpu_fn: impl Fn(f64) -> f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_elementwise_unary(ctx, buf, op);
                return self.from_gpu_buffer(out);
            }
        }
        self.map(cpu_fn)
    }

    /// An element-wise operation between tensors (`+`, `-`, `.*`, `./`,
    /// `.^`, named by its sign), or the reason their shapes do not meet as
    /// a value the caller reports: never a panic. The words are the ones
    /// the lowering refuses a program with when it knows both shapes.
    pub fn try_elementwise(&self, other: &CrowTensor, sign: &str) -> Result<CrowTensor, String> {
        self.meets(other, sign)?;
        Ok(match sign {
            "+" => self.add(other),
            "-" => self.sub(other),
            ".*" => self.mul(other),
            "./" => self.div(other),
            _ => self.pow(other),
        })
    }

    /// Whether two tensors' shapes broadcast for an element-wise operation
    /// named by `sign`: each pair of dimensions equal, or one of them 1.
    pub fn meets(&self, other: &CrowTensor, sign: &str) -> Result<(), String> {
        Self::meets_shapes(self.shape(), other.shape(), sign)
    }

    /// Whether two shapes broadcast for an element-wise operation named by
    /// `sign` (`meets`, by the shapes alone: a fused chain checks each
    /// operator's operands before it computes any).
    pub fn meets_shapes(a: &[usize], b: &[usize], sign: &str) -> Result<(), String> {
        let n = a.len().max(b.len());
        for i in 0..n {
            let da = if i < n - a.len() { 1 } else { a[i - (n - a.len())] };
            let db = if i < n - b.len() { 1 } else { b[i - (n - b.len())] };
            if da != db && da != 1 && db != 1 {
                return Err(format!(
                    "`{}` of tensors of shapes {:?} and {:?}: dimension {} is {} in one and {} in the other — an element-wise operation needs each pair equal, or one of them 1",
                    sign, a, b, i + 1, da, db
                ));
            }
        }
        Ok(())
    }

    pub fn add(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, gpu::BinaryOp::Add, |a, b| a + b)
    }

    pub fn sub(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, gpu::BinaryOp::Sub, |a, b| a - b)
    }

    pub fn mul(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, gpu::BinaryOp::Mul, |a, b| a * b)
    }

    pub fn div(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, gpu::BinaryOp::Div, |a, b| a / b)
    }

    pub fn neg(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Neg, |x| -x)
    }

    pub fn abs(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Abs, |x| x.abs())
    }

    pub fn exp(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Exp, |x| x.exp())
    }

    pub fn log(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Log, |x| x.ln())
    }

    pub fn log2(&self) -> CrowTensor {
        self.map(|x| x.log2())
    }

    pub fn log10(&self) -> CrowTensor {
        self.map(|x| x.log10())
    }

    pub fn sqrt(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Sqrt, |x| x.sqrt())
    }

    pub fn sin(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Sin, |x| x.sin())
    }

    pub fn cos(&self) -> CrowTensor {
        self.gpu_unary_or_cpu(gpu::UnaryOp::Cos, |x| x.cos())
    }

    pub fn pow(&self, exponent: &CrowTensor) -> CrowTensor {
        // pow not available as GPU shader yet — fall back to CPU
        self.zip_with(exponent, |a, b| a.powf(b))
    }

    /// Add a scalar to every element.
    pub fn add_scalar(&self, s: f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_scalar_op(ctx, buf, s as f32, gpu::ScalarOp::Add);
                return self.from_gpu_buffer(out);
            }
        }
        self.map(|x| x + s)
    }

    /// Multiply every element by a scalar.
    pub fn mul_scalar(&self, s: f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_scalar_op(ctx, buf, s as f32, gpu::ScalarOp::Mul);
                return self.from_gpu_buffer(out);
            }
        }
        self.map(|x| x * s)
    }

    pub fn sub_scalar(&self, s: f64) -> CrowTensor {
        self.map(|x| x - s)
    }

    // ── Additional element-wise math ────────────────────────────────────

    pub fn sign(&self) -> CrowTensor {
        self.map(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 })
    }

    pub fn floor(&self) -> CrowTensor {
        self.map(|x| x.floor())
    }

    pub fn ceil(&self) -> CrowTensor {
        self.map(|x| x.ceil())
    }

    pub fn round(&self) -> CrowTensor {
        self.map(|x| x.round())
    }

    pub fn clip(&self, lo: f64, hi: f64) -> CrowTensor {
        self.map(|x| x.max(lo).min(hi))
    }

    pub fn modulo(&self, other: &CrowTensor) -> CrowTensor {
        self.zip_with(other, |a, b| a % b)
    }

    pub fn modulo_scalar(&self, s: f64) -> CrowTensor {
        self.map(|x| x % s)
    }

    // ── Trigonometry ────────────────────────────────────────────────────

    pub fn tan(&self) -> CrowTensor {
        // (a string op once named a shader the GPU did not have: `tan` of a
        // tensor on the GPU panicked)
        self.gpu_unary_or_cpu(gpu::UnaryOp::Tan, |x| x.tan())
    }

    pub fn asin(&self) -> CrowTensor {
        self.map(|x| x.asin())
    }

    pub fn acos(&self) -> CrowTensor {
        self.map(|x| x.acos())
    }

    pub fn atan(&self) -> CrowTensor {
        self.map(|x| x.atan())
    }

    pub fn atan2(&self, other: &CrowTensor) -> CrowTensor {
        self.zip_with(other, |a, b| a.atan2(b))
    }

    pub fn sinh(&self) -> CrowTensor {
        self.map(|x| x.sinh())
    }

    pub fn cosh(&self) -> CrowTensor {
        self.map(|x| x.cosh())
    }

    pub fn tanh(&self) -> CrowTensor {
        self.map(|x| x.tanh())
    }
}


// ── Reductions ──────────────────────────────────────────────────────────

impl CrowTensor {
    /// Iterate over all elements without allocating.
    /// For contiguous tensors, iterates directly over the data slice.
    /// For non-contiguous or GPU tensors, falls back to `to_vec()`.
    #[inline]
    fn reduce<F>(&self, init: f64, f: F) -> f64
    where F: Fn(f64, f64) -> f64 {
        if self.is_gpu() {
            let v = self.to_cpu().to_vec();
            return v.iter().copied().fold(init, &f);
        }
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            let slice = &cpu[self.offset..self.offset + self.len()];
            slice.iter().copied().fold(init, f)
        } else {
            // Non-contiguous: iterate via index calculation (no alloc)
            let n = self.len();
            let ndim = self.shape.len();
            let mut acc = init;
            let mut idx = vec![0usize; ndim];
            for _ in 0..n {
                let offset: usize = idx.iter().zip(self.strides.iter()).map(|(i, s)| i * s).sum::<usize>() + self.offset;
                acc = f(acc, cpu[offset]);
                // Increment index (row-major order)
                for d in (0..ndim).rev() {
                    idx[d] += 1;
                    if idx[d] < self.shape[d] {
                        break;
                    }
                    idx[d] = 0;
                }
            }
            acc
        }
    }

    /// Sum all elements.
    pub fn sum(&self) -> f64 {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                return gpu::gpu_sum(ctx, buf);
            }
        }
        if self.is_gpu() {
            return self.to_cpu().sum();
        }
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            // Use chunked summation for better auto-vectorization
            let slice = &cpu[self.offset..self.offset + self.len()];
            chunked_sum(slice)
        } else {
            self.reduce(0.0, |a, b| a + b)
        }
    }

    /// Mean of all elements.
    pub fn mean(&self) -> f64 {
        let n = self.len();
        if n == 0 {
            return f64::NAN;
        }
        self.sum() / n as f64
    }

    /// Maximum element.
    pub fn max(&self) -> f64 {
        if self.is_gpu() {
            return self.to_cpu().max();
        }
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            let slice = &cpu[self.offset..self.offset + self.len()];
            chunked_reduce(slice, f64::NEG_INFINITY, |a, b| a.max(b))
        } else {
            self.reduce(f64::NEG_INFINITY, f64::max)
        }
    }

    /// Minimum element.
    pub fn min(&self) -> f64 {
        if self.is_gpu() {
            return self.to_cpu().min();
        }
        let cpu = self.cpu_data();
        if self.is_contiguous() {
            let slice = &cpu[self.offset..self.offset + self.len()];
            chunked_reduce(slice, f64::INFINITY, |a, b| a.min(b))
        } else {
            self.reduce(f64::INFINITY, f64::min)
        }
    }

    /// Product of all elements.
    pub fn prod(&self) -> f64 {
        self.reduce(1.0, |a, b| a * b)
    }

    /// Index of maximum element (flat index).
    pub fn argmax(&self) -> usize {
        let data = self.to_vec();
        if data.is_empty() {
            return 0;
        }
        let mut max_idx = 0;
        let mut max_val = data[0];
        for (i, &v) in data.iter().enumerate().skip(1) {
            if v > max_val {
                max_val = v;
                max_idx = i;
            }
        }
        max_idx
    }

    /// Index of minimum element (flat index).
    pub fn argmin(&self) -> usize {
        let data = self.to_vec();
        if data.is_empty() {
            return 0;
        }
        let mut min_idx = 0;
        let mut min_val = data[0];
        for (i, &v) in data.iter().enumerate().skip(1) {
            if v < min_val {
                min_val = v;
                min_idx = i;
            }
        }
        min_idx
    }

    /// Sum along a given axis, reducing that dimension.
    pub fn sum_axis(&self, axis: usize) -> CrowTensor {
        assert!(axis < self.ndim(), "sum_axis: axis {} out of range", axis);
        let mut new_shape: Vec<usize> = self.shape.clone();
        let _axis_size = new_shape.remove(axis);
        if new_shape.is_empty() {
            // Reducing a 1D tensor to scalar
            return CrowTensor::scalar(self.sum());
        }
        let out_n = Self::numel(&new_shape);
        let mut data = vec![0.0; out_n];
        let out_strides = Self::row_major_strides(&new_shape);
        let src = self.to_vec();
        let src_strides = Self::row_major_strides(&self.shape);
        for flat in 0..self.len() {
            // Compute multi-dim index in source
            let mut remaining = flat;
            let mut out_flat = 0usize;
            let mut out_dim = 0;
            for d in 0..self.ndim() {
                let idx = remaining / src_strides[d];
                remaining %= src_strides[d];
                if d != axis {
                    out_flat += idx * out_strides[out_dim];
                    out_dim += 1;
                }
            }
            data[out_flat] += src[flat];
        }
        Self::from_vec(data, new_shape)
    }

    /// Mean along a given axis.
    pub fn mean_axis(&self, axis: usize) -> CrowTensor {
        let axis_size = self.shape[axis] as f64;
        self.sum_axis(axis).mul_scalar(1.0 / axis_size)
    }

    /// Generic reduction along an axis with an initial value and combining function.
    fn reduce_axis(&self, axis: usize, init: f64, f: impl Fn(f64, f64) -> f64) -> CrowTensor {
        assert!(axis < self.ndim(), "reduce_axis: axis {} out of range", axis);
        let mut new_shape: Vec<usize> = self.shape.clone();
        let _axis_size = new_shape.remove(axis);
        if new_shape.is_empty() {
            let data = self.to_vec();
            let result = data.iter().copied().fold(init, &f);
            return CrowTensor::scalar(result);
        }
        let out_n = Self::numel(&new_shape);
        let mut data = vec![init; out_n];
        let out_strides = Self::row_major_strides(&new_shape);
        let src = self.to_vec();
        let src_strides = Self::row_major_strides(&self.shape);
        for flat in 0..self.len() {
            let mut remaining = flat;
            let mut out_flat = 0usize;
            let mut out_dim = 0;
            for d in 0..self.ndim() {
                let idx = remaining / src_strides[d];
                remaining %= src_strides[d];
                if d != axis {
                    out_flat += idx * out_strides[out_dim];
                    out_dim += 1;
                }
            }
            data[out_flat] = f(data[out_flat], src[flat]);
        }
        Self::from_vec(data, new_shape)
    }

    /// Max along a given axis.
    pub fn max_axis(&self, axis: usize) -> CrowTensor {
        self.reduce_axis(axis, f64::NEG_INFINITY, f64::max)
    }

    /// Min along a given axis.
    pub fn min_axis(&self, axis: usize) -> CrowTensor {
        self.reduce_axis(axis, f64::INFINITY, f64::min)
    }

    /// Product along a given axis.
    pub fn prod_axis(&self, axis: usize) -> CrowTensor {
        self.reduce_axis(axis, 1.0, |a, b| a * b)
    }

    // ── Sorting & Selection ────────────────────────────────────────────

    /// Sort elements in ascending order (flattened).
    pub fn sort(&self) -> CrowTensor {
        let mut data = self.to_vec();
        data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = data.len();
        Self::from_vec(data, vec![len])
    }

    /// Return indices that would sort the tensor (flattened).
    pub fn argsort(&self) -> CrowTensor {
        let data = self.to_vec();
        let mut indices: Vec<usize> = (0..data.len()).collect();
        indices.sort_by(|&a, &b| data[a].partial_cmp(&data[b]).unwrap_or(std::cmp::Ordering::Equal));
        let len = indices.len();
        Self::from_vec(indices.into_iter().map(|i| i as f64).collect(), vec![len])
    }

    /// Return the top-k largest values (sorted descending).
    pub fn topk(&self, k: usize) -> CrowTensor {
        let mut data = self.to_vec();
        data.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let k = k.min(data.len());
        Self::from_vec(data[..k].to_vec(), vec![k])
    }

    /// Return unique values (sorted).
    pub fn unique(&self) -> CrowTensor {
        let mut data = self.to_vec();
        data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        data.dedup();
        let len = data.len();
        Self::from_vec(data, vec![len])
    }

    // ── Cumulative Ops ──────────────────────────────────────────────

    /// Cumulative sum (flattened).
    pub fn cumsum(&self) -> CrowTensor {
        let data = self.to_vec();
        let mut result = Vec::with_capacity(data.len());
        let mut acc = 0.0;
        for &v in &data {
            acc += v;
            result.push(acc);
        }
        Self::from_vec(result, self.shape.clone())
    }

    /// Cumulative product (flattened).
    pub fn cumprod(&self) -> CrowTensor {
        let data = self.to_vec();
        let mut result = Vec::with_capacity(data.len());
        let mut acc = 1.0;
        for &v in &data {
            acc *= v;
            result.push(acc);
        }
        Self::from_vec(result, self.shape.clone())
    }

    /// Conditional selection: where cond is nonzero, take from self, else from other.
    pub fn where_cond(&self, on_true: &CrowTensor, on_false: &CrowTensor) -> CrowTensor {
        let cond = self.to_vec();
        let a = on_true.to_vec();
        let b = on_false.to_vec();
        assert_eq!(cond.len(), a.len(), "where: cond and a must have same length");
        assert_eq!(cond.len(), b.len(), "where: cond and b must have same length");
        let data: Vec<f64> = cond.iter().zip(a.iter().zip(b.iter()))
            .map(|(&c, (&av, &bv))| if c != 0.0 { av } else { bv })
            .collect();
        Self::from_vec(data, self.shape.clone())
    }

    // ── Advanced Indexing ─────────────────────────────────────────────

    /// Gather elements by index array. For a 1D tensor, returns elements at the given indices.
    /// The indices tensor contains integer indices (as f64).
    /// `t[indices]`: the elements at the indices (a negative one counts
    /// from the end), or the reason one is outside — the same words as
    /// `xs[i]`, in both modes (the kernel asserted it: a panic).
    pub fn try_gather(&self, indices: &CrowTensor) -> Result<CrowTensor, String> {
        let src = self.to_vec();
        let mut data = Vec::with_capacity(indices.len());
        for i in indices.to_vec() {
            data.push(src[crate::stops::item(i as i64, src.len())?]);
        }
        Ok(Self::from_vec(data, indices.shape.clone()))
    }

    pub fn gather(&self, indices: &CrowTensor) -> CrowTensor {
        let src = self.to_vec();
        let idx = indices.to_vec();
        let n = src.len() as i64;
        let data: Vec<f64> = idx.iter().map(|&i| {
            let mut ii = i as i64;
            if ii < 0 { ii += n; }
            assert!(ii >= 0 && ii < n, "gather: index {} out of bounds for tensor of length {}", i as i64, n);
            src[ii as usize]
        }).collect();
        Self::from_vec(data, indices.shape.clone())
    }

    // ── Combining Ops ──────────────────────────────────────────────────

    /// Concatenate two tensors along an axis.
    /// Index along axis 0 of a rank>=2 tensor, yielding a rank-1-lower tensor
    /// (the "element" of a matrix is a row). Negative-index handling is the
    /// caller's job.
    pub fn index0(&self, i: usize) -> CrowTensor {
        assert!(self.ndim() >= 2, "index0 requires rank >= 2");
        assert!(i < self.shape[0], "row index {} out of bounds (rows: {})", i, self.shape[0]);
        let inner_shape: Vec<usize> = self.shape[1..].to_vec();
        let chunk = Self::numel(&inner_shape);
        let data = self.to_vec();
        Self::from_vec(data[i * chunk..(i + 1) * chunk].to_vec(), inner_shape)
    }

    /// Split along axis 0 into element tensors (rows for rank 2).
    pub fn rows(&self) -> Vec<CrowTensor> {
        if self.ndim() <= 1 {
            return self.to_vec().into_iter().map(Self::scalar).collect();
        }
        (0..self.shape[0]).map(|i| self.index0(i)).collect()
    }

    /// push[t, x] on a rank-1 tensor: append one element.
    pub fn push_scalar(&self, v: f64) -> Result<CrowTensor, String> {
        if self.ndim() != 1 {
            return Err(format!(
                "shape mismatch — cannot push a scalar onto a rank-{} tensor (push a row instead)",
                self.ndim()
            ));
        }
        let mut data = self.to_vec();
        data.push(v);
        let n = data.len();
        Ok(Self::from_vec(data, vec![n]))
    }

    /// push[m, row] on a rank-N tensor: append one element along axis 0.
    /// The row's shape must equal the tensor's inner shape.
    pub fn push_row(&self, row: &CrowTensor) -> Result<CrowTensor, String> {
        if self.ndim() < 2 {
            return Err("shape mismatch — cannot push a row onto a rank-1 tensor (push a scalar, or use stack)".to_string());
        }
        let inner: Vec<usize> = self.shape[1..].to_vec();
        if row.shape() != inner.as_slice() {
            return Err(format!(
                "shape mismatch — cannot append a row of length {} to a tensor with rows of length {}",
                row.len(),
                Self::numel(&inner)
            ));
        }
        let mut data = self.to_vec();
        data.extend_from_slice(&row.to_vec());
        let mut new_shape = self.shape.clone();
        new_shape[0] += 1;
        Ok(Self::from_vec(data, new_shape))
    }

    /// Build a rank-(N+1) tensor from equally shaped element tensors.
    /// Errors on ragged shapes ("all inner arrays must be the same length").
    pub fn from_elements(items: &[CrowTensor]) -> Result<CrowTensor, String> {
        let first_shape = items.first().map(|t| t.shape().to_vec()).unwrap_or_default();
        for t in items {
            if t.shape() != first_shape.as_slice() {
                return Err("all inner arrays must be the same length".to_string());
            }
        }
        let mut data = Vec::with_capacity(items.len() * Self::numel(&first_shape));
        for t in items {
            data.extend_from_slice(&t.to_vec());
        }
        let mut shape = vec![items.len()];
        shape.extend_from_slice(&first_shape);
        Ok(Self::from_vec(data, shape))
    }

    /// `concat[a, b]`, `concat[a, b, axis]` of tensors: one tensor, or the
    /// reason the two do not join — ranks, an axis, a shape across it (the
    /// kernel asserted them: a panic).
    pub fn try_concat(&self, other: &CrowTensor, axis: i64) -> Result<CrowTensor, String> {
        if self.ndim() != other.ndim() {
            return Err(format!("concat: tensors of {} and {} dimensions do not join", self.ndim(), other.ndim()));
        }
        let ax = usize::try_from(axis).ok().filter(|a| *a < self.ndim().max(1)).ok_or_else(|| format!("concat: axis {} is out of range for a {}D tensor", axis, self.ndim()))?;
        for d in 0..self.ndim() {
            if d != ax && self.shape[d] != other.shape[d] {
                return Err(format!("concat: shapes {:?} and {:?} differ across axis {}", self.shape, other.shape, d));
            }
        }
        Ok(self.concat(other, ax))
    }

    pub fn concat(&self, other: &CrowTensor, axis: usize) -> CrowTensor {
        assert_eq!(self.ndim(), other.ndim(), "concat: tensors must have same number of dimensions");
        assert!(axis < self.ndim(), "concat: axis {} out of range", axis);
        for d in 0..self.ndim() {
            if d != axis {
                assert_eq!(self.shape[d], other.shape[d],
                    "concat: shapes must match on all axes except concat axis (axis {}: {} vs {})", d, self.shape[d], other.shape[d]);
            }
        }
        if self.ndim() == 1 {
            let mut data = self.to_vec();
            data.extend_from_slice(&other.to_vec());
            let len = data.len();
            return Self::from_vec(data, vec![len]);
        }
        // General N-D concat
        let mut new_shape = self.shape.clone();
        new_shape[axis] = self.shape[axis] + other.shape[axis];
        let out_n = Self::numel(&new_shape);
        let mut data = vec![0.0; out_n];
        let out_strides = Self::row_major_strides(&new_shape);
        let src_a = self.to_vec();
        let src_b = other.to_vec();
        let strides_a = Self::row_major_strides(&self.shape);
        let strides_b = Self::row_major_strides(&other.shape);
        // Copy from self
        for flat in 0..self.len() {
            let mut remaining = flat;
            let mut out_flat = 0;
            for d in 0..self.ndim() {
                let idx = remaining / strides_a[d];
                remaining %= strides_a[d];
                out_flat += idx * out_strides[d];
            }
            data[out_flat] = src_a[flat];
        }
        // Copy from other (offset along concat axis)
        for flat in 0..other.len() {
            let mut remaining = flat;
            let mut out_flat = 0;
            for d in 0..other.ndim() {
                let idx = remaining / strides_b[d];
                remaining %= strides_b[d];
                let out_idx = if d == axis { idx + self.shape[axis] } else { idx };
                out_flat += out_idx * out_strides[d];
            }
            data[out_flat] = src_b[flat];
        }
        Self::from_vec(data, new_shape)
    }

    /// Stack tensors along a new axis. All tensors must have the same shape.
    pub fn stack(tensors: &[CrowTensor], axis: usize) -> CrowTensor {
        assert!(!tensors.is_empty(), "stack: need at least one tensor");
        let base_shape = tensors[0].shape();
        for t in &tensors[1..] {
            assert_eq!(t.shape(), base_shape, "stack: all tensors must have the same shape");
        }
        assert!(axis <= base_shape.len(), "stack: axis {} out of range for ndim {}", axis, base_shape.len());
        // New shape: insert tensors.len() at position axis
        let mut new_shape = base_shape.to_vec();
        new_shape.insert(axis, tensors.len());
        let mut data = Vec::with_capacity(Self::numel(&new_shape));
        if axis == 0 {
            // Simple: just concatenate all data
            for t in tensors {
                data.extend_from_slice(&t.to_vec());
            }
        } else {
            // General case: interleave
            let out_strides = Self::row_major_strides(&new_shape);
            let base_strides = Self::row_major_strides(base_shape);
            let out_n = Self::numel(&new_shape);
            data.resize(out_n, 0.0);
            for (ti, t) in tensors.iter().enumerate() {
                let src = t.to_vec();
                for flat in 0..t.len() {
                    let mut remaining = flat;
                    let mut out_flat = ti * out_strides[axis];
                    let mut out_dim = 0;
                    for d in 0..base_shape.len() {
                        if out_dim == axis { out_dim += 1; }
                        let idx = remaining / base_strides[d];
                        remaining %= base_strides[d];
                        out_flat += idx * out_strides[out_dim];
                        out_dim += 1;
                    }
                    data[out_flat] = src[flat];
                }
            }
        }
        Self::from_vec(data, new_shape)
    }
}

// ── Display ─────────────────────────────────────────────────────────────

impl std::fmt::Display for CrowTensor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.to_vec();
        if self.ndim() == 0 {
            // Scalar
            return write!(f, "{}", data[0]);
        }
        // Nested brackets for any rank: [1, 2] / [[1, 2], [3, 4]] / ...
        fn fmt_nested(data: &[f64], shape: &[usize]) -> String {
            if shape.len() == 1 {
                let parts: Vec<String> = data.iter().map(|v| v.to_string()).collect();
                return format!("[{}]", parts.join(", "));
            }
            let chunk: usize = shape[1..].iter().product();
            let parts: Vec<String> = data
                .chunks(chunk.max(1))
                .map(|c| fmt_nested(c, &shape[1..]))
                .collect();
            format!("[{}]", parts.join(", "))
        }
        write!(f, "{}", fmt_nested(&data, &self.shape))
    }
}

impl crate::display::CrowDisplay for CrowTensor {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

// ── Equality ────────────────────────────────────────────────────────────

/// Two tensors are equal when they have the same shape and the same numbers.
impl PartialEq for CrowTensor {
    fn eq(&self, other: &Self) -> bool {
        self.shape == other.shape && self.to_vec() == other.to_vec()
    }
}

/// The numbers a list holds, whether it is a tensor, a list of numbers, or a
/// list of lists: `eq` compares numbers and shape, not the representation
/// (an all-numeric array is a tensor only for speed).
pub trait Numbers {
    fn numbers_into(&self, out: &mut Vec<f64>);
    fn shape_into(&self, shape: &mut Vec<usize>);
}

impl Numbers for f64 {
    fn numbers_into(&self, out: &mut Vec<f64>) {
        out.push(*self);
    }
    fn shape_into(&self, _: &mut Vec<usize>) {}
}

impl Numbers for i64 {
    fn numbers_into(&self, out: &mut Vec<f64>) {
        out.push(*self as f64);
    }
    fn shape_into(&self, _: &mut Vec<usize>) {}
}

impl Numbers for CrowTensor {
    fn numbers_into(&self, out: &mut Vec<f64>) {
        out.extend(self.to_vec());
    }
    fn shape_into(&self, shape: &mut Vec<usize>) {
        shape.extend_from_slice(&self.shape);
    }
}

impl<T: Numbers> Numbers for Vec<T> {
    fn numbers_into(&self, out: &mut Vec<f64>) {
        for x in self {
            x.numbers_into(out);
        }
    }
    fn shape_into(&self, shape: &mut Vec<usize>) {
        shape.push(self.len());
        if let Some(first) = self.first() {
            first.shape_into(shape);
        }
    }
}

/// `a eq b` for two numeric lists held differently (a list built by `push`
/// against a literal, a list of rows against a matrix).
pub fn numbers_equal<A: Numbers, B: Numbers>(a: &A, b: &B) -> bool {
    let (mut sa, mut sb) = (Vec::new(), Vec::new());
    a.shape_into(&mut sa);
    b.shape_into(&mut sb);
    let (mut na, mut nb) = (Vec::new(), Vec::new());
    a.numbers_into(&mut na);
    b.numbers_into(&mut nb);
    sa == sb && na == nb
}


#[cfg(test)]
pub(crate) mod tests {

    /// An index outside, and tensors that do not join, are the reason —
    /// the kernels asserted them, a panic in both modes.
    #[test]
    fn a_gather_or_a_concat_that_does_not_fit_is_the_reason() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0], vec![3]);
        let far = CrowTensor::from_vec(vec![0.0, 9.0], vec![2]);
        assert_eq!(a.try_gather(&far).map(|_| ()), Err("index 9 is out of range for an array of 3 items".to_string()));
        assert_eq!(a.try_gather(&CrowTensor::from_vec(vec![-1.0], vec![1])).map(|t| t.to_vec()), Ok(vec![30.0]));
        let m = CrowTensor::zeros(vec![2, 2]);
        assert_eq!(a.try_concat(&m, 0).map(|_| ()), Err("concat: tensors of 1 and 2 dimensions do not join".to_string()));
        assert_eq!(m.try_concat(&CrowTensor::zeros(vec![2, 3]), 0).map(|_| ()), Err("concat: shapes [2, 2] and [2, 3] differ across axis 1".to_string()));
        assert_eq!(m.try_concat(&m, 5).map(|_| ()), Err("concat: axis 5 is out of range for a 2D tensor".to_string()));
    }
    use super::*;

    #[test]
    fn numbers_compare_by_value_and_shape_whatever_holds_them() {
        let v = CrowTensor::from_vec(vec![0.0, 60.0], vec![2]);
        assert!(numbers_equal(&vec![0.0f64, 60.0], &v));
        assert!(numbers_equal(&vec![0i64, 60], &v));
        assert!(!numbers_equal(&vec![0.0f64, 61.0], &v));
        assert!(!numbers_equal(&vec![0.0f64], &v));
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        assert!(numbers_equal(&vec![vec![1.0f64, 2.0], vec![3.0, 4.0]], &m));
        assert!(!numbers_equal(&vec![1.0f64, 2.0, 3.0, 4.0], &m));
        // a ragged list is never a matrix
        assert!(!numbers_equal(&vec![vec![1.0f64, 2.0, 3.0], vec![4.0]], &m));
        // two tensors: the same numbers in another shape are not equal
        assert_eq!(m, CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]));
        assert_ne!(m, CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]));
        assert!(numbers_equal(&Vec::<f64>::new(), &CrowTensor::from_vec(vec![], vec![0])));
    }

    #[test]
    fn a_tensor_round_trips_through_json() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let j = serde_json::to_value(&t).unwrap();
        assert_eq!(j["shape"], serde_json::json!([2, 3]));
        let back: CrowTensor = serde_json::from_value(j).unwrap();
        assert_eq!(back.shape(), &[2, 3]);
        assert_eq!(back.to_vec(), t.to_vec());
        // a transposed view serializes its values in its own row-major order
        let tt = t.transpose();
        let back2: CrowTensor = serde_json::from_value(serde_json::to_value(&tt).unwrap()).unwrap();
        assert_eq!(back2.shape(), &[3, 2]);
        assert_eq!(back2.to_vec(), tt.to_vec());
    }

    /// The seeded RNG is process-global (`crate::rng`), so tests that draw
    /// random numbers must not overlap: each holds this lock while it runs.
    pub(crate) static RNG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn rng_lock() -> std::sync::MutexGuard<'static, ()> {
        RNG_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hold the lock and seed the generator; the guard keeps other random
    /// tests out until the test ends.
    fn seeded(seed: u64) -> std::sync::MutexGuard<'static, ()> {
        let guard = rng_lock();
        crate::rng::set_seed(seed);
        guard
    }

    // ── Creation ────────────────────────────────────────────────────

    #[test]
    fn test_from_vec() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        assert_eq!(t.shape(), &[2, 3]);
        assert_eq!(t.strides(), &[3, 1]);
        assert_eq!(t.ndim(), 2);
        assert_eq!(t.len(), 6);
        assert!(t.is_contiguous());
    }

    #[test]
    fn test_scalar() {
        let t = CrowTensor::scalar(42.0);
        assert_eq!(t.shape(), &[] as &[usize]);
        assert_eq!(t.len(), 1);
        assert_eq!(t.to_vec(), vec![42.0]);
    }

    #[test]
    fn test_zeros() {
        let t = CrowTensor::zeros(vec![2, 3]);
        assert_eq!(t.to_vec(), vec![0.0; 6]);
        assert_eq!(t.shape(), &[2, 3]);
    }

    #[test]
    fn test_ones() {
        let t = CrowTensor::ones(vec![3]);
        assert_eq!(t.to_vec(), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_full() {
        let t = CrowTensor::full(vec![2, 2], 7.0);
        assert_eq!(t.to_vec(), vec![7.0, 7.0, 7.0, 7.0]);
    }

    #[test]
    fn test_arange() {
        let t = CrowTensor::arange(0.0, 5.0, 1.0);
        assert_eq!(t.to_vec(), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(t.shape(), &[5]);
    }

    #[test]
    fn test_arange_step() {
        let t = CrowTensor::arange(0.0, 10.0, 2.5);
        assert_eq!(t.to_vec(), vec![0.0, 2.5, 5.0, 7.5]);
    }

    #[test]
    fn test_linspace() {
        let t = CrowTensor::linspace(0.0, 1.0, 5);
        assert_eq!(t.len(), 5);
        assert!((t.get_flat(0) - 0.0).abs() < 1e-10);
        assert!((t.get_flat(4) - 1.0).abs() < 1e-10);
        assert!((t.get_flat(2) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_eye() {
        let t = CrowTensor::eye(3);
        assert_eq!(t.shape(), &[3, 3]);
        assert_eq!(
            t.to_vec(),
            vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn test_diag_from_1d() {
        let v = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let d = CrowTensor::diag(&v).expect("a vector has a diagonal matrix");
        assert_eq!(d.shape(), &[3, 3]);
        assert_eq!(
            d.to_vec(),
            vec![1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0]
        );
    }

    #[test]
    fn test_diag_from_2d() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let d = CrowTensor::diag(&m).expect("a matrix has a diagonal");
        assert_eq!(d.shape(), &[3]);
        assert_eq!(d.to_vec(), vec![1.0, 5.0, 9.0]);
    }

    // ── Views & Reshaping ───────────────────────────────────────────

    #[test]
    fn test_transpose() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let t = m.transpose();
        assert_eq!(t.shape(), &[3, 2]);
        assert_eq!(t.to_vec(), vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert!(!t.is_contiguous());
        // Original unchanged
        assert_eq!(m.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_transpose_shares_data() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let t = m.transpose();
        // Both point to the same Arc
        assert!(Arc::ptr_eq(&m.data, &t.data));
    }

    #[test]
    fn test_reshape() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let r = t.reshape(vec![3, 2]);
        assert_eq!(r.shape(), &[3, 2]);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_flatten() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let f = t.flatten();
        assert_eq!(f.shape(), &[4]);
        assert_eq!(f.to_vec(), vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_slice() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        // Slice first row
        let s = t.slice(0, 0, 1);
        assert_eq!(s.shape(), &[1, 3]);
        assert_eq!(s.to_vec(), vec![1.0, 2.0, 3.0]);
        // Slice shares data
        assert!(Arc::ptr_eq(&t.data, &s.data));
    }

    #[test]
    fn test_to_contiguous() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let t = m.transpose();
        assert!(!t.is_contiguous());
        let c = t.to_contiguous();
        assert!(c.is_contiguous());
        assert_eq!(c.to_vec(), vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    // ── Element-wise ops ────────────────────────────────────────────

    #[test]
    fn test_add() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        assert_eq!(a.add(&b).to_vec(), vec![5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_sub() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0], vec![2]);
        let b = CrowTensor::from_vec(vec![3.0, 7.0], vec![2]);
        assert_eq!(a.sub(&b).to_vec(), vec![7.0, 13.0]);
    }

    #[test]
    fn test_mul() {
        let a = CrowTensor::from_vec(vec![2.0, 3.0], vec![2]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0], vec![2]);
        assert_eq!(a.mul(&b).to_vec(), vec![8.0, 15.0]);
    }

    #[test]
    fn test_div() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0], vec![2]);
        let b = CrowTensor::from_vec(vec![2.0, 5.0], vec![2]);
        assert_eq!(a.div(&b).to_vec(), vec![5.0, 4.0]);
    }

    #[test]
    fn test_neg() {
        let a = CrowTensor::from_vec(vec![1.0, -2.0, 3.0], vec![3]);
        assert_eq!(a.neg().to_vec(), vec![-1.0, 2.0, -3.0]);
    }

    #[test]
    fn test_add_scalar() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        assert_eq!(a.add_scalar(10.0).to_vec(), vec![11.0, 12.0, 13.0]);
    }

    #[test]
    fn test_mul_scalar() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        assert_eq!(a.mul_scalar(2.0).to_vec(), vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn test_broadcast_add() {
        // [3, 1] + [1, 4] → [3, 4]
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3, 1]);
        let b = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0], vec![1, 4]);
        let c = a.add(&b);
        assert_eq!(c.shape(), &[3, 4]);
        assert_eq!(
            c.to_vec(),
            vec![11.0, 21.0, 31.0, 41.0, 12.0, 22.0, 32.0, 42.0, 13.0, 23.0, 33.0, 43.0]
        );
    }

    #[test]
    fn test_broadcast_scalar() {
        // [3] + scalar → [3]
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::scalar(10.0);
        let c = a.add(&b);
        assert_eq!(c.shape(), &[3]);
        assert_eq!(c.to_vec(), vec![11.0, 12.0, 13.0]);
    }

    #[test]
    fn test_exp_log_roundtrip() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let result = a.exp().log();
        for (i, v) in result.to_vec().iter().enumerate() {
            assert!((v - a.get_flat(i)).abs() < 1e-10);
        }
    }

    // ── Reductions ──────────────────────────────────────────────────

    #[test]
    fn test_sum() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        assert_eq!(t.sum(), 10.0);
    }

    #[test]
    fn test_mean() {
        let t = CrowTensor::from_vec(vec![2.0, 4.0, 6.0], vec![3]);
        assert_eq!(t.mean(), 4.0);
    }

    #[test]
    fn test_max_min() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.0, 5.0], vec![5]);
        assert_eq!(t.max(), 5.0);
        assert_eq!(t.min(), 1.0);
    }

    #[test]
    fn test_sum_axis() {
        // [[1, 2, 3], [4, 5, 6]] → sum axis 0 → [5, 7, 9]
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let s = t.sum_axis(0);
        assert_eq!(s.shape(), &[3]);
        assert_eq!(s.to_vec(), vec![5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_sum_axis_1() {
        // [[1, 2, 3], [4, 5, 6]] → sum axis 1 → [6, 15]
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let s = t.sum_axis(1);
        assert_eq!(s.shape(), &[2]);
        assert_eq!(s.to_vec(), vec![6.0, 15.0]);
    }

    #[test]
    fn test_mean_axis() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let m = t.mean_axis(0);
        assert_eq!(m.shape(), &[3]);
        assert_eq!(m.to_vec(), vec![2.5, 3.5, 4.5]);
    }

    #[test]
    fn test_prod() {
        let t = CrowTensor::from_vec(vec![2.0, 3.0, 4.0], vec![3]);
        assert_eq!(t.prod(), 24.0);
    }

    #[test]
    fn test_prod_empty() {
        let t = CrowTensor::from_vec(vec![], vec![0]);
        assert_eq!(t.prod(), 1.0); // empty product is 1
    }

    #[test]
    fn test_argmax() {
        let t = CrowTensor::from_vec(vec![1.0, 5.0, 3.0, 2.0], vec![4]);
        assert_eq!(t.argmax(), 1);
    }

    #[test]
    fn test_argmin() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.5], vec![4]);
        assert_eq!(t.argmin(), 1);
    }

    #[test]
    fn test_argmax_first_occurrence() {
        // When multiple elements share the max, return first index
        let t = CrowTensor::from_vec(vec![5.0, 5.0, 3.0], vec![3]);
        assert_eq!(t.argmax(), 0);
    }

    #[test]
    fn test_max_axis() {
        // [[1, 5, 3], [4, 2, 6]] → max axis 0 → [4, 5, 6]
        let t = CrowTensor::from_vec(vec![1.0, 5.0, 3.0, 4.0, 2.0, 6.0], vec![2, 3]);
        let r = t.max_axis(0);
        assert_eq!(r.shape(), &[3]);
        assert_eq!(r.to_vec(), vec![4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_min_axis() {
        let t = CrowTensor::from_vec(vec![1.0, 5.0, 3.0, 4.0, 2.0, 6.0], vec![2, 3]);
        let r = t.min_axis(0);
        assert_eq!(r.shape(), &[3]);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_prod_axis() {
        // [[2, 3], [4, 5]] → prod axis 0 → [8, 15]
        let t = CrowTensor::from_vec(vec![2.0, 3.0, 4.0, 5.0], vec![2, 2]);
        let r = t.prod_axis(0);
        assert_eq!(r.shape(), &[2]);
        assert_eq!(r.to_vec(), vec![8.0, 15.0]);
    }

    #[test]
    fn test_sort() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.0, 5.0], vec![5]);
        let s = t.sort();
        assert_eq!(s.to_vec(), vec![1.0, 1.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn test_argsort() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.0, 5.0], vec![5]);
        let s = t.argsort();
        assert_eq!(s.to_vec(), vec![1.0, 3.0, 0.0, 2.0, 4.0]);
    }

    #[test]
    fn test_topk() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.0, 5.0], vec![5]);
        let top = t.topk(3);
        assert_eq!(top.to_vec(), vec![5.0, 4.0, 3.0]);
        assert_eq!(top.shape(), &[3]);
    }

    #[test]
    fn test_unique() {
        let t = CrowTensor::from_vec(vec![3.0, 1.0, 4.0, 1.0, 3.0], vec![5]);
        let u = t.unique();
        assert_eq!(u.to_vec(), vec![1.0, 3.0, 4.0]);
        assert_eq!(u.shape(), &[3]);
    }

    #[test]
    fn test_where_cond() {
        let cond = CrowTensor::from_vec(vec![1.0, 0.0, 1.0, 0.0], vec![4]);
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0], vec![4]);
        let b = CrowTensor::from_vec(vec![100.0, 200.0, 300.0, 400.0], vec![4]);
        let r = cond.where_cond(&a, &b);
        assert_eq!(r.to_vec(), vec![10.0, 200.0, 30.0, 400.0]);
    }

    #[test]
    fn test_cumsum() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        let r = t.cumsum();
        assert_eq!(r.to_vec(), vec![1.0, 3.0, 6.0, 10.0]);
    }

    #[test]
    fn test_cumprod() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        let r = t.cumprod();
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 6.0, 24.0]);
    }

    #[test]
    fn test_concat_1d() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0], vec![2]);
        let r = a.concat(&b, 0);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(r.shape(), &[5]);
    }

    #[test]
    fn test_concat_2d() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let b = CrowTensor::from_vec(vec![5.0, 6.0], vec![1, 2]);
        let r = a.concat(&b, 0);
        assert_eq!(r.shape(), &[3, 2]);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_stack_1d() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        let r = CrowTensor::stack(&[a, b], 0);
        assert_eq!(r.shape(), &[2, 3]);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_stack_axis1() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        let r = CrowTensor::stack(&[a, b], 1);
        assert_eq!(r.shape(), &[3, 2]);
        assert_eq!(r.to_vec(), vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    // ===== Broadcasting tests =====

    #[test]
    fn test_broadcast_add_3x1_1x4() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3, 1]);
        let b = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0], vec![1, 4]);
        let r = a.add(&b);
        assert_eq!(r.shape(), &[3, 4]);
        assert_eq!(r.to_vec(), vec![11.0, 21.0, 31.0, 41.0, 12.0, 22.0, 32.0, 42.0, 13.0, 23.0, 33.0, 43.0]);
    }

    #[test]
    fn test_broadcast_mul_1d_2d() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0], vec![2, 3]);
        let r = a.mul(&b);
        assert_eq!(r.shape(), &[2, 3]);
        assert_eq!(r.to_vec(), vec![10.0, 40.0, 90.0, 40.0, 100.0, 180.0]);
    }

    // ===== Advanced indexing (gather) tests =====

    #[test]
    fn test_gather_basic() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0, 50.0], vec![5]);
        let idx = CrowTensor::from_vec(vec![0.0, 2.0, 4.0], vec![3]);
        let r = a.gather(&idx);
        assert_eq!(r.to_vec(), vec![10.0, 30.0, 50.0]);
        assert_eq!(r.shape(), &[3]);
    }

    #[test]
    fn test_gather_negative_index() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0], vec![3]);
        let idx = CrowTensor::from_vec(vec![-1.0, -2.0], vec![2]);
        let r = a.gather(&idx);
        assert_eq!(r.to_vec(), vec![30.0, 20.0]);
    }

    #[test]
    fn test_gather_repeated_indices() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0], vec![3]);
        let idx = CrowTensor::from_vec(vec![0.0, 0.0, 1.0, 1.0], vec![4]);
        let r = a.gather(&idx);
        assert_eq!(r.to_vec(), vec![10.0, 10.0, 20.0, 20.0]);
    }

    #[test]
    #[should_panic(expected = "gather: index")]
    fn test_gather_out_of_bounds() {
        let a = CrowTensor::from_vec(vec![10.0, 20.0, 30.0], vec![3]);
        let idx = CrowTensor::from_vec(vec![5.0], vec![1]);
        let _ = a.gather(&idx);
    }


    #[test]
    fn test_broadcast_scalar_tensor() {
        let a = CrowTensor::from_vec(vec![5.0], vec![1]);
        let b = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let r = a.add(&b);
        assert_eq!(r.shape(), &[3]);
        assert_eq!(r.to_vec(), vec![6.0, 7.0, 8.0]);
    }

    #[test]
    #[should_panic(expected = "broadcast: incompatible shapes")]
    fn test_broadcast_incompatible() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        let _ = a.add(&b);
    }






    // ── Display ─────────────────────────────────────────────────────

    #[test]
    fn test_display_1d() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        assert_eq!(format!("{t}"), "[1, 2, 3]");
    }

    #[test]
    fn test_display_scalar() {
        let t = CrowTensor::scalar(42.0);
        assert_eq!(format!("{t}"), "42");
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn test_empty_tensor() {
        let t = CrowTensor::zeros(vec![0]);
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    #[should_panic(expected = "data length")]
    fn test_from_vec_shape_mismatch() {
        CrowTensor::from_vec(vec![1.0, 2.0], vec![3]);
    }

    #[test]
    #[should_panic(expected = "transpose: need at least 2")]
    fn test_transpose_1d_panics() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        t.transpose();
    }

    #[test]
    #[should_panic(expected = "reshape: cannot reshape")]
    fn test_reshape_mismatch() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        t.reshape(vec![2, 2]);
    }

    // ── Rand ────────────────────────────────────────────────────────

    #[test]
    fn test_rand() {
        let _rng = rng_lock();
        let t = CrowTensor::rand(vec![100]);
        assert_eq!(t.shape(), &[100]);
        assert_eq!(t.len(), 100);
        // All values in [0, 1)
        for v in t.to_vec() {
            assert!(v >= 0.0 && v < 1.0);
        }
    }

    #[test]
    fn test_randn() {
        let _rng = rng_lock();
        let t = CrowTensor::randn(vec![1000]);
        assert_eq!(t.shape(), &[1000]);
        // Mean should be near 0
        let mean = t.mean();
        assert!(mean.abs() < 0.2, "randn mean too far from 0: {mean}");
    }

    #[test]
    fn test_rand_range() {
        let _rng = rng_lock();
        let t = CrowTensor::rand_range(vec![100], 5.0, 10.0);
        for v in t.to_vec() {
            assert!(v >= 5.0 && v < 10.0);
        }
    }

    #[test]
    fn test_rand_2d_shape() {
        let _rng = rng_lock();
        let t = CrowTensor::rand(vec![3, 4]);
        assert_eq!(t.shape(), &[3, 4]);
        assert_eq!(t.len(), 12);
    }

    #[test]
    fn test_gpu_flag_default_off() {
        // GPU flag should be off by default — tensors created on CPU
        gpu::set_gpu_enabled(false);
        let t = CrowTensor::zeros(vec![4]);
        assert!(!t.is_gpu());
    }

    #[test]
    fn test_maybe_to_gpu_when_disabled() {
        gpu::set_gpu_enabled(false);
        let t = CrowTensor::ones(vec![3]);
        // maybe_to_gpu should be a no-op when flag is off
        let t2 = t.clone().maybe_to_gpu();
        assert!(!t2.is_gpu());
        assert_eq!(t2.to_vec(), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_gpu_binary_fallback_cpu() {
        // When both tensors are on CPU, binary ops should work normally
        gpu::set_gpu_enabled(false);
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        let c = a.add(&b);
        assert_eq!(c.to_vec(), vec![5.0, 7.0, 9.0]);
    }

    #[test]
    fn test_gpu_unary_fallback_cpu() {
        gpu::set_gpu_enabled(false);
        let a = CrowTensor::from_vec(vec![-1.0, 2.0, -3.0], vec![3]);
        let b = a.neg();
        assert_eq!(b.to_vec(), vec![1.0, -2.0, 3.0]);
        let c = a.abs();
        assert_eq!(c.to_vec(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_gpu_scalar_fallback_cpu() {
        gpu::set_gpu_enabled(false);
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = a.add_scalar(10.0);
        assert_eq!(b.to_vec(), vec![11.0, 12.0, 13.0]);
        let c = a.mul_scalar(2.0);
        assert_eq!(c.to_vec(), vec![2.0, 4.0, 6.0]);
    }

    // ===== Layer 3: Random & Distributions =====

    #[test]
    fn test_seed_reproducibility() {
        let _rng = seeded(42);
        let a = CrowTensor::rand(vec![5]);
        crate::rng::set_seed(42);
        let b = CrowTensor::rand(vec![5]);
        assert_eq!(a.to_vec(), b.to_vec(), "same seed should give same values");
        crate::rng::clear_seed();
    }

    #[test]
    fn test_normal_shape() {
        let _rng = seeded(99);
        let t = CrowTensor::normal(vec![3, 2], 5.0, 0.1);
        assert_eq!(t.shape, vec![3, 2]);
        assert_eq!(t.to_vec().len(), 6);
        // Values should be near 5.0
        for v in t.to_vec() {
            assert!((v - 5.0).abs() < 2.0, "expected near 5.0, got {}", v);
        }
        crate::rng::clear_seed();
    }

    #[test]
    fn test_poisson_shape() {
        let _rng = seeded(7);
        let t = CrowTensor::poisson(vec![10], 3.5).expect("its parameters are good");
        assert_eq!(t.shape, vec![10]);
        // All values should be non-negative integers
        for v in t.to_vec() {
            assert!(v >= 0.0 && v == v.floor(), "expected non-negative int, got {}", v);
        }
        crate::rng::clear_seed();
    }

    #[test]
    fn test_bernoulli_values() {
        let _rng = seeded(123);
        let t = CrowTensor::bernoulli(vec![20], 0.5).expect("its parameters are good");
        assert_eq!(t.shape, vec![20]);
        for v in t.to_vec() {
            assert!(v == 0.0 || v == 1.0, "expected 0.0 or 1.0, got {}", v);
        }
        crate::rng::clear_seed();
    }

    #[test]
    fn test_shuffle_preserves_elements() {
        let _rng = seeded(1);
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0], vec![5]);
        let s = t.shuffle();
        let mut sv = s.to_vec();
        sv.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(sv, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        crate::rng::clear_seed();
    }

    #[test]
    fn test_choice_subset() {
        let _rng = seeded(2);
        let t = CrowTensor::from_vec(vec![10.0, 20.0, 30.0, 40.0, 50.0], vec![5]);
        let c = t.choice(3);
        assert_eq!(c.shape, vec![3]);
        let orig = t.to_vec();
        for v in c.to_vec() {
            assert!(orig.contains(&v), "choice value {} not in original", v);
        }
        crate::rng::clear_seed();
    }

    #[test]
    fn test_permutation_range() {
        let _rng = seeded(3);
        let p = CrowTensor::permutation(5);
        assert_eq!(p.shape, vec![5]);
        let mut vals = p.to_vec();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(vals, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        crate::rng::clear_seed();
    }

    // ── log2 / log10 ───────────────────────────────────────────────────

    #[test]
    fn test_log2() {
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 4.0, 8.0], vec![4]);
        let r = t.log2();
        let v = r.to_vec();
        assert!((v[0] - 0.0).abs() < 1e-12);
        assert!((v[1] - 1.0).abs() < 1e-12);
        assert!((v[2] - 2.0).abs() < 1e-12);
        assert!((v[3] - 3.0).abs() < 1e-12);
    }

    #[test]
    fn test_log10() {
        let t = CrowTensor::from_vec(vec![1.0, 10.0, 100.0, 1000.0], vec![4]);
        let r = t.log10();
        let v = r.to_vec();
        assert!((v[0] - 0.0).abs() < 1e-12);
        assert!((v[1] - 1.0).abs() < 1e-12);
        assert!((v[2] - 2.0).abs() < 1e-12);
        assert!((v[3] - 3.0).abs() < 1e-12);
    }

    // ── gamma_dist ──────────────────────────────────────────────────────

    #[test]
    fn test_gamma_dist_shape_and_positive() {
        let _rng = seeded(42);
        let t = CrowTensor::gamma_dist(vec![100], 2.0, 1.0).expect("its parameters are good");
        assert_eq!(t.shape, vec![100]);
        // Gamma(alpha, beta) values should all be positive
        assert!(t.to_vec().iter().all(|&x| x > 0.0));
        crate::rng::clear_seed();
    }

    // ── beta_dist ───────────────────────────────────────────────────────

    #[test]
    fn test_beta_dist_shape_and_range() {
        let _rng = seeded(42);
        let t = CrowTensor::beta_dist(vec![100], 2.0, 5.0).expect("its parameters are good");
        assert_eq!(t.shape, vec![100]);
        // Beta values should be in (0, 1)
        assert!(t.to_vec().iter().all(|&x| x > 0.0 && x < 1.0));
        crate::rng::clear_seed();
    }

    // ── exponential ─────────────────────────────────────────────────────

    #[test]
    fn test_exponential_shape_and_positive() {
        let _rng = seeded(42);
        let t = CrowTensor::exponential(vec![100], 1.5).expect("its parameters are good");
        assert_eq!(t.shape, vec![100]);
        // Exponential values should all be positive
        assert!(t.to_vec().iter().all(|&x| x > 0.0));
        crate::rng::clear_seed();
    }

    #[test]
    fn test_exponential_2d() {
        let _rng = seeded(42);
        let t = CrowTensor::exponential(vec![3, 4], 1.0).expect("its parameters are good");
        assert_eq!(t.shape, vec![3, 4]);
        assert_eq!(t.to_vec().len(), 12);
        crate::rng::clear_seed();
    }
}