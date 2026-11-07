use std::f64::consts::PI;
use std::sync::Arc;

use realfft::num_complex::Complex;

use crate::Phase;
use crate::cache::Cache;
use crate::fft::RealFft;
use crate::math::{bit_occupancy, exact_shift, minimum_phase_transform, normalize_fir};
use crate::sinc::{self, KaiserParams};

pub(crate) const DEFAULT_CACHE_LIMIT: usize = 96;
const KAISER_BETA: f64 = 125.0;
/// Frequency oversampling of the minimum-phase transform
const MINIMUM_PHASE_LENGTH_MULTIPLIER: usize = 16;

static FILTERS: Cache<FirFilter> = Cache::new(DEFAULT_CACHE_LIMIT);

pub(crate) fn set_cache_limit(limit: usize) {
    FILTERS.set_limit(limit);
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Request {
    /// The start of the stop-band, as a fraction of the Nyquist frequency
    pub(crate) norm_freq: f64,
    /// In percent of the band up to `norm_freq`
    pub(crate) transition_band: f64,
    /// In decibels
    pub(crate) attenuation: f64,
    pub(crate) phase: Phase,
    pub(crate) gain: f64,
    pub(crate) down_factor: usize,
    pub(crate) extended_fft: bool,
}

/// Spectrum of a low-pass kernel, ready for overlap-save convolution
pub(crate) struct FirFilter {
    request: Request,
    /// Whole part, in samples. The kernel of a linear-phase filter is centred
    /// on sample 0, which leaves the convolver to skip this much by hand.
    pub(crate) latency: usize,
    pub(crate) latency_frac: f64,
    pub(crate) kernel_len: usize,
    pub(crate) block_len_bits: u32,
    kernel: Kernel,
    down_shift_nyquist: Complex<f64>,
}

/// Shortened by the down shift, if any, as the convolver's last FFT is too
enum Kernel {
    /// Real by its symmetry, and cheaper to apply so
    ZeroPhase(Vec<f64>),
    MinimumPhase(Vec<Complex<f64>>),
}

impl FirFilter {
    pub(crate) fn low_pass(request: Request) -> Arc<Self> {
        FILTERS.get_or_build(|filter| filter.request == request, || Self::build(request))
    }

    pub(crate) fn is_zero_phase(&self) -> bool {
        matches!(self.kernel, Kernel::ZeroPhase(_))
    }

    /// What the Nyquist bin of the down-shifted output takes, from the input's
    /// bin at that frequency. The FFT of the shorter output has no room for its
    /// imaginary part.
    pub(crate) fn nyquist_after_down_shift(&self, bin: Complex<f64>) -> f64 {
        self.down_shift_nyquist.re * bin.re - self.down_shift_nyquist.im * bin.im
    }

    pub(crate) fn apply(&self, spectrum: &mut [Complex<f64>]) {
        match &self.kernel {
            Kernel::ZeroPhase(kernel) => {
                for (bin, gain) in spectrum.iter_mut().zip(kernel) {
                    *bin = bin.scale(*gain);
                }
            }
            Kernel::MinimumPhase(kernel) => {
                for (bin, gain) in spectrum.iter_mut().zip(kernel) {
                    *bin *= *gain;
                }
            }
        }
    }

    fn build(request: Request) -> Self {
        let design = design(request.transition_band * 0.01, request.attenuation);
        let half_len = 0.25 * design.half_length / request.norm_freq;
        let whole_half_len = half_len.floor() as usize;
        let kernel_len = 2 * whole_half_len + 1;
        let block_len_bits = bit_occupancy(kernel_len - 1) + u32::from(request.extended_fft);
        let doubled_block_len = 2usize << block_len_bits;

        let mut block = vec![0.0; doubled_block_len];
        sinc::low_pass(
            &mut block[..kernel_len],
            half_len,
            PI * (1.0 - design.cutoff_offset) * request.norm_freq,
            KaiserParams {
                beta: KAISER_BETA,
                power: design.power,
            },
        );

        let fft = RealFft::new(doubled_block_len);
        let gain = fft.inverse_scale() * request.gain;
        let (latency, latency_frac) = match request.phase {
            Phase::Linear => {
                center_on_zero(&mut block, whole_half_len, gain);
                (whole_half_len, 0.0)
            }
            Phase::Minimum => {
                let delay = minimum_phase_transform(
                    &mut block[..kernel_len],
                    MINIMUM_PHASE_LENGTH_MULTIPLIER,
                );
                normalize_fir(&mut block[..kernel_len], 1, gain);
                (delay as usize, delay - delay.trunc())
            }
        };

        let mut spectrum = vec![Complex::default(); doubled_block_len / 2 + 1];
        let mut scratch = vec![Complex::default(); fft.scratch_len()];
        fft.forward(&mut block, &mut spectrum, &mut scratch);

        let down_shift = exact_shift(request.down_factor).unwrap_or(0);
        let nyquist_bin = (doubled_block_len >> down_shift) / 2;
        let down_shift_nyquist = if down_shift > 0 {
            spectrum[nyquist_bin]
        } else {
            Complex::default()
        };
        spectrum.truncate(nyquist_bin + 1);
        let kernel = match request.phase {
            Phase::Linear => Kernel::ZeroPhase(spectrum.iter().map(|bin| bin.re).collect()),
            Phase::Minimum => Kernel::MinimumPhase(spectrum),
        };

        Self {
            request,
            latency,
            latency_frac,
            kernel_len,
            block_len_bits,
            kernel,
            down_shift_nyquist,
        }
    }
}

/// Moves the kernel in `block`, of `half_len * 2 + 1` taps, so that it wraps
/// around sample 0, at a DC gain of `gain`.
fn center_on_zero(block: &mut [f64], half_len: usize, gain: f64) {
    let len = block.len();
    let sum: f64 = block[..=2 * half_len].iter().sum();
    let scale = gain / sum;
    for index in 0..=half_len {
        block[index] = block[half_len + index] * scale;
    }
    for index in 1..=half_len {
        block[len - index] = block[index];
    }
    block[half_len + 1..len - half_len].fill(0.0);
}

struct Design {
    /// To raise the Kaiser window to, making its effective beta `125 * power`
    power: f64,
    half_length: f64,
    /// How far the sinc's cutoff lies under the start of the stop-band, as a
    /// fraction of the band up to there
    cutoff_offset: f64,
}

/// Closed form of what the original fitted to exact solutions, for a transition
/// band of 0.005 to 0.45 and an attenuation of 49 to 218 dB.
fn design(transition_band: f64, attenuation: f64) -> Design {
    const BETA: [f64; 10] = [
        14.409935832973984,
        9.6626843693138884,
        -0.17435120318333536,
        0.47264599329014079,
        0.4940426137302133,
        -1.5662364718692721,
        -0.71006021971136335,
        2.0294051576357957,
        0.22363464796569277,
        -0.78254591651630068,
    ];
    const DELTA_0: [f64; 7] = [
        -1.0753246533566314,
        -0.75662395688736628,
        -0.0047894065672392377,
        0.08577566962378394,
        -0.17001888073403126,
        -0.013628259555023822,
        0.093950306915379783,
    ];
    const DELTA_2: [f64; 3] = [
        1.1482209047708865,
        -0.052152189285352819,
        -0.020187508756994623,
    ];
    const DELTA_3: [f64; 3] = [
        -1.4201039645189557,
        0.1242900889796041,
        -0.028854770456694875,
    ];
    const DELTA_4: [f64; 3] = [
        0.55500996383110457,
        -0.062635292370927881,
        0.023661985245151548,
    ];
    const Y_0: [f64; 5] = [
        0.8399122624879678,
        0.39839640583388353,
        0.015028625919905413,
        -0.005569192458538463,
        0.001494097359614131,
    ];
    const Y_2: [f64; 3] = [
        0.00069542490312080887,
        -0.00062890127106954123,
        -0.00071460219184551251,
    ];
    const Y_3: [f64; 2] = [-0.00065713366734133005, 0.0011106064299533634];

    let t = transition_band;
    let a = (attenuation - 133.5) / 84.5;
    let b = polynomial(&BETA, a) + 0.0523 + 4.07 * t * t;
    let u = (b.ln() - 2.3555) / 0.8585;

    let s = (t / 0.45) * (t / 0.45);
    let s2 = s * s;
    let s3 = s2 * s;
    let s4 = s2 * s2;

    let delta = (polynomial(&DELTA_0, u)
        + s2 * polynomial(&DELTA_2, u)
        + s3 * polynomial(&DELTA_3, u)
        + s4 * polynomial(&DELTA_4, u))
    .exp();
    let y = (polynomial(&Y_0, u) + s2 * polynomial(&Y_2, u) + s3 * polynomial(&Y_3, u)).exp();
    let x = (4.0 / PI) * ((b * b + PI * PI).sqrt() - delta);

    Design {
        power: b / KAISER_BETA,
        half_length: (x + y) / t,
        cutoff_offset: x * t / (x + y),
    }
}

/// `coefficients` by ascending power
fn polynomial(coefficients: &[f64], x: f64) -> f64 {
    coefficients.iter().rev().fold(0.0, |sum, c| sum * x + c)
}
