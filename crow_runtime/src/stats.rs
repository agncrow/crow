use crate::tensor::CrowTensor;

/// Median of a 1D tensor.
pub fn median(t: &CrowTensor) -> f64 {
    let sorted = t.sort();
    let data = sorted.to_vec();
    let n = data.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        data[n / 2]
    } else {
        (data[n / 2 - 1] + data[n / 2]) / 2.0
    }
}

/// Population variance of a 1D tensor.
pub fn var(t: &CrowTensor) -> f64 {
    let data = t.to_vec();
    let n = data.len();
    if n == 0 {
        return f64::NAN;
    }
    let mean = t.mean();
    let sum_sq: f64 = data.iter().map(|x| (x - mean) * (x - mean)).sum();
    sum_sq / n as f64
}

/// Population standard deviation of a 1D tensor.
pub fn std(t: &CrowTensor) -> f64 {
    var(t).sqrt()
}

/// Percentile (0-100) of a 1D tensor using linear interpolation.
pub fn percentile(t: &CrowTensor, p: f64) -> f64 {
    let sorted = t.sort();
    let data = sorted.to_vec();
    let n = data.len();
    if n == 0 {
        return f64::NAN;
    }
    if p <= 0.0 {
        return data[0];
    }
    if p >= 100.0 {
        return data[n - 1];
    }
    let rank = (p / 100.0) * (n as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        data[lo]
    } else {
        let frac = rank - lo as f64;
        data[lo] * (1.0 - frac) + data[hi] * frac
    }
}

/// Z-score normalization: (x - mean) / std. Returns a new tensor.
pub fn zscore(t: &CrowTensor) -> CrowTensor {
    let mean = t.mean();
    let s = std(t);
    if s == 0.0 {
        // All values are the same — return zeros
        return CrowTensor::zeros(t.shape().to_vec());
    }
    let data: Vec<f64> = t.to_vec().iter().map(|x| (x - mean) / s).collect();
    CrowTensor::from_vec(data, t.shape().to_vec())
}

/// Pearson correlation coefficient between two 1D tensors.
pub fn corr(x: &CrowTensor, y: &CrowTensor) -> f64 {
    let c = cov(x, y);
    let sx = std(x);
    let sy = std(y);
    if sx == 0.0 || sy == 0.0 {
        return f64::NAN;
    }
    c / (sx * sy)
}

/// Population covariance between two 1D tensors.
pub fn cov(x: &CrowTensor, y: &CrowTensor) -> f64 {
    let xd = x.to_vec();
    let yd = y.to_vec();
    let n = xd.len();
    if n == 0 || yd.len() != n {
        return f64::NAN;
    }
    let mx = x.mean();
    let my = y.mean();
    let sum: f64 = xd.iter().zip(yd.iter()).map(|(a, b)| (a - mx) * (b - my)).sum();
    sum / n as f64
}

/// Simple linear regression: y = slope * x + intercept.
/// Returns (slope, intercept, r_squared).
pub fn linreg(x: &CrowTensor, y: &CrowTensor) -> (f64, f64, f64) {
    let xd = x.to_vec();
    let yd = y.to_vec();
    let n = xd.len() as f64;
    if xd.len() < 2 || yd.len() != xd.len() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mx = x.mean();
    let my = y.mean();
    let ss_xy: f64 = xd.iter().zip(yd.iter()).map(|(xi, yi)| (xi - mx) * (yi - my)).sum();
    let ss_xx: f64 = xd.iter().map(|xi| (xi - mx) * (xi - mx)).sum();
    if ss_xx == 0.0 {
        return (f64::NAN, my, f64::NAN);
    }
    let slope = ss_xy / ss_xx;
    let intercept = my - slope * mx;
    // R² = 1 - SS_res / SS_tot
    let ss_res: f64 = xd.iter().zip(yd.iter()).map(|(xi, yi)| {
        let pred = slope * xi + intercept;
        (yi - pred) * (yi - pred)
    }).sum();
    let ss_tot: f64 = yd.iter().map(|yi| (yi - my) * (yi - my)).sum();
    let r2 = if ss_tot == 0.0 { 1.0 } else { 1.0 - ss_res / ss_tot };
    (slope, intercept, r2)
}

/// Two-sample Welch's t-test. Returns (t_statistic, p_value).
/// Uses the approximation for the two-tailed p-value from the t-distribution.
pub fn ttest(x: &CrowTensor, y: &CrowTensor) -> (f64, f64) {
    let xd = x.to_vec();
    let yd = y.to_vec();
    let n1 = xd.len() as f64;
    let n2 = yd.len() as f64;
    if n1 < 2.0 || n2 < 2.0 {
        return (f64::NAN, f64::NAN);
    }
    let m1 = x.mean();
    let m2 = y.mean();
    // Sample variances
    let s1: f64 = xd.iter().map(|v| (v - m1) * (v - m1)).sum::<f64>() / (n1 - 1.0);
    let s2: f64 = yd.iter().map(|v| (v - m2) * (v - m2)).sum::<f64>() / (n2 - 1.0);
    let se = (s1 / n1 + s2 / n2).sqrt();
    if se == 0.0 {
        return (f64::NAN, f64::NAN);
    }
    let t = (m1 - m2) / se;
    // Welch-Satterthwaite degrees of freedom
    let num = (s1 / n1 + s2 / n2).powi(2);
    let den = (s1 / n1).powi(2) / (n1 - 1.0) + (s2 / n2).powi(2) / (n2 - 1.0);
    let df = num / den;
    // Approximate two-tailed p-value using regularized incomplete beta function
    let p = two_tailed_p(t.abs(), df);
    (t, p)
}

/// Approximate two-tailed p-value from a t-distribution.
/// Uses the relationship: p = I(df/(df+t²), df/2, 1/2) where I is the regularized
/// incomplete beta function.
fn two_tailed_p(t_abs: f64, df: f64) -> f64 {
    let x = df / (df + t_abs * t_abs);
    regularized_incomplete_beta(x, df / 2.0, 0.5)
}

/// Regularized incomplete beta function I_x(a, b) using continued fraction (Lentz's method).
fn regularized_incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Use the symmetry relation if x > (a+1)/(a+b+2)
    if x > (a + 1.0) / (a + b + 2.0) {
        return 1.0 - regularized_incomplete_beta(1.0 - x, b, a);
    }
    let ln_beta = ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b);
    let front = (x.ln() * a + (1.0 - x).ln() * b - ln_beta).exp() / a;
    // Continued fraction (Lentz's method)
    let mut c = 1.0;
    let mut d = 1.0 - (a + b) * x / (a + 1.0);
    if d.abs() < 1e-30 {
        d = 1e-30;
    }
    d = 1.0 / d;
    let mut f = d;
    for m in 1..=200 {
        let m = m as f64;
        // Even step
        let num = m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m));
        d = 1.0 + num * d;
        if d.abs() < 1e-30 { d = 1e-30; }
        c = 1.0 + num / c;
        if c.abs() < 1e-30 { c = 1e-30; }
        d = 1.0 / d;
        f *= c * d;
        // Odd step
        let num = -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0));
        d = 1.0 + num * d;
        if d.abs() < 1e-30 { d = 1e-30; }
        c = 1.0 + num / c;
        if c.abs() < 1e-30 { c = 1e-30; }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() < 1e-10 {
            break;
        }
    }
    front * f
}

/// Log-gamma function using Stirling's approximation (Lanczos).
fn ln_gamma(x: f64) -> f64 {
    // Lanczos approximation coefficients (g=7)
    let coeffs = [
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
    if x < 0.5 {
        let s = std::f64::consts::PI / (std::f64::consts::PI * x).sin();
        return s.ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = coeffs[0];
    let t = x + 7.5;
    for (i, &c) in coeffs.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (t.ln() * (x + 0.5)) - t + a.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(data: &[f64]) -> CrowTensor {
        CrowTensor::from_vec(data.to_vec(), vec![data.len()])
    }

    #[test]
    fn test_median_odd() {
        let t = tensor(&[3.0, 1.0, 2.0]);
        assert!((median(&t) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_median_even() {
        let t = tensor(&[4.0, 1.0, 3.0, 2.0]);
        assert!((median(&t) - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_var_std() {
        let t = tensor(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        let v = var(&t);
        assert!((v - 4.0).abs() < 1e-10);
        assert!((std(&t) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_percentile() {
        let t = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!((percentile(&t, 0.0) - 1.0).abs() < 1e-10);
        assert!((percentile(&t, 50.0) - 3.0).abs() < 1e-10);
        assert!((percentile(&t, 100.0) - 5.0).abs() < 1e-10);
        assert!((percentile(&t, 25.0) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_zscore() {
        let t = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let z = zscore(&t);
        let zd = z.to_vec();
        // Mean should be ~0
        let zmean: f64 = zd.iter().sum::<f64>() / zd.len() as f64;
        assert!(zmean.abs() < 1e-10);
        // Std should be ~1
        let zvar: f64 = zd.iter().map(|x| x * x).sum::<f64>() / zd.len() as f64;
        assert!((zvar - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_corr_perfect() {
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[2.0, 4.0, 6.0, 8.0, 10.0]);
        assert!((corr(&x, &y) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_corr_negative() {
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[5.0, 4.0, 3.0, 2.0, 1.0]);
        assert!((corr(&x, &y) - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_cov() {
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[2.0, 4.0, 6.0, 8.0, 10.0]);
        assert!((cov(&x, &y) - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_ttest_same() {
        // Same data → t ≈ 0, p ≈ 1
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let (t, p) = ttest(&x, &y);
        assert!(t.abs() < 1e-10);
        assert!((p - 1.0).abs() < 0.05);
    }

    #[test]
    fn test_ttest_different() {
        // Very different data → small p
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[100.0, 200.0, 300.0, 400.0, 500.0]);
        let (t, p) = ttest(&x, &y);
        assert!(t.abs() > 3.0, "t-statistic should be large, got {}", t);
        assert!(p < 0.05, "p-value should be small, got {}", p);
    }

    #[test]
    fn test_median_empty() {
        let t = tensor(&[]);
        assert!(median(&t).is_nan());
    }

    #[test]
    fn test_median_single() {
        let t = tensor(&[42.0]);
        assert!((median(&t) - 42.0).abs() < 1e-10);
    }

    // ── linreg ──────────────────────────────────────────────────────────

    #[test]
    fn test_linreg_perfect() {
        // y = 2x + 1, perfect fit
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[3.0, 5.0, 7.0, 9.0, 11.0]);
        let (slope, intercept, r2) = linreg(&x, &y);
        assert!((slope - 2.0).abs() < 1e-10);
        assert!((intercept - 1.0).abs() < 1e-10);
        assert!((r2 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_linreg_negative_slope() {
        // y = -3x + 10
        let x = tensor(&[1.0, 2.0, 3.0, 4.0]);
        let y = tensor(&[7.0, 4.0, 1.0, -2.0]);
        let (slope, intercept, r2) = linreg(&x, &y);
        assert!((slope - (-3.0)).abs() < 1e-10);
        assert!((intercept - 10.0).abs() < 1e-10);
        assert!((r2 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_linreg_noisy() {
        // Noisy data, R² should be < 1 but > 0
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[2.1, 3.9, 6.2, 7.8, 10.1]);
        let (slope, _intercept, r2) = linreg(&x, &y);
        assert!(slope > 1.5 && slope < 2.5, "slope should be ~2, got {}", slope);
        assert!(r2 > 0.95 && r2 <= 1.0, "r2 should be high, got {}", r2);
    }

    #[test]
    fn test_linreg_too_few_points() {
        let x = tensor(&[1.0]);
        let y = tensor(&[2.0]);
        let (slope, _intercept, _r2) = linreg(&x, &y);
        assert!(slope.is_nan());
    }
}

