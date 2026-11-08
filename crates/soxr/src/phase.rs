//! Minimum, intermediate and maximum phase, from a linear-phase filter by its
//! real cepstrum
//!
//! Centred on 0, a linear-phase filter has a real spectrum `A`, which changes
//! sign at each zero of the stop-band. Those zeros lie on the unit circle,
//! where the logarithm diverges and its cepstrum aliases however long the
//! transform, so they are found and taken out first: `A = Z · Q`, where `Z` is
//! the product of `2 (cos ωᵢ − cos ω)` over the zeros and `Q` has none on the
//! circle. The phase `φ` of the minimum-phase `Q` then comes from the cepstrum
//! of `log |Q|`. `A · e^(jφ)` is the minimum-phase filter, ahead by as many
//! samples as there are zeros, as `Z` has linear phase; `A · e^(jαφ)` goes
//! part of the way, with the magnitude kept exactly at every `α`.
//!
//! The deep stop-bands ask for more than `f64` gives without care: the zeros
//! are refined in double-double arithmetic, and the spectrum is averaged over
//! transforms that round differently.

use std::f64::consts::{PI, TAU};
use std::ops::Range;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealToComplex};

use crate::real::Real;

/// The phase response that leaves a filter as it is
pub(crate) const LINEAR: f64 = 50.0;
/// The cepstrum's transform per tap, rounded up to a power of two
const TRANSFORM_PER_TAP: usize = 8;
/// How far under the stop-band what is cut from the ends may reach
const CUT_DB: f64 = 12.0;
/// Factors of `Z` multiplied before a logarithm, few enough to stay in range
const FACTORS_PER_LOG: usize = 16;
/// Bins nearer a zero than this, in bins, take their logarithm from around them
const NEAREST_TO_ZERO: f64 = 0.05;
/// About how much of the pass-band a transform rounds each bin by
const ROUNDING: f64 = 2e-16;
/// How much of the stop-band the rounding of the spectrum may be, after
/// averaging, for the phase to come out smooth
const QUIET_ROUNDING: f64 = 4e-7;
/// The most transforms averaged
const MOST_SHIFTS: usize = 256;
/// How much finer than the transform the spectrum of what is cut is checked
const CHECK_GRID: usize = 4;
/// How much lower each threshold for cutting is than the one before
const THRESHOLD_STEP: f64 = 4.0;
/// The most steps for each zero
const MOST_ZERO_STEPS: usize = 60;
/// The most of those in `f64`
const MOST_PLAIN_STEPS: usize = 8;
/// How small a step in `f64`, in bins, hands the search to double-double
const PLAIN_SETTLED: f64 = 1e-6;
/// How small a step in double-double, in bins, ends the search
const SETTLED: f64 = 1e-9;
/// A bound on the rounding of the spectrum summed in `f64`, per tap and unit
/// of their magnitudes
const PLAIN_ROUNDING: f64 = 4.0 * f64::EPSILON;

/// `taps` of an odd-length linear-phase filter with a gain of 1 at 0 Hz,
/// moved `response` percent of the way from minimum phase (0) through linear
/// (50) to maximum phase (100) with the same magnitude, to well under
/// `attenuation` dB; and the position of its peak
pub(crate) fn shaped(taps: &[f64], response: f64, attenuation: f64) -> (Vec<f64>, usize) {
    let centre = taps.len() / 2;
    if response == LINEAR {
        return (taps.to_vec(), centre);
    }
    let mirrored = response > LINEAR;
    let minimum_share = 1.0 - response.min(100.0 - response) / LINEAR;
    let len = taps.len().next_power_of_two() * TRANSFORM_PER_TAP;
    let (forward, inverse) = f64::plan(len);
    let stop_band = 10f64.powf(-attenuation / 20.0);
    let shifts =
        ((ROUNDING / stop_band / QUIET_ROUNDING).powi(2).ceil() as usize).clamp(1, MOST_SHIFTS);
    let amplitude = amplitude(taps, shifts, &*forward);
    let zeros = zeros(&taps[centre..], &amplitude);
    let phase = minimum_phase(&amplitude, &zeros, &*forward, &*inverse);
    let mut shaped: Vec<Complex<f64>> = amplitude
        .iter()
        .zip(&phase)
        .map(|(amplitude, phase)| Complex::from_polar(*amplitude, minimum_share * phase))
        .collect();
    zero_edges(&mut shaped);
    let mut time = vec![0.0; len];
    inverse
        .process(&mut shaped, &mut time)
        .expect("the buffers are the transform's own");
    time.rotate_right(len / 2);
    let budget = 10f64.powf(-CUT_DB / 20.0) * stop_band * len as f64;
    let kept = &time[kept_range(&time, budget)];
    let sum: f64 = kept.iter().sum();
    let mut taps: Vec<f64> = kept.iter().map(|sample| sample / sum).collect();
    if mirrored {
        taps.reverse();
    }
    let peak = taps
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(0, |(index, _)| index);
    (taps, peak)
}

/// The real spectrum of `taps` centred on 0, as the mean over transforms of
/// the taps at `shifts` shifts, which round differently
fn amplitude(taps: &[f64], shifts: usize, forward: &dyn RealToComplex<f64>) -> Vec<f64> {
    let len = forward.len();
    let centre = taps.len() / 2;
    let mut sum = vec![0.0; len / 2 + 1];
    let mut time = vec![0.0; len];
    let mut spectrum = forward.make_output_vec();
    for shift in 0..shifts {
        time.fill(0.0);
        for (index, tap) in taps.iter().enumerate() {
            time[(index + len + shift - centre) % len] = *tap;
        }
        transform(forward, &mut time, &mut spectrum);
        for (bin, (sum, value)) in sum.iter_mut().zip(&spectrum).enumerate() {
            let turn = TAU * ((bin * shift) % len) as f64 / len as f64;
            *sum += (value * Complex::from_polar(1.0, turn)).re;
        }
    }
    sum.iter().map(|sum| sum / shifts as f64).collect()
}

/// The zeros in `(0, π)` of the spectrum whose bins are `amplitude` and whose
/// taps from the centre on are `half`, in bins
fn zeros(half: &[f64], amplitude: &[f64]) -> Vec<f64> {
    let bin_width = PI / (amplitude.len() - 1) as f64;
    amplitude
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| (pair[0] < 0.0) != (pair[1] < 0.0))
        .map(|(bin, pair)| {
            let low = bin as f64 * bin_width;
            let guess = low + bin_width * pair[0] / (pair[0] - pair[1]);
            let bracket = Bracket {
                low,
                high: low + bin_width,
                low_negative: pair[0] < 0.0,
            };
            refined(half, guess, bracket) / bin_width
        })
        .collect()
}

/// Where a zero lies, and which side of it is negative
#[derive(Clone, Copy, Debug)]
struct Bracket {
    low: f64,
    high: f64,
    low_negative: bool,
}

/// The zero in `bracket`, from `guess`, by Newton's method falling back to
/// halving the bracket: in `f64` until rounding takes over, and then in
/// double-double until it settles
fn refined(half: &[f64], guess: f64, mut bracket: Bracket) -> f64 {
    let width = bracket.high - bracket.low;
    let rounding =
        PLAIN_ROUNDING * half.len() as f64 * half.iter().map(|tap| tap.abs()).sum::<f64>();
    let mut omega = guess;
    let mut double = false;
    let mut previous: Option<(f64, f64)> = None;
    for step in 0..MOST_ZERO_STEPS {
        let value = if double {
            amplitude_at_double(half, omega)
        } else {
            amplitude_at(half, omega)
        };
        if value == 0.0 {
            break;
        }
        let trusted = double || value.abs() > rounding;
        if trusted {
            if (value < 0.0) == bracket.low_negative {
                bracket.low = omega;
            } else {
                bracket.high = omega;
            }
        }
        // Near 0 and π the recurrence for the slope rounds badly, so once the
        // values are exact the slope comes from the last two
        let slope = match previous {
            Some((before, at_before)) if before != omega => (value - at_before) / (omega - before),
            _ => slope_at(half, omega),
        };
        let newton = omega - value / slope;
        let moved = (newton - omega).abs();
        if double {
            if moved <= SETTLED * width {
                return newton;
            }
            previous = Some((omega, value));
        } else if !trusted || moved <= PLAIN_SETTLED * width || step >= MOST_PLAIN_STEPS {
            double = true;
        }
        omega = if (bracket.low..=bracket.high).contains(&newton) {
            newton
        } else {
            0.5 * (bracket.low + bracket.high)
        };
    }
    omega
}

/// The real spectrum at `omega`, from the taps from the centre on, by
/// Clenshaw's recurrence
fn amplitude_at(half: &[f64], omega: f64) -> f64 {
    let x = omega.cos();
    let (mut next, mut after) = (0.0, 0.0);
    for tap in half[1..].iter().rev() {
        let value = 2.0 * x * next - after + 2.0 * tap;
        after = next;
        next = value;
    }
    x * next - after + half[0]
}

/// The real spectrum at `omega`, from the taps from the centre on, by
/// Clenshaw's recurrence in double-double arithmetic. In plain `f64` the sum
/// rounds to about 1e-14, more than the deepest stop-bands, which would put
/// their zeros a hundredth of a bin out.
fn amplitude_at_double(half: &[f64], omega: f64) -> f64 {
    let x = omega.cos();
    let (mut next, mut after) = (Double::ZERO, Double::ZERO);
    for tap in half[1..].iter().rev() {
        let value = next.times(2.0 * x) - after + Double::from(2.0 * tap);
        after = next;
        next = value;
    }
    (next.times(x) - after + Double::from(half[0])).value()
}

/// The derivative of the real spectrum at `omega`, by Clenshaw's recurrence
/// for `sin(nω) = sin ω · U₍ₙ₋₁₎(cos ω)`
fn slope_at(half: &[f64], omega: f64) -> f64 {
    let x = omega.cos();
    let (mut next, mut after) = (0.0, 0.0);
    for (index, tap) in half.iter().enumerate().skip(1).rev() {
        let value = 2.0 * x * next - after + 2.0 * index as f64 * tap;
        after = next;
        next = value;
    }
    -omega.sin() * next
}

/// For each bin, the phase of the minimum-phase filter with the magnitude of
/// `amplitude` over the factors of its `zeros`, given in bins
fn minimum_phase(
    amplitude: &[f64],
    zeros: &[f64],
    forward: &dyn RealToComplex<f64>,
    inverse: &dyn ComplexToReal<f64>,
) -> Vec<f64> {
    let len = forward.len();
    let bin_width = TAU / len as f64;
    let cosines: Vec<f64> = zeros.iter().map(|zero| (zero * bin_width).cos()).collect();
    let mut log_q: Vec<f64> = amplitude
        .iter()
        .enumerate()
        .map(|(bin, amplitude)| {
            let cos = (bin as f64 * bin_width).cos();
            let factors: f64 = cosines
                .chunks(FACTORS_PER_LOG)
                .map(|chunk| {
                    let product: f64 = chunk.iter().map(|zero| 2.0 * (zero - cos)).product();
                    product.abs().max(f64::MIN_POSITIVE).ln()
                })
                .sum();
            amplitude.abs().max(f64::MIN_POSITIVE).ln() - factors
        })
        .collect();
    // A bin almost on a zero divides one rounding by another, so it takes the
    // smooth logarithm around it instead
    for zero in zeros {
        let bin = zero.round() as usize;
        if (zero - bin as f64).abs() < NEAREST_TO_ZERO && (2..log_q.len() - 2).contains(&bin) {
            log_q[bin] =
                (4.0 * (log_q[bin - 1] + log_q[bin + 1]) - log_q[bin - 2] - log_q[bin + 2]) / 6.0;
        }
    }
    let mut log: Vec<Complex<f64>> = log_q.iter().map(|log| Complex::new(*log, 0.0)).collect();
    let mut cepstrum = vec![0.0; len];
    inverse
        .process(&mut log, &mut cepstrum)
        .expect("the buffers are the transform's own");
    // Causal, so that the transform's imaginary part is the minimum phase. The
    // fold tapers over the second quarter, as a cut at half the transform
    // would bring the rounding left there back as an echo.
    let scale = 1.0 / len as f64;
    for (index, value) in cepstrum.iter_mut().enumerate() {
        *value *= if index == 0 {
            scale
        } else if index < len / 2 {
            2.0 * scale * taper(index, len / 4)
        } else {
            0.0
        };
    }
    let mut spectrum = forward.make_output_vec();
    transform(forward, &mut cepstrum, &mut spectrum);
    spectrum.iter().map(|log| log.im).collect()
}

/// 1 up to `reach`, falling to 0 at twice that along half a cosine
fn taper(index: usize, reach: usize) -> f64 {
    let past = index.saturating_sub(reach) as f64 / reach as f64;
    if past >= 1.0 {
        0.0
    } else {
        0.5 + 0.5 * (PI * past).cos()
    }
}

/// The shortest run of `samples` past whose ends nothing is over a threshold,
/// and whose leaving out changes their spectrum by at most `budget`. That is
/// checked on a grid finer than the transform, as the rounding left in the
/// deepest stop-bands adds up to far less than its magnitudes.
fn kept_range(samples: &[f64], budget: f64) -> Range<usize> {
    let len = samples.len();
    let (forward, _) = f64::plan(CHECK_GRID * len);
    let mut time = vec![0.0; CHECK_GRID * len];
    let mut spectrum = forward.make_output_vec();
    // Between the points of the grid the spectrum reaches up to
    // 1 / cos(π / (2 · CHECK_GRID)) times higher
    let limit = budget * (PI / (2 * CHECK_GRID) as f64).cos();
    let mut threshold = budget;
    loop {
        let start = samples
            .iter()
            .position(|sample| sample.abs() > threshold)
            .unwrap_or(0);
        let end = samples
            .iter()
            .rposition(|sample| sample.abs() > threshold)
            .map_or(len, |last| last + 1);
        if start == 0 && end == len {
            return 0..len;
        }
        time.fill(0.0);
        time[..start].copy_from_slice(&samples[..start]);
        time[end..len].copy_from_slice(&samples[end..]);
        transform(&*forward, &mut time, &mut spectrum);
        if spectrum.iter().all(|bin| bin.norm() <= limit) {
            return start..end;
        }
        threshold /= THRESHOLD_STEP;
    }
}

fn transform(forward: &dyn RealToComplex<f64>, time: &mut [f64], spectrum: &mut [Complex<f64>]) {
    forward
        .process(time, spectrum)
        .expect("the buffers are the transform's own");
}

/// The inverse transform takes no imaginary part at 0 Hz or the Nyquist
/// frequency
fn zero_edges(spectrum: &mut [Complex<f64>]) {
    if let Some(first) = spectrum.first_mut() {
        first.im = 0.0;
    }
    if let Some(last) = spectrum.last_mut() {
        last.im = 0.0;
    }
}

/// A number as the unevaluated sum of two `f64`s, which carries about 106 bits
#[derive(Clone, Copy, Debug)]
struct Double {
    high: f64,
    low: f64,
}

impl Double {
    const ZERO: Self = Self {
        high: 0.0,
        low: 0.0,
    };

    fn value(self) -> f64 {
        self.high + self.low
    }

    /// The product with an `f64`, to about 106 bits
    fn times(self, factor: f64) -> Self {
        let high = self.high * factor;
        let error = self.high.mul_add(factor, -high);
        two_sum(high, self.low.mul_add(factor, error))
    }
}

impl From<f64> for Double {
    fn from(value: f64) -> Self {
        Self {
            high: value,
            low: 0.0,
        }
    }
}

/// Knuth's two-sum: `a + b` exactly, as the rounded sum and its error
fn two_sum(a: f64, b: f64) -> Double {
    let high = a + b;
    let b_part = high - a;
    let low = (a - (high - b_part)) + (b - b_part);
    Double { high, low }
}

impl std::ops::Add for Double {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let sum = two_sum(self.high, other.high);
        two_sum(sum.high, sum.low + self.low + other.low)
    }
}

impl std::ops::Sub for Double {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self + Self {
            high: -other.high,
            low: -other.low,
        }
    }
}

#[cfg(test)]
mod tests;
