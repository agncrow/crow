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

/// Mean Squared Error: mean((pred - target)^2).
pub fn mse(pred: &CrowTensor, target: &CrowTensor) -> f64 {
    let diff = pred.sub(target);
    let sq = diff.mul(&diff);
    sq.mean()
}

/// Cross-entropy loss with numerical stability (log-sum-exp trick).
/// logits: raw unnormalized scores, targets: integer class indices (as floats).
/// Computes: mean(-logits[target] + log(sum(exp(logits))))
pub fn crossentropy(logits: &CrowTensor, targets: &CrowTensor) -> f64 {
    let logits_data = logits.to_vec();
    let targets_data = targets.to_vec();
    let shape = logits.shape();

    if shape.len() == 1 {
        // Single sample: logits is [num_classes], targets is [1] (class index)
        let max_val = logits_data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let log_sum_exp: f64 = logits_data.iter().map(|&x| (x - max_val).exp()).sum::<f64>().ln() + max_val;
        let target_idx = targets_data[0] as usize;
        let target_logit = if target_idx < logits_data.len() { logits_data[target_idx] } else { 0.0 };
        -target_logit + log_sum_exp
    } else if shape.len() == 2 {
        // Batch: logits is [batch, num_classes], targets is [batch]
        let batch = shape[0];
        let num_classes = shape[1];
        let mut total = 0.0;
        for b in 0..batch {
            let row = &logits_data[b * num_classes..(b + 1) * num_classes];
            let max_val = row.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let log_sum_exp = row.iter().map(|&x| (x - max_val).exp()).sum::<f64>().ln() + max_val;
            let target_idx = targets_data[b] as usize;
            let target_logit = if target_idx < num_classes { row[target_idx] } else { 0.0 };
            total += -target_logit + log_sum_exp;
        }
        total / batch as f64
    } else {
        f64::NAN
    }
}

/// Binary cross-entropy: -mean(target * ln(pred) + (1 - target) * ln(1 - pred)).
/// Clamps pred to [epsilon, 1-epsilon] for numerical stability.
pub fn bce(pred: &CrowTensor, target: &CrowTensor) -> f64 {
    let eps = 1e-12;
    let pd = pred.to_vec();
    let td = target.to_vec();
    let n = pd.len();
    if n == 0 || td.len() != n {
        return f64::NAN;
    }
    let mut total = 0.0;
    for i in 0..n {
        let p = pd[i].max(eps).min(1.0 - eps);
        total += td[i] * p.ln() + (1.0 - td[i]) * (1.0 - p).ln();
    }
    -total / n as f64
}

/// Linear layer: x * w + b (matmul + bias).
pub fn linear(x: &CrowTensor, w: &CrowTensor, b: &CrowTensor) -> CrowTensor {
    x.matmul(w).add(b)
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
        assert!(mse(&a, &a).abs() < 1e-10);
    }

    #[test]
    fn test_mse_value() {
        let a = tensor(&[1.0, 2.0, 3.0]);
        let b = tensor(&[2.0, 3.0, 4.0]);
        // Each diff is 1, squared is 1, mean is 1
        assert!((mse(&a, &b) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_crossentropy_perfect() {
        // logits = [10, 0, 0], target = [0] → loss should be small
        let logits = tensor(&[10.0, 0.0, 0.0]);
        let targets = tensor(&[0.0]);
        let loss = crossentropy(&logits, &targets);
        assert!(loss < 0.01, "loss should be small for correct prediction, got {}", loss);
    }

    #[test]
    fn test_crossentropy_wrong() {
        // logits = [0, 0, 10], target = [0] → loss should be large
        let logits = tensor(&[0.0, 0.0, 10.0]);
        let targets = tensor(&[0.0]);
        let loss = crossentropy(&logits, &targets);
        assert!(loss > 5.0, "loss should be large for wrong prediction, got {}", loss);
    }

    #[test]
    fn test_crossentropy_batch() {
        let logits = tensor2d(&[10.0, 0.0, 0.0, 10.0], 2, 2);
        let targets = tensor(&[0.0, 1.0]); // both correct
        let loss = crossentropy(&logits, &targets);
        assert!(loss < 0.01, "batch loss should be small, got {}", loss);
    }

    #[test]
    fn test_bce_perfect() {
        let pred = tensor(&[1.0, 0.0, 1.0]);
        let target = tensor(&[1.0, 0.0, 1.0]);
        let loss = bce(&pred, &target);
        assert!(loss < 0.01, "loss should be near zero, got {}", loss);
    }

    #[test]
    fn test_bce_wrong() {
        let pred = tensor(&[0.0, 1.0]);
        let target = tensor(&[1.0, 0.0]);
        let loss = bce(&pred, &target);
        assert!(loss > 10.0, "loss should be large, got {}", loss);
    }

    #[test]
    fn test_linear() {
        // x=[1,2], w=[[1,0],[0,1]], b=[1,1] → x*w+b = [2,3]
        let x = tensor2d(&[1.0, 2.0], 1, 2);
        let w = tensor2d(&[1.0, 0.0, 0.0, 1.0], 2, 2);
        let b = tensor(&[1.0, 1.0]);
        let r = linear(&x, &w, &b);
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
}
