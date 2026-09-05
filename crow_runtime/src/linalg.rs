//! Linear algebra operations on CrowTensor, powered by faer.
//!
//! All operations require 2D tensors (matrices) and work on CPU data.
//! GPU tensors are automatically moved to CPU before computation.

use faer::prelude::*;
use faer::linalg::solvers::DenseSolveCore;
use faer::{Mat, MatRef, Side, Unbind};


use crate::tensor::CrowTensor;

// ── Conversion helpers ──────────────────────────────────────────────────

/// Convert a 2D CrowTensor (row-major) to a faer Mat<f64> (column-major).
fn tensor_to_mat(t: &CrowTensor) -> Mat<f64> {
    let t = t.to_contiguous();
    let shape = t.shape();
    assert_eq!(shape.len(), 2, "linalg: expected 2D tensor, got {}D", shape.len());
    let (rows, cols) = (shape[0], shape[1]);
    let data = t.to_vec();
    Mat::from_fn(rows, cols, |i, j| data[i * cols + j])
}

/// Convert a faer MatRef<f64> to a CrowTensor.
fn matref_to_tensor(m: MatRef<'_, f64>) -> CrowTensor {
    let rows = m.nrows();
    let cols = m.ncols();
    let mut data = Vec::with_capacity(rows * cols);
    for i in 0..rows {
        for j in 0..cols {
            data.push(m[(i, j)]);
        }
    }
    CrowTensor::from_vec(data, vec![rows, cols])
}

/// Convert an owned faer Mat<f64> to a CrowTensor.
fn mat_to_tensor(m: &Mat<f64>) -> CrowTensor {
    matref_to_tensor(m.as_ref())
}

// ── Linear algebra operations ───────────────────────────────────────────

impl CrowTensor {
    /// Matrix multiplication: self (m×k) * other (k×n) → (m×n).
    /// Dispatches to GPU if both tensors are on GPU, otherwise uses CPU (faer).
    pub fn matmul(&self, other: &CrowTensor) -> CrowTensor {
        let shape_a = self.shape();
        let shape_b = other.shape();
        assert_eq!(shape_a.len(), 2, "linalg: expected 2D tensor, got {}D", shape_a.len());
        assert_eq!(shape_b.len(), 2, "linalg: expected 2D tensor, got {}D", shape_b.len());
        assert_eq!(
            shape_a[1], shape_b[0],
            "matmul: incompatible shapes [{},{}] × [{},{}]",
            shape_a[0], shape_a[1], shape_b[0], shape_b[1]
        );

        // GPU path: if both tensors are on GPU, use the GPU matmul shader
        if let (Some(a_buf), Some(b_buf)) = (self.gpu_buffer(), other.gpu_buffer()) {
            if let Some(ctx) = crate::gpu::GpuContext::get() {
                let m = shape_a[0] as u32;
                let k = shape_a[1] as u32;
                let n = shape_b[1] as u32;
                let out_buf = crate::gpu::gpu_matmul(ctx, a_buf, b_buf, m, k, n);
                return CrowTensor::from_gpu_with_shape(out_buf, vec![m as usize, n as usize]);
            }
        }

        // CPU path: use faer
        let a = tensor_to_mat(self);
        let b = tensor_to_mat(other);
        let c = &a * &b;
        mat_to_tensor(&c)
    }

    /// Solve the linear system Ax = b where self is A.
    /// A must be square. b can be a column vector (n×1) or a matrix (n×m).
    pub fn solve(&self, b: &CrowTensor) -> CrowTensor {
        let a = tensor_to_mat(self);
        // A plain vector right-hand side (`solve[A, [1.0, 2.0]]`) is the
        // common call; treat it as one column and hand back a vector.
        let b_is_vec = b.shape().len() == 1;
        let b2 = if b_is_vec { b.reshape(vec![b.shape()[0], 1]) } else { b.clone() };
        let b_mat = tensor_to_mat(&b2);
        let lu = a.partial_piv_lu();
        let x = lu.solve(&b_mat);
        let out = mat_to_tensor(&x);
        if b_is_vec { out.reshape(vec![out.shape()[0]]) } else { out }
    }

    /// Matrix inverse. Self must be square and non-singular.
    pub fn inv(&self) -> CrowTensor {
        let a = tensor_to_mat(self);
        let lu = a.partial_piv_lu();
        let inv = lu.inverse();
        mat_to_tensor(&inv)
    }

    /// Determinant of a square matrix.
    pub fn det(&self) -> f64 {
        let a = tensor_to_mat(self);
        let n = a.nrows();
        assert_eq!(a.nrows(), a.ncols(), "det: matrix must be square");
        let lu = a.partial_piv_lu();
        // det(A) = sign(P) * product(diag(U)), since L has unit diagonal
        let u = lu.U();
        let mut det = 1.0;
        for i in 0..n {
            det *= u[(i, i)];
        }
        // Count transpositions in permutation to determine sign
        let perm = lu.P();
        let (fwd, _) = perm.arrays();
        let mut visited = vec![false; n];
        let mut sign = 1.0;
        for i in 0..n {
            if !visited[i] {
                visited[i] = true;
                let mut j = fwd[i].unbound();
                while j != i {
                    sign *= -1.0;
                    visited[j] = true;
                    j = fwd[j].unbound();
                }
            }
        }
        det * sign
    }

    /// Singular Value Decomposition: self = U * diag(S) * Vt.
    /// Returns (U, S, Vt) where S is a 1D tensor of singular values.
    pub fn svd_decomp(&self) -> (CrowTensor, CrowTensor, CrowTensor) {
        let a = tensor_to_mat(self);
        let svd = a.thin_svd().expect("svd: decomposition failed");
        let u = matref_to_tensor(svd.U());
        // S is diagonal — extract as 1D
        let s_diag = svd.S();
        let k = s_diag.dim();
        let s_col = s_diag.column_vector();
        let s_data: Vec<f64> = (0..k).map(|i| s_col[i]).collect();
        let s = CrowTensor::from_vec(s_data, vec![k]);
        let vt = matref_to_tensor(svd.V()).transpose();
        (u, s, vt)
    }

    /// QR decomposition: self = Q * R.
    /// Returns (Q, R).
    pub fn qr_decomp(&self) -> (CrowTensor, CrowTensor) {
        let a = tensor_to_mat(self);
        let qr = a.qr();
        // Q is computed from the thin QR
        let q_mat = qr.compute_Q();
        let q = mat_to_tensor(&q_mat);
        let r = matref_to_tensor(qr.R());
        (q, r)
    }

    /// Cholesky decomposition: self = L * L^T.
    /// Self must be symmetric positive definite. Returns L (lower triangular).
    pub fn cholesky_decomp(&self) -> CrowTensor {
        let a = tensor_to_mat(self);
        let llt = a.llt(Side::Lower).expect("cholesky: matrix is not positive definite");
        matref_to_tensor(llt.L())
    }

    /// Vector/matrix norm.
    /// p = 2.0 → L2 (Euclidean) norm (default).
    /// p = 1.0 → L1 norm (sum of absolute values).
    /// p = f64::INFINITY → L∞ norm (max absolute value).
    /// Any other p → Lp norm (sum |x_i|^p)^(1/p).
    pub fn norm(&self, p: f64) -> f64 {
        let data = self.to_vec();
        if p == 2.0 {
            data.iter().map(|x| x * x).sum::<f64>().sqrt()
        } else if p == 1.0 {
            data.iter().map(|x| x.abs()).sum::<f64>()
        } else if p.is_infinite() && p > 0.0 {
            data.iter().map(|x| x.abs()).fold(0.0_f64, f64::max)
        } else {
            data.iter().map(|x| x.abs().powf(p)).sum::<f64>().powf(1.0 / p)
        }
    }

    /// Vector dot product (inner product). Both tensors must be 1D with same length.
    /// Returns a scalar (sum of element-wise products).
    pub fn dot(&self, other: &CrowTensor) -> f64 {
        let a = self.to_vec();
        let b = other.to_vec();
        assert_eq!(a.len(), b.len(), "dot: vectors must have same length ({} vs {})", a.len(), b.len());
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    /// Moore-Penrose pseudo-inverse via SVD: A⁺ = V * diag(1/σᵢ) * Uᵀ.
    /// Works for any m×n matrix.
    pub fn pinv(&self) -> CrowTensor {
        let (u, s, vt) = self.svd_decomp();
        let s_data = s.to_vec();
        let tol = s_data[0] * (self.shape()[0].max(self.shape()[1]) as f64) * f64::EPSILON;
        // S_inv: k×k diagonal with 1/σᵢ (or 0 if below tolerance)
        let k = s_data.len();
        let mut s_inv_data = vec![0.0; k * k];
        for i in 0..k {
            if s_data[i] > tol {
                s_inv_data[i * k + i] = 1.0 / s_data[i];
            }
        }
        let s_inv = CrowTensor::from_vec(s_inv_data, vec![k, k]);
        // pinv = Vᵀᵀ * S_inv * Uᵀ = V * S_inv * Uᵀ
        let v = vt.transpose();
        let ut = u.transpose();
        v.matmul(&s_inv).matmul(&ut)
    }

    /// Least-squares solve: find x that minimizes ||Ax - b||².
    /// Uses pseudo-inverse: x = pinv(A) * b.
    pub fn lstsq(&self, b: &CrowTensor) -> CrowTensor {
        // a plain vector right-hand side is one column (same as solve)
        let b_is_vec = b.shape().len() == 1;
        let b2 = if b_is_vec { b.reshape(vec![b.shape()[0], 1]) } else { b.clone() };
        let out = self.pinv().matmul(&b2);
        if b_is_vec { out.reshape(vec![out.shape()[0]]) } else { out }
    }

    /// Extract upper triangular part of a matrix. Elements below the main diagonal are set to 0.
    pub fn triu(&self) -> CrowTensor {
        let shape = self.shape();
        assert_eq!(shape.len(), 2, "triu: expected 2D tensor");
        let (rows, cols) = (shape[0], shape[1]);
        let data = self.to_vec();
        let mut result = vec![0.0; rows * cols];
        for i in 0..rows {
            for j in i..cols {
                result[i * cols + j] = data[i * cols + j];
            }
        }
        CrowTensor::from_vec(result, vec![rows, cols])
    }

    /// Extract lower triangular part of a matrix. Elements above the main diagonal are set to 0.
    pub fn tril(&self) -> CrowTensor {
        let shape = self.shape();
        assert_eq!(shape.len(), 2, "tril: expected 2D tensor");
        let (rows, cols) = (shape[0], shape[1]);
        let data = self.to_vec();
        let mut result = vec![0.0; rows * cols];
        for i in 0..rows {
            for j in 0..=i.min(cols - 1) {
                result[i * cols + j] = data[i * cols + j];
            }
        }
        CrowTensor::from_vec(result, vec![rows, cols])
    }

    /// Matrix rank via SVD. Counts singular values above a tolerance threshold.
    pub fn rank(&self) -> usize {
        let (_, s, _) = self.svd_decomp();
        let s_data = s.to_vec();
        if s_data.is_empty() {
            return 0;
        }
        let tol = s_data[0] * (self.shape()[0].max(self.shape()[1]) as f64) * f64::EPSILON;
        s_data.iter().filter(|&&v| v > tol).count()
    }

    /// Condition number of a matrix (ratio of largest to smallest singular value).
    /// Uses the 2-norm by default. Returns f64::INFINITY for singular matrices.
    pub fn cond(&self) -> f64 {
        let (_, s, _) = self.svd_decomp();
        let s_data = s.to_vec();
        if s_data.is_empty() {
            return f64::NAN;
        }
        let s_max = s_data[0];
        let s_min = s_data[s_data.len() - 1];
        if s_min == 0.0 {
            f64::INFINITY
        } else {
            s_max / s_min
        }
    }

    /// Eigendecomposition of a square matrix.
    /// Returns (eigenvalues_real, eigenvalues_imag).
    /// For symmetric matrices, imaginary parts will be zero.
    pub fn eigen_decomp(&self) -> (CrowTensor, CrowTensor) {
        let a = tensor_to_mat(self);
        let n = a.nrows();
        assert_eq!(a.nrows(), a.ncols(), "eigen: matrix must be square");
        let eigen = a.eigen().expect("eigen: decomposition failed");
        let s = eigen.S();
        let mut real_parts = Vec::with_capacity(n);
        let mut imag_parts = Vec::with_capacity(n);
        for i in 0..n {
            let s_col = s.column_vector();
            let c = s_col[i];
            real_parts.push(c.re);
            imag_parts.push(c.im);
        }
        let eigenvalues_real = CrowTensor::from_vec(real_parts, vec![n]);
        let eigenvalues_imag = CrowTensor::from_vec(imag_parts, vec![n]);
        (eigenvalues_real, eigenvalues_imag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "expected {} ≈ {}, diff = {}", a, b, (a - b).abs());
    }

    fn tensor_approx_eq(a: &CrowTensor, b: &CrowTensor, tol: f64) {
        assert_eq!(a.shape(), b.shape(), "shape mismatch");
        let av = a.to_vec();
        let bv = b.to_vec();
        for (i, (x, y)) in av.iter().zip(bv.iter()).enumerate() {
            assert!((x - y).abs() < tol, "element {} differs: {} vs {}", i, x, y);
        }
    }

    #[test]
    fn test_matmul_2x2() {
        // [[1,2],[3,4]] * [[5,6],[7,8]] = [[19,22],[43,50]]
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let b = CrowTensor::from_vec(vec![5.0, 6.0, 7.0, 8.0], vec![2, 2]);
        let c = a.matmul(&b);
        assert_eq!(c.shape(), &[2, 2]);
        assert_eq!(c.to_vec(), vec![19.0, 22.0, 43.0, 50.0]);
    }

    #[test]
    fn test_matmul_identity() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let eye = CrowTensor::eye(2);
        let c = a.matmul(&eye);
        tensor_approx_eq(&c, &a, 1e-12);
    }

    #[test]
    #[should_panic(expected = "matmul: incompatible shapes")]
    fn test_matmul_incompatible() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let b = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        a.matmul(&b);
    }

    #[test]
    fn test_solve_2x2() {
        // A = [[2,1],[5,3]], b = [[4],[7]]  =>  x = [[5],[-6]]
        let a = CrowTensor::from_vec(vec![2.0, 1.0, 5.0, 3.0], vec![2, 2]);
        let b = CrowTensor::from_vec(vec![4.0, 7.0], vec![2, 1]);
        let x = a.solve(&b);
        approx_eq(x.to_vec()[0], 5.0, 1e-10);
        approx_eq(x.to_vec()[1], -6.0, 1e-10);
    }

    #[test]
    fn test_inv_2x2() {
        let a = CrowTensor::from_vec(vec![4.0, 7.0, 2.0, 6.0], vec![2, 2]);
        let a_inv = a.inv();
        let product = a.matmul(&a_inv);
        let eye = CrowTensor::eye(2);
        tensor_approx_eq(&product, &eye, 1e-10);
    }

    #[test]
    fn test_det_identity() {
        let eye = CrowTensor::eye(3);
        approx_eq(eye.det(), 1.0, 1e-12);
    }

    #[test]
    fn test_det_known() {
        // det([[1,2],[3,4]]) = 1*4 - 2*3 = -2
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        approx_eq(a.det(), -2.0, 1e-10);
    }

    #[test]
    fn test_det_singular() {
        // Singular matrix: rows are linearly dependent
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 2.0, 4.0], vec![2, 2]);
        approx_eq(a.det(), 0.0, 1e-10);
    }

    #[test]
    fn test_svd_reconstruct() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let (u, s, vt) = a.svd_decomp();
        // Reconstruct: U * diag(S) * Vt ≈ A
        let s_vec = s.to_vec();
        let k = s_vec.len();
        // Build diagonal matrix
        let mut s_diag = vec![0.0; u.shape()[1] * vt.shape()[0]];
        for i in 0..k {
            s_diag[i * vt.shape()[0] + i] = s_vec[i];
        }
        let s_mat = CrowTensor::from_vec(s_diag, vec![u.shape()[1], vt.shape()[0]]);
        let reconstructed = u.matmul(&s_mat).matmul(&vt);
        tensor_approx_eq(&reconstructed, &a, 1e-10);
    }

    #[test]
    fn test_qr_reconstruct() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let (q, r) = a.qr_decomp();
        let reconstructed = q.matmul(&r);
        tensor_approx_eq(&reconstructed, &a, 1e-10);
    }

    #[test]
    fn test_cholesky() {
        // Symmetric positive definite: [[4,2],[2,3]]
        let a = CrowTensor::from_vec(vec![4.0, 2.0, 2.0, 3.0], vec![2, 2]);
        let l = a.cholesky_decomp();
        let lt = l.transpose();
        let reconstructed = l.matmul(&lt);
        tensor_approx_eq(&reconstructed, &a, 1e-10);
    }

    #[test]
    fn test_eigen_symmetric() {
        // Symmetric: [[2,1],[1,2]] has eigenvalues 1 and 3
        let a = CrowTensor::from_vec(vec![2.0, 1.0, 1.0, 2.0], vec![2, 2]);
        let (real, imag) = a.eigen_decomp();
        let mut evals: Vec<f64> = real.to_vec();
        evals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        approx_eq(evals[0], 1.0, 1e-10);
        approx_eq(evals[1], 3.0, 1e-10);
        // Imaginary parts should be zero for symmetric
        for v in imag.to_vec() {
            approx_eq(v, 0.0, 1e-10);
        }
    }

    #[test]
    #[should_panic(expected = "linalg: expected 2D tensor")]
    fn test_matmul_1d_panics() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        a.matmul(&b);
    }

    #[test]
    fn test_det_3x3() {
        // [[6,1,1],[4,-2,5],[2,8,7]] det = -306
        let a = CrowTensor::from_vec(
            vec![6.0, 1.0, 1.0, 4.0, -2.0, 5.0, 2.0, 8.0, 7.0],
            vec![3, 3],
        );
        approx_eq(a.det(), -306.0, 1e-8);
    }

    // ===== Norm tests =====

    #[test]
    fn test_norm_l2() {
        // ||[3, 4]|| = 5
        let a = CrowTensor::from_vec(vec![3.0, 4.0], vec![2]);
        approx_eq(a.norm(2.0), 5.0, 1e-12);
    }

    #[test]
    fn test_norm_l1() {
        // L1([3, -4, 5]) = 12
        let a = CrowTensor::from_vec(vec![3.0, -4.0, 5.0], vec![3]);
        approx_eq(a.norm(1.0), 12.0, 1e-12);
    }

    #[test]
    fn test_norm_linf() {
        // L∞([-3, 7, -5]) = 7
        let a = CrowTensor::from_vec(vec![-3.0, 7.0, -5.0], vec![3]);
        approx_eq(a.norm(f64::INFINITY), 7.0, 1e-12);
    }

    #[test]
    fn test_norm_lp() {
        // L3([1, 2, 3]) = (1 + 8 + 27)^(1/3) = 36^(1/3)
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        approx_eq(a.norm(3.0), 36.0_f64.powf(1.0 / 3.0), 1e-12);
    }

    #[test]
    fn test_norm_l2_2d() {
        // Frobenius norm of [[1,2],[3,4]] = sqrt(1+4+9+16) = sqrt(30)
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        approx_eq(a.norm(2.0), 30.0_f64.sqrt(), 1e-12);
    }

    // ── dot ─────────────────────────────────────────────────────────────

    #[test]
    fn test_dot_basic() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![4.0, 5.0, 6.0], vec![3]);
        // 1*4 + 2*5 + 3*6 = 32
        approx_eq(a.dot(&b), 32.0, 1e-12);
    }

    #[test]
    fn test_dot_orthogonal() {
        let a = CrowTensor::from_vec(vec![1.0, 0.0], vec![2]);
        let b = CrowTensor::from_vec(vec![0.0, 1.0], vec![2]);
        approx_eq(a.dot(&b), 0.0, 1e-12);
    }

    #[test]
    #[should_panic(expected = "dot: vectors must have same length")]
    fn test_dot_length_mismatch() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0], vec![2]);
        let b = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        a.dot(&b);
    }

    // ── pinv ────────────────────────────────────────────────────────────

    #[test]
    fn test_pinv_square() {
        // pinv of invertible matrix ≈ inv
        let a = CrowTensor::from_vec(vec![4.0, 7.0, 2.0, 6.0], vec![2, 2]);
        let a_pinv = a.pinv();
        let a_inv = a.inv();
        tensor_approx_eq(&a_pinv, &a_inv, 1e-10);
    }

    #[test]
    fn test_pinv_rectangular() {
        // A * pinv(A) * A ≈ A for any matrix
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let ap = a.pinv();
        let result = a.matmul(&ap).matmul(&a);
        tensor_approx_eq(&result, &a, 1e-10);
    }

    // ── lstsq ───────────────────────────────────────────────────────────

    #[test]
    fn test_lstsq_exact() {
        // Exact system: [[1,0],[0,1]] * x = [[3],[5]] → x = [[3],[5]]
        let a = CrowTensor::eye(2);
        let b = CrowTensor::from_vec(vec![3.0, 5.0], vec![2, 1]);
        let x = a.lstsq(&b);
        tensor_approx_eq(&x, &b, 1e-10);
    }

    #[test]
    fn test_lstsq_overdetermined() {
        // y = 2x: points (1,2), (2,4), (3,6)
        // A = [[1,1],[2,1],[3,1]], b = [[2],[4],[6]]
        let a = CrowTensor::from_vec(vec![1.0, 1.0, 2.0, 1.0, 3.0, 1.0], vec![3, 2]);
        let b = CrowTensor::from_vec(vec![2.0, 4.0, 6.0], vec![3, 1]);
        let x = a.lstsq(&b);
        let xv = x.to_vec();
        approx_eq(xv[0], 2.0, 1e-10); // slope
        approx_eq(xv[1], 0.0, 1e-10); // intercept
    }

    // ── triu / tril ─────────────────────────────────────────────────────

    #[test]
    fn test_triu() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let u = a.triu();
        let expected = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 0.0, 5.0, 6.0, 0.0, 0.0, 9.0], vec![3, 3]);
        tensor_approx_eq(&u, &expected, 1e-12);
    }

    #[test]
    fn test_tril() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], vec![3, 3]);
        let l = a.tril();
        let expected = CrowTensor::from_vec(vec![1.0, 0.0, 0.0, 4.0, 5.0, 0.0, 7.0, 8.0, 9.0], vec![3, 3]);
        tensor_approx_eq(&l, &expected, 1e-12);
    }

    #[test]
    fn test_triu_tril_sum_equals_original_plus_diag() {
        // triu(A) + tril(A) = A + diag(A) (diagonal counted twice)
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let u = a.triu();
        let l = a.tril();
        let sum = u.add(&l);
        // A + diag = [[1,2],[3,4]] + [[1,0],[0,4]] = [[2,2],[3,8]]
        let expected = CrowTensor::from_vec(vec![2.0, 2.0, 3.0, 8.0], vec![2, 2]);
        tensor_approx_eq(&sum, &expected, 1e-12);
    }

    // ── rank ─────────────────────────────────────────────────────────────

    #[test]
    fn test_rank_full() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        assert_eq!(a.rank(), 2);
    }

    #[test]
    fn test_rank_deficient() {
        // Rank-1: rows are linearly dependent
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 2.0, 4.0], vec![2, 2]);
        assert_eq!(a.rank(), 1);
    }

    #[test]
    fn test_rank_identity() {
        let a = CrowTensor::eye(4);
        assert_eq!(a.rank(), 4);
    }

    #[test]
    fn test_rank_rectangular() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        assert_eq!(a.rank(), 2);
    }

    // ── cond ─────────────────────────────────────────────────────────────

    #[test]
    fn test_cond_identity() {
        let a = CrowTensor::eye(3);
        approx_eq(a.cond(), 1.0, 1e-10);
    }

    #[test]
    fn test_cond_singular() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 2.0, 4.0], vec![2, 2]);
        assert!(a.cond().is_infinite());
    }

    #[test]
    fn test_cond_well_conditioned() {
        // [[2,0],[0,1]] → σ_max=2, σ_min=1, cond=2
        let a = CrowTensor::from_vec(vec![2.0, 0.0, 0.0, 1.0], vec![2, 2]);
        approx_eq(a.cond(), 2.0, 1e-10);
    }

    #[test]
    fn test_triu_rectangular() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], vec![2, 3]);
        let u = a.triu();
        let expected = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 0.0, 5.0, 6.0], vec![2, 3]);
        tensor_approx_eq(&u, &expected, 1e-12);
    }
}
