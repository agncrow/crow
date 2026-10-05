//! The special functions `use math.stats` stands on, validated kernels
//! the module written in Crow (src/modules/stats.co) calls through the sys
//! layer: the regularized incomplete beta and gamma functions, which the
//! t, F and chi-square distributions are written over, and log-gamma. The
//! statistics themselves — the median, the moments, the tests — are
//! formulas in Crow (design rule 20).

/// `sys.betainc[x, a, b]`: the regularized incomplete beta function
/// I_x(a, b), by Lentz's continued fraction; 0 at or below 0, 1 at or
/// above 1.
pub fn betainc(x: f64, a: f64, b: f64) -> f64 {
    regularized_incomplete_beta(x, a, b)
}

/// `sys.gammainc[a, x]`: the lower regularized incomplete gamma function
/// P(a, x) = γ(a, x) / Γ(a), by a series for small x and a continued
/// fraction for large; 0 at or below 0.
pub fn gammainc(a: f64, x: f64) -> f64 {
    lower_regularized_gamma(a, x)
}

/// `sys.lgamma[x]`: the natural log of the gamma function, by Lanczos.
pub fn lgamma(x: f64) -> f64 {
    ln_gamma(x)
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

    /// The kernels at known values: I_x(1, 1) = x, P(1, x) = 1 - e^-x,
    /// ln Γ(5) = ln 24, and the symmetry I_x(a, b) = 1 - I_{1-x}(b, a)
    #[test]
    fn the_special_functions_at_known_values() {
        assert!((betainc(0.3, 1.0, 1.0) - 0.3).abs() < 1e-12);
        assert!((betainc(0.5, 2.0, 2.0) - 0.5).abs() < 1e-12);
        assert!((betainc(0.2, 2.0, 3.0) - (1.0 - betainc(0.8, 3.0, 2.0))).abs() < 1e-12);
        assert_eq!((betainc(0.0, 2.0, 3.0), betainc(1.0, 2.0, 3.0)), (0.0, 1.0));
        assert!((gammainc(1.0, 2.0) - (1.0 - (-2.0f64).exp())).abs() < 1e-12);
        assert!((gammainc(2.5, 0.5) - 0.037_434_2).abs() < 1e-6);
        assert_eq!(gammainc(2.0, 0.0), 0.0);
        assert!((lgamma(5.0) - 24.0f64.ln()).abs() < 1e-10);
        assert!((lgamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-10);
    }
}
