use crate::Error;
use crate::bits;

/// Past the four limits of the band, how far a value may lie, relative to the
/// limit, and still count as on it
const BAND_TOLERANCE: f64 = 1e-5;
/// The same for the limit on imaging, relative to `1 - passband_end`
const IMAGING_TOLERANCE: f64 = 5e-5;
const LEAST_TRANSITION_BANDWIDTH: f64 = 0.002;
const MOST_TRANSITION_BANDWIDTH: f64 = 0.5;
const LEAST_PASSBAND_END: f64 = 0.5;
const MOST_STOPBAND_BEGIN: f64 = 1.5;
const LEAST_PRECISION: f64 = 15.0;
const MOST_PRECISION: f64 = 33.0;
/// Over this, `passband_end` and `stopband_begin` are percentages of the API
/// before libsoxr 0.1.1. Valid fractions are at most 1.5, and valid
/// percentages more than 50.
const PERCENT_THRESHOLD: f64 = 2.0;
/// Up to this precision without [`QualitySpec::double_precision`], the
/// resampler computes in 32-bit floating point
const MOST_SINGLE_PRECISION: f64 = 20.0;

/// The presets of the quality numbers of libsoxr's recipes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    /// Cubic interpolation with no band-limiting filter, "QQ"
    Quick,
    /// 16-bit with a narrow pass-band, "LQ"
    Low,
    /// 16-bit, "MQ"
    Medium,
    Bits16,
    /// 20-bit, "HQ"
    #[default]
    High,
    Bits24,
    /// 28-bit, "VHQ"
    VeryHigh,
    Bits32,
    /// What libsoxr's libsamplerate layer calls "Best sinc"
    LsrBest,
    /// What libsoxr's libsamplerate layer calls "Medium sinc"
    LsrMedium,
    /// What libsoxr's libsamplerate layer calls "Fast sinc"
    LsrFast,
}

/// The phase presets of libsoxr's recipes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Phase {
    #[default]
    Linear,
    /// Between minimum and linear, at 25
    Intermediate,
    Maximum,
    Minimum,
}

impl Phase {
    /// As [`QualitySpec::phase_response`]
    pub fn response(self) -> f64 {
        match self {
            Self::Linear => 50.0,
            Self::Intermediate => 25.0,
            Self::Maximum => 100.0,
            Self::Minimum => 0.0,
        }
    }
}

/// What a libsoxr recipe chooses: the values of a [`QualitySpec`]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Recipe {
    pub quality: Quality,
    pub phase: Phase,
    /// Widens the pass-band, which makes the filters longer
    pub steep: bool,
}

impl Recipe {
    pub const fn new(quality: Quality) -> Self {
        Self {
            quality,
            phase: Phase::Linear,
            steep: false,
        }
    }

    pub const fn with_phase(self, phase: Phase) -> Self {
        Self { phase, ..self }
    }

    pub const fn steep(self, steep: bool) -> Self {
        Self { steep, ..self }
    }

    /// From libsoxr's numeric recipe, where quality numbers 11 and 12 mean
    /// [`Quality::Quick`], 13 to 15 [`Quality::VeryHigh`], and the bits over
    /// those of [`bits`] are ignored.
    pub const fn from_bits(recipe: u32) -> Self {
        let quality = match recipe & 0xf {
            0 | 11 | 12 => Quality::Quick,
            1 => Quality::Low,
            2 => Quality::Medium,
            3 => Quality::Bits16,
            4 => Quality::High,
            5 => Quality::Bits24,
            7 => Quality::Bits32,
            8 => Quality::LsrBest,
            9 => Quality::LsrMedium,
            10 => Quality::LsrFast,
            _ => Quality::VeryHigh,
        };
        let phase = match recipe & 0x30 {
            bits::LINEAR_PHASE => Phase::Linear,
            bits::INTERMEDIATE_PHASE => Phase::Intermediate,
            bits::MAXIMUM_PHASE => Phase::Maximum,
            _ => Phase::Minimum,
        };
        Self {
            quality,
            phase,
            steep: recipe & bits::STEEP_FILTER != 0,
        }
    }
}

impl From<Quality> for Recipe {
    fn from(quality: Quality) -> Self {
        Self::new(quality)
    }
}

/// How far the gain may fall at [`QualitySpec::passband_end`]. This resampler
/// keeps the whole pass-band flat to the precision whichever is chosen, so it
/// meets each of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rolloff {
    /// At most 0.01 dB
    #[default]
    Small,
    /// At most 0.35 dB
    Medium,
    /// No margin is taken
    None,
}

/// What a resampler computes with
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    /// A constant ratio in 32-bit floating point
    Single,
    /// A constant ratio in 64-bit floating point
    Double,
    /// A ratio that may change
    Variable,
}

impl Engine {
    /// libsoxr's name for the engine of the same kind
    pub fn name(self) -> &'static str {
        match self {
            Self::Single => "cr32",
            Self::Double => "cr64",
            Self::Variable => "vr32",
        }
    }
}

/// How a resampler filters. Frequencies are fractions of the lower of the two
/// Nyquist frequencies.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct QualitySpec {
    /// In bits. 0 is cubic interpolation with no band-limiting filter.
    /// Otherwise from 15 to 33, not only whole numbers: the stop-band is
    /// `(precision + 1) * 6.02` dB down.
    pub precision: f64,
    /// 0 for minimum phase, 50 linear and 100 maximum, with intermediate
    /// phases between
    pub phase_response: f64,
    /// Where the pass-band ends. Over 2 it is a percentage: 91.3 is 0.913.
    pub passband_end: f64,
    /// Where the stop-band begins. 1 lets nothing alias or image at the
    /// Nyquist frequency or above, and more lets some through above it. Over
    /// 2 it is a percentage of the Nyquist frequency down from twice it, as in
    /// libsoxr before 0.1.1: 95 is 1.05.
    pub stopband_begin: f64,
    pub rolloff: Rolloff,
    /// Kept for libsoxr's flag. The clock is always this precise.
    pub high_precision_clock: bool,
    /// Computes in 64-bit floating point even at a precision of 20 or less
    pub double_precision: bool,
    /// For [`crate::Resampler::set_io_ratio`] to change the ratio. The other
    /// values then have no effect and are not checked, as in libsoxr.
    pub variable_rate: bool,
    /// Keeps the ratio across [`crate::Resampler::clear`] when it is not
    /// variable. Without it, as with the LSR presets, a clear lets
    /// [`crate::Resampler::set_io_ratio`] set a new one.
    pub keep_ratio_on_clear: bool,
}

impl QualitySpec {
    pub fn new(recipe: impl Into<Recipe>) -> Self {
        let recipe = recipe.into();
        let preset = Preset::of(recipe.quality);
        let passband_end = if recipe.steep {
            preset.steep_passband_end
        } else {
            preset.passband_end
        };
        Self {
            precision: preset.precision,
            phase_response: recipe.phase.response(),
            passband_end,
            stopband_begin: 1.0,
            rolloff: preset.rolloff,
            high_precision_clock: false,
            double_precision: false,
            variable_rate: false,
            keep_ratio_on_clear: !matches!(
                recipe.quality,
                Quality::LsrBest | Quality::LsrMedium | Quality::LsrFast
            ),
        }
    }

    /// As libsoxr's `soxr_quality_spec(recipe, flags)`: the flags of [`bits`]
    /// are added to the recipe's. Rolloff bits of 3 are [`Rolloff::None`].
    pub fn from_bits(recipe: u32, flags: u32) -> Self {
        let recipe = Recipe::from_bits(recipe);
        let mut spec = Self::new(recipe);
        let preset_rolloff = match spec.rolloff {
            Rolloff::Small => bits::ROLLOFF_SMALL,
            Rolloff::Medium => bits::ROLLOFF_MEDIUM,
            Rolloff::None => bits::ROLLOFF_NONE,
        };
        spec.rolloff = match (preset_rolloff | flags) & 3 {
            bits::ROLLOFF_SMALL => Rolloff::Small,
            bits::ROLLOFF_MEDIUM => Rolloff::Medium,
            _ => Rolloff::None,
        };
        spec.high_precision_clock = flags & bits::HI_PREC_CLOCK != 0;
        spec.double_precision = flags & bits::DOUBLE_PRECISION != 0;
        spec.variable_rate = flags & bits::VR != 0;
        spec.keep_ratio_on_clear |= flags & bits::KEEP_RATIO_ON_CLEAR != 0;
        spec
    }

    pub fn engine(&self) -> Engine {
        if self.variable_rate {
            Engine::Variable
        } else if self.precision <= MOST_SINGLE_PRECISION && !self.double_precision {
            Engine::Single
        } else {
            Engine::Double
        }
    }

    /// The band as fractions, with percentages converted
    pub fn band(&self) -> (f64, f64) {
        let passband_end = if self.passband_end > PERCENT_THRESHOLD {
            self.passband_end / 100.0
        } else {
            self.passband_end
        };
        let stopband_begin = if self.stopband_begin > PERCENT_THRESHOLD {
            2.0 - self.stopband_begin / 100.0
        } else {
            self.stopband_begin
        };
        (passband_end, stopband_begin)
    }

    /// Checks in libsoxr's order and returns the first error it would.
    /// Values on a limit pass.
    pub fn validate(&self, upsampling: bool) -> Result<(), Error> {
        if self.variable_rate {
            return Ok(());
        }
        let (passband_end, stopband_begin) = self.band();
        let imaging = (stopband_begin - 1.0) - (1.0 - passband_end);
        if upsampling && imaging > IMAGING_TOLERANCE * (1.0 - passband_end).max(0.0) {
            return Err(Error::ImagingGreaterThanRolloff);
        }
        let bandwidth = stopband_begin - passband_end;
        let least_bandwidth = LEAST_TRANSITION_BANDWIDTH * (1.0 - BAND_TOLERANCE);
        let most_bandwidth = MOST_TRANSITION_BANDWIDTH * (1.0 + BAND_TOLERANCE);
        if !(least_bandwidth..=most_bandwidth).contains(&bandwidth) {
            return Err(Error::TransitionBandwidth);
        }
        let least_passband_end = LEAST_PASSBAND_END * (1.0 - BAND_TOLERANCE);
        let most_stopband_begin = MOST_STOPBAND_BEGIN * (1.0 + BAND_TOLERANCE);
        if !(passband_end >= least_passband_end && stopband_begin <= most_stopband_begin) {
            return Err(Error::TransitionBand);
        }
        if self.precision != 0.0 && !(LEAST_PRECISION..=MOST_PRECISION).contains(&self.precision) {
            return Err(Error::Precision);
        }
        if !(0.0..=100.0).contains(&self.phase_response) {
            return Err(Error::PhaseResponse);
        }
        Ok(())
    }
}

impl Default for QualitySpec {
    fn default() -> Self {
        Self::new(Quality::High)
    }
}

impl From<Quality> for QualitySpec {
    fn from(quality: Quality) -> Self {
        Self::new(quality)
    }
}

impl From<Recipe> for QualitySpec {
    fn from(recipe: Recipe) -> Self {
        Self::new(recipe)
    }
}

/// libsoxr's values, as `soxr_quality_spec` returns them
struct Preset {
    precision: f64,
    passband_end: f64,
    /// With [`Recipe::steep`]
    steep_passband_end: f64,
    rolloff: Rolloff,
}

impl Preset {
    // The values are libsoxr's to the bit, as observed
    #[allow(clippy::excessive_precision)]
    const fn of(quality: Quality) -> Self {
        let (precision, passband_end, steep_passband_end, rolloff) = match quality {
            Quality::Quick => (
                0.0,
                0.93886648602569966,
                0.98777329720513996,
                Rolloff::Medium,
            ),
            Quality::Low => (16.0, 0.67626953125, 0.9831762098353134, Rolloff::Medium),
            Quality::Medium => (
                16.0,
                0.9158810491765671,
                0.9831762098353134,
                Rolloff::Medium,
            ),
            Quality::Bits16 => (16.0, 0.9158810491765671, 0.9831762098353134, Rolloff::Small),
            Quality::High => (
                20.0,
                0.91374365327330664,
                0.98274873065466128,
                Rolloff::Small,
            ),
            Quality::Bits24 => (
                24.0,
                0.91232195381988845,
                0.98246439076397774,
                Rolloff::Small,
            ),
            Quality::VeryHigh => (
                28.0,
                0.91132831989516627,
                0.98226566397903325,
                Rolloff::Small,
            ),
            Quality::Bits32 => (
                32.0,
                0.91050184934434353,
                0.98210036986886873,
                Rolloff::Small,
            ),
            Quality::LsrBest => (23.0, 0.931_f32 as f64, 0.9825253148143992, Rolloff::Small),
            Quality::LsrMedium => (19.0, 0.832_f32 as f64, 0.98283977491453212, Rolloff::Small),
            Quality::LsrFast => (15.0, 0.663_f32 as f64, 0.98331294043370743, Rolloff::None),
        };
        Self {
            precision,
            passband_end,
            steep_passband_end,
            rolloff,
        }
    }
}
