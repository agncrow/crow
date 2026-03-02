use std::sync::Arc;

use rand::RngExt;
use rand_distr::{Distribution, StandardNormal};

use crate::gpu::{self, GpuBuffer, GpuContext};

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

    /// Get the underlying CPU data slice.
    /// Panics if tensor is on GPU — call `to_cpu()` first.
    fn cpu_data(&self) -> &[f64] {
        match self.data.as_ref() {
            TensorStorage::Cpu(v) => v,
            TensorStorage::Gpu(_) => panic!("tensor is on GPU, call to_cpu() first"),
        }
    }

    /// Transfer tensor data from GPU to CPU.
    /// If already on CPU, returns self (no-op).
    pub fn to_cpu(&self) -> CrowTensor {
        match self.data.as_ref() {
            TensorStorage::Cpu(_) => self.clone(),
            TensorStorage::Gpu(gpu_buf) => {
                let ctx = crate::gpu::GpuContext::get()
                    .expect("GPU context lost");
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
    /// If already on GPU, returns self (no-op).
    pub fn to_gpu(&self) -> CrowTensor {
        match self.data.as_ref() {
            TensorStorage::Gpu(_) => self.clone(),
            TensorStorage::Cpu(_) => {
                let ctx = crate::gpu::GpuContext::get()
                    .expect("no GPU available");
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
    fn gpu_buffer(&self) -> Option<&GpuBuffer> {
        match self.data.as_ref() {
            TensorStorage::Gpu(buf) => Some(buf),
            _ => None,
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

    /// Create a diagonal matrix from a 1D tensor, or extract the diagonal of a 2D tensor.
    pub fn diag(tensor: &CrowTensor) -> Self {
        if tensor.ndim() == 1 {
            // 1D → diagonal matrix
            let n = tensor.len();
            let mut data = vec![0.0; n * n];
            for i in 0..n {
                data[i * n + i] = tensor.get_flat(i);
            }
            Self::from_vec(data, vec![n, n])
        } else if tensor.ndim() == 2 {
            // 2D → extract diagonal
            let n = tensor.shape[0].min(tensor.shape[1]);
            let data: Vec<f64> = (0..n)
                .map(|i| {
                    let physical = tensor.offset + i * tensor.strides[0] + i * tensor.strides[1];
                    tensor.cpu_data()[physical]
                })
                .collect();
            Self::from_vec(data, vec![n])
        } else {
            panic!("diag: expected 1D or 2D tensor, got {}D", tensor.ndim());
        }
    }

    /// Create a tensor with uniform random values in [0, 1).
    pub fn rand(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        let mut rng = rand::rng();
        let data: Vec<f64> = (0..n).map(|_| rng.random::<f64>()).collect();
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with standard normal random values (mean=0, std=1).
    pub fn randn(shape: Vec<usize>) -> Self {
        let n = Self::numel(&shape);
        let mut rng = rand::rng();
        let data: Vec<f64> = (0..n).map(|_| StandardNormal.sample(&mut rng)).collect();
        Self::from_vec(data, shape).maybe_to_gpu()
    }

    /// Create a tensor with uniform random values in [low, high).
    pub fn rand_range(shape: Vec<usize>, low: f64, high: f64) -> Self {
        let n = Self::numel(&shape);
        let mut rng = rand::rng();
        let data: Vec<f64> = (0..n)
            .map(|_| rng.random::<f64>() * (high - low) + low)
            .collect();
        Self::from_vec(data, shape).maybe_to_gpu()
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
            let a_val = self.cpu_data()[self.offset + a_idx];
            let b_val = other.cpu_data()[other.offset + b_idx];
            data.push(f(a_val, b_val));
        }
        Self::from_vec(data, out_shape)
    }

    /// Compute broadcast-compatible output shape.
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
    fn gpu_binary_or_cpu(&self, other: &CrowTensor, op: &str, cpu_fn: impl Fn(f64, f64) -> f64) -> CrowTensor {
        if let (Some(a_buf), Some(b_buf)) = (self.gpu_buffer(), other.gpu_buffer()) {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_elementwise_binary(ctx, a_buf, b_buf, op);
                return self.from_gpu_buffer(out);
            }
        }
        self.zip_with(other, cpu_fn)
    }

    /// GPU-dispatched unary op, or CPU fallback.
    fn gpu_unary_or_cpu(&self, op: &str, cpu_fn: impl Fn(f64) -> f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_elementwise_unary(ctx, buf, op);
                return self.from_gpu_buffer(out);
            }
        }
        self.map(cpu_fn)
    }

    pub fn add(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, "add", |a, b| a + b)
    }

    pub fn sub(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, "sub", |a, b| a - b)
    }

    pub fn mul(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, "mul", |a, b| a * b)
    }

    pub fn div(&self, other: &CrowTensor) -> CrowTensor {
        self.gpu_binary_or_cpu(other, "div", |a, b| a / b)
    }

    pub fn neg(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("neg", |x| -x)
    }

    pub fn abs(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("abs", |x| x.abs())
    }

    pub fn exp(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("exp", |x| x.exp())
    }

    pub fn log(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("log", |x| x.ln())
    }

    pub fn sqrt(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("sqrt", |x| x.sqrt())
    }

    pub fn sin(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("sin", |x| x.sin())
    }

    pub fn cos(&self) -> CrowTensor {
        self.gpu_unary_or_cpu("cos", |x| x.cos())
    }

    pub fn pow(&self, exponent: &CrowTensor) -> CrowTensor {
        // pow not available as GPU shader yet — fall back to CPU
        self.zip_with(exponent, |a, b| a.powf(b))
    }

    /// Add a scalar to every element.
    pub fn add_scalar(&self, s: f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_scalar_op(ctx, buf, s as f32, "add");
                return self.from_gpu_buffer(out);
            }
        }
        self.map(|x| x + s)
    }

    /// Multiply every element by a scalar.
    pub fn mul_scalar(&self, s: f64) -> CrowTensor {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                let out = gpu::gpu_scalar_op(ctx, buf, s as f32, "mul");
                return self.from_gpu_buffer(out);
            }
        }
        self.map(|x| x * s)
    }
}


// ── Reductions ──────────────────────────────────────────────────────────

impl CrowTensor {
    /// Sum all elements.
    pub fn sum(&self) -> f64 {
        if let Some(buf) = self.gpu_buffer() {
            if let Some(ctx) = GpuContext::get() {
                return gpu::gpu_sum(ctx, buf);
            }
        }
        self.to_vec().iter().sum()
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
        self.to_vec()
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Minimum element.
    pub fn min(&self) -> f64 {
        self.to_vec()
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
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
}

// ── Display ─────────────────────────────────────────────────────────────

impl std::fmt::Display for CrowTensor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.to_vec();
        if self.ndim() == 0 {
            // Scalar
            return write!(f, "{}", data[0]);
        }
        if self.ndim() == 1 {
            write!(f, "[")?;
            for (i, v) in data.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{v}")?;
            }
            return write!(f, "]");
        }
        // 2D+ — show as nested brackets
        write!(f, "tensor({:?}, shape={:?})", data, self.shape)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

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
        let d = CrowTensor::diag(&v);
        assert_eq!(d.shape(), &[3, 3]);
        assert_eq!(
            d.to_vec(),
            vec![1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0]
        );
    }

    #[test]
    fn test_diag_from_2d() {
        let m = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let d = CrowTensor::diag(&m);
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
        let t = CrowTensor::randn(vec![1000]);
        assert_eq!(t.shape(), &[1000]);
        // Mean should be near 0
        let mean = t.mean();
        assert!(mean.abs() < 0.2, "randn mean too far from 0: {mean}");
    }

    #[test]
    fn test_rand_range() {
        let t = CrowTensor::rand_range(vec![100], 5.0, 10.0);
        for v in t.to_vec() {
            assert!(v >= 5.0 && v < 10.0);
        }
    }

    #[test]
    fn test_rand_2d_shape() {
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
}