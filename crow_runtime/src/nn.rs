//! `use math.nn`: activations, losses and a linear layer, each answering
//! its value or the reason the arguments have none (docs/bug-classes.md
//! §4): shapes that do not meet, where a kernel panicked; a target that is
//! no class, where the loss read a logit of 0 (or the first class, for a
//! negative one).

use crate::tensor::CrowTensor;

/// ReLU activation: max(x, 0) element-wise.
pub fn relu(t: &CrowTensor) -> CrowTensor {
    t.map(|x| x.max(0.0))
}

/// Sigmoid activation: 1 / (1 + exp(-x)) element-wise.
pub fn sigmoid(t: &CrowTensor) -> CrowTensor {
    t.map(|x| 1.0 / (1.0 + (-x).exp()))
}

/// Numerically stable softmax over a 1D tensor.
/// Uses the log-sum-exp trick: softmax(x)_i = exp(x_i - max(x)) / sum(exp(x - max(x)))
pub fn softmax(t: &CrowTensor) -> CrowTensor {
    let data = t.to_vec();
    let max_val = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = data.iter().map(|&x| (x - max_val).exp()).collect();
    let sum: f64 = exps.iter().sum();
    let result: Vec<f64> = exps.iter().map(|&e| e / sum).collect();
    CrowTensor::from_vec(result, t.shape().to_vec())
}

/// GELU activation: x * 0.5 * (1 + erf(x / sqrt(2))) element-wise.
pub fn gelu(t: &CrowTensor) -> CrowTensor {
    let sqrt2 = std::f64::consts::SQRT_2;
    t.map(|x| {
        let z = x / sqrt2;
        let sign = if z >= 0.0 { 1.0 } else { -1.0 };
        let az = z.abs();
        let t = 1.0 / (1.0 + 0.3275911 * az);
        let poly = t * (0.254829592
            + t * (-0.284496736
                + t * (1.421413741
                    + t * (-1.453152027
                        + t * 1.061405429))));
        let erf_val = sign * (1.0 - poly * (-az * az).exp());
        x * 0.5 * (1.0 + erf_val)
    })
}

/// Mean Squared Error: mean((pred - target)^2), the shapes broadcasting.
pub fn mse(pred: &CrowTensor, target: &CrowTensor) -> Result<f64, String> {
    pred.meets(target, "mse")?;
    let diff = pred.sub(target);
    let sq = diff.mul(&diff);
    Ok(sq.mean())
}

/// Cross-entropy of raw scores against class indices, with the log-sum-exp
/// trick for stability: logits `[classes]` and one target, or `[batch,
/// classes]` and a target per row, each a class from 0 up to classes − 1.
/// Computes mean(-logits[target] + log(sum(exp(logits)))).
pub fn crossentropy(logits: &CrowTensor, targets: &CrowTensor) -> Result<f64, String> {
    let (batch, classes) = match *logits.shape() {
        [c] => (1, c),
        [b, c] => (b, c),
        ref other => return Err(format!("crossentropy: logits are [classes] or [batch, classes], got {} dimensions", other.len())),
    };
    if classes == 0 {
        return Err("crossentropy: the logits have no classes".to_string());
    }
    if targets.len() != batch {
        return Err(format!("crossentropy: needs one target per row of logits ({}), got {}", batch, targets.len()));
    }
    let logits_data = logits.to_vec();
    let mut total = 0.0;
    for (b, t) in targets.to_vec().into_iter().enumerate() {
        if t.fract() != 0.0 || t < 0.0 || t >= classes as f64 {
            return Err(format!("crossentropy: target {} is not a class of {} (0 to {})", t, classes, classes - 1));
        }
        let row = &logits_data[b * classes..(b + 1) * classes];
        let max_val = row.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let log_sum_exp = row.iter().map(|&x| (x - max_val).exp()).sum::<f64>().ln() + max_val;
        total += -row[t as usize] + log_sum_exp;
    }
    Ok(total / batch as f64)
}

/// Binary cross-entropy: -mean(target * ln(pred) + (1 - target) * ln(1 - pred)).
/// Clamps pred to [epsilon, 1-epsilon] for numerical stability.
pub fn bce(pred: &CrowTensor, target: &CrowTensor) -> Result<f64, String> {
    if pred.len() != target.len() {
        return Err(format!("bce: pred and target must be one length, got {} and {}", pred.len(), target.len()));
    }
    let eps = BCE_EPS;
    let pd = pred.to_vec();
    let td = target.to_vec();
    let n = pd.len();
    if n == 0 {
        return Ok(f64::NAN);
    }
    let mut total = 0.0;
    for i in 0..n {
        let p = pd[i].max(eps).min(1.0 - eps);
        total += td[i] * p.ln() + (1.0 - td[i]) * (1.0 - p).ln();
    }
    Ok(-total / n as f64)
}

/// How close to 0 or 1 a probability is read in the binary cross-entropy:
/// a prediction is read within [eps, 1 - eps], so a sure wrong answer is a
/// large loss, never an infinite one (the loss's gradient reads it the
/// same way, crate::autograd).
pub const BCE_EPS: f64 = 1e-12;

/// Linear layer: x * w + b, matrices x and w, the bias broadcasting.
pub fn linear(x: &CrowTensor, w: &CrowTensor, b: &CrowTensor) -> Result<CrowTensor, String> {
    match (x.shape(), w.shape()) {
        ([_, k], [r, _]) if k == r => {}
        ([_, k], [r, _]) => return Err(format!("linear: x has {} columns and w {} rows — x * w needs them equal", k, r)),
        (xs, ws) => return Err(format!("linear: x and w are matrices, got shapes {:?} and {:?}", xs, ws)),
    }
    let xw = x.matmul(w);
    if xw.meets(b, "+").is_err() {
        return Err(format!("linear: a bias of shape {:?} does not meet x * w of shape {:?}", b.shape(), xw.shape()));
    }
    Ok(xw.add(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(data: &[f64]) -> CrowTensor {
        CrowTensor::from_vec(data.to_vec(), vec![data.len()])
    }

    fn tensor2d(data: &[f64], rows: usize, cols: usize) -> CrowTensor {
        CrowTensor::from_vec(data.to_vec(), vec![rows, cols])
    }

    #[test]
    fn test_relu() {
        let t = tensor(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
        let r = relu(&t);
        let d = r.to_vec();
        assert_eq!(d, vec![0.0, 0.0, 0.0, 1.0, 2.0]);
    }

    #[test]
    fn test_sigmoid() {
        let t = tensor(&[0.0]);
        let r = sigmoid(&t);
        assert!((r.to_vec()[0] - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_sigmoid_large() {
        let t = tensor(&[-100.0, 100.0]);
        let r = sigmoid(&t);
        let d = r.to_vec();
        assert!(d[0] < 1e-10);
        assert!((d[1] - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_softmax_sums_to_one() {
        let t = tensor(&[1.0, 2.0, 3.0]);
        let r = softmax(&t);
        let sum: f64 = r.to_vec().iter().sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_softmax_monotonic() {
        let t = tensor(&[1.0, 2.0, 3.0]);
        let d = softmax(&t).to_vec();
        assert!(d[0] < d[1]);
        assert!(d[1] < d[2]);
    }

    #[test]
    fn test_gelu_zero() {
        let t = tensor(&[0.0]);
        assert!(gelu(&t).to_vec()[0].abs() < 1e-10);
    }

    #[test]
    fn test_gelu_positive() {
        let t = tensor(&[1.0]);
        let v = gelu(&t).to_vec()[0];
        // GELU(1) ≈ 0.8413
        assert!((v - 0.8413).abs() < 0.001);
    }

    #[test]
    fn test_mse_zero() {
        let a = tensor(&[1.0, 2.0, 3.0]);
        assert!(mse(&a, &a).expect("one shape").abs() < 1e-10);
    }

    #[test]
    fn test_mse_value() {
        let a = tensor(&[1.0, 2.0, 3.0]);
        let b = tensor(&[2.0, 3.0, 4.0]);
        // Each diff is 1, squared is 1, mean is 1
        assert!((mse(&a, &b).expect("one shape") - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_crossentropy_perfect() {
        // logits = [10, 0, 0], target = [0] → loss should be small
        let logits = tensor(&[10.0, 0.0, 0.0]);
        let targets = tensor(&[0.0]);
        let loss = crossentropy(&logits, &targets).expect("a class");
        assert!(loss < 0.01, "loss should be small for correct prediction, got {}", loss);
    }

    #[test]
    fn test_crossentropy_wrong() {
        // logits = [0, 0, 10], target = [0] → loss should be large
        let logits = tensor(&[0.0, 0.0, 10.0]);
        let targets = tensor(&[0.0]);
        let loss = crossentropy(&logits, &targets).expect("a class");
        assert!(loss > 5.0, "loss should be large for wrong prediction, got {}", loss);
    }

    #[test]
    fn test_crossentropy_batch() {
        let logits = tensor2d(&[10.0, 0.0, 0.0, 10.0], 2, 2);
        let targets = tensor(&[0.0, 1.0]); // both correct
        let loss = crossentropy(&logits, &targets).expect("a class");
        assert!(loss < 0.01, "batch loss should be small, got {}", loss);
    }

    #[test]
    fn test_bce_perfect() {
        let pred = tensor(&[1.0, 0.0, 1.0]);
        let target = tensor(&[1.0, 0.0, 1.0]);
        let loss = bce(&pred, &target).expect("one length");
        assert!(loss < 0.01, "loss should be near zero, got {}", loss);
    }

    #[test]
    fn test_bce_wrong() {
        let pred = tensor(&[0.0, 1.0]);
        let target = tensor(&[1.0, 0.0]);
        let loss = bce(&pred, &target).expect("one length");
        assert!(loss > 10.0, "loss should be large, got {}", loss);
    }

    #[test]
    fn test_linear() {
        // x=[1,2], w=[[1,0],[0,1]], b=[1,1] → x*w+b = [2,3]
        let x = tensor2d(&[1.0, 2.0], 1, 2);
        let w = tensor2d(&[1.0, 0.0, 0.0, 1.0], 2, 2);
        let b = tensor(&[1.0, 1.0]);
        let r = linear(&x, &w, &b).expect("shapes that meet");
        let d = r.to_vec();
        assert!((d[0] - 2.0).abs() < 1e-10);
        assert!((d[1] - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_relu_all_negative() {
        let t = tensor(&[-5.0, -3.0, -1.0]);
        let d = relu(&t).to_vec();
        assert_eq!(d, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_softmax_large_values() {
        // Should not overflow due to log-sum-exp trick
        let t = tensor(&[1000.0, 1001.0, 1002.0]);
        let r = softmax(&t);
        let sum: f64 = r.to_vec().iter().sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_gelu_negative() {
        let t = tensor(&[-3.0]);
        let v = gelu(&t).to_vec()[0];
        // GELU(-3) ≈ -0.00404
        assert!(v.abs() < 0.01);
    }

    /// A misuse is the reason, where a kernel panicked (shapes) or a loss
    /// read a logit of 0 (a target that is no class).
    #[test]
    fn a_misuse_is_the_reason() {
        let (a, b) = (tensor(&[1.0, 2.0, 3.0]), tensor(&[1.0, 2.0]));
        assert!(mse(&a, &b).expect_err("shapes that do not meet").starts_with("`mse` of tensors of shapes [3] and [2]"));
        assert_eq!(bce(&a, &b), Err("bce: pred and target must be one length, got 3 and 2".to_string()));
        assert_eq!(crossentropy(&a, &tensor(&[7.0])), Err("crossentropy: target 7 is not a class of 3 (0 to 2)".to_string()));
        assert_eq!(crossentropy(&a, &tensor(&[-1.0])), Err("crossentropy: target -1 is not a class of 3 (0 to 2)".to_string()));
        assert_eq!(crossentropy(&a, &tensor(&[])), Err("crossentropy: needs one target per row of logits (1), got 0".to_string()));
        let (x, w) = (tensor2d(&[1.0, 2.0], 1, 2), tensor2d(&[1.0, 0.0, 0.0, 1.0, 1.0, 1.0], 3, 2));
        assert_eq!(linear(&x, &w, &a).map(|_| ()), Err("linear: x has 2 columns and w 3 rows — x * w needs them equal".to_string()));
        assert_eq!(linear(&a, &w, &a).map(|_| ()), Err("linear: x and w are matrices, got shapes [3] and [3, 2]".to_string()));
    }
}
