//! `use math.stats`: the functions a program calls, each answering its
//! value or the reason the arguments have none (docs/bug-classes.md §4).
//! Two series of different lengths, a percent outside 0 to 100, a
//! distribution the function does not know: the reason, where a mismatch
//! was answered NaN, an unknown name 0.5 or an empty map.

use std::collections::BTreeMap;

use crate::tensor::CrowTensor;

/// Two series one statistic pairs up: of one length.
fn one_length(what: &str, names: &str, a: &CrowTensor, b: &CrowTensor) -> Result<(), String> {
    if a.len() != b.len() {
        return Err(format!("{}: {} must be one length, got {} and {}", what, names, a.len(), b.len()));
    }
    Ok(())
}

/// A test's numbers by name, as the map a program reads (`r['p']`).
fn named(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

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
pub fn percentile(t: &CrowTensor, p: f64) -> Result<f64, String> {
    if !(0.0..=100.0).contains(&p) {
        return Err(format!("percentile: p is a percent, from 0 to 100 (got {})", p));
    }
    let sorted = t.sort();
    let data = sorted.to_vec();
    let n = data.len();
    if n == 0 {
        return Ok(f64::NAN);
    }
    if p == 0.0 {
        return Ok(data[0]);
    }
    if p == 100.0 {
        return Ok(data[n - 1]);
    }
    let rank = (p / 100.0) * (n as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        Ok(data[lo])
    } else {
        let frac = rank - lo as f64;
        Ok(data[lo] * (1.0 - frac) + data[hi] * frac)
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
pub fn corr(x: &CrowTensor, y: &CrowTensor) -> Result<f64, String> {
    one_length("corr", "x and y", x, y)?;
    let c = covariance(x, y);
    let sx = std(x);
    let sy = std(y);
    if sx == 0.0 || sy == 0.0 {
        return Ok(f64::NAN);
    }
    Ok(c / (sx * sy))
}

/// Population covariance between two 1D tensors.
pub fn cov(x: &CrowTensor, y: &CrowTensor) -> Result<f64, String> {
    one_length("cov", "x and y", x, y)?;
    Ok(covariance(x, y))
}

/// The covariance of two series of one length; NaN of none.
fn covariance(x: &CrowTensor, y: &CrowTensor) -> f64 {
    let xd = x.to_vec();
    let yd = y.to_vec();
    let n = xd.len();
    if n == 0 {
        return f64::NAN;
    }
    let mx = x.mean();
    let my = y.mean();
    let sum: f64 = xd.iter().zip(yd.iter()).map(|(a, b)| (a - mx) * (b - my)).sum();
    sum / n as f64
}

/// Simple linear regression: y = slope * x + intercept, as `slope`,
/// `intercept` and `r2`.
pub fn linreg(x: &CrowTensor, y: &CrowTensor) -> Result<BTreeMap<String, f64>, String> {
    one_length("linreg", "x and y", x, y)?;
    let (slope, intercept, r2) = regression(x, y);
    Ok(named(&[("slope", slope), ("intercept", intercept), ("r2", r2)]))
}

/// (slope, intercept, r²) of two series of one length; NaN of fewer than
/// two points.
fn regression(x: &CrowTensor, y: &CrowTensor) -> (f64, f64, f64) {
    let xd = x.to_vec();
    let yd = y.to_vec();
    if xd.len() < 2 {
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

/// Two-sample Welch's t-test, as `t` and `p`.
pub fn ttest(x: &CrowTensor, y: &CrowTensor) -> BTreeMap<String, f64> {
    let (t, p) = welch(x, y);
    named(&[("t", t), ("p", p)])
}

/// Welch's (t, p): the two-tailed p-value from the t-distribution.
fn welch(x: &CrowTensor, y: &CrowTensor) -> (f64, f64) {
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

/// Chi-square goodness-of-fit test, as `chi2` and `p`: `observed` and
/// `expected` of one length.
pub fn chisq(observed: &CrowTensor, expected: &CrowTensor) -> Result<BTreeMap<String, f64>, String> {
    one_length("chisq", "observed and expected", observed, expected)?;
    let obs = observed.to_vec();
    let exp = expected.to_vec();
    let n = obs.len();
    if n == 0 {
        return Ok(named(&[("chi2", f64::NAN), ("p", f64::NAN)]));
    }
    let chi2: f64 = obs.iter().zip(exp.iter())
        .map(|(o, e)| if *e == 0.0 { 0.0 } else { (o - e) * (o - e) / e })
        .sum();
    let df = (n - 1) as f64;
    let p = 1.0 - chi2_cdf(chi2, df);
    Ok(named(&[("chi2", chi2), ("p", p)]))
}

/// One-way ANOVA of two groups or more (`anova[a, b, c]`), as `f` and `p`.
pub fn anova(groups: &[&CrowTensor]) -> Result<BTreeMap<String, f64>, String> {
    if groups.len() < 2 {
        return Err(format!("anova: compares 2 groups or more, got {}", groups.len()));
    }
    let (f, p) = one_way(groups);
    Ok(named(&[("f", f), ("p", p)]))
}

/// The one-way ANOVA's (F, p) of two groups or more.
fn one_way(groups: &[&CrowTensor]) -> (f64, f64) {
    let k = groups.len();
    let group_data: Vec<Vec<f64>> = groups.iter().map(|g| g.to_vec()).collect();
    let n_total: usize = group_data.iter().map(|g| g.len()).sum();
    if n_total <= k {
        return (f64::NAN, f64::NAN);
    }
    let grand_mean: f64 = group_data.iter().flat_map(|g| g.iter()).sum::<f64>() / n_total as f64;

    // Between-group sum of squares
    let ss_between: f64 = group_data.iter()
        .map(|g| {
            let ni = g.len() as f64;
            let gi_mean = g.iter().sum::<f64>() / ni;
            ni * (gi_mean - grand_mean) * (gi_mean - grand_mean)
        })
        .sum();

    // Within-group sum of squares
    let ss_within: f64 = group_data.iter()
        .map(|g| {
            let gi_mean = g.iter().sum::<f64>() / g.len() as f64;
            g.iter().map(|x| (x - gi_mean) * (x - gi_mean)).sum::<f64>()
        })
        .sum();

    let df_between = (k - 1) as f64;
    let df_within = (n_total - k) as f64;
    if df_within <= 0.0 || ss_within == 0.0 {
        return (f64::INFINITY, 0.0);
    }
    let ms_between = ss_between / df_between;
    let ms_within = ss_within / df_within;
    let f = ms_between / ms_within;
    let p = 1.0 - f_cdf(f, df_between, df_within);
    (f, p)
}

/// Kolmogorov-Smirnov test against `'normal'` (the data's own mean and
/// spread) or `'uniform'` (0 to 1), as `d` and `p`.
pub fn kstest(data: &CrowTensor, dist: &str) -> Result<BTreeMap<String, f64>, String> {
    if !matches!(dist, "normal" | "uniform") {
        return Err(format!("kstest: no distribution '{}' — write 'normal' or 'uniform'", dist));
    }
    let (d, p) = kolmogorov(data, dist);
    Ok(named(&[("d", d), ("p", p)]))
}

/// The KS test's (D, p) against a distribution `kstest` knows.
fn kolmogorov(data: &CrowTensor, dist: &str) -> (f64, f64) {
    let mut sorted = data.to_vec();
    let n = sorted.len();
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Standardize if testing against normal
    let (mean, std_dev) = if dist == "normal" {
        let m = sorted.iter().sum::<f64>() / n as f64;
        let v = sorted.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n as f64;
        (m, v.sqrt())
    } else {
        (0.0, 1.0)
    };

    let cdf_fn = |x: f64| -> f64 {
        match dist {
            "normal" => {
                let z = if std_dev > 0.0 { (x - mean) / std_dev } else { 0.0 };
                0.5 * (1.0 + erf_approx(z / std::f64::consts::SQRT_2))
            }
            _ => x.clamp(0.0, 1.0),
        }
    };

    let mut d_max = 0.0_f64;
    for (i, &x) in sorted.iter().enumerate() {
        let f_x = cdf_fn(x);
        let d1 = ((i + 1) as f64 / n as f64 - f_x).abs();
        let d2 = (f_x - i as f64 / n as f64).abs();
        d_max = d_max.max(d1).max(d2);
    }

    // Approximate p-value (Kolmogorov's asymptotic formula)
    let sqrt_n = (n as f64).sqrt();
    let lambda = (sqrt_n + 0.12 + 0.11 / sqrt_n) * d_max;
    let p = ks_pvalue(lambda);
    (d_max, p)
}

/// A distribution's parameters fitted to data: `'normal'` → `mu`,
/// `sigma`; `'exponential'` → `lambda`; `'uniform'` → `a`, `b`;
/// `'poisson'` → `lambda`. Each is NaN of no data.
pub fn fit(data: &CrowTensor, dist: &str) -> Result<BTreeMap<String, f64>, String> {
    let d = data.to_vec();
    let n = d.len() as f64;
    // of no data, the mean and the bounds are NaN, as a statistic of none is
    let mean = if d.is_empty() { f64::NAN } else { d.iter().sum::<f64>() / n };
    Ok(match dist {
        "normal" => {
            let sigma = if d.is_empty() { f64::NAN } else { (d.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n).sqrt() };
            named(&[("mu", mean), ("sigma", sigma)])
        }
        "exponential" => named(&[("lambda", if mean > 0.0 { 1.0 / mean } else { f64::NAN })]),
        "uniform" => {
            let bound = |f: fn(f64, f64) -> f64, from: f64| if d.is_empty() { f64::NAN } else { d.iter().cloned().fold(from, f) };
            named(&[("a", bound(f64::min, f64::INFINITY)), ("b", bound(f64::max, f64::NEG_INFINITY))])
        }
        "poisson" => named(&[("lambda", mean)]),
        _ => return Err(format!("fit: no distribution '{}' — write 'normal', 'exponential', 'uniform' or 'poisson'", dist)),
    })
}

// ── Helper functions ────────────────────────────────────────────────────

/// Approximate error function using Abramowitz & Stegun formula 7.1.26.
fn erf_approx(x: f64) -> f64 {
    let sign = if x >= 0.0 { 1.0 } else { -1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    sign * (1.0 - poly * (-x * x).exp())
}

/// Chi-square CDF via the regularized lower incomplete gamma function.
/// P(X ≤ x) = γ(k/2, x/2) / Γ(k/2) where k = degrees of freedom.
fn chi2_cdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    lower_regularized_gamma(df / 2.0, x / 2.0)
}

/// F-distribution CDF via the regularized incomplete beta function.
fn f_cdf(x: f64, d1: f64, d2: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    let z = d1 * x / (d1 * x + d2);
    regularized_incomplete_beta(z, d1 / 2.0, d2 / 2.0)
}

/// KS test p-value from Kolmogorov's distribution (approximation).
fn ks_pvalue(lambda: f64) -> f64 {
    if lambda <= 0.0 { return 1.0; }
    let mut sum = 0.0;
    for j in 1..=100 {
        let j = j as f64;
        let sign = if j as i64 % 2 == 0 { -1.0 } else { 1.0 };
        let term = sign * (-2.0 * j * j * lambda * lambda).exp();
        sum += term;
        if term.abs() < 1e-15 { break; }
    }
    (2.0 * sum).clamp(0.0, 1.0)
}

/// Lower regularized incomplete gamma function P(a, x) = γ(a, x) / Γ(a).
/// Uses series expansion for small x, continued fraction for large x.
fn lower_regularized_gamma(a: f64, x: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    if x < a + 1.0 {
        // Series expansion
        gamma_series(a, x)
    } else {
        // Continued fraction
        1.0 - gamma_cf(a, x)
    }
}

/// Series expansion for lower regularized incomplete gamma.
fn gamma_series(a: f64, x: f64) -> f64 {
    let ln_gamma_a = ln_gamma(a);
    let mut sum = 1.0 / a;
    let mut term = 1.0 / a;
    for n in 1..200 {
        term *= x / (a + n as f64);
        sum += term;
        if term.abs() < sum.abs() * 1e-14 { break; }
    }
    sum * (-x + a * x.ln() - ln_gamma_a).exp()
}

/// Continued fraction for upper regularized incomplete gamma.
fn gamma_cf(a: f64, x: f64) -> f64 {
    let ln_gamma_a = ln_gamma(a);
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / 1e-30;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..200 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < 1e-30 { d = 1e-30; }
        c = b + an / c;
        if c.abs() < 1e-30 { c = 1e-30; }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 { break; }
    }
    (-x + a * x.ln() - ln_gamma_a).exp() * h
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
        assert_eq!(percentile(&t, 0.0), Ok(1.0));
        assert_eq!(percentile(&t, 50.0), Ok(3.0));
        assert_eq!(percentile(&t, 100.0), Ok(5.0));
        assert_eq!(percentile(&t, 25.0), Ok(2.0));
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
        assert!((corr(&x, &y).expect("one length") - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_corr_negative() {
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[5.0, 4.0, 3.0, 2.0, 1.0]);
        assert!((corr(&x, &y).expect("one length") - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_cov() {
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[2.0, 4.0, 6.0, 8.0, 10.0]);
        assert!((cov(&x, &y).expect("one length") - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_ttest_same() {
        // Same data → t ≈ 0, p ≈ 1
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let r = ttest(&x, &y);
        assert!(r["t"].abs() < 1e-10);
        assert!((r["p"] - 1.0).abs() < 0.05);
    }

    #[test]
    fn test_ttest_different() {
        // Very different data → small p
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[100.0, 200.0, 300.0, 400.0, 500.0]);
        let r = ttest(&x, &y);
        assert!(r["t"].abs() > 3.0, "t-statistic should be large, got {}", r["t"]);
        assert!(r["p"] < 0.05, "p-value should be small, got {}", r["p"]);
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
        let (slope, intercept, r2) = regression(&x, &y);
        assert!((slope - 2.0).abs() < 1e-10);
        assert!((intercept - 1.0).abs() < 1e-10);
        assert!((r2 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_linreg_negative_slope() {
        // y = -3x + 10
        let x = tensor(&[1.0, 2.0, 3.0, 4.0]);
        let y = tensor(&[7.0, 4.0, 1.0, -2.0]);
        let (slope, intercept, r2) = regression(&x, &y);
        assert!((slope - (-3.0)).abs() < 1e-10);
        assert!((intercept - 10.0).abs() < 1e-10);
        assert!((r2 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_linreg_noisy() {
        // Noisy data, R² should be < 1 but > 0
        let x = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let y = tensor(&[2.1, 3.9, 6.2, 7.8, 10.1]);
        let (slope, _intercept, r2) = regression(&x, &y);
        assert!(slope > 1.5 && slope < 2.5, "slope should be ~2, got {}", slope);
        assert!(r2 > 0.95 && r2 <= 1.0, "r2 should be high, got {}", r2);
    }

    #[test]
    fn test_linreg_too_few_points() {
        let x = tensor(&[1.0]);
        let y = tensor(&[2.0]);
        let (slope, _intercept, _r2) = regression(&x, &y);
        assert!(slope.is_nan());
    }

    // ── chisq ───────────────────────────────────────────────────────────

    #[test]
    fn test_chisq_perfect_match() {
        let obs = tensor(&[10.0, 20.0, 30.0]);
        let exp = tensor(&[10.0, 20.0, 30.0]);
        let r = chisq(&obs, &exp).expect("one length");
        let (chi2, p) = (r["chi2"], r["p"]);
        assert!((chi2 - 0.0).abs() < 1e-10);
        assert!((p - 1.0).abs() < 0.05);
    }

    #[test]
    fn test_chisq_different() {
        let obs = tensor(&[50.0, 30.0, 20.0]);
        let exp = tensor(&[33.3, 33.3, 33.4]);
        let r = chisq(&obs, &exp).expect("one length");
        let (chi2, p) = (r["chi2"], r["p"]);
        assert!(chi2 > 5.0, "chi2 should be large, got {}", chi2);
        assert!(p < 0.1, "p should be small, got {}", p);
    }

    // ── anova ───────────────────────────────────────────────────────────

    #[test]
    fn test_anova_same_groups() {
        let g1 = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let g2 = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let (f, p) = one_way(&[&g1, &g2]);
        assert!(f.abs() < 1e-10, "F should be ~0, got {}", f);
        assert!(p > 0.9, "p should be ~1, got {}", p);
    }

    #[test]
    fn test_anova_different_groups() {
        let g1 = tensor(&[1.0, 2.0, 3.0]);
        let g2 = tensor(&[10.0, 11.0, 12.0]);
        let g3 = tensor(&[20.0, 21.0, 22.0]);
        let (f, p) = one_way(&[&g1, &g2, &g3]);
        assert!(f > 10.0, "F should be large, got {}", f);
        assert!(p < 0.01, "p should be small, got {}", p);
    }

    // ── kstest ──────────────────────────────────────────────────────────

    #[test]
    fn test_kstest_uniform() {
        // Data that is close to uniform [0,1]
        let data = tensor(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]);
        let (d, p) = kolmogorov(&data, "uniform");
        assert!(d < 0.2, "d should be small for uniform data, got {}", d);
        assert!(p > 0.05, "p should be large for uniform data, got {}", p);
    }

    // ── fit ──────────────────────────────────────────────────────────────

    #[test]
    fn test_fit_normal() {
        // Data with known mean=5, sigma≈1
        let data = tensor(&[4.0, 4.5, 5.0, 5.5, 6.0]);
        let params = fit(&data, "normal").expect("a distribution fit knows");
        assert_eq!(params.len(), 2);
        assert!((params["mu"] - 5.0).abs() < 1e-10, "mu should be 5.0");
        assert!(params["sigma"] > 0.0, "sigma should be positive");
    }

    #[test]
    fn test_fit_uniform() {
        let data = tensor(&[1.0, 3.0, 5.0, 7.0, 9.0]);
        let params = fit(&data, "uniform").expect("a distribution fit knows");
        assert_eq!(params.len(), 2);
        assert!((params["a"] - 1.0).abs() < 1e-10, "a should be 1.0");
        assert!((params["b"] - 9.0).abs() < 1e-10, "b should be 9.0");
    }

    #[test]
    fn test_fit_exponential() {
        let data = tensor(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let params = fit(&data, "exponential").expect("a distribution fit knows");
        assert_eq!(params.len(), 1);
        // mean=3, so lambda=1/3
        assert!((params["lambda"] - 1.0 / 3.0).abs() < 1e-10);
    }

    /// A misuse is the reason, where two lengths were answered NaN, a
    /// distribution no test knows 0.5, and one no fit knows an empty map.
    #[test]
    fn a_misuse_is_the_reason() {
        let (a, b) = (tensor(&[1.0, 2.0, 3.0]), tensor(&[1.0, 2.0]));
        assert_eq!(corr(&a, &b), Err("corr: x and y must be one length, got 3 and 2".to_string()));
        assert_eq!(cov(&a, &b), Err("cov: x and y must be one length, got 3 and 2".to_string()));
        assert_eq!(linreg(&a, &b).map(|_| ()), Err("linreg: x and y must be one length, got 3 and 2".to_string()));
        assert_eq!(chisq(&a, &b).map(|_| ()), Err("chisq: observed and expected must be one length, got 3 and 2".to_string()));
        assert_eq!(anova(&[&a]).map(|_| ()), Err("anova: compares 2 groups or more, got 1".to_string()));
        assert_eq!(kstest(&a, "gauss").map(|_| ()), Err("kstest: no distribution 'gauss' — write 'normal' or 'uniform'".to_string()));
        assert_eq!(fit(&a, "gauss").map(|_| ()), Err("fit: no distribution 'gauss' — write 'normal', 'exponential', 'uniform' or 'poisson'".to_string()));
        assert_eq!(percentile(&a, 150.0), Err("percentile: p is a percent, from 0 to 100 (got 150)".to_string()));
        assert!(fit(&tensor(&[]), "normal").expect("known")["mu"].is_nan());
    }
}

