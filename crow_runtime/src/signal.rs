//! Signal processing operations on CrowTensor.
//!
//! FFT (via rustfft), convolution, and windowing functions.

use rustfft::{FftPlanner, num_complex::Complex};
use std::f64::consts::PI;

use crate::tensor::CrowTensor;

impl CrowTensor {
    /// 1D FFT. Returns (real, imag) tensors.
    pub fn fft(&self) -> (CrowTensor, CrowTensor) {
        assert!(self.shape().len() == 1, "fft: expected 1D tensor, got {}D", self.shape().len());
        let n = self.shape()[0];
        let data = self.to_vec();
        let mut buffer: Vec<Complex<f64>> = data.iter().map(|&v| Complex::new(v, 0.0)).collect();
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(n);
        fft.process(&mut buffer);
        let real: Vec<f64> = buffer.iter().map(|c| c.re).collect();
        let imag: Vec<f64> = buffer.iter().map(|c| c.im).collect();
        (CrowTensor::from_vec(real, vec![n]), CrowTensor::from_vec(imag, vec![n]))
    }

    /// Inverse 1D FFT from (real, imag) tensors. Returns real-valued tensor.
    /// Takes the real and imaginary parts as separate tensors.
    pub fn ifft(real: &CrowTensor, imag: &CrowTensor) -> CrowTensor {
        assert!(real.shape().len() == 1, "ifft: expected 1D tensors");
        assert_eq!(real.shape(), imag.shape(), "ifft: real and imag must have same shape");
        let n = real.shape()[0];
        let r = real.to_vec();
        let im = imag.to_vec();
        let mut buffer: Vec<Complex<f64>> = r.iter().zip(im.iter())
            .map(|(&re, &im)| Complex::new(re, im))
            .collect();
        let mut planner = FftPlanner::new();
        let ifft = planner.plan_fft_inverse(n);
        ifft.process(&mut buffer);
        // rustfft doesn't normalize — divide by n
        let result: Vec<f64> = buffer.iter().map(|c| c.re / n as f64).collect();
        CrowTensor::from_vec(result, vec![n])
    }

    /// 2D FFT. Returns (real, imag) tensors with same shape as input.
    pub fn fft2(&self) -> (CrowTensor, CrowTensor) {
        assert!(self.shape().len() == 2, "fft2: expected 2D tensor, got {}D", self.shape().len());
        let rows = self.shape()[0];
        let cols = self.shape()[1];
        let data = self.to_vec();

        // Build complex buffer (row-major)
        let mut buffer: Vec<Complex<f64>> = data.iter().map(|&v| Complex::new(v, 0.0)).collect();

        let mut planner = FftPlanner::new();

        // FFT each row
        let fft_row = planner.plan_fft_forward(cols);
        for r in 0..rows {
            let start = r * cols;
            let row_slice = &mut buffer[start..start + cols];
            fft_row.process(row_slice);
        }

        // FFT each column (need to extract, transform, put back)
        let fft_col = planner.plan_fft_forward(rows);
        let mut col_buf = vec![Complex::new(0.0, 0.0); rows];
        for c in 0..cols {
            for r in 0..rows {
                col_buf[r] = buffer[r * cols + c];
            }
            fft_col.process(&mut col_buf);
            for r in 0..rows {
                buffer[r * cols + c] = col_buf[r];
            }
        }

        let real: Vec<f64> = buffer.iter().map(|c| c.re).collect();
        let imag: Vec<f64> = buffer.iter().map(|c| c.im).collect();
        (CrowTensor::from_vec(real, vec![rows, cols]), CrowTensor::from_vec(imag, vec![rows, cols]))
    }

    /// 1D convolution (full). Result length = len(a) + len(b) - 1.
    pub fn conv(&self, kernel: &CrowTensor) -> CrowTensor {
        assert!(self.shape().len() == 1, "conv: expected 1D tensors");
        assert!(kernel.shape().len() == 1, "conv: expected 1D kernel");
        let a = self.to_vec();
        let b = kernel.to_vec();
        let n = a.len() + b.len() - 1;
        let mut result = vec![0.0; n];
        for i in 0..a.len() {
            for j in 0..b.len() {
                result[i + j] += a[i] * b[j];
            }
        }
        CrowTensor::from_vec(result, vec![n])
    }

    /// 2D convolution (full). Result shape = (h1+h2-1, w1+w2-1).
    pub fn conv2d(&self, kernel: &CrowTensor) -> CrowTensor {
        assert!(self.shape().len() == 2, "conv2d: expected 2D tensor");
        assert!(kernel.shape().len() == 2, "conv2d: expected 2D kernel");
        let h1 = self.shape()[0];
        let w1 = self.shape()[1];
        let h2 = kernel.shape()[0];
        let w2 = kernel.shape()[1];
        let a = self.to_vec();
        let b = kernel.to_vec();
        let oh = h1 + h2 - 1;
        let ow = w1 + w2 - 1;
        let mut result = vec![0.0; oh * ow];
        for i in 0..h1 {
            for j in 0..w1 {
                let av = a[i * w1 + j];
                for ki in 0..h2 {
                    for kj in 0..w2 {
                        result[(i + ki) * ow + (j + kj)] += av * b[ki * w2 + kj];
                    }
                }
            }
        }
        CrowTensor::from_vec(result, vec![oh, ow])
    }

    /// Hamming window of length n.
    pub fn hamming(n: usize) -> CrowTensor {
        let data: Vec<f64> = (0..n)
            .map(|i| 0.54 - 0.46 * (2.0 * PI * i as f64 / (n as f64 - 1.0)).cos())
            .collect();
        CrowTensor::from_vec(data, vec![n])
    }

    /// Hanning (Hann) window of length n.
    pub fn hanning(n: usize) -> CrowTensor {
        let data: Vec<f64> = (0..n)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (n as f64 - 1.0)).cos()))
            .collect();
        CrowTensor::from_vec(data, vec![n])
    }

    /// Short-Time Fourier Transform (STFT).
    /// `window_size`: length of each FFT frame.
    /// `hop_size`: step between frames.
    /// Returns (real_2d, imag_2d) tensors with shape [num_frames, window_size].
    pub fn stft(&self, window_size: usize, hop_size: usize) -> (CrowTensor, CrowTensor) {
        assert!(self.shape().len() == 1, "stft: expected 1D tensor");
        assert!(window_size > 0 && hop_size > 0, "stft: window_size and hop_size must be > 0");
        let data = self.to_vec();
        let n = data.len();
        let window = CrowTensor::hanning(window_size);
        let w = window.to_vec();

        let mut real_out = Vec::new();
        let mut imag_out = Vec::new();
        let mut num_frames = 0usize;

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(window_size);

        let mut pos = 0;
        while pos + window_size <= n {
            // Apply window
            let mut buffer: Vec<Complex<f64>> = (0..window_size)
                .map(|i| Complex::new(data[pos + i] * w[i], 0.0))
                .collect();
            fft.process(&mut buffer);
            real_out.extend(buffer.iter().map(|c| c.re));
            imag_out.extend(buffer.iter().map(|c| c.im));
            num_frames += 1;
            pos += hop_size;
        }

        if num_frames == 0 {
            return (
                CrowTensor::from_vec(vec![], vec![0, window_size]),
                CrowTensor::from_vec(vec![], vec![0, window_size]),
            );
        }

        (
            CrowTensor::from_vec(real_out, vec![num_frames, window_size]),
            CrowTensor::from_vec(imag_out, vec![num_frames, window_size]),
        )
    }

    /// Spectrogram: magnitude-squared of STFT.
    /// Returns a 2D tensor [num_frames, window_size] of power values.
    pub fn spectrogram(&self, window_size: usize, hop_size: usize) -> CrowTensor {
        let (real, imag) = self.stft(window_size, hop_size);
        let r = real.to_vec();
        let im = imag.to_vec();
        let power: Vec<f64> = r.iter().zip(im.iter())
            .map(|(re, im)| re * re + im * im)
            .collect();
        CrowTensor::from_vec(power, real.shape().to_vec())
    }

    /// Lowpass filter using FFT. Zeroes frequencies above `cutoff_hz`.
    /// `sample_rate` is in Hz.
    pub fn lowpass(&self, cutoff_hz: f64, sample_rate: f64) -> CrowTensor {
        self.freq_filter(sample_rate, |freq| freq.abs() <= cutoff_hz)
    }

    /// Highpass filter using FFT. Zeroes frequencies below `cutoff_hz`.
    /// `sample_rate` is in Hz.
    pub fn highpass(&self, cutoff_hz: f64, sample_rate: f64) -> CrowTensor {
        self.freq_filter(sample_rate, |freq| freq.abs() >= cutoff_hz)
    }

    /// Bandpass filter using FFT. Keeps frequencies between `low_hz` and `high_hz`.
    /// `sample_rate` is in Hz.
    pub fn bandpass(&self, low_hz: f64, high_hz: f64, sample_rate: f64) -> CrowTensor {
        self.freq_filter(sample_rate, |freq| {
            let f = freq.abs();
            f >= low_hz && f <= high_hz
        })
    }

    /// Internal: Apply a frequency-domain filter via FFT → mask → IFFT.
    fn freq_filter<F: Fn(f64) -> bool>(&self, sample_rate: f64, keep: F) -> CrowTensor {
        assert!(self.shape().len() == 1, "filter: expected 1D tensor");
        let n = self.shape()[0];
        let data = self.to_vec();
        let mut buffer: Vec<Complex<f64>> = data.iter().map(|&v| Complex::new(v, 0.0)).collect();

        let mut planner = FftPlanner::new();
        let fft_fwd = planner.plan_fft_forward(n);
        fft_fwd.process(&mut buffer);

        // Zero out frequencies that don't pass the filter
        let freq_resolution = sample_rate / n as f64;
        for i in 0..n {
            let freq = if i <= n / 2 {
                i as f64 * freq_resolution
            } else {
                (i as f64 - n as f64) * freq_resolution
            };
            if !keep(freq) {
                buffer[i] = Complex::new(0.0, 0.0);
            }
        }

        let fft_inv = planner.plan_fft_inverse(n);
        fft_inv.process(&mut buffer);
        let result: Vec<f64> = buffer.iter().map(|c| c.re / n as f64).collect();
        CrowTensor::from_vec(result, vec![n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "expected {} ≈ {}, diff = {}", a, b, (a - b).abs());
    }

    #[test]
    fn test_fft_dc_signal() {
        // All ones → DC component = n, rest = 0
        let x = CrowTensor::from_vec(vec![1.0, 1.0, 1.0, 1.0], vec![4]);
        let (real, imag) = x.fft();
        approx_eq(real.to_vec()[0], 4.0, 1e-10);
        for i in 1..4 {
            approx_eq(real.to_vec()[i], 0.0, 1e-10);
            approx_eq(imag.to_vec()[i], 0.0, 1e-10);
        }
    }

    #[test]
    fn test_fft_ifft_roundtrip() {
        let x = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![4]);
        let (real, imag) = x.fft();
        let recovered = CrowTensor::ifft(&real, &imag);
        for (a, b) in x.to_vec().iter().zip(recovered.to_vec().iter()) {
            approx_eq(*a, *b, 1e-10);
        }
    }

    #[test]
    fn test_fft2_dc_signal() {
        let x = CrowTensor::from_vec(vec![1.0; 6], vec![2, 3]);
        let (real, imag) = x.fft2();
        assert_eq!(real.shape(), &[2, 3]);
        approx_eq(real.to_vec()[0], 6.0, 1e-10); // DC = sum of all elements
    }

    #[test]
    fn test_conv_simple() {
        // [1, 2, 3] * [1, 1] = [1, 3, 5, 3]
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![1.0, 1.0], vec![2]);
        let c = a.conv(&b);
        assert_eq!(c.shape(), &[4]);
        let v = c.to_vec();
        approx_eq(v[0], 1.0, 1e-10);
        approx_eq(v[1], 3.0, 1e-10);
        approx_eq(v[2], 5.0, 1e-10);
        approx_eq(v[3], 3.0, 1e-10);
    }

    #[test]
    fn test_conv_identity() {
        // Convolve with [1] = identity
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0], vec![3]);
        let b = CrowTensor::from_vec(vec![1.0], vec![1]);
        let c = a.conv(&b);
        assert_eq!(c.to_vec(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_conv2d_simple() {
        let a = CrowTensor::from_vec(vec![1.0, 2.0, 3.0, 4.0], vec![2, 2]);
        let k = CrowTensor::from_vec(vec![1.0, 0.0, 0.0, 1.0], vec![2, 2]);
        let c = a.conv2d(&k);
        assert_eq!(c.shape(), &[3, 3]);
    }

    #[test]
    fn test_hamming_endpoints() {
        let w = CrowTensor::hamming(5);
        assert_eq!(w.shape(), &[5]);
        let v = w.to_vec();
        approx_eq(v[0], 0.08, 1e-10);
        approx_eq(v[4], 0.08, 1e-10);
    }

    #[test]
    fn test_hanning_endpoints() {
        let w = CrowTensor::hanning(5);
        assert_eq!(w.shape(), &[5]);
        let v = w.to_vec();
        approx_eq(v[0], 0.0, 1e-10);
        approx_eq(v[4], 0.0, 1e-10);
    }

    #[test]
    fn test_hanning_peak() {
        let w = CrowTensor::hanning(5);
        approx_eq(w.to_vec()[2], 1.0, 1e-10); // Center should be 1.0
    }

    #[test]
    fn test_hamming_symmetry() {
        let w = CrowTensor::hamming(8);
        let v = w.to_vec();
        for i in 0..4 {
            approx_eq(v[i], v[7 - i], 1e-10);
        }
    }

    // ── stft ─────────────────────────────────────────────────────────

    #[test]
    fn test_stft_shape() {
        // 16 samples, window=4, hop=2 → (16-4)/2 + 1 = 7 frames
        let x = CrowTensor::from_vec(vec![1.0; 16], vec![16]);
        let (real, imag) = x.stft(4, 2);
        assert_eq!(real.shape()[0], 7); // num_frames
        assert_eq!(real.shape()[1], 4); // window_size
        assert_eq!(imag.shape(), real.shape());
    }

    #[test]
    fn test_stft_dc_signal() {
        // Constant signal → each frame's DC component should be non-zero
        let x = CrowTensor::from_vec(vec![1.0; 8], vec![8]);
        let (real, _imag) = x.stft(4, 2);
        let r = real.to_vec();
        // First element of each frame (DC) should be positive
        for frame in 0..real.shape()[0] {
            assert!(r[frame * 4] > 0.0, "DC component should be positive");
        }
    }

    // ── spectrogram ──────────────────────────────────────────────────

    #[test]
    fn test_spectrogram_shape() {
        let x = CrowTensor::from_vec(vec![1.0; 16], vec![16]);
        let spec = x.spectrogram(4, 2);
        assert_eq!(spec.shape()[0], 7);
        assert_eq!(spec.shape()[1], 4);
    }

    #[test]
    fn test_spectrogram_nonnegative() {
        let x = CrowTensor::from_vec(vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0], vec![8]);
        let spec = x.spectrogram(4, 2);
        for v in spec.to_vec() {
            assert!(v >= 0.0, "spectrogram values must be non-negative, got {}", v);
        }
    }

    // ── lowpass / highpass / bandpass ─────────────────────────────────

    #[test]
    fn test_lowpass_dc_preserved() {
        // Pure DC signal should pass through lowpass unchanged
        let x = CrowTensor::from_vec(vec![5.0; 8], vec![8]);
        let filtered = x.lowpass(100.0, 1000.0);
        for v in filtered.to_vec() {
            approx_eq(v, 5.0, 1e-10);
        }
    }

    #[test]
    fn test_highpass_removes_dc() {
        // Pure DC signal should be zeroed by highpass
        let x = CrowTensor::from_vec(vec![5.0; 8], vec![8]);
        let filtered = x.highpass(1.0, 1000.0);
        for v in filtered.to_vec() {
            approx_eq(v, 0.0, 1e-10);
        }
    }

    #[test]
    fn test_bandpass_preserves_in_band() {
        // Generate a pure tone at 100 Hz, sample rate 1000 Hz
        let n = 64;
        let sr = 1000.0;
        let freq = 100.0;
        let data: Vec<f64> = (0..n).map(|i| (2.0 * PI * freq * i as f64 / sr).sin()).collect();
        let x = CrowTensor::from_vec(data.clone(), vec![n]);
        let filtered = x.bandpass(50.0, 150.0, sr);
        // Energy should be mostly preserved
        let orig_energy: f64 = data.iter().map(|v| v * v).sum();
        let filt_energy: f64 = filtered.to_vec().iter().map(|v| v * v).sum();
        assert!(filt_energy > 0.9 * orig_energy, "in-band signal should be preserved, got ratio {}", filt_energy / orig_energy);
    }

    #[test]
    fn test_bandpass_removes_out_of_band() {
        // Generate a pure tone at 400 Hz, bandpass 50-150 Hz should remove it
        let n = 64;
        let sr = 1000.0;
        let freq = 400.0;
        let data: Vec<f64> = (0..n).map(|i| (2.0 * PI * freq * i as f64 / sr).sin()).collect();
        let orig_energy: f64 = data.iter().map(|v| v * v).sum();
        let x = CrowTensor::from_vec(data, vec![n]);
        let filtered = x.bandpass(50.0, 150.0, sr);
        let filt_energy: f64 = filtered.to_vec().iter().map(|v| v * v).sum();
        assert!(filt_energy < 0.1 * orig_energy, "out-of-band signal should be mostly removed, got ratio {}", filt_energy / orig_energy);
    }
}

