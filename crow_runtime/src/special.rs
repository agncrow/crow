//! Special mathematical functions and complex number operations for CrowTensor.

use crate::tensor::CrowTensor;
use std::f64::consts::PI;

// ── Scalar special functions ───────────────────────────────────────────

/// Lanczos approximation for the gamma function.
fn gamma_scalar(x: f64) -> f64 {
    if x < 0.5 {
        // Reflection formula: Γ(1-z)Γ(z) = π/sin(πz)
        PI / (PI * x).sin() / gamma_scalar(1.0 - x)
    } else {
        let x = x - 1.0;
        // Lanczos coefficients (g=7, n=9)
        let p = [
            0.99999999999980993,
            676.5203681218851,
            -1259.1392167224028,
            771.32342877765313,
            -176.61502916214059,
            12.507343278686905,
            -0.13857109526572012,
            9.9843695780195716e-6,
            1.5056327351493116e-7,
        ];
        let mut sum = p[0];
        for (i, &coef) in p[1..].iter().enumerate() {
            sum += coef / (x + i as f64 + 1.0);
        }
        let t = x + 7.5; // g + 0.5
        (2.0 * PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * sum
    }
}

/// Log-gamma function for numerical stability in choose().
fn lgamma_scalar(x: f64) -> f64 {
    gamma_scalar(x).abs().ln()
}

/// The error function, to the last bit (libm's; an approximation good
/// to 1.5e-7 had served, and `gelu`, written over `erf` in Crow, showed it).
pub fn erf_scalar(x: f64) -> f64 {
    libm::erf(x)
}

/// Bessel function of the first kind J_n(x), using series expansion.
fn bessel_scalar(x: f64, n: i64) -> f64 {
    let n_abs = n.unsigned_abs() as usize;
    let mut sum = 0.0;
    for m in 0..50 {
        let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
        let denom = gamma_scalar((m + 1) as f64) * gamma_scalar((m + n_abs + 1) as f64);
        sum += sign / denom * (x / 2.0).powi((2 * m + n_abs) as i32);
    }
    if n < 0 && n_abs % 2 == 1 {
        -sum
    } else {
        sum
    }
}

// ── CrowTensor methods ─────────────────────────────────────────────────

impl CrowTensor {
    /// Element-wise gamma function.
    pub fn gamma(&self) -> CrowTensor {
        self.map(gamma_scalar)
    }

    /// Element-wise error function.
    pub fn erf(&self) -> CrowTensor {
        self.map(erf_scalar)
    }

    /// Bessel function J_n(x) element-wise. `n` is the integer order.
    pub fn bessel(&self, n: i64) -> CrowTensor {
        self.map(|x| bessel_scalar(x, n))
    }

    /// Element-wise factorial. Computes Γ(x+1) for each element.
    pub fn factorial(&self) -> CrowTensor {
        self.map(|x| gamma_scalar(x + 1.0))
    }

    /// Element-wise binomial coefficient C(n, k) = n! / (k! * (n-k)!).
    /// Uses log-gamma for numerical stability.
    pub fn choose(&self, k: &CrowTensor) -> CrowTensor {
        self.zip_with(k, |n, k| {
            if k < 0.0 || k > n || n < 0.0 {
                0.0
            } else {
                (lgamma_scalar(n + 1.0) - lgamma_scalar(k + 1.0) - lgamma_scalar(n - k + 1.0))
                    .exp()
                    .round()
            }
        })
    }

    // ── Complex number operations ──────────────────────────────────────
    // Complex numbers are represented as [real_tensor, imag_tensor] pairs.

    /// Create a complex tensor from real and imaginary scalar values.
    /// Returns (real_tensor, imag_tensor).
    pub fn complex(re: f64, im: f64) -> (CrowTensor, CrowTensor) {
        (
            CrowTensor::from_vec(vec![re], vec![1]),
            CrowTensor::from_vec(vec![im], vec![1]),
        )
    }

    /// Magnitude (absolute value) of a complex tensor: sqrt(re² + im²).
    pub fn magnitude(re: &CrowTensor, im: &CrowTensor) -> CrowTensor {
        re.zip_with(im, |r: f64, i: f64| (r * r + i * i).sqrt())
    }

    /// Phase (angle) of a complex tensor: atan2(im, re).
    pub fn phase(re: &CrowTensor, im: &CrowTensor) -> CrowTensor {
        im.zip_with(re, |i: f64, r: f64| i.atan2(r))
    }

    /// Complex conjugate: (re, -im).
    pub fn conjugate(re: &CrowTensor, im: &CrowTensor) -> (CrowTensor, CrowTensor) {
        (re.clone(), im.map(|x| -x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gamma_integers() {
        // Γ(n) = (n-1)! for positive integers
        let t = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0], vec![5]);
        let g = t.gamma();
        let v = g.to_vec();
        assert!((v[0] - 1.0).abs() < 1e-10, "Γ(1) = 1"); // 0!
        assert!((v[1] - 1.0).abs() < 1e-10, "Γ(2) = 1"); // 1!
        assert!((v[2] - 2.0).abs() < 1e-10, "Γ(3) = 2"); // 2!
        assert!((v[3] - 6.0).abs() < 1e-10, "Γ(4) = 6"); // 3!
        assert!((v[4] - 24.0).abs() < 1e-10, "Γ(5) = 24"); // 4!
    }

    #[test]
    fn test_gamma_half() {
        // Γ(0.5) = √π
        let t = CrowTensor::from_vec(vec![0.5], vec![1]);
        let g = t.gamma();
        assert!((g.to_vec()[0] - PI.sqrt()).abs() < 1e-10, "Γ(0.5) = √π");
    }

    #[test]
    fn test_erf_values() {
        let t = CrowTensor::from_vec(vec![0.0, 1.0, -1.0, 2.0], vec![4]);
        let e = t.erf();
        let v = e.to_vec();
        assert!((v[0]).abs() < 1e-6, "erf(0) = 0");
        assert!((v[1] - 0.8427007929).abs() < 1e-5, "erf(1) ≈ 0.8427");
        assert!((v[2] + 0.8427007929).abs() < 1e-5, "erf(-1) ≈ -0.8427");
        assert!((v[3] - 0.9953222650).abs() < 1e-5, "erf(2) ≈ 0.9953");
    }

    #[test]
    fn test_bessel_j0() {
        // J_0(0) = 1
        let t = CrowTensor::from_vec(vec![0.0], vec![1]);
        let b = t.bessel(0);
        assert!((b.to_vec()[0] - 1.0).abs() < 1e-10, "J_0(0) = 1");
    }

    #[test]
    fn test_bessel_j1() {
        // J_1(0) = 0
        let t = CrowTensor::from_vec(vec![0.0], vec![1]);
        let b = t.bessel(1);
        assert!((b.to_vec()[0]).abs() < 1e-10, "J_1(0) = 0");
    }

    #[test]
    fn test_factorial() {
        let t = CrowTensor::from_vec(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0], vec![6]);
        let f = t.factorial();
        let v = f.to_vec();
        assert!((v[0] - 1.0).abs() < 1e-10, "0! = 1");
        assert!((v[1] - 1.0).abs() < 1e-10, "1! = 1");
        assert!((v[2] - 2.0).abs() < 1e-10, "2! = 2");
        assert!((v[3] - 6.0).abs() < 1e-10, "3! = 6");
        assert!((v[4] - 24.0).abs() < 1e-10, "4! = 24");
        assert!((v[5] - 120.0).abs() < 1e-10, "5! = 120");
    }

    #[test]
    fn test_choose() {
        let n = CrowTensor::from_vec(vec![5.0, 5.0, 5.0, 10.0], vec![4]);
        let k = CrowTensor::from_vec(vec![0.0, 2.0, 5.0, 3.0], vec![4]);
        let c = n.choose(&k);
        let v = c.to_vec();
        assert!((v[0] - 1.0).abs() < 1e-8, "C(5,0) = 1");
        assert!((v[1] - 10.0).abs() < 1e-8, "C(5,2) = 10");
        assert!((v[2] - 1.0).abs() < 1e-8, "C(5,5) = 1");
        assert!((v[3] - 120.0).abs() < 1e-8, "C(10,3) = 120");
    }

    #[test]
    fn test_complex_create() {
        let result: (CrowTensor, CrowTensor) = CrowTensor::complex(3.0, 4.0);
        assert!((result.0.get_flat(0) - 3.0).abs() < 1e-10);
        assert!((result.1.get_flat(0) - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_magnitude() {
        let re = CrowTensor::from_vec(vec![3.0], vec![1]);
        let im = CrowTensor::from_vec(vec![4.0], vec![1]);
        let m = CrowTensor::magnitude(&re, &im);
        assert!((m.to_vec()[0] - 5.0).abs() < 1e-10, "|3+4i| = 5");
    }

    #[test]
    fn test_phase() {
        let re = CrowTensor::from_vec(vec![1.0, 0.0], vec![2]);
        let im = CrowTensor::from_vec(vec![0.0, 1.0], vec![2]);
        let p = CrowTensor::phase(&re, &im);
        let v = p.to_vec();
        assert!((v[0]).abs() < 1e-10, "phase(1+0i) = 0");
        assert!((v[1] - PI / 2.0).abs() < 1e-10, "phase(0+1i) = π/2");
    }

    #[test]
    fn test_conjugate() {
        let re = CrowTensor::from_vec(vec![3.0], vec![1]);
        let im = CrowTensor::from_vec(vec![4.0], vec![1]);
        let result: (CrowTensor, CrowTensor) = CrowTensor::conjugate(&re, &im);
        assert!((result.0.get_flat(0) - 3.0).abs() < 1e-10);
        assert!((result.1.get_flat(0) - (-4.0)).abs() < 1e-10);
    }
}

