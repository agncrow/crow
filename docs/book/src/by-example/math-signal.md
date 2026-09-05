# Math: signal processing

```crow
use math

spec = fft[[1.0, 0.0, 0.0, 0.0]]
log[spec[0]]
log[spec[1]]
back = ifft[spec[0], spec[1]]
log[round[back[0]]]
f2 = fft2[[[1.0, 0.0], [0.0, 0.0]]]
log[f2[0]]
log[conv[[1.0, 2.0, 3.0], [1.0, 1.0]]]
log[conv2d[[[1.0, 1.0], [1.0, 1.0]], [[1.0]]]]
log[fmt[hamming[3][1], decimals = 2]]
log[fmt[hanning[3][1], decimals = 2]]

wave = linspace[0.0, 1.0, 64]
frames = stft[wave, 16, 8]
log[frames[0].shape]
log[spectrogram[wave, 16, 8].shape]
log[lowpass[wave, 5.0, 64.0].shape]
log[highpass[wave, 5.0, 64.0].shape]
log[bandpass[wave, 2.0, 10.0, 64.0].shape]
```

```text
[1, 1, 1, 1]
[0, 0, 0, 0]
1
[[1, 1], [1, 1]]
[1, 3, 5, 3]
[[1, 1], [1, 1]]
1.00
1.00
[7, 16]
[7, 16]
[64]
[64]
[64]
```

Reference: spec section 13 (math → Signal Processing).
