use crate::block_convolver::BlockConvolver;
use crate::filter_bank;
use crate::fir_filter::{self, FirFilter, Request};
use crate::half_band::{Downsampler, Upsampler};
use crate::interpolator::{Interpolator, whole_stepping};
use crate::processor::Processor;

const LEAST_TRANSITION_BAND: f64 = 0.5;
const MOST_TRANSITION_BAND: f64 = 45.0;
const LEAST_ATTENUATION: f64 = 49.0;
const MOST_ATTENUATION: f64 = 218.0;
/// How much wider, per percent of the transition band, the filter after the
/// interpolation may be, while the pass-band stays flat
const INTERMEDIATE_WIDENING: f64 = 0.0175;
const COMMON_RATIOS: [(usize, usize); 5] = [(1, 2), (1, 3), (2, 3), (3, 2), (3, 4)];
const DEFAULT_MAX_INPUT_LEN: usize = 1024;
/// Fewer make the bank's outer rows delay by a whole number of samples other
/// than 0 or 1, where the sinc filter divides by zero
const LEAST_FILTER_FRACTIONS: u32 = 5;
const LEAST_CACHE_LIMIT: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("sample rates must be positive and finite")]
    InvalidSampleRate,
    #[error("the transition band must be from 0.5 to 45 percent")]
    InvalidTransitionBand,
    #[error("the attenuation must be from 49 to 218 dB")]
    InvalidAttenuation,
    #[error("the maximum input length must be at least 1")]
    InvalidMaxInputLen,
    #[error("the number of filter fractions must be at least 5")]
    InvalidFilterFractions,
    #[error("a cache limit must be at least 2")]
    InvalidCacheLimit,
}

/// Of the steep filter. The interpolation and half-band filters are always
/// linear-phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Linear,
    /// Far less latency, for a phase that depends on frequency. As a filter
    /// this rarely comes to a whole number of samples, the output can also stay
    /// delayed by [`Resampler::latency_frac`].
    Minimum,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Options {
    /// In percent, from 0.5 to 45: the part of the input's band, or of the
    /// output's when downsampling, from the filter's -3 dB point to the Nyquist
    /// frequency. Less keeps more of the highs and takes more latency.
    pub transition_band: f64,
    /// In decibels, from 49 to 218: how far under the loudest input signal
    /// aliasing stays. The actual figure may be up to 4.46 dB higher.
    pub attenuation: f64,
    pub phase: Phase,
    /// The most input planned for one call of [`Resampler::process`], which the
    /// intermediate buffers are made for, and how much [`Resampler::oneshot`]
    /// passes on at a time. More in one call makes them grow. Larger helps
    /// downsampling, and smaller takes less memory when upsampling by a lot.
    pub max_input_len: usize,
    /// Doubles the FFT block of the steep filters with zeros, which usually
    /// runs faster and takes more latency. The output stays the same up to
    /// rounding.
    pub extended_fft: bool,
    /// Rates at a ratio of small whole numbers take a row of filter taps for
    /// each output position, which keeps time exactly. Without this they
    /// interpolate as any others do.
    pub whole_stepping: bool,
    /// Positions per sample of the filters that interpolation goes between, at
    /// least 5. `None` picks enough for [`Self::attenuation`]. Fewer interpolate
    /// worse, and more take more memory. A set number's filters are kept only
    /// while a resampler uses them. Rates that step wholly do not use these.
    pub filter_fractions: Option<u32>,
}

impl Options {
    pub const BITS_16: Self = Self::linear_phase(136.45);
    /// For impulse responses, which need less than dynamic signals
    pub const BITS_16_IMPULSE_RESPONSE: Self = Self::linear_phase(109.56);
    /// Also for 32-bit floating point
    pub const BITS_24: Self = Self::linear_phase(180.15);
    pub const BITS_27: Self = Self::linear_phase(206.91);

    const fn linear_phase(attenuation: f64) -> Self {
        Self {
            transition_band: 2.0,
            attenuation,
            phase: Phase::Linear,
            max_input_len: DEFAULT_MAX_INPUT_LEN,
            extended_fft: false,
            whole_stepping: true,
            filter_fractions: None,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::BITS_27
    }
}

/// How many items the caches shared by all resamplers keep once they are out of
/// use. Items in use stay past these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CacheLimits {
    /// Of steep filters, at least 2
    pub filters: usize,
    /// Of whole-step banks, at least 2. These are large, so a small number is
    /// best.
    pub filter_banks: usize,
}

impl CacheLimits {
    /// For all resamplers from now on.
    pub fn apply(self) -> Result<(), Error> {
        if self.filters < LEAST_CACHE_LIMIT || self.filter_banks < LEAST_CACHE_LIMIT {
            return Err(Error::InvalidCacheLimit);
        }
        fir_filter::set_cache_limit(self.filters);
        filter_bank::set_cache_limit(self.filter_banks);
        Ok(())
    }
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            filters: fir_filter::DEFAULT_CACHE_LIMIT,
            filter_banks: filter_bank::DEFAULT_CACHE_LIMIT,
        }
    }
}

/// Converts one channel. Output starts at the input's first sample, with the
/// latency removed, once enough input has come. Feed it until enough output
/// has come out, which may be nothing at first and then varies a little from
/// call to call.
pub struct Resampler {
    steps: Vec<Box<dyn Processor>>,
    latency_frac: f64,
    max_input_len: usize,
    max_output_len: usize,
    front: Vec<f64>,
    back: Vec<f64>,
}

impl Resampler {
    /// Only the ratio of the two rates matters. Rates at a power-of-two or
    /// three-times-that ratio take a much faster route, with no interpolation.
    pub fn new(source_rate: f64, target_rate: f64, options: Options) -> Result<Self, Error> {
        let is_valid_rate = |rate: f64| rate.is_finite() && rate > 0.0;
        if !is_valid_rate(source_rate) || !is_valid_rate(target_rate) {
            return Err(Error::InvalidSampleRate);
        }
        if !(LEAST_TRANSITION_BAND..=MOST_TRANSITION_BAND).contains(&options.transition_band) {
            return Err(Error::InvalidTransitionBand);
        }
        if !(LEAST_ATTENUATION..=MOST_ATTENUATION).contains(&options.attenuation) {
            return Err(Error::InvalidAttenuation);
        }
        if options.max_input_len == 0 {
            return Err(Error::InvalidMaxInputLen);
        }
        if options
            .filter_fractions
            .is_some_and(|fractions| fractions < LEAST_FILTER_FRACTIONS)
        {
            return Err(Error::InvalidFilterFractions);
        }

        let mut chain = Chain {
            steps: Vec::new(),
            latency_frac: 0.0,
            options,
        };
        chain.build(source_rate, target_rate);

        // Steps write to the two buffers in turn
        let mut capacities = [0, 0];
        let mut max_output_len = options.max_input_len;
        for (index, step) in chain.steps.iter().enumerate() {
            max_output_len = step.max_output_len(max_output_len);
            capacities[index % 2] = capacities[index % 2].max(max_output_len);
        }
        Ok(Self {
            steps: chain.steps,
            latency_frac: chain.latency_frac,
            max_input_len: options.max_input_len,
            max_output_len,
            front: Vec::with_capacity(capacities[1]),
            back: Vec::with_capacity(capacities[0]),
        })
    }

    /// The output this much input brings, which a next call replaces. It is
    /// `input` itself if the rates are equal. Any amount may be given at once,
    /// and the resampler keeps what it needs from it.
    pub fn process<'a>(&'a mut self, input: &'a [f64]) -> &'a [f64] {
        if self.steps.is_empty() {
            return input;
        }
        let mut front = &mut self.front;
        let mut back = &mut self.back;
        let mut range = 0..0;
        for (index, step) in self.steps.iter_mut().enumerate() {
            let source = if index == 0 {
                input
            } else {
                &front[range.clone()]
            };
            back.clear();
            let start = step.process(source, back);
            range = start..back.len();
            std::mem::swap(&mut front, &mut back);
        }
        &front[range]
    }

    /// Back to the state after creation, faster than creating another: all
    /// input so far is gone.
    pub fn clear(&mut self) {
        for step in &mut self.steps {
            step.clear();
        }
    }

    /// The most output that a call with up to [`Options::max_input_len`] samples
    /// brings.
    pub fn max_output_len(&self) -> usize {
        self.max_output_len
    }

    /// In samples of the output, from 0 up to 1. Always 0 unless [`Phase::Minimum`].
    pub fn latency_frac(&self) -> f64 {
        self.latency_frac
    }

    /// Of a cleared resampler, how much input it takes to reach the output
    /// sample `position`, not counting that sample's own input.
    pub fn input_before_output(&self, position: usize) -> usize {
        self.steps
            .iter()
            .rev()
            .fold(position as i64, |position, step| {
                step.input_before_output(position)
            }) as usize
    }

    /// Of a cleared resampler, how much input it takes to bring `samples` of
    /// output.
    pub fn input_required_for_output(&self, samples: usize) -> usize {
        match samples {
            0 => 0,
            _ => self.input_before_output(samples - 1) + 1,
        }
    }

    /// Converts `input` as a whole into `output`, however long that is, with
    /// silence after `input` if needed, and clears the resampler.
    pub fn oneshot(&mut self, input: &[f64], output: &mut [f64]) {
        let mut chunks = input.chunks(self.max_input_len);
        let silence = vec![0.0; self.max_input_len];
        let mut left = output;
        while !left.is_empty() {
            let chunk = chunks.next().unwrap_or(&silence[..]);
            let produced = self.process(chunk);
            let count = produced.len().min(left.len());
            let (filled, rest) = left.split_at_mut(count);
            filled.copy_from_slice(&produced[..count]);
            left = rest;
        }
        self.clear();
    }
}

/// Builds the steps, each taking the latency the previous one left
struct Chain {
    steps: Vec<Box<dyn Processor>>,
    latency_frac: f64,
    options: Options,
}

impl Chain {
    fn build(&mut self, source: f64, target: f64) {
        if source == target
            || self.common_ratio(source, target)
            || self.whole_upsampling(source, target)
        {
            return;
        }
        if target * 2.0 > source {
            self.upsampling_or_slight_downsampling(source, target);
        } else {
            self.downsampling(source, target);
        }
    }

    fn add(&mut self, step: impl Processor + 'static) {
        self.latency_frac = step.latency_frac();
        self.steps.push(Box::new(step));
    }

    fn add_convolver(
        &mut self,
        norm_freq: f64,
        transition_band: f64,
        gain: f64,
        up_factor: usize,
        down_factor: usize,
    ) {
        let filter = FirFilter::low_pass(Request {
            norm_freq,
            transition_band,
            attenuation: self.options.attenuation,
            phase: self.options.phase,
            gain,
            down_factor,
            extended_fft: self.options.extended_fft,
        });
        self.add(BlockConvolver::new(
            filter,
            up_factor,
            down_factor,
            self.latency_frac,
        ));
    }

    fn add_interpolator(&mut self, source: f64, target: f64, is_third: bool) {
        self.add(Interpolator::new(
            source,
            target,
            &self.options,
            is_third,
            self.latency_frac,
        ));
    }

    fn add_half_band_up(&mut self, steep_index: usize, is_third: bool) {
        self.add(Upsampler::new(
            self.options.attenuation,
            steep_index,
            is_third,
            self.latency_frac,
        ));
    }

    /// A ratio that a single step can take
    fn common_ratio(&mut self, source: f64, target: f64) -> bool {
        let found = COMMON_RATIOS
            .into_iter()
            .find(|(up, down)| source * *up as f64 == target * *down as f64);
        let Some((up, down)) = found else {
            return false;
        };
        let transition_band = self.options.transition_band;
        self.add_convolver(
            1.0 / up.max(down) as f64,
            transition_band,
            up as f64,
            up,
            down,
        );
        true
    }

    /// Up by a power of two or three times one
    fn whole_upsampling(&mut self, source: f64, target: f64) -> bool {
        for first_factor in 2..=3usize {
            let mut multiple = first_factor as f64;
            let mut doublings = 0;
            loop {
                let rate = source * multiple;
                if rate == target {
                    let transition_band = self.options.transition_band;
                    let factor = first_factor as f64;
                    self.add_convolver(1.0 / factor, transition_band, factor, first_factor, 1);
                    let is_third_first = first_factor == 3;
                    for doubling in 0..doublings {
                        self.add_half_band_up(doubling, is_third_first || doubling > 1);
                    }
                    return true;
                }
                if rate > target {
                    break;
                }
                multiple *= 2.0;
                doublings += 1;
            }
        }
        false
    }

    /// First to twice the source rate, or to a rate twice this that lies a
    /// power of two under the target if that leaves few enough taps
    fn upsampling_or_slight_downsampling(&mut self, source: f64, target: f64) {
        let norm_freq = if target > source {
            0.5
        } else {
            0.5 * target / source
        };
        self.add_convolver(norm_freq, self.options.transition_band, 2.0, 2, 1);

        // Keeps the later filter's band from getting steeper than asked,
        // which holds its latency down
        let threshold = source / (1.0 - INTERMEDIATE_WIDENING * self.options.transition_band);
        let mut doublings = 0;
        let mut divisor = 1.0;
        while target >= threshold * (divisor * 2.0) {
            divisor *= 2.0;
            doublings += 1;
        }

        let doubled_source = source * 2.0;
        if doublings == 1 && whole_stepping(doubled_source, target).is_some() {
            doublings = 0;
        }

        if doublings == 0 {
            self.add_interpolator(doubled_source, target, false);
            return;
        }

        self.add_interpolator(doubled_source * divisor, target, false);
        let transition_band =
            ((1.0 - source * divisor / target) / INTERMEDIATE_WIDENING).min(MOST_TRANSITION_BAND);
        self.add_convolver(0.5, transition_band, 2.0, 2, 1);
        for doubling in 1..doublings {
            self.add_half_band_up(doubling - 1, doubling > 2);
        }
    }

    /// Halves until under four times the target, then filters, and interpolates
    /// unless what is left is a whole 2 or 3
    fn downsampling(&mut self, source: f64, target: f64) {
        let mut check_rate = target * 4.0;
        let mut divisor = 1.0;
        let mut halvings = 0;
        while check_rate <= source {
            halvings += 1;
            check_rate *= 2.0;
            divisor *= 2.0;
        }

        let whole_factor = [2usize, 3]
            .into_iter()
            .find(|factor| target * divisor * *factor as f64 == source);
        let (norm_freq, down_factor, is_third_first) = match whole_factor {
            Some(factor) => (1.0 / factor as f64, factor, factor == 3),
            None => {
                let norm_freq = target * divisor / source;
                (norm_freq, 1, norm_freq * 3.0 <= 1.0)
            }
        };

        for halving in 0..halvings {
            // Very relaxed filters, only guaranteeing a stop-band from 0.75 of
            // the Nyquist frequency at the last one. What aliases from
            // 0.5-0.75 lands on 0.25-0.5, which the next filter takes out.
            self.add(Downsampler::new(
                self.options.attenuation,
                halvings - 1 - halving,
                is_third_first || halving + 2 < halvings,
                self.latency_frac,
            ));
        }

        self.add_convolver(
            norm_freq,
            self.options.transition_band,
            1.0 / divisor,
            1,
            down_factor,
        );
        if whole_factor.is_none() {
            self.add_interpolator(source, target * divisor, is_third_first);
        }
    }
}
