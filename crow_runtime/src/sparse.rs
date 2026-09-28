/// Sparse array support for Crow, backed by sprs (CSR format).

use crate::tensor::CrowTensor;
use sprs::{CsMat, TriMat};

/// A sparse matrix stored in CSR (Compressed Sparse Row) format.
#[derive(Debug, Clone)]
pub struct CrowSparse {
    mat: CsMat<f64>,
}

impl CrowSparse {
    /// Create a sparse matrix from a dense CrowTensor (2D).
    pub fn from_dense(tensor: &CrowTensor) -> Self {
        let shape = tensor.shape();
        assert!(shape.len() == 2, "sparse: expected 2D tensor, got {}D", shape.len());
        let rows = shape[0];
        let cols = shape[1];
        let data = tensor.to_vec();
        let mut tri = TriMat::new((rows, cols));
        for r in 0..rows {
            for c in 0..cols {
                let v = data[r * cols + c];
                if v != 0.0 {
                    tri.add_triplet(r, c, v);
                }
            }
        }
        CrowSparse { mat: tri.to_csr() }
    }

    /// Create a sparse matrix from COO triplets (row_indices, col_indices, values, nrows, ncols).
    pub fn from_coo(rows: &[usize], cols: &[usize], vals: &[f64], nrows: usize, ncols: usize) -> Self {
        assert!(rows.len() == cols.len() && cols.len() == vals.len(),
            "sparse COO: rows, cols, vals must have same length");
        let mut tri = TriMat::new((nrows, ncols));
        for i in 0..rows.len() {
            tri.add_triplet(rows[i], cols[i], vals[i]);
        }
        CrowSparse { mat: tri.to_csr() }
    }

    /// Convert sparse matrix to dense CrowTensor.
    pub fn to_dense(&self) -> CrowTensor {
        let (rows, cols) = self.mat.shape();
        let mut data = vec![0.0; rows * cols];
        for (val, (r, c)) in self.mat.iter() {
            data[r * cols + c] = *val;
        }
        CrowTensor::from_vec(data, vec![rows, cols])
    }

    /// Sparse matrix × a dense vector or matrix; shapes that do not meet
    /// are the reason (they panicked both modes).
    pub fn matmul_dense(&self, other: &CrowTensor) -> Result<CrowTensor, String> {
        let other_shape = other.shape();
        let (srows, scols) = self.mat.shape();

        if other_shape.len() == 1 {
            // Sparse × vector
            if scols != other_shape[0] {
                return Err(format!("sparse matmul: incompatible shapes {}×{} and {}", srows, scols, other_shape[0]));
            }
            let v = other.to_vec();
            let mut result = vec![0.0; srows];
            for (row_idx, row) in self.mat.outer_iterator().enumerate() {
                let mut sum = 0.0;
                for (col_idx, val) in row.iter() {
                    sum += val * v[col_idx];
                }
                result[row_idx] = sum;
            }
            Ok(CrowTensor::from_vec(result, vec![srows]))
        } else if other_shape.len() == 2 {
            // Sparse × dense matrix
            let bcols = other_shape[1];
            if scols != other_shape[0] {
                return Err(format!("sparse matmul: incompatible shapes {}×{} and {}×{}", srows, scols, other_shape[0], bcols));
            }
            let bdata = other.to_vec();
            let mut result = vec![0.0; srows * bcols];
            for (row_idx, row) in self.mat.outer_iterator().enumerate() {
                for (col_idx, val) in row.iter() {
                    for bc in 0..bcols {
                        result[row_idx * bcols + bc] += val * bdata[col_idx * bcols + bc];
                    }
                }
            }
            Ok(CrowTensor::from_vec(result, vec![srows, bcols]))
        } else {
            Err(format!("sparse matmul: expected a vector or a matrix, got a {}D tensor", other_shape.len()))
        }
    }

    /// Sparse + Sparse addition.
    pub fn add(&self, other: &CrowSparse) -> CrowSparse {
        let result = &self.mat + &other.mat;
        CrowSparse { mat: result }
    }

    /// Sparse solve: self * x = b → x (converts to dense, uses faer).
    pub fn solve(&self, b: &CrowTensor) -> CrowTensor {
        let dense = self.to_dense();
        // Reshape 1D vector to column vector for faer
        let b_2d = if b.shape().len() == 1 {
            let n = b.shape()[0];
            CrowTensor::from_vec(b.to_vec(), vec![n, 1])
        } else {
            b.clone()
        };
        dense.solve(&b_2d)
    }

    /// Number of stored (non-zero) entries.
    pub fn nnz(&self) -> usize {
        self.mat.nnz()
    }

    /// Shape as (rows, cols).
    pub fn shape(&self) -> (usize, usize) {
        self.mat.shape()
    }

    /// Display format: shows shape and nnz.
    pub fn display(&self) -> String {
        let (r, c) = self.mat.shape();
        format!("sparse[{}x{}, nnz={}]", r, c, self.mat.nnz())
    }
}

impl std::fmt::Display for CrowSparse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_dense_and_back() {
        let t = CrowTensor::from_vec(vec![1.0, 0.0, 0.0, 2.0, 0.0, 3.0, 0.0, 0.0, 4.0], vec![3, 3]);
        let s = CrowSparse::from_dense(&t);
        assert_eq!(s.nnz(), 4);
        assert_eq!(s.shape(), (3, 3));
        let d = s.to_dense();
        assert_eq!(d.to_vec(), vec![1.0, 0.0, 0.0, 2.0, 0.0, 3.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn test_from_coo() {
        let s = CrowSparse::from_coo(
            &[0, 1, 2], &[0, 1, 2], &[5.0, 6.0, 7.0], 3, 3
        );
        assert_eq!(s.nnz(), 3);
        let d = s.to_dense();
        assert_eq!(d.to_vec(), vec![5.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 7.0]);
    }

    #[test]
    fn test_matmul_vector() {
        // [[1,0,2],[0,3,0],[4,0,5]] * [1,2,3]
        let s = CrowSparse::from_coo(
            &[0, 0, 1, 2, 2], &[0, 2, 1, 0, 2],
            &[1.0, 2.0, 3.0, 4.0, 5.0], 3, 3
        );
        let v = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let r = s.matmul_dense(&v).expect("the shapes meet");
        assert_eq!(r.to_vec(), vec![7.0, 6.0, 19.0]);
    }

    #[test]
    fn test_matmul_matrix() {
        // identity sparse * dense matrix
        let s = CrowSparse::from_coo(&[0, 1], &[0, 1], &[1.0, 1.0], 2, 2);
        let m = CrowTensor::from_vec(vec![3.0, 4.0, 5.0, 6.0], vec![2, 2]);
        let r = s.matmul_dense(&m).expect("the shapes meet");
        assert_eq!(r.to_vec(), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_sparse_add() {
        let a = CrowSparse::from_coo(&[0, 1], &[0, 1], &[1.0, 2.0], 2, 2);
        let b = CrowSparse::from_coo(&[0, 1], &[1, 0], &[3.0, 4.0], 2, 2);
        let c = a.add(&b);
        let d = c.to_dense();
        assert_eq!(d.to_vec(), vec![1.0, 3.0, 4.0, 2.0]);
    }

    #[test]
    fn test_sparse_solve() {
        // [[2,0],[0,3]] * x = [4,9] => x = [2,3]
        let s = CrowSparse::from_coo(&[0, 1], &[0, 1], &[2.0, 3.0], 2, 2);
        let b = CrowTensor::from_vec(vec![4.0, 9.0], vec![2, 1]);
        let x = s.solve(&b);
        let v = x.to_vec();
        assert!((v[0] - 2.0).abs() < 1e-10);
        assert!((v[1] - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_display() {
        let s = CrowSparse::from_coo(&[0, 1, 2], &[0, 1, 2], &[1.0, 2.0, 3.0], 3, 3);
        assert_eq!(format!("{}", s), "sparse[3x3, nnz=3]");
    }

    #[test]
    fn test_all_zeros() {
        let t = CrowTensor::from_vec(vec![0.0, 0.0, 0.0, 0.0], vec![2, 2]);
        let s = CrowSparse::from_dense(&t);
        assert_eq!(s.nnz(), 0);
        let d = s.to_dense();
        assert_eq!(d.to_vec(), vec![0.0, 0.0, 0.0, 0.0]);
    }
}

