use std::f64::consts::LN_2;

use realfft::num_complex::Complex;

/// Mantissa bits of `2^(j/32)`
const EXP_TABLE: [i64; 33] = [
    0x0000000000000,
    0x059B0D3158574,
    0x0B5586CF9890F,
    0x11301D0125B51,
    0x172B83C7D517B,
    0x1D4873168B9AA,
    0x2387A6E756238,
    0x29E9DF51FDEE1,
    0x306FE0A31B715,
    0x371A7373AA9CB,
    0x3DEA64C123422,
    0x44E086061892D,
    0x4BFDAD5362A27,
    0x5342B569D4F82,
    0x5AB07DD485429,
    0x6247EB03A5585,
    0x6A09E667F3BCD,
    0x71F75E8EC5F74,
    0x7A11473EB0187,
    0x82589994CCE13,
    0x8ACE5422AA0DB,
    0x93737B0CDC5E5,
    0x9C49182A3F090,
    0xA5503B23E255D,
    0xAE89F995AD3AD,
    0xB7F76F2FB5E47,
    0xC199BDD85529C,
    0xCB720DCEF9069,
    0xD5818DCFBA487,
    0xDFC97337B9B5F,
    0xEA4AFA2A490DA,
    0xF50765B6E4540,
    0x10000000000000,
];

/// For each of the 32 mantissa buckets, the [`EXP_TABLE`] entry nearest its
/// centre, up to 31
const LOG_BUCKET_CENTERS: [i32; 32] = [
    1, 2, 3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 25, 26,
    27, 28, 29, 29, 30, 31, 31,
];

const ONE_BITS: i64 = 0x3ff0_0000_0000_0000;

/// Sine wave by recurrence, far cheaper per step than `sin`
pub(crate) struct SineGenerator {
    current: f64,
    previous: f64,
    multiplier: f64,
}

impl SineGenerator {
    pub(crate) fn new(increment: f64, phase: f64, gain: f64) -> Self {
        Self {
            current: phase.sin() * gain,
            previous: (phase - increment).sin() * gain,
            multiplier: 2.0 * increment.cos(),
        }
    }

    pub(crate) fn next_value(&mut self) -> f64 {
        let value = self.current;
        self.current = self.multiplier * value - self.previous;
        self.previous = value;
        value
    }
}

/// Bits that `value` takes, and `1` for `0`
pub(crate) fn bit_occupancy(value: usize) -> u32 {
    (usize::BITS - value.leading_zeros()).max(1)
}

/// `log2` of `factor`, if it is a power of two
pub(crate) fn exact_shift(factor: usize) -> Option<u32> {
    factor.is_power_of_two().then(|| factor.trailing_zeros())
}

pub(crate) fn normalize_fir(taps: &mut [f64], stride: usize, dc_gain: f64) {
    let sum: f64 = taps.iter().step_by(stride).sum();
    let scale = dc_gain / sum;
    for tap in taps.iter_mut().step_by(stride) {
        *tap *= scale;
    }
}

pub(crate) fn group_delay_at_dc(taps: &[f64]) -> f64 {
    const STEP: f64 = 1e-9;
    let phase_at_step = {
        let (re, im) = response(taps, STEP);
        im.atan2(re)
    };
    let phase_at_dc = {
        let (re, im) = response(taps, 0.0);
        im.atan2(re)
    };
    (phase_at_step - phase_at_dc) / STEP
}

/// Of `taps` at the circular frequency `theta`, as real and imaginary parts
fn response(taps: &[f64], theta: f64) -> (f64, f64) {
    let multiplier = 2.0 * theta.cos();
    let mut cos1 = 1.0;
    let mut sin1 = 0.0;
    let mut cos2 = (-theta).cos();
    let mut sin2 = (-theta).sin();
    let mut re = 0.0;
    let mut im = 0.0;
    for tap in taps {
        re += cos1 * tap;
        im += sin1 * tap;
        let next_cos = multiplier * cos1 - cos2;
        cos2 = cos1;
        cos1 = next_cos;
        let next_sin = multiplier * sin1 - sin2;
        sin2 = sin1;
        sin1 = next_sin;
    }
    (re, im)
}

/// Quadratic through eight equally spaced points, as the coefficients of `x^0`
/// to `x^2` between the fourth and the fifth point
pub(crate) fn spline_coefficients(points: [f64; 8]) -> [f64; 3] {
    const SCALE: f64 = 1.31578947368421052e-2;
    let [xm3, xm2, xm1, x0, x1, x2, x3, x4] = points;
    [
        x0,
        (61.0 * (x1 - xm1) + 16.0 * (xm2 - x2) + 3.0 * (x3 - xm3)) * SCALE,
        (106.0 * (xm1 + x1) + 10.0 * x3 + 6.0 * xm3 - 3.0 * x4 - 29.0 * (xm2 + x2) - 167.0 * x0)
            * SCALE,
    ]
}

/// Peak relative error about 2e-13 against `exp`, at a fraction of its cost.
/// A filter's table takes this many thousand times over.
pub(crate) fn exp_fast(x: f64) -> f64 {
    let y = x * 46.166241308446828 + 32736.0;
    if y < 32.0 {
        return 0.0;
    }
    if y >= 65504.0 {
        return f64::INFINITY;
    }
    let whole = y as i64;
    let fraction = y - whole as f64;
    let mantissa = EXP_TABLE[(whole & 31) as usize];
    let scale = f64::from_bits((((whole >> 5) << 52) | mantissa) as u64);
    let polynomial = 1.0
        + fraction
            * (2.1660849389716318e-02
                + fraction
                    * (2.3459622527899164e-04
                        + fraction * (1.6937691434532236e-06 + fraction * 9.2698881870148910e-09)));
    polynomial * scale
}

/// Peak relative error about 1.7e-10 against `ln`, for a positive normal `x`
pub(crate) fn log_fast(x: f64) -> f64 {
    let bits = x.to_bits() as i64;
    let mut exponent = (bits >> 52) as i32 - 1023;
    let significand = bits & 0x000f_ffff_ffff_ffff;

    let mut node = LOG_BUCKET_CENTERS[(significand >> 47) as usize];
    let doubled = significand << 1;
    let mut node_mantissa = EXP_TABLE[node as usize];
    let mut y_bits = ONE_BITS | significand;
    let below = EXP_TABLE[node as usize - 1];

    if doubled < below + node_mantissa {
        node -= 1;
        node_mantissa = below;
    } else {
        let above = EXP_TABLE[node as usize + 1];
        if doubled >= node_mantissa + above {
            node += 1;
            node_mantissa = above;
            if node == 32 {
                exponent += 1;
                // Lowers the exponent field by one, which halves y exactly
                y_bits ^= 0x0010_0000_0000_0000;
                node_mantissa = 0;
                node = 0;
            }
        }
    }

    let y = f64::from_bits(y_bits as u64);
    let t = f64::from_bits((ONE_BITS | node_mantissa) as u64);
    let z = (y - t) / (y + t);
    let z2 = z * z;
    let correction = z * (2.0 + z2 * (2.0 / 3.0));
    (f64::from(exponent) + f64::from(node) * 0.03125) * LN_2 + correction
}

/// `|value|` to the power of `power`
pub(crate) fn pow_abs(value: f64, power: f64) -> f64 {
    exp_fast(power * log_fast(value.abs() + 1e-300))
}

/// Zeroth-order modified Bessel function of the first kind
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let y = x * 2.6666666666666667e-1;
        let z = y * y;
        let y = z * z;
        return (1.0000000116e+00 + z * 3.5156234257e+00)
            + y * ((3.0899368029e+00 + z * 1.2067640177e+00)
                + y * ((2.6596186713e-01 + z * 3.6073913744e-02) + y * 4.5857130152e-03));
    }
    let y = 3.75 / ax;
    let z = y * y;
    exp_fast(ax) / ax.sqrt()
        * ((3.9894232649e-01 + y * 1.3293145671e-02)
            + z * ((2.1133548948e-03 + y * -5.8935267150e-04)
                + z * ((5.6653456233e-03 + y * -1.3682399954e-02)
                    + z * ((1.8689111903e-02 + y * -1.1978264040e-02) + z * 2.8410020052e-03))))
}

/// The minimum-phase filter of the magnitudes of `taps`, written over them and
/// at a gain to be normalized afterwards, through a discrete Hilbert transform
/// of the cepstrum. Returns its group delay at DC, in samples.
///
/// `length_multiplier` oversamples in frequency: too little smooths the
/// stop-band and misses a high attenuation.
pub(crate) fn minimum_phase_transform(taps: &mut [f64], length_multiplier: usize) -> f64 {
    // Around the FFT's rounding noise, and far under the deepest stop-band of
    // -218 dB. The original's 1e-300 assumes that no bin of the noise comes to
    // exactly zero, and rustfft's do: that one bin's logarithm then shifts the
    // phase of the whole filter.
    const BIAS: f64 = 1e-18;
    let len = 1 << bit_occupancy(taps.len() * length_multiplier - 1);
    let half = len >> 1;
    let fft = crate::fft::RealFft::new(len);
    let mut scratch = vec![Complex::default(); fft.scratch_len()];
    let mut time = vec![0.0; len];
    let mut spectrum = vec![Complex::default(); half + 1];
    time[..taps.len()].copy_from_slice(taps);
    fft.forward(&mut time, &mut spectrum, &mut scratch);

    let edge_gains = (spectrum[0].re, spectrum[half].re);
    spectrum[0] = Complex::new((spectrum[0].re.abs() + BIAS).ln(), 0.0);
    spectrum[half] = Complex::new((spectrum[half].re.abs() + BIAS).ln(), 0.0);
    let magnitudes: Vec<f64> = spectrum[1..half].iter().map(|bin| bin.norm()).collect();
    for (bin, magnitude) in spectrum[1..half].iter_mut().zip(&magnitudes) {
        *bin = Complex::new((magnitude + BIAS).ln(), 0.0);
    }

    fft.inverse(&mut spectrum, &mut time, &mut scratch);
    let scale = fft.inverse_scale();
    time[0] = 0.0;
    for sample in &mut time[1..half] {
        *sample *= scale;
    }
    time[half] = 0.0;
    for sample in &mut time[half + 1..] {
        *sample *= -scale;
    }

    fft.forward(&mut time, &mut spectrum, &mut scratch);
    for (bin, magnitude) in spectrum[1..half].iter_mut().zip(&magnitudes) {
        *bin = Complex::new(bin.im.cos() * magnitude, bin.im.sin() * magnitude);
    }
    spectrum[0] = Complex::new(edge_gains.0, 0.0);
    spectrum[half] = Complex::new(edge_gains.1, 0.0);
    fft.inverse(&mut spectrum, &mut time, &mut scratch);

    taps.copy_from_slice(&time[..taps.len()]);
    group_delay_at_dc(taps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_exp_and_log_stay_within_their_stated_errors() {
        for step in -700..700 {
            let x = f64::from(step) * 0.997;
            let relative = (exp_fast(x) - x.exp()).abs() / x.exp();
            assert!(relative < 3e-13, "exp({x}) off by {relative}");
        }
        for power in -300..300 {
            let x = 1.37 * 10f64.powi(power);
            let exact = x.ln();
            let relative = (log_fast(x) - exact).abs() / exact.abs().max(1e-3);
            assert!(relative < 1e-9, "ln({x}) off by {relative}");
        }
    }

    #[test]
    fn bit_occupancy_counts_significant_bits() {
        let counted = [0, 1, 2, 3, 255, 256, 1 << 20].map(bit_occupancy);
        assert_eq!(counted, [1, 1, 2, 2, 8, 9, 21]);
    }

    #[test]
    fn bessel_is_continuous_where_its_two_forms_meet() {
        let below = bessel_i0(3.75 - 1e-9);
        let above = bessel_i0(3.75 + 1e-9);
        assert!((below - above).abs() / below < 1e-6);
    }
}
