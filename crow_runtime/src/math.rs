//! The math module's builtins, once (docs/bug-classes.md §4). Each takes
//! what a Crow program hands it — sizes as ints, parameters as floats,
//! tensors — checks it, and answers the tensor or the reason. The
//! interpreter and the compiled program both call these functions through
//! the builtin table: a size the interpreter refused and the compiled
//! program cast to a huge `usize` (`zeros[-2]`: "capacity overflow") is one
//! check now. The tensor kernels below them assume what these checked.

use crate::stops;
use crate::tensor::CrowTensor;

/// The sizes a constructor was handed, each a count.
fn shape(what: &str, sizes: &[i64]) -> Result<Vec<usize>, String> {
    sizes.iter().map(|&n| stops::size(what, n)).collect()
}

/// A sample's shape: its sizes, or one value when none are given.
fn sample_shape(what: &str, sizes: &[i64]) -> Result<Vec<usize>, String> {
    let s = shape(what, sizes)?;
    Ok(if s.is_empty() { vec![1] } else { s })
}

/// `zeros[d1, d2, …]`.
pub fn zeros(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::zeros(shape("zeros", sizes)?))
}

/// `ones[d1, d2, …]`.
pub fn ones(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::ones(shape("ones", sizes)?))
}

/// `randn[d1, d2, …]`: standard normal draws.
pub fn randn(sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::randn(shape("randn", sizes)?))
}

/// `full[d1, d2, …, value]`.
pub fn full(sizes: &[i64], value: f64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::full(shape("full", sizes)?, value))
}

/// `eye[n]`: the n×n identity.
pub fn eye(n: i64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::eye(stops::size("eye", n)?))
}

/// `linspace[start, stop, n]`: n evenly spaced values, both ends included.
pub fn linspace(start: f64, stop: f64, n: i64) -> Result<CrowTensor, String> {
    Ok(CrowTensor::linspace(start, stop, stops::size("linspace", n)?))
}

/// `arange[start, stop, step]`: from start toward stop, stop excluded.
pub fn arange(start: f64, stop: f64, step: f64) -> Result<CrowTensor, String> {
    if step == 0.0 {
        return Err("arange: step cannot be zero".to_string());
    }
    Ok(CrowTensor::arange(start, stop, step))
}

/// `diag[t]`: a vector's diagonal matrix, or a matrix's diagonal.
pub fn diag(t: &CrowTensor) -> Result<CrowTensor, String> {
    CrowTensor::diag(t)
}

/// `normal[mean, std, d1, …]`.
pub fn normal(mean: f64, std: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    if !(std >= 0.0) {
        return Err(format!("normal: std must be 0 or more (got {})", std));
    }
    Ok(CrowTensor::normal(sample_shape("normal", sizes)?, mean, std))
}

/// `uniform[low, high, d1, …]`.
pub fn uniform(low: f64, high: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    Ok(CrowTensor::rand_range(sample_shape("uniform", sizes)?, low, high))
}

/// `poisson[lambda, d1, …]`.
pub fn poisson(lambda: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::poisson(sample_shape("poisson", sizes)?, lambda)
}

/// `bernoulli[p, d1, …]`.
pub fn bernoulli(p: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::bernoulli(sample_shape("bernoulli", sizes)?, p)
}

/// `gamma_dist[alpha, beta, d1, …]`.
pub fn gamma_dist(alpha: f64, beta: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::gamma_dist(sample_shape("gamma_dist", sizes)?, alpha, beta)
}

/// `beta_dist[alpha, beta, d1, …]`.
pub fn beta_dist(alpha: f64, beta: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::beta_dist(sample_shape("beta_dist", sizes)?, alpha, beta)
}

/// `exponential[lambda, d1, …]`.
pub fn exponential(lambda: f64, sizes: &[i64]) -> Result<CrowTensor, String> {
    CrowTensor::exponential(sample_shape("exponential", sizes)?, lambda)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_checked_once_for_both_modes() {
        assert_eq!(zeros(&[2, 3]).map(|t| t.shape().to_vec()), Ok(vec![2, 3]));
        assert_eq!(zeros(&[-2]).map(|_| ()), Err("zeros: a size cannot be negative (got -2)".to_string()));
        assert_eq!(eye(-1).map(|_| ()), Err("eye: a size cannot be negative (got -1)".to_string()));
        assert_eq!(full(&[2], 7.0).map(|t| t.to_vec()), Ok(vec![7.0, 7.0]));
        assert_eq!(arange(0.0, 1.0, 0.0).map(|_| ()), Err("arange: step cannot be zero".to_string()));
        assert_eq!(linspace(0.0, 1.0, 3).map(|t| t.to_vec()), Ok(vec![0.0, 0.5, 1.0]));
    }

    #[test]
    fn a_distributions_parameters_are_checked() {
        assert_eq!(poisson(-1.0, &[]).map(|_| ()), Err("poisson: lambda must be > 0".to_string()));
        assert_eq!(bernoulli(2.0, &[3]).map(|_| ()), Err("bernoulli: p must be in [0, 1]".to_string()));
        assert_eq!(exponential(-1.0, &[]).map(|_| ()), Err("exponential: lambda must be 0 or more".to_string()));
        assert_eq!(normal(0.0, -1.0, &[]).map(|_| ()), Err("normal: std must be 0 or more (got -1)".to_string()));
        assert_eq!(poisson(2.0, &[]).map(|t| t.shape().to_vec()), Ok(vec![1]));
        assert_eq!(uniform(0.0, 1.0, &[4]).map(|t| t.shape().to_vec()), Ok(vec![4]));
    }

    #[test]
    fn a_diagonal_needs_one_or_two_dimensions() {
        assert!(diag(&CrowTensor::zeros(vec![2, 2, 2])).is_err());
        assert_eq!(diag(&CrowTensor::from_vec(vec![1.0, 2.0], vec![2])).map(|t| t.shape().to_vec()), Ok(vec![2, 2]));
    }
}
