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
    pub fn matmul(&self, other: &CrowTensor) -> CrowTensor {
        let a = tensor_to_mat(self);
        let b = tensor_to_mat(other);
        assert_eq!(
            a.ncols(), b.nrows(),
            "matmul: incompatible shapes [{},{}] × [{},{}]",
            a.nrows(), a.ncols(), b.nrows(), b.ncols()
        );
        let c = &a * &b;
        mat_to_tensor(&c)
    }

    /// Solve the linear system Ax = b where self is A.
    /// A must be square. b can be a column vector (n×1) or a matrix (n×m).
    pub fn solve(&self, b: &CrowTensor) -> CrowTensor {
        let a = tensor_to_mat(self);
        let b_mat = tensor_to_mat(b);
        let lu = a.partial_piv_lu();
        let x = lu.solve(&b_mat);
        mat_to_tensor(&x)
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
}
