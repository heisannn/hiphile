use rubato::WindowFunction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResamplingWindow {
    Hann,
    Hann2,
    Blackman,
    Blackman2,
    BlackmanHarris,
    #[default]
    BlackmanHarris2,
}

impl From<ResamplingWindow> for WindowFunction {
    fn from(window: ResamplingWindow) -> Self {
        match window {
            ResamplingWindow::Hann => Self::Hann,
            ResamplingWindow::Hann2 => Self::Hann2,
            ResamplingWindow::Blackman => Self::Blackman,
            ResamplingWindow::Blackman2 => Self::Blackman2,
            ResamplingWindow::BlackmanHarris => Self::BlackmanHarris,
            ResamplingWindow::BlackmanHarris2 => Self::BlackmanHarris2,
        }
    }
}

/// The DSD bit rate as a multiple of 44.1 kHz or 48 kHz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DopRate {
    Dsd64,
    Dsd128,
    #[default]
    Dsd256,
}

impl std::fmt::Display for DopRate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Dsd64 => "DSD64",
            Self::Dsd128 => "DSD128",
            Self::Dsd256 => "DSD256",
        })
    }
}

impl DopRate {
    pub const ALL: [Self; 3] = [Self::Dsd64, Self::Dsd128, Self::Dsd256];

    /// The rate of the DoP frames, of 16 bits per channel, that fits a source
    /// at `source_rate`: `176_400` for DSD64 from 44.1 kHz.
    pub fn frame_rate_for(self, source_rate: u32) -> u32 {
        self.frame_rate(source_rate.is_multiple_of(11_025))
    }

    fn frame_rate(self, is_44_1_khz_family: bool) -> u32 {
        let base = if is_44_1_khz_family { 176_400 } else { 192_000 };
        match self {
            Self::Dsd64 => base,
            Self::Dsd128 => base * 2,
            Self::Dsd256 => base * 4,
        }
    }
}

/// How many taps the filter of the first oversampling stage has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SincLength {
    L256,
    L512,
    #[default]
    L1024,
    L2048,
}

impl SincLength {
    pub const ALL: [Self; 4] = [Self::L256, Self::L512, Self::L1024, Self::L2048];

    pub fn taps(self) -> usize {
        match self {
            Self::L256 => 256,
            Self::L512 => 512,
            Self::L1024 => 1024,
            Self::L2048 => 2048,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// Sinc interpolation, then a short FFT stage up to the DSD rate.
    Sinc {
        length: SincLength,
        window: ResamplingWindow,
        /// As a fraction of the source's Nyquist frequency. `None` picks the
        /// highest that the length and window leave free of aliasing.
        cutoff: Option<f32>,
    },
    /// One FFT stage from the source rate to the DSD rate.
    Fft {
        /// The least input frames one FFT takes.
        size: u32,
        window: ResamplingWindow,
    },
}

impl Default for Filter {
    fn default() -> Self {
        Self::Sinc {
            length: SincLength::default(),
            window: ResamplingWindow::default(),
            cutoff: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    Three,
    Five,
    #[default]
    Seven,
    Nine,
}

impl Order {
    pub const ALL: [Self; 4] = [Self::Three, Self::Five, Self::Seven, Self::Nine];

    pub(crate) fn count(self) -> usize {
        match self {
            Self::Three => 3,
            Self::Five => 5,
            Self::Seven => 7,
            Self::Nine => 9,
        }
    }
}

/// The band the modulator keeps its noise out of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoiseBand {
    #[default]
    To20Khz,
    /// Only worth it at DSD256: at DSD64 it costs about 50 dB below 20 kHz.
    To48Khz,
}

impl NoiseBand {
    pub(crate) fn hertz(self) -> f64 {
        match self {
            Self::To20Khz => 20_000.0,
            Self::To48Khz => 48_000.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modulation {
    pub(crate) order: Order,
    pub(crate) noise_gain: f64,
    pub(crate) noise_band: NoiseBand,
    pub(crate) optimized_zeros: bool,
    pub(crate) dither_db: Option<f64>,
    pub(crate) input_level_db: f64,
    pub(crate) limiter_ceiling_db: Option<f64>,
}

impl Default for Modulation {
    fn default() -> Self {
        Self {
            order: Order::default(),
            noise_gain: 1.4,
            noise_band: NoiseBand::default(),
            optimized_zeros: true,
            dither_db: None,
            input_level_db: 0.0,
            limiter_ceiling_db: Some(1.0),
        }
    }
}

impl Modulation {
    pub fn with_order(mut self, order: Order) -> Self {
        self.order = order;
        self
    }

    /// The gain of the noise transfer function out of band. Above about 1.45
    /// loud music makes the modulator unstable.
    pub fn with_noise_gain(mut self, noise_gain: f64) -> Self {
        self.noise_gain = noise_gain;
        self
    }

    pub fn with_noise_band(mut self, noise_band: NoiseBand) -> Self {
        self.noise_band = noise_band;
        self
    }

    /// Spreads the zeros of the noise transfer function over the noise band
    /// rather than putting them all at 0 Hz.
    pub fn with_optimized_zeros(mut self, optimized_zeros: bool) -> Self {
        self.optimized_zeros = optimized_zeros;
        self
    }

    /// Triangular dither at the quantizer, peaking at this level against its
    /// output of ±1.
    pub fn with_dither_db(mut self, dither_db: Option<f64>) -> Self {
        self.dither_db = dither_db;
        self
    }

    pub fn with_input_level_db(mut self, input_level_db: f64) -> Self {
        self.input_level_db = input_level_db;
        self
    }

    /// The peak the limiter lets through, in dB against full scale PCM after
    /// oversampling. `None` turns it off.
    pub fn with_limiter_ceiling_db(mut self, limiter_ceiling_db: Option<f64>) -> Self {
        self.limiter_ceiling_db = limiter_ceiling_db;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    pub(crate) rate: DopRate,
    pub(crate) filter: Filter,
    pub(crate) modulation: Modulation,
    matches_source_family: bool,
}

impl Settings {
    pub fn new(rate: DopRate, filter: Filter, modulation: Modulation) -> Self {
        Self {
            rate,
            filter,
            modulation,
            matches_source_family: false,
        }
    }

    /// Runs sources of the 48 kHz family at the 48 kHz family's DSD rates,
    /// rather than at the standard ones, which all come from 44.1 kHz. Some
    /// DACs take DoP at those rates for PCM, which is loud noise.
    pub fn with_matching_source_family(mut self, matches_source_family: bool) -> Self {
        self.matches_source_family = matches_source_family;
        self
    }

    /// The rates of the DoP frames for a source at `source_rate`, best first.
    /// The second is the other family, for a device that lacks the first.
    pub fn frame_rates_for(&self, source_rate: u32) -> [u32; 2] {
        let is_44_1_khz_family = !self.matches_source_family || source_rate.is_multiple_of(11_025);
        [
            self.rate.frame_rate(is_44_1_khz_family),
            self.rate.frame_rate(!is_44_1_khz_family),
        ]
    }

    pub fn rate(&self) -> DopRate {
        self.rate
    }
}
