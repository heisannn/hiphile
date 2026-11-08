//! The stages of a constant ratio, and one channel's run through them
//!
//! Upsampling doubles the rate with the steep filter, by FFT, and then
//! interpolates to the output rate. Downsampling halves the rate in short
//! steps until the ratio is 4 or less, interpolates to twice the output rate,
//! and then halves it with the steep filter. Either way the steep filter runs
//! at twice the lower rate, where the signal fills only half of the band, and
//! the interpolation then needs only a short filter.

use std::sync::Arc;

use crate::clock::Ratio;
use crate::design::{self, Kernel};
use crate::fft_stage::{Factor, FftFilter, FftStage};
use crate::phase;
use crate::poly::{CYCLE_BYTES, Layout, PolyFilter, PolyStage};
use crate::real::Real;
use crate::{CoefInterpolation, RuntimeSpec};

/// Above this ratio, downsampling first halves the rate in short steps
const MOST_INTERPOLATED_DOWN: f64 = 4.0;
/// How much further down the interpolating filters reach than the steep one,
/// so that they add little to its error
const INTERPOLATION_MARGIN_DB: f64 = 10.0;
/// The transform of the steep filter is the power of two nearest this many
/// times its length, unless the runtime spec says otherwise. Longer takes
/// more latency. In `f64` twice as long costs 3 to 10 % less at VHQ and
/// 32-bit. In `f32` it would round more, by up to 0.6 dB at HQ, where the
/// rounding lies near the precision.
const DFT_PER_TAP_32: usize = 4;
const DFT_PER_TAP_64: usize = 8;
/// How many samples a unit of time takes to find the gain of a kernel at 0 Hz
pub(crate) const DC_SAMPLES: u32 = 8;
/// The most memory that [`CoefInterpolation::Low`] takes for its table. At
/// high precisions it would take far more, where cubic interpolation keeps
/// the same precision in a much smaller table.
pub(crate) const MOST_LINEAR_BYTES: usize = 8 << 20;

/// What a constant ratio filters with, from a checked quality spec
#[derive(Clone, Copy, Debug)]
pub(crate) struct Filtering {
    pub(crate) precision: f64,
    pub(crate) passband_end: f64,
    pub(crate) stopband_begin: f64,
    pub(crate) phase_response: f64,
}

/// One design of the stages, which every channel shares
pub(crate) enum StageDesign<T> {
    Fft(Arc<FftFilter<T>>),
    Poly(Arc<PolyFilter<T>>),
    /// Keeps back the last this many samples until more come
    Hold(usize),
}

impl<T: Real> StageDesign<T> {
    fn instantiate(&self) -> Stage<T> {
        match self {
            Self::Fft(filter) => Stage::Fft(FftStage::new(Arc::clone(filter))),
            Self::Poly(filter) => Stage::Poly(Box::new(PolyStage::new(Arc::clone(filter)))),
            Self::Hold(len) => Stage::Hold(Hold {
                len: *len,
                held: Vec::new(),
            }),
        }
    }
}

pub(crate) fn design<T: Real>(
    ratio: Ratio,
    filtering: &Filtering,
    runtime: &RuntimeSpec,
) -> Vec<StageDesign<T>> {
    if ratio.is_one() {
        return Vec::new();
    }
    if filtering.precision == 0.0 {
        let budget = runtime.coef_size_kbytes.saturating_mul(1024);
        return vec![StageDesign::Poly(Arc::new(PolyFilter::cubic(
            ratio, budget,
        )))];
    }
    let builder = Builder {
        filtering,
        runtime,
        attenuation: design::attenuation(filtering.precision),
        element_size: size_of::<T>(),
    };
    if ratio.value < 1.0 {
        builder.upsampling(ratio)
    } else {
        builder.downsampling(ratio)
    }
}

struct Builder<'a> {
    filtering: &'a Filtering,
    runtime: &'a RuntimeSpec,
    attenuation: f64,
    element_size: usize,
}

impl Builder<'_> {
    fn upsampling<T: Real>(&self, ratio: Ratio) -> Vec<StageDesign<T>> {
        let mut stages = vec![self.steep(Factor::Up2)];
        let step = ratio.scaled(2, 1);
        if !step.is_one() {
            // At twice the input rate, in cycles per sample
            let band = self.filtering.stopband_begin / 4.0;
            stages.push(self.interpolation(band, 1.0 - band, step));
        }
        stages
    }

    fn downsampling<T: Real>(&self, ratio: Ratio) -> Vec<StageDesign<T>> {
        let mut stages = Vec::new();
        // The band to keep, in units of the output rate
        let band = self.filtering.stopband_begin / 2.0;
        let mut ratio = ratio;
        while ratio.value > MOST_INTERPOLATED_DOWN {
            let rate = ratio.value;
            stages.push(self.interpolation(
                band / rate,
                (rate / 2.0 - band) / rate,
                Ratio {
                    value: 2.0,
                    fraction: Some((2, 1)),
                },
            ));
            ratio = ratio.scaled(1, 2);
        }
        let step = ratio.scaled(1, 2);
        if !step.is_one() {
            let rate = ratio.value;
            stages.push(self.interpolation(band / rate, (2.0 - band) / rate, step));
        }
        stages.push(self.steep(Factor::Down2));
        stages
    }

    /// At twice the lower rate, where the lower Nyquist frequency is a
    /// quarter of the rate. It alone takes the phase response, as the
    /// interpolating filters are short and stay linear-phase.
    fn steep<T: Real>(&self, factor: Factor) -> StageDesign<T> {
        let passband_end = self.filtering.passband_end / 4.0;
        let stopband_begin = self.filtering.stopband_begin / 4.0;
        let kernel = Kernel::low_pass(passband_end, stopband_begin, self.attenuation, 1.0);
        let taps = kernel.with_half_len(kernel.half_len().ceil()).taps();
        let (taps, delay) = phase::shaped(&taps, self.filtering.phase_response, self.attenuation);
        let len = self.dft_len(taps.len());
        StageDesign::Fft(Arc::new(FftFilter::new(factor, &taps, delay, len)))
    }

    fn dft_len(&self, taps: usize) -> usize {
        let least = (2 * taps).next_power_of_two();
        let per_tap = if self.element_size == size_of::<f32>() {
            DFT_PER_TAP_32
        } else {
            DFT_PER_TAP_64
        };
        let nearest = 1usize << ((per_tap * taps) as f64).log2().round() as u32;
        let (min_size, large_size) = self.runtime.dft_sizes();
        let wanted = nearest.max(1 << min_size).min(1 << large_size);
        wanted.max(least)
    }

    /// A short filter that keeps `[0, passband_end]` and removes from
    /// `stopband_begin` on, in cycles per input sample
    fn interpolation<T: Real>(
        &self,
        passband_end: f64,
        stopband_begin: f64,
        step: Ratio,
    ) -> StageDesign<T> {
        let attenuation = self.attenuation + INTERPOLATION_MARGIN_DB;
        let kernel = Kernel::low_pass(passband_end, stopband_begin, attenuation, 1.0);
        let len = (2.0 * kernel.half_len()).ceil() as usize;
        let len = len.next_multiple_of(2).max(2);
        let kernel = kernel.with_half_len(len as f64 / 2.0);
        let kernel = kernel.with_gain(1.0 / kernel.dc_gain(DC_SAMPLES));
        let layout = self.layout(len, step, stopband_begin);
        StageDesign::Poly(Arc::new(PolyFilter::new(&kernel, len, step, layout)))
    }

    /// For [`CoefInterpolation::Auto`], exact rows if they fit in the runtime
    /// spec's memory. Otherwise as few segments as keep the interpolation's
    /// error under the precision.
    fn layout(&self, len: usize, step: Ratio, highest_frequency: f64) -> Layout {
        let budget = self.runtime.coef_size_kbytes.saturating_mul(1024);
        let interpolation = self.runtime.coef_interpolation;
        if interpolation == CoefInterpolation::Auto
            && let Some((_, denominator)) = step.fraction
            && usize::try_from(denominator)
                .ok()
                .and_then(|rows| rows.checked_mul(len * self.element_size + CYCLE_BYTES))
                .is_some_and(|bytes| bytes <= budget)
        {
            return Layout::Exact;
        }
        // Relative to the kernel's peak, and under the rounding of `f32` at HQ
        let error = 2f64.powf(-(self.filtering.precision + 4.0));
        let omega = std::f64::consts::TAU * highest_frequency;
        let linear = (omega / (8.0 * error).sqrt()).max(2.0).log2().ceil() as u32;
        let cubic = (omega * (0.0234 / error).powf(0.25)).max(2.0).log2().ceil() as u32;
        let linear_bytes = (1usize << linear) * 2 * len * size_of::<f64>();
        let order = match interpolation {
            CoefInterpolation::Low if linear_bytes <= MOST_LINEAR_BYTES => 1,
            CoefInterpolation::Auto if linear_bytes <= budget => 1,
            _ => 3,
        };
        let segments_log2 = if order == 1 { linear } else { cubic };
        Layout::Interpolated {
            segments_log2,
            order,
        }
    }
}

enum Stage<T> {
    Fft(FftStage<T>),
    Poly(Box<PolyStage<T>>),
    Hold(Hold<T>),
}

struct Hold<T> {
    len: usize,
    held: Vec<T>,
}

impl<T: Real> Hold<T> {
    fn push(&mut self, input: &[T], out: &mut Vec<T>) {
        self.held.extend_from_slice(input);
        let released = self.held.len().saturating_sub(self.len);
        out.extend(self.held.drain(..released));
    }
}

/// One channel's run through the stages
pub(crate) struct Pipeline<T> {
    stages: Vec<Stage<T>>,
    buffers: [Vec<T>; 2],
    flush: Option<Flush>,
}

/// After the end of the input: the stage that zeros go into, and how many it
/// has taken
#[derive(Clone, Copy, Debug)]
struct Flush {
    stage: usize,
    zeros: usize,
}

impl<T: Real> Pipeline<T> {
    pub(crate) fn new(designs: &[StageDesign<T>]) -> Self {
        Self {
            stages: designs.iter().map(StageDesign::instantiate).collect(),
            buffers: [Vec::new(), Vec::new()],
            flush: None,
        }
    }

    /// Appends to `out` the output that `input` completes, except that an
    /// interpolation at the end keeps its input until [`Self::produce`] asks
    /// for output, as one block of a steep filter can make millions of
    /// samples at a large ratio
    pub(crate) fn push(&mut self, input: &[T], out: &mut Vec<T>) {
        self.push_from(0, input, out);
    }

    /// As [`Self::push`] with `len` zeros, past the end of the input. Once a
    /// stage holds nothing but zeros, its output is +0.0 to the bit, as its
    /// sums start at +0.0 and adding a zero of either sign leaves them alone.
    /// So zeros go straight to the first stage that still holds input, which
    /// at a large ratio saves the earlier stages from millions of them.
    pub(crate) fn push_zeros(&mut self, len: usize, out: &mut Vec<T>) {
        let mut flush = self.flush.unwrap_or(Flush { stage: 0, zeros: 0 });
        while flush.stage + 1 < self.stages.len()
            && flush.zeros >= self.stages[flush.stage].memory()
        {
            flush = Flush {
                stage: flush.stage + 1,
                zeros: 0,
            };
        }
        let zeros = vec![T::ZERO; len];
        self.push_from(flush.stage, &zeros, out);
        flush.zeros += len;
        self.flush = Some(flush);
    }

    fn push_from(&mut self, first: usize, input: &[T], out: &mut Vec<T>) {
        let Some((last, rest)) = self.stages[first..].split_last_mut() else {
            out.extend_from_slice(input);
            return;
        };
        let [front, back] = &mut self.buffers;
        let mut source: &[T] = input;
        for stage in rest {
            back.clear();
            stage.push(source, back);
            std::mem::swap(front, back);
            source = front;
        }
        match last {
            Stage::Poly(stage) => stage.feed(source),
            stage => stage.push(source, out),
        }
    }

    /// Appends up to `most` samples of output that the input so far makes
    pub(crate) fn produce(&mut self, most: usize, out: &mut Vec<T>) {
        if let Some(Stage::Poly(stage)) = self.stages.last_mut() {
            stage.produce(most, out);
        }
    }

    pub(crate) fn clear(&mut self) {
        for stage in &mut self.stages {
            match stage {
                Stage::Fft(stage) => stage.clear(),
                Stage::Poly(stage) => stage.clear(),
                Stage::Hold(stage) => stage.held.clear(),
            }
        }
        self.flush = None;
    }
}

impl<T: Real> Stage<T> {
    fn push(&mut self, input: &[T], out: &mut Vec<T>) {
        match self {
            Self::Fft(stage) => stage.push(input, out),
            Self::Poly(stage) => stage.push(input, out),
            Self::Hold(stage) => stage.push(input, out),
        }
    }

    /// Input samples after which nothing fed before reaches the output
    fn memory(&self) -> usize {
        match self {
            Self::Fft(stage) => stage.memory(),
            Self::Poly(stage) => stage.memory(),
            Self::Hold(stage) => stage.len,
        }
    }
}
